//! The sample-processing kernels — the separable 8×8 transforms, half-sample
//! motion compensation, adding a residual to a prediction, and the 16×16
//! SAD of the encoder's motion search — in a scalar version and in SIMD
//! versions chosen at run time.
//!
//! Every SIMD version returns exactly what the scalar version returns, for
//! every input: the integer kernels by construction, the transforms because
//! they perform the very same IEEE-754 double-precision operations (each
//! product rounded, then each sum, in the scalar code's order; no fused
//! multiply-add) — only several output samples at once. So the decoder's
//! output and the encoder's stream never depend on the processor. The tests
//! at the end of this file and in the per-architecture files check that
//! against the scalar code on random and extreme inputs.
//!
//! The rung is picked once: the widest the processor has (x86-64: SSE2,
//! which every x86-64 processor has, then AVX2, then AVX-512F for the
//! transforms; AArch64: NEON). `MPEG2_FORCE_SCALAR=1` in the environment
//! forces the scalar code (CI runs the test suite both ways).

use std::sync::OnceLock;

#[cfg(target_arch = "aarch64")]
mod neon;
#[cfg(target_arch = "x86_64")]
mod x86;

/// One prediction block's source: `w` × `h` samples (`w` 8 or 16) at
/// `off` in `src`, rows `stride` apart, interpolated half a sample to the
/// right (`hx`) and / or down (`hy`). The caller guarantees the block and
/// its extra half-sample column / row lie inside `src` (the kernels check,
/// and panic otherwise).
#[derive(Clone, Copy)]
pub(crate) struct McSrc<'a> {
    pub src: &'a [u8],
    pub off: usize,
    pub stride: usize,
    pub w: usize,
    pub h: usize,
    pub hx: bool,
    pub hy: bool,
}

impl McSrc<'_> {
    /// Panics unless every sample the block reads is inside `src`.
    #[inline]
    pub(crate) fn check(&self) {
        assert!(self.w == 8 || self.w == 16, "prediction width {}", self.w);
        assert!(self.h > 0);
        let last = self.off + (self.h - 1 + usize::from(self.hy)) * self.stride + self.w + usize::from(self.hx);
        assert!(last <= self.src.len(), "prediction block outside its plane");
    }
}

/// Panics unless an 8×8 block at `off`, rows `stride` apart, is inside a
/// buffer of `len` bytes.
#[inline]
pub(crate) fn check_block8(len: usize, off: usize, stride: usize) {
    assert!(off + 7 * stride + 8 <= len, "8x8 block outside its buffer");
}

/// Panics unless a 16×16 block at `off`, rows `stride` apart (0: one row
/// repeated), is inside a buffer of `len` bytes.
#[inline]
pub(crate) fn check_block16(len: usize, off: usize, stride: usize) {
    assert!(off + 15 * stride + 16 <= len, "16x16 block outside its buffer");
}

/// A 16×16 block: its first sample at `off` in `buf`, rows `stride` apart
/// (0: one row repeated).
#[derive(Clone, Copy)]
pub(crate) struct Blk<'a> {
    pub buf: &'a [u8],
    pub off: usize,
    pub stride: usize,
}

impl Blk<'_> {
    /// Panics unless the block is inside its buffer.
    #[inline]
    pub(crate) fn check16(&self) {
        check_block16(self.buf.len(), self.off, self.stride);
    }
}

/// The separable transform's signature: input, matrix, output, clamp range.
pub(crate) type TransformFn = fn(&[i32; 64], &[[f64; 8]; 8], &mut [i32; 64], i32, i32);

/// A set of kernels.
pub(crate) struct Dsp {
    /// The rung's name, for tests and benchmarks.
    pub name: &'static str,
    /// `out[y][x] = round(Σ_v M[v][y] · Σ_u in[v][u] · M[u][x])` evaluated as
    /// the separable double-precision sums of [`transform_scalar`], rounded
    /// half away from zero and clamped to [`lo`, `hi`].
    pub transform: TransformFn,
    /// Half-sample prediction of a block into `dst` at `off` (rows
    /// `stride` apart), replacing what is there or, with `avg`, averaged
    /// with it (`(d + p + 1) >> 1`).
    pub mc: fn(&McSrc, &mut [u8], usize, usize, bool),
    /// `dst = clamp(dst + residual, 0, 255)` over an 8×8 block.
    pub add_block: fn(&mut [u8], usize, usize, &[i32; 64]),
    /// `dst = clamp(residual, 0, 255)` over an 8×8 block.
    pub put_block: fn(&mut [u8], usize, usize, &[i32; 64]),
    /// Sum of absolute differences of two 16×16 blocks — or, if the sum
    /// over the first 8 rows alone reaches `limit`, that partial sum (the
    /// motion search's early exit: such a candidate cannot win). Every rung
    /// returns the same value.
    pub sad16: fn(Blk, Blk, u32) -> u32,
}

/// The scalar kernels.
pub(crate) static SCALAR: Dsp = Dsp {
    name: "scalar",
    transform: transform_scalar,
    mc: mc_scalar,
    add_block: add_block_scalar,
    put_block: put_block_scalar,
    sad16: sad16_scalar,
};

/// Every rung this processor runs, scalar first. The kernel tests compare
/// each with the scalar one.
pub(crate) fn rungs() -> Vec<&'static Dsp> {
    let mut v = vec![&SCALAR];
    #[cfg(target_arch = "x86_64")]
    v.extend(x86::rungs());
    #[cfg(target_arch = "aarch64")]
    v.extend(neon::rungs());
    v
}

/// The kernels in use: the processor's widest rung, or the scalar one under
/// `MPEG2_FORCE_SCALAR=1`.
#[inline]
pub(crate) fn dsp() -> &'static Dsp {
    static D: OnceLock<&'static Dsp> = OnceLock::new();
    D.get_or_init(|| {
        let force = std::env::var_os("MPEG2_FORCE_SCALAR").is_some_and(|v| v != "0" && !v.is_empty());
        if force { &SCALAR } else { rungs().pop().expect("the scalar rung") }
    })
}

/// The name of the rung [`dsp`] picked (for benchmarks and reports).
pub(crate) fn active_rung() -> &'static str {
    dsp().name
}

/// Rounds half away from zero, as `f64::round`, without a library call:
/// `x - trunc(x)` is exact for every finite `x`. `|x|` must be below 2³¹.
#[inline(always)]
pub(crate) fn round_away(x: f64) -> i32 {
    let t = x as i32;
    let d = x - f64::from(t);
    // Branch-free: which way a sample rounds is unpredictable.
    t + i32::from(d >= 0.5) - i32::from(d <= -0.5)
}

/// How a coefficient block's non-zero rows lie: one past the last of rows
/// 0–6 holding a non-zero coefficient, and whether row 7 holds one. Row 7
/// is apart because mismatch control (7.4.4) so often makes F[7][7] odd
/// in a block that is otherwise all low frequencies.
#[inline(always)]
pub(crate) fn live_extent(input: &[i32; 64]) -> (usize, bool) {
    let mut k = 0;
    for v in 0..7 {
        let any = input[v * 8..v * 8 + 8].iter().fold(0, |a, &c| a | c);
        if any != 0 {
            k = v + 1;
        }
    }
    (k, input[56..].iter().fold(0, |a, &c| a | c) != 0)
}

/// The separable transform: rows (`tmp[v][x] = Σ_u in[v][u]·M[u][x]`, in
/// order of `u`), then columns (`out[y][x] = Σ_v tmp[v][x]·M[v][y]`, in
/// order of `v`). A row of zero coefficients is skipped in both passes: its
/// row-pass sums are +0 exactly, and adding ±0 to a sum that started at +0
/// leaves it unchanged, so the skip changes no result.
pub(crate) fn transform_scalar(input: &[i32; 64], m: &[[f64; 8]; 8], out: &mut [i32; 64], lo: i32, hi: i32) {
    let mut tmp = [[0.0f64; 8]; 8];
    let mut live = [false; 8];
    for v in 0..8 {
        let row = &input[v * 8..v * 8 + 8];
        if row.iter().all(|&c| c == 0) {
            continue;
        }
        live[v] = true;
        let mut acc = [0.0f64; 8];
        for (u, &c) in row.iter().enumerate() {
            let c = f64::from(c);
            for x in 0..8 {
                acc[x] += c * m[u][x];
            }
        }
        tmp[v] = acc;
    }
    for y in 0..8 {
        let mut acc = [0.0f64; 8];
        for v in 0..8 {
            if !live[v] {
                continue;
            }
            let k = m[v][y];
            for x in 0..8 {
                acc[x] += tmp[v][x] * k;
            }
        }
        for x in 0..8 {
            out[y * 8 + x] = round_away(acc[x]).clamp(lo, hi);
        }
    }
}

pub(crate) fn mc_scalar(s: &McSrc, dst: &mut [u8], off: usize, stride: usize, avg: bool) {
    s.check();
    for j in 0..s.h {
        let a = &s.src[s.off + j * s.stride..];
        let b = &s.src[s.off + (j + usize::from(s.hy)) * s.stride..];
        let d = &mut dst[off + j * stride..][..s.w];
        for i in 0..s.w {
            let p = match (s.hx, s.hy) {
                (false, false) => u32::from(a[i]),
                (true, false) => (u32::from(a[i]) + u32::from(a[i + 1]) + 1) >> 1,
                (false, true) => (u32::from(a[i]) + u32::from(b[i]) + 1) >> 1,
                (true, true) => {
                    (u32::from(a[i]) + u32::from(a[i + 1]) + u32::from(b[i]) + u32::from(b[i + 1]) + 2) >> 2
                }
            };
            d[i] = if avg { ((u32::from(d[i]) + p + 1) >> 1) as u8 } else { p as u8 };
        }
    }
}

pub(crate) fn add_block_scalar(dst: &mut [u8], off: usize, stride: usize, res: &[i32; 64]) {
    check_block8(dst.len(), off, stride);
    for y in 0..8 {
        let row = &mut dst[off + y * stride..][..8];
        for x in 0..8 {
            row[x] = (i32::from(row[x]) + res[y * 8 + x]).clamp(0, 255) as u8;
        }
    }
}

pub(crate) fn put_block_scalar(dst: &mut [u8], off: usize, stride: usize, res: &[i32; 64]) {
    check_block8(dst.len(), off, stride);
    for y in 0..8 {
        let row = &mut dst[off + y * stride..][..8];
        for x in 0..8 {
            row[x] = res[y * 8 + x].clamp(0, 255) as u8;
        }
    }
}

pub(crate) fn sad16_scalar(a: Blk, b: Blk, limit: u32) -> u32 {
    a.check16();
    b.check16();
    let mut sad = 0u32;
    for j in 0..16 {
        if j == 8 && sad >= limit {
            break;
        }
        let ra = &a.buf[a.off + j * a.stride..][..16];
        let rb = &b.buf[b.off + j * b.stride..][..16];
        for i in 0..16 {
            sad += u32::from(ra[i].abs_diff(rb[i]));
        }
    }
    sad
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::idct::BASIS;

    pub(crate) struct Rng(u64);
    impl Rng {
        pub(crate) fn new(seed: u64) -> Rng {
            Rng(seed | 1)
        }
        pub(crate) fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        pub(crate) fn range(&mut self, lo: i32, hi: i32) -> i32 {
            lo + (self.next() % (hi - lo + 1) as u64) as i32
        }
    }

    fn transpose(m: &[[f64; 8]; 8]) -> [[f64; 8]; 8] {
        let mut t = [[0.0; 8]; 8];
        for (i, row) in m.iter().enumerate() {
            for (j, &v) in row.iter().enumerate() {
                t[j][i] = v;
            }
        }
        t
    }

    /// The SIMD rung the CI hosts must have; with `MPEG2_REQUIRE_SIMD=1` its
    /// absence is a failure, not a silent pass of the scalar code against
    /// itself.
    #[test]
    fn the_required_rung_is_present() {
        let names: Vec<_> = rungs().iter().map(|d| d.name).collect();
        eprintln!("dsp rungs: {names:?}; in use: {}", active_rung());
        if std::env::var_os("MPEG2_REQUIRE_SIMD").is_some_and(|v| v == "1") {
            let want = if cfg!(target_arch = "x86_64") {
                "avx2"
            } else if cfg!(target_arch = "aarch64") {
                "neon"
            } else {
                "scalar"
            };
            assert!(names.contains(&want), "MPEG2_REQUIRE_SIMD: rung {want} missing from {names:?}");
        }
    }

    #[test]
    fn round_away_is_f64_round() {
        let mut r = Rng::new(7);
        for _ in 0..200_000 {
            let x = (r.next() as i64 >> 24) as f64 / 4096.0 * f64::from(r.range(0, 3));
            assert_eq!(round_away(x), x.round() as i32, "{x}");
        }
        for x in [0.5, -0.5, 1.5, -1.5, 2.5, 0.49999999999999994, -0.49999999999999994, 0.0, -0.0, 255.5, -256.5] {
            assert_eq!(round_away(x), x.round() as i32, "{x}");
        }
    }

    /// Random blocks: dense, sparse (a few coefficients, as most coded
    /// blocks are), rows of zeros, and the extremes.
    fn blocks() -> Vec<[i32; 64]> {
        let mut r = Rng::new(0x1180);
        let mut out = Vec::new();
        for i in 0..20_000 {
            let mut b = [0i32; 64];
            match i % 5 {
                0 => b.iter_mut().for_each(|c| *c = r.range(-2048, 2047)),
                1 => b.iter_mut().for_each(|c| *c = r.range(-300, 300)),
                2 => {
                    for _ in 0..r.range(1, 6) {
                        b[r.range(0, 63) as usize] = r.range(-2048, 2047);
                    }
                }
                3 => {
                    for v in 0..8 {
                        if r.next() & 1 == 0 {
                            for u in 0..8 {
                                b[v * 8 + u] = r.range(-64, 64);
                            }
                        }
                    }
                }
                _ => b.iter_mut().for_each(|c| *c = if r.next() & 1 == 0 { -2048 } else { 2047 }),
            }
            out.push(b);
        }
        out.push([2047; 64]);
        out.push([-2048; 64]);
        out.push([0; 64]);
        out
    }

    #[test]
    fn transforms_match_the_scalar_code() {
        let bt = transpose(&BASIS);
        let sets: [(&[[f64; 8]; 8], i32, i32); 3] =
            [(&BASIS, -256, 255), (&bt, i32::MIN, i32::MAX), (&BASIS, i32::MIN, i32::MAX)];
        let blocks = blocks();
        for d in rungs() {
            for &(m, lo, hi) in &sets {
                for b in &blocks {
                    let mut want = [0; 64];
                    let mut got = [0; 64];
                    transform_scalar(b, m, &mut want, lo, hi);
                    (d.transform)(b, m, &mut got, lo, hi);
                    assert_eq!(got, want, "{}: transform of {b:?}", d.name);
                }
            }
        }
    }

    #[test]
    fn motion_compensation_matches_the_scalar_code() {
        let mut r = Rng::new(42);
        let stride = 64;
        let src: Vec<u8> = (0..stride * 40).map(|i| if i % 7 == 0 { 255 } else { (r.next() & 255) as u8 }).collect();
        for d in rungs() {
            for _ in 0..4000 {
                let w = if r.next() & 1 == 0 { 8 } else { 16 };
                let h = r.range(1, 16) as usize;
                let (hx, hy) = (r.next() & 1 == 0, r.next() & 1 == 0);
                let step = r.range(1, 2) as usize;
                let x = r.range(0, (stride - w - 1) as i32) as usize;
                let y = r.range(0, (40 / step - h - 1) as i32) as usize;
                let s = McSrc { src: &src, off: y * step * stride + x, stride: stride * step, w, h, hx, hy };
                let avg = r.next() & 1 == 0;
                let dstride = 16 * r.range(1, 2) as usize;
                let init: Vec<u8> = (0..16 * 34).map(|_| (r.next() & 255) as u8).collect();
                let doff = r.range(0, 1) as usize * 16;
                let (mut want, mut got) = (init.clone(), init);
                mc_scalar(&s, &mut want, doff, dstride, avg);
                (d.mc)(&s, &mut got, doff, dstride, avg);
                assert_eq!(got, want, "{}: w {w} h {h} hx {hx} hy {hy} avg {avg}", d.name);
            }
        }
    }

    #[test]
    fn residual_addition_matches_the_scalar_code() {
        let mut r = Rng::new(99);
        for d in rungs() {
            for i in 0..5000 {
                let mut res = [0i32; 64];
                let (lo, hi) = if i % 3 == 0 { (-256, 255) } else { (-2048, 2047) };
                res.iter_mut().for_each(|c| *c = r.range(lo, hi));
                let stride = 8 * r.range(1, 4) as usize;
                let init: Vec<u8> = (0..stride * 8 + 8).map(|_| (r.next() & 255) as u8).collect();
                let off = r.range(0, 8) as usize;
                let (mut want, mut got) = (init.clone(), init.clone());
                add_block_scalar(&mut want, off, stride, &res);
                (d.add_block)(&mut got, off, stride, &res);
                assert_eq!(got, want, "{}: add", d.name);
                let (mut want, mut got) = (init.clone(), init);
                put_block_scalar(&mut want, off, stride, &res);
                (d.put_block)(&mut got, off, stride, &res);
                assert_eq!(got, want, "{}: put", d.name);
            }
        }
    }

    #[test]
    fn sad_matches_the_scalar_code() {
        let mut r = Rng::new(5);
        let a: Vec<u8> = (0..80 * 40).map(|_| (r.next() & 255) as u8).collect();
        let mut b: Vec<u8> = (0..80 * 40).map(|_| (r.next() & 255) as u8).collect();
        b[..80].fill(0);
        let mut a2 = a.clone();
        a2[..80].fill(255);
        for d in rungs() {
            for _ in 0..20_000 {
                let (ao, bo) = (r.range(0, 24 * 80) as usize, r.range(0, 24 * 80) as usize);
                let bs = [0, 16, 80][r.range(0, 2) as usize];
                let limit = [u32::MAX, 0, r.range(0, 20_000) as u32, r.range(0, 9000) as u32][r.range(0, 3) as usize];
                let (x, y) = (Blk { buf: &a, off: ao, stride: 80 }, Blk { buf: &b, off: bo, stride: bs });
                let want = sad16_scalar(x, y, limit);
                assert_eq!((d.sad16)(x, y, limit), want, "{}", d.name);
                // The full sum below the limit; a partial one reaching it.
                let full = sad16_scalar(x, y, u32::MAX);
                assert!(want == full || (want >= limit && want <= full));
            }
            // The largest sum: 256 × 255.
            let (x, y) = (Blk { buf: &a2, off: 0, stride: 0 }, Blk { buf: &b, off: 0, stride: 0 });
            assert_eq!((d.sad16)(x, y, u32::MAX), 256 * 255, "{}", d.name);
        }
    }
}
