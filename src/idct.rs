//! The 8×8 inverse DCT (H.262 Annex A) and the encoder's forward DCT.
//!
//! The decoder's IDCT is a separable fixed-point matrix product: the basis
//! `C(u)/2 · cos((2x+1)uπ/16)` scaled by 2^20 and rounded, a row pass that
//! keeps twelve fraction bits, a column pass, rounding (ties away from zero)
//! and saturation to [−256, 255]. It is checked against the real-number IDCT by the procedure
//! of IEEE Std 1180-1990, which Annex A requires, and by Annex A's own
//! items 3 and 4 (the tests at the end of this file).

use std::sync::OnceLock;

/// Fraction bits of the fixed-point basis.
const KBITS: u32 = 20;
/// Fraction bits kept between the row and the column pass.
const MID_BITS: u32 = 12;
const ROW_SHIFT: u32 = KBITS - MID_BITS;
const COL_SHIFT: u32 = KBITS + MID_BITS;

/// `(x + 2^(s-1)) >> s` with ties away from zero, like Annex A's round().
#[inline]
fn round_shift(x: i64, s: u32) -> i64 {
    let half = 1i64 << (s - 1);
    if x >= 0 { (x + half) >> s } else { -((-x + half) >> s) }
}

/// `basis()[u][x]` = C(u)/2 · cos((2x+1)uπ/16), C(0) = 1/√2, else 1.
fn basis() -> &'static [[f64; 8]; 8] {
    static B: OnceLock<[[f64; 8]; 8]> = OnceLock::new();
    B.get_or_init(|| {
        let mut b = [[0.0; 8]; 8];
        for (u, row) in b.iter_mut().enumerate() {
            let c = if u == 0 { std::f64::consts::FRAC_1_SQRT_2 } else { 1.0 };
            for (x, v) in row.iter_mut().enumerate() {
                *v = c / 2.0
                    * ((2 * x + 1) as f64 * u as f64 * std::f64::consts::PI / 16.0).cos();
            }
        }
        b
    })
}

fn kbasis() -> &'static [[i64; 8]; 8] {
    static K: OnceLock<[[i64; 8]; 8]> = OnceLock::new();
    K.get_or_init(|| {
        let b = basis();
        let mut k = [[0i64; 8]; 8];
        for u in 0..8 {
            for x in 0..8 {
                k[u][x] = (b[u][x] * f64::from(1u32 << KBITS)).round() as i64;
            }
        }
        k
    })
}

/// Inverse DCT of a block of coefficients `F[v][u]` (raster order, each in
/// [−2048, 2047]) in place, giving samples `f[y][x]` saturated to
/// [−256, 255].
///
/// The basis has 20 fraction bits and 12 are kept between the passes, so the
/// result differs from the real-number IDCT only where that lies within
/// about 10^-4 of a rounding boundary; a block with only a DC coefficient
/// is computed exactly (f = F[0][0] / 8).
pub(crate) fn idct(block: &mut [i32; 64]) {
    if block[1..].iter().all(|&c| c == 0) {
        // f(x, y) = F[0][0] · (1/√2)² / 4 = F[0][0] / 8 everywhere.
        let v = round_shift(i64::from(block[0]), 3).clamp(-256, 255) as i32;
        block.fill(v);
        return;
    }
    let k = kbasis();
    let mut tmp = [0i64; 64];
    // Rows: tmp[v][x] = Σ_u F[v][u] · K[u][x].
    for v in 0..8 {
        let row = &block[v * 8..v * 8 + 8];
        if row.iter().all(|&c| c == 0) {
            continue;
        }
        for x in 0..8 {
            let mut acc: i64 = 0;
            for u in 0..8 {
                acc += i64::from(row[u]) * k[u][x];
            }
            tmp[v * 8 + x] = round_shift(acc, ROW_SHIFT);
        }
    }
    // Columns: f[y][x] = Σ_v tmp[v][x] · K[v][y].
    for x in 0..8 {
        for y in 0..8 {
            let mut acc: i64 = 0;
            for v in 0..8 {
                acc += tmp[v * 8 + x] * k[v][y];
            }
            block[y * 8 + x] = round_shift(acc, COL_SHIFT).clamp(-256, 255) as i32;
        }
    }
}

/// The real-number forward DCT (Annex A's definition), samples to
/// coefficients, for the encoder and the accuracy tests.
pub(crate) fn fdct_f64(input: &[f64; 64]) -> [f64; 64] {
    let b = basis();
    let mut tmp = [0.0; 64];
    // tmp[y][u] = Σ_x f[y][x] · B[u][x]
    for y in 0..8 {
        for u in 0..8 {
            tmp[y * 8 + u] = (0..8).map(|x| input[y * 8 + x] * b[u][x]).sum();
        }
    }
    let mut out = [0.0; 64];
    for u in 0..8 {
        for v in 0..8 {
            out[v * 8 + u] = (0..8).map(|y| tmp[y * 8 + u] * b[v][y]).sum();
        }
    }
    out
}

/// The real-number inverse DCT (Annex A's definition).
#[cfg(test)]
pub(crate) fn idct_f64(input: &[f64; 64]) -> [f64; 64] {
    let b = basis();
    let mut tmp = [0.0; 64];
    // tmp[v][x] = Σ_u F[v][u] · B[u][x]
    for v in 0..8 {
        for x in 0..8 {
            tmp[v * 8 + x] = (0..8).map(|u| input[v * 8 + u] * b[u][x]).sum();
        }
    }
    let mut out = [0.0; 64];
    for x in 0..8 {
        for y in 0..8 {
            out[y * 8 + x] = (0..8).map(|v| tmp[v * 8 + x] * b[v][y]).sum();
        }
    }
    out
}

/// The forward DCT the encoder uses: real-number, rounded to integers.
pub(crate) fn fdct(input: &[i32; 64]) -> [i32; 64] {
    let mut f = [0.0; 64];
    for (d, &s) in f.iter_mut().zip(input) {
        *d = f64::from(s);
    }
    let c = fdct_f64(&f);
    let mut out = [0; 64];
    for (d, s) in out.iter_mut().zip(c) {
        *d = s.round() as i32;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round to nearest, half away from zero (Annex A's round()).
    fn round_away(x: f64) -> i64 {
        x.round() as i64 // f64::round rounds half away from zero
    }

    /// IEEE Std 1180-1990's random number generator (its §3.2), with the
    /// 32-bit `long` arithmetic of the original.
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
            let coeffs = fdct_f64(&samples);
            let mut ci = [0i32; 64];
            let mut cf = [0.0f64; 64];
            for i in 0..64 {
                let c = round_away(coeffs[i]).clamp(-2048, 2047);
                ci[i] = c as i32;
                cf[i] = c as f64;
            }
            let reference = idct_f64(&cf);
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
                    "IEEE 1180 L={l} H={h} sign={sign:+}: peak {} pmse {:.4} omse {:.5} pme {:.4} ome {:.5}",
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
            let r = idct_f64(&f);
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
        while checked < 20_000 {
            let mut s = [0.0f64; 64];
            for v in s.iter_mut() {
                *v = rng.next(384, 383) as f64;
            }
            let c = fdct_f64(&s);
            let mut ci = [0i32; 64];
            let mut cf = [0.0; 64];
            for i in 0..64 {
                ci[i] = (c[i].round() as i32).clamp(-2048, 2047);
                cf[i] = f64::from(ci[i]);
            }
            let r = idct_f64(&cf);
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
