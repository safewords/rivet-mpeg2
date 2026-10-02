//! The 8×8 inverse DCT (H.262 Annex A) and the encoder's forward DCT.
//!
//! The decoder's IDCT is the separable real-number IDCT in double
//! precision — rows, then columns, each a plain sum over the basis
//! `C(u)/2 · cos((2x+1)uπ/16)` — rounded (ties away from zero) and
//! saturated to [−256, 255]: Annex A's saturated mathematical
//! integer-number IDCT, up to double rounding. The basis is written out as
//! literal constants rather than computed with the platform's `cos`, so the
//! output is the same on every platform (Rust does not fuse or reorder
//! floating-point operations).
//!
//! It is checked against an independent, non-separable evaluation of the
//! definition by the procedure of IEEE Std 1180-1990, which Annex A
//! requires, and by Annex A's own items 3 and 4 (the tests at the end of
//! this file). The conformance suite's traces, made by decoders with a
//! double-precision IDCT, are matched sample for sample.

/// `BASIS[u][x]` = C(u)/2 · cos((2x+1)uπ/16), C(0) = 1/√2, else 1.
const BASIS: [[f64; 8]; 8] = [
    [
        0.35355339059327373,
        0.35355339059327373,
        0.35355339059327373,
        0.35355339059327373,
        0.35355339059327373,
        0.35355339059327373,
        0.35355339059327373,
        0.35355339059327373,
    ],
    [
        0.4903926402016152,
        0.4157348061512726,
        0.27778511650980114,
        0.09754516100806417,
        -0.0975451610080641,
        -0.277785116509801,
        -0.4157348061512727,
        -0.4903926402016152,
    ],
    [
        0.46193976625564337,
        0.19134171618254492,
        -0.19134171618254486,
        -0.46193976625564337,
        -0.4619397662556434,
        -0.19134171618254517,
        0.191341716182545,
        0.46193976625564326,
    ],
    [
        0.4157348061512726,
        -0.0975451610080641,
        -0.4903926402016152,
        -0.2777851165098011,
        0.2777851165098009,
        0.4903926402016152,
        0.09754516100806439,
        -0.41573480615127256,
    ],
    [
        0.3535533905932738,
        -0.35355339059327373,
        -0.35355339059327384,
        0.3535533905932737,
        0.35355339059327384,
        -0.35355339059327334,
        -0.35355339059327356,
        0.3535533905932733,
    ],
    [
        0.27778511650980114,
        -0.4903926402016152,
        0.09754516100806415,
        0.4157348061512728,
        -0.41573480615127256,
        -0.09754516100806401,
        0.4903926402016153,
        -0.27778511650980076,
    ],
    [
        0.19134171618254492,
        -0.4619397662556434,
        0.46193976625564326,
        -0.19134171618254495,
        -0.19134171618254528,
        0.46193976625564337,
        -0.4619397662556432,
        0.19134171618254478,
    ],
    [
        0.09754516100806417,
        -0.2777851165098011,
        0.4157348061512728,
        -0.4903926402016153,
        0.49039264020161527,
        -0.4157348061512725,
        0.27778511650980076,
        -0.09754516100806429,
    ],
];

/// Inverse DCT of a block of coefficients `F[v][u]` (raster order, each in
/// [−2048, 2047]) in place, giving samples `f[y][x]` saturated to
/// [−256, 255].
pub(crate) fn idct(block: &mut [i32; 64]) {
    if block[1..].iter().all(|&c| c == 0) {
        // f(x, y) = F[0][0] · (1/√2)² / 4 = F[0][0] / 8 everywhere, exactly.
        let v = (f64::from(block[0]) / 8.0).round().clamp(-256.0, 255.0) as i32;
        block.fill(v);
        return;
    }
    let mut tmp = [0.0f64; 64];
    // Rows: tmp[v][x] = Σ_u F[v][u] · B[u][x].
    for v in 0..8 {
        let row = &block[v * 8..v * 8 + 8];
        if row.iter().all(|&c| c == 0) {
            continue;
        }
        for x in 0..8 {
            let mut acc = 0.0;
            for u in 0..8 {
                acc += f64::from(row[u]) * BASIS[u][x];
            }
            tmp[v * 8 + x] = acc;
        }
    }
    // Columns: f[y][x] = Σ_v tmp[v][x] · B[v][y].
    for x in 0..8 {
        for y in 0..8 {
            let mut acc = 0.0;
            for v in 0..8 {
                acc += tmp[v * 8 + x] * BASIS[v][y];
            }
            // f64::round rounds half away from zero, as Annex A's round().
            block[y * 8 + x] = acc.round().clamp(-256.0, 255.0) as i32;
        }
    }
}

/// The forward DCT the encoder uses: separable, double precision, rounded
/// to integers.
pub(crate) fn fdct(input: &[i32; 64]) -> [i32; 64] {
    let mut tmp = [0.0f64; 64];
    // tmp[y][u] = Σ_x f[y][x] · B[u][x]
    for y in 0..8 {
        for u in 0..8 {
            let mut acc = 0.0;
            for x in 0..8 {
                acc += f64::from(input[y * 8 + x]) * BASIS[u][x];
            }
            tmp[y * 8 + u] = acc;
        }
    }
    let mut out = [0; 64];
    for u in 0..8 {
        for v in 0..8 {
            let mut acc = 0.0;
            for y in 0..8 {
                acc += tmp[y * 8 + u] * BASIS[v][y];
            }
            out[v * 8 + u] = acc.round() as i32;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::{FRAC_1_SQRT_2, PI};

    fn c(u: usize) -> f64 {
        if u == 0 { FRAC_1_SQRT_2 } else { 1.0 }
    }

    /// `COS[k][u]` = cos((2k+1)uπ/16), from the platform's cos (the
    /// reference is independent of the literal basis).
    fn cos_table() -> &'static [[f64; 8]; 8] {
        static T: std::sync::OnceLock<[[f64; 8]; 8]> = std::sync::OnceLock::new();
        T.get_or_init(|| {
            let mut t = [[0.0; 8]; 8];
            for (k, row) in t.iter_mut().enumerate() {
                for (u, v) in row.iter_mut().enumerate() {
                    *v = ((2 * k + 1) as f64 * u as f64 * PI / 16.0).cos();
                }
            }
            t
        })
    }

    /// Annex A's forward DCT, evaluated directly from its definition (a
    /// double sum per coefficient — not the separable code above).
    fn reference_fdct(f: &[f64; 64]) -> [f64; 64] {
        let mut out = [0.0; 64];
        for v in 0..8 {
            for u in 0..8 {
                let cs = cos_table();
                let mut s = 0.0;
                for y in 0..8 {
                    for x in 0..8 {
                        s += f[y * 8 + x] * cs[x][u] * cs[y][v];
                    }
                }
                out[v * 8 + u] = 0.25 * c(u) * c(v) * s;
            }
        }
        out
    }

    /// Annex A's real-number IDCT, evaluated directly from its definition.
    fn reference_idct(f: &[f64; 64]) -> [f64; 64] {
        let mut out = [0.0; 64];
        for y in 0..8 {
            for x in 0..8 {
                let cs = cos_table();
                let mut s = 0.0;
                for v in 0..8 {
                    for u in 0..8 {
                        s += c(u) * c(v) * f[v * 8 + u] * cs[x][u] * cs[y][v];
                    }
                }
                out[y * 8 + x] = 0.25 * s;
            }
        }
        out
    }

    /// Annex A's round(): to nearest, half away from zero.
    fn round_away(x: f64) -> i64 {
        x.round() as i64
    }

    #[test]
    fn basis_constants_are_the_definition() {
        for (u, row) in BASIS.iter().enumerate() {
            for (x, &b) in row.iter().enumerate() {
                let want = c(u) / 2.0 * ((2 * x + 1) as f64 * u as f64 * PI / 16.0).cos();
                assert!((b - want).abs() < 1e-15, "B[{u}][{x}]");
            }
        }
    }

    /// IEEE Std 1180-1990's random number generator, with the 32-bit `long`
    /// arithmetic of its listing.
    struct Ieee1180Rand(u32);
    impl Ieee1180Rand {
        fn next(&mut self, l: i64, h: i64) -> i64 {
            self.0 = self.0.wrapping_mul(1_103_515_245).wrapping_add(12345);
            let i = self.0 & 0x7fff_fffe;
            let x = f64::from(i) / f64::from(0x7fff_ffffu32) * (l + h + 1) as f64;
            x as i64 - l
        }
    }

    struct Stats {
        peak: i64,
        pmse: f64,
        omse: f64,
        pme: f64,
        ome: f64,
    }

    /// One IEEE 1180 run: 10 000 blocks of random samples in [−l, h] (negated
    /// when `sign` is −1), forward-transformed in double precision, rounded
    /// and clipped to [−2048, 2047]; the reference IDCT's output (rounded,
    /// clipped to [−256, 255]) compared with ours.
    fn ieee1180_run(l: i64, h: i64, sign: i64) -> Stats {
        const BLOCKS: usize = 10_000;
        let mut rng = Ieee1180Rand(1);
        let mut err_sum = [0i64; 64];
        let mut err_sq = [0i64; 64];
        let mut peak = 0i64;
        for _ in 0..BLOCKS {
            let mut samples = [0.0f64; 64];
            for s in samples.iter_mut() {
                *s = (rng.next(l, h) * sign) as f64;
            }
            let coeffs = reference_fdct(&samples);
            let mut ci = [0i32; 64];
            let mut cf = [0.0f64; 64];
            for i in 0..64 {
                let c = round_away(coeffs[i]).clamp(-2048, 2047);
                ci[i] = c as i32;
                cf[i] = c as f64;
            }
            let reference = reference_idct(&cf);
            let mut test = ci;
            idct(&mut test);
            for i in 0..64 {
                let r = round_away(reference[i]).clamp(-256, 255);
                let e = i64::from(test[i]) - r;
                peak = peak.max(e.abs());
                err_sum[i] += e;
                err_sq[i] += e * e;
            }
        }
        let n = BLOCKS as f64;
        Stats {
            peak,
            pmse: err_sq.iter().map(|&s| s as f64 / n).fold(0.0, f64::max),
            omse: err_sq.iter().sum::<i64>() as f64 / (n * 64.0),
            pme: err_sum.iter().map(|&s| (s as f64 / n).abs()).fold(0.0, f64::max),
            ome: (err_sum.iter().sum::<i64>() as f64 / (n * 64.0)).abs(),
        }
    }

    /// IEEE Std 1180-1990 §3 (H.262 Annex A item 2): every range and both
    /// signs must give peak error ≤ 1, per-sample mean square error ≤ 0.06,
    /// overall mean square error ≤ 0.02, per-sample mean error ≤ 0.015 and
    /// overall mean error ≤ 0.0015.
    #[test]
    fn ieee_1180_accuracy() {
        for (l, h) in [(256, 255), (5, 5), (300, 300)] {
            for sign in [1, -1] {
                let s = ieee1180_run(l, h, sign);
                eprintln!(
                    "IEEE 1180 L={l} H={h} sign={sign:+}: peak {} pmse {:.6} omse {:.6} pme {:.6} ome {:.6}",
                    s.peak, s.pmse, s.omse, s.pme, s.ome
                );
                assert!(s.peak <= 1, "peak error {}", s.peak);
                assert!(s.pmse <= 0.06, "pmse {}", s.pmse);
                assert!(s.omse <= 0.02, "omse {}", s.omse);
                assert!(s.pme <= 0.015, "pme {}", s.pme);
                assert!(s.ome <= 0.0015, "ome {}", s.ome);
            }
        }
    }

    /// IEEE 1180 §3.6: an all-zero block transforms to all zeros.
    #[test]
    fn zero_in_zero_out() {
        let mut b = [0i32; 64];
        idct(&mut b);
        assert!(b.iter().all(|&s| s == 0));
    }

    /// H.262 Annex A item 4: blocks with only F[0][0] = i − 2048 and
    /// F[7][7] = 1 when F[0][0] is even must be within 1 of the saturated
    /// reference everywhere.
    #[test]
    fn annex_a_item_4() {
        let mut worst = 0;
        for i in 0..4096i32 {
            let mut f = [0.0f64; 64];
            let mut b = [0i32; 64];
            b[0] = i - 2048;
            b[63] = i32::from(b[0] % 2 == 0);
            f[0] = f64::from(b[0]);
            f[63] = f64::from(b[63]);
            let r = reference_idct(&f);
            idct(&mut b);
            for k in 0..64 {
                let e = (i64::from(b[k]) - round_away(r[k]).clamp(-256, 255)).abs();
                worst = worst.max(e);
            }
        }
        eprintln!("Annex A item 4: worst error {worst}");
        assert!(worst <= 1);
    }

    /// H.262 Annex A item 3: when every f'(x, y) is in [−384, 383], outputs
    /// whose f' is above 256 must be 255, below −257 must be −256, and the
    /// rest within 2 of the saturated reference. Random blocks spanning that
    /// range.
    #[test]
    fn annex_a_item_3() {
        let mut rng = Ieee1180Rand(12345);
        let mut checked = 0;
        let mut worst = 0;
        while checked < 5_000 {
            let mut s = [0.0f64; 64];
            for v in s.iter_mut() {
                *v = rng.next(384, 383) as f64;
            }
            let c = reference_fdct(&s);
            let mut ci = [0i32; 64];
            let mut cf = [0.0; 64];
            for i in 0..64 {
                ci[i] = (round_away(c[i]) as i32).clamp(-2048, 2047);
                cf[i] = f64::from(ci[i]);
            }
            let r = reference_idct(&cf);
            let fp: Vec<i64> = r.iter().map(|&x| round_away(x)).collect();
            if fp.iter().any(|&v| !(-384..=383).contains(&v)) {
                continue;
            }
            checked += 1;
            idct(&mut ci);
            for k in 0..64 {
                if fp[k] > 256 {
                    assert_eq!(ci[k], 255);
                } else if fp[k] < -257 {
                    assert_eq!(ci[k], -256);
                } else {
                    let e = (i64::from(ci[k]) - fp[k].clamp(-256, 255)).abs();
                    worst = worst.max(e);
                }
            }
        }
        eprintln!("Annex A item 3: worst error {worst} over {checked} blocks");
        assert!(worst <= 2);
    }

    #[test]
    fn forward_then_inverse_is_close() {
        let mut s = [0i32; 64];
        for (i, v) in s.iter_mut().enumerate() {
            *v = ((i * 37) % 255) as i32 - 128;
        }
        let mut c = fdct(&s);
        idct(&mut c);
        for i in 0..64 {
            assert!((c[i] - s[i]).abs() <= 1);
        }
    }
}
