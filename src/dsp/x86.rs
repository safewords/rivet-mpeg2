//! x86-64 kernels: SSE2 (every x86-64 processor has it), the transform in
//! AVX2 (four doubles per register) and AVX-512F (a whole row of eight).
//!
//! The transforms perform the scalar code's operations one for one — the
//! same products and sums, rounded the same, in the same order — on several
//! columns at once; there is no fused multiply-add, which would round once
//! where the scalar code rounds twice. Half-away-from-zero rounding is
//! `trunc(x)` plus or minus one where `x - trunc(x)` reaches ±0.5 (exact:
//! the fraction of a double is a double).

// The kernels index several arrays by the same row / column number.
#![allow(clippy::needless_range_loop)]

use super::{Blk, Dsp, McSrc, check_block8, live_extent};
use std::arch::x86_64::*;

static SSE2: Dsp = Dsp {
    name: "sse2",
    transform: transform_sse2,
    mc: mc_sse2,
    add_block: add_block_sse2,
    put_block: put_block_sse2,
    sad16: sad16_sse2,
};

static AVX2: Dsp = Dsp {
    name: "avx2",
    transform: transform_avx2_entry,
    ..SSE2
};

static AVX512: Dsp = Dsp {
    name: "avx512",
    transform: transform_avx512_entry,
    ..SSE2
};

/// The rungs this processor runs, narrowest first.
pub(super) fn rungs() -> Vec<&'static Dsp> {
    let mut v = vec![&SSE2];
    if is_x86_feature_detected!("avx2") {
        v.push(&AVX2);
    }
    if is_x86_feature_detected!("avx512f") {
        v.push(&AVX512);
    }
    v
}

// ---------------------------------------------------------------- transform

/// One row of a matrix as two-double vectors.
#[target_feature(enable = "sse2")]
#[inline]
fn row2(r: &[f64; 8]) -> [__m128d; 4] {
    [
        _mm_set_pd(r[1], r[0]),
        _mm_set_pd(r[3], r[2]),
        _mm_set_pd(r[5], r[4]),
        _mm_set_pd(r[7], r[6]),
    ]
}

/// Rounds half away from zero, clamps to [lo, hi] and truncates to i32 (two
/// lanes, in the low half of the result). |x| < 2³¹.
#[target_feature(enable = "sse2")]
#[inline]
fn round_clamp_sse2(x: __m128d, lo: __m128d, hi: __m128d) -> __m128i {
    let half = _mm_set1_pd(0.5);
    let one = _mm_set1_pd(1.0);
    let t = _mm_cvtepi32_pd(_mm_cvttpd_epi32(x));
    let d = _mm_sub_pd(x, t);
    let up = _mm_and_pd(_mm_cmpge_pd(d, half), one);
    let down = _mm_and_pd(_mm_cmple_pd(d, _mm_sub_pd(_mm_setzero_pd(), half)), one);
    let r = _mm_add_pd(t, _mm_sub_pd(up, down));
    _mm_cvttpd_epi32(_mm_max_pd(_mm_min_pd(r, hi), lo))
}

fn transform_sse2(input: &[i32; 64], m: &[[f64; 8]; 8], out: &mut [i32; 64], lo: i32, hi: i32) {
    // SAFETY: SSE2 is part of the x86-64 baseline: every x86-64 processor
    // has it.
    unsafe { transform_sse2_impl(input, m, out, lo, hi) }
}

#[target_feature(enable = "sse2")]
fn transform_sse2_impl(
    input: &[i32; 64],
    m: &[[f64; 8]; 8],
    out: &mut [i32; 64],
    lo: i32,
    hi: i32,
) {
    match live_extent(input) {
        (0, false) => tr_sse2::<0, false>(input, m, out, lo, hi),
        (1, false) => tr_sse2::<1, false>(input, m, out, lo, hi),
        (2, false) => tr_sse2::<2, false>(input, m, out, lo, hi),
        (3, false) => tr_sse2::<3, false>(input, m, out, lo, hi),
        (4, false) => tr_sse2::<4, false>(input, m, out, lo, hi),
        (5, false) => tr_sse2::<5, false>(input, m, out, lo, hi),
        (6, false) => tr_sse2::<6, false>(input, m, out, lo, hi),
        (7, false) => tr_sse2::<7, false>(input, m, out, lo, hi),
        (0, true) => tr_sse2::<0, true>(input, m, out, lo, hi),
        (1, true) => tr_sse2::<1, true>(input, m, out, lo, hi),
        (2, true) => tr_sse2::<2, true>(input, m, out, lo, hi),
        (3, true) => tr_sse2::<3, true>(input, m, out, lo, hi),
        (4, true) => tr_sse2::<4, true>(input, m, out, lo, hi),
        (5, true) => tr_sse2::<5, true>(input, m, out, lo, hi),
        (6, true) => tr_sse2::<6, true>(input, m, out, lo, hi),
        _ => tr_sse2::<7, true>(input, m, out, lo, hi),
    }
}

/// The transform for blocks whose non-zero coefficients all lie in rows
/// `0..K` and, if `R7`, row 7: the other rows would sum to +0 in the row
/// pass and add ±0 — nothing — in the column pass, so they are left out.
/// Rows below `K` are computed whether zero or not (exactly +0 if zero, the
/// same nothing); row 7, the last in every column sum, is added last.
/// `K` and `R7` are constants, so every loop unrolls and the sums stay in
/// registers.
#[target_feature(enable = "sse2")]
#[inline]
fn tr_sse2<const K: usize, const R7: bool>(
    input: &[i32; 64],
    m: &[[f64; 8]; 8],
    out: &mut [i32; 64],
    lo: i32,
    hi: i32,
) {
    let mrows: [[__m128d; 4]; 8] = std::array::from_fn(|u| row2(&m[u]));
    // Rows: tmp[v][x] = Σ_u in[v][u]·M[u][x], in order of u.
    let mut tmp = [[_mm_setzero_pd(); 4]; K];
    let mut tmp7 = [_mm_setzero_pd(); 4];
    for u in 0..8 {
        for v in 0..K {
            let c = _mm_set1_pd(f64::from(input[v * 8 + u]));
            for q in 0..4 {
                tmp[v][q] = _mm_add_pd(tmp[v][q], _mm_mul_pd(c, mrows[u][q]));
            }
        }
        if R7 {
            let c = _mm_set1_pd(f64::from(input[56 + u]));
            for q in 0..4 {
                tmp7[q] = _mm_add_pd(tmp7[q], _mm_mul_pd(c, mrows[u][q]));
            }
        }
    }
    // Columns: out[y][x] = Σ_v tmp[v][x]·M[v][y], in order of v.
    let (lo, hi) = (_mm_set1_pd(f64::from(lo)), _mm_set1_pd(f64::from(hi)));
    for y in 0..8 {
        let mut acc = [_mm_setzero_pd(); 4];
        for v in 0..K {
            let k = _mm_set1_pd(m[v][y]);
            for q in 0..4 {
                acc[q] = _mm_add_pd(acc[q], _mm_mul_pd(tmp[v][q], k));
            }
        }
        if R7 {
            let k = _mm_set1_pd(m[7][y]);
            for q in 0..4 {
                acc[q] = _mm_add_pd(acc[q], _mm_mul_pd(tmp7[q], k));
            }
        }
        let r0 = _mm_unpacklo_epi64(
            round_clamp_sse2(acc[0], lo, hi),
            round_clamp_sse2(acc[1], lo, hi),
        );
        let r1 = _mm_unpacklo_epi64(
            round_clamp_sse2(acc[2], lo, hi),
            round_clamp_sse2(acc[3], lo, hi),
        );
        let o = &mut out[y * 8..y * 8 + 8];
        // SAFETY: `o` is 8 i32s (32 bytes); the stores write bytes 0..16 and
        // 16..32 of it, unaligned stores.
        unsafe {
            _mm_storeu_si128(o.as_mut_ptr().cast(), r0);
            _mm_storeu_si128(o.as_mut_ptr().add(4).cast(), r1);
        }
    }
}

fn transform_avx2_entry(
    input: &[i32; 64],
    m: &[[f64; 8]; 8],
    out: &mut [i32; 64],
    lo: i32,
    hi: i32,
) {
    // SAFETY: this entry is only reachable through the AVX2 rung, which
    // rungs() installs after is_x86_feature_detected!("avx2").
    unsafe { transform_avx2(input, m, out, lo, hi) }
}

#[target_feature(enable = "avx2")]
fn transform_avx2(input: &[i32; 64], m: &[[f64; 8]; 8], out: &mut [i32; 64], lo: i32, hi: i32) {
    match live_extent(input) {
        (0, false) => tr_avx2::<0, false>(input, m, out, lo, hi),
        (1, false) => tr_avx2::<1, false>(input, m, out, lo, hi),
        (2, false) => tr_avx2::<2, false>(input, m, out, lo, hi),
        (3, false) => tr_avx2::<3, false>(input, m, out, lo, hi),
        (4, false) => tr_avx2::<4, false>(input, m, out, lo, hi),
        (5, false) => tr_avx2::<5, false>(input, m, out, lo, hi),
        (6, false) => tr_avx2::<6, false>(input, m, out, lo, hi),
        (7, false) => tr_avx2::<7, false>(input, m, out, lo, hi),
        (0, true) => tr_avx2::<0, true>(input, m, out, lo, hi),
        (1, true) => tr_avx2::<1, true>(input, m, out, lo, hi),
        (2, true) => tr_avx2::<2, true>(input, m, out, lo, hi),
        (3, true) => tr_avx2::<3, true>(input, m, out, lo, hi),
        (4, true) => tr_avx2::<4, true>(input, m, out, lo, hi),
        (5, true) => tr_avx2::<5, true>(input, m, out, lo, hi),
        (6, true) => tr_avx2::<6, true>(input, m, out, lo, hi),
        _ => tr_avx2::<7, true>(input, m, out, lo, hi),
    }
}

/// The transform for blocks whose non-zero coefficients all lie in rows
/// `0..K` and, if `R7`, row 7: the other rows would sum to +0 in the row
/// pass and add ±0 — nothing — in the column pass, so they are left out.
/// Rows below `K` are computed whether zero or not (exactly +0 if zero, the
/// same nothing); row 7, the last in every column sum, is added last.
/// `K` and `R7` are constants, so every loop unrolls and the sums stay in
/// registers.
#[target_feature(enable = "avx2")]
#[inline]
fn tr_avx2<const K: usize, const R7: bool>(
    input: &[i32; 64],
    m: &[[f64; 8]; 8],
    out: &mut [i32; 64],
    lo: i32,
    hi: i32,
) {
    // SAFETY: each load reads f64s 0..4 or 4..8 of an 8-element row.
    let mrows: [[__m256d; 2]; 8] = std::array::from_fn(|u| unsafe {
        [
            _mm256_loadu_pd(m[u].as_ptr()),
            _mm256_loadu_pd(m[u].as_ptr().add(4)),
        ]
    });
    // The live rows' coefficients as doubles, for broadcasting.
    let mut ind = [0.0f64; 64];
    for (d, &c) in ind[..K * 8].iter_mut().zip(&input[..K * 8]) {
        *d = f64::from(c);
    }
    if R7 {
        for (d, &c) in ind[56..].iter_mut().zip(&input[56..]) {
            *d = f64::from(c);
        }
    }
    let mut tmp = [[_mm256_setzero_pd(); 2]; K];
    let mut tmp7 = [_mm256_setzero_pd(); 2];
    for u in 0..8 {
        for v in 0..K {
            let c = _mm256_set1_pd(ind[v * 8 + u]);
            tmp[v][0] = _mm256_add_pd(tmp[v][0], _mm256_mul_pd(c, mrows[u][0]));
            tmp[v][1] = _mm256_add_pd(tmp[v][1], _mm256_mul_pd(c, mrows[u][1]));
        }
        if R7 {
            let c = _mm256_set1_pd(ind[56 + u]);
            tmp7[0] = _mm256_add_pd(tmp7[0], _mm256_mul_pd(c, mrows[u][0]));
            tmp7[1] = _mm256_add_pd(tmp7[1], _mm256_mul_pd(c, mrows[u][1]));
        }
    }
    let (lo, hi) = (_mm256_set1_pd(f64::from(lo)), _mm256_set1_pd(f64::from(hi)));
    let half = _mm256_set1_pd(0.5);
    let mhalf = _mm256_set1_pd(-0.5);
    let one = _mm256_set1_pd(1.0);
    for y in 0..8 {
        let mut acc = [_mm256_setzero_pd(); 2];
        for v in 0..K {
            let k = _mm256_set1_pd(m[v][y]);
            acc[0] = _mm256_add_pd(acc[0], _mm256_mul_pd(tmp[v][0], k));
            acc[1] = _mm256_add_pd(acc[1], _mm256_mul_pd(tmp[v][1], k));
        }
        if R7 {
            let k = _mm256_set1_pd(m[7][y]);
            acc[0] = _mm256_add_pd(acc[0], _mm256_mul_pd(tmp7[0], k));
            acc[1] = _mm256_add_pd(acc[1], _mm256_mul_pd(tmp7[1], k));
        }
        let o = &mut out[y * 8..y * 8 + 8];
        for (q, a) in acc.into_iter().enumerate() {
            let t = _mm256_round_pd::<{ _MM_FROUND_TO_ZERO | _MM_FROUND_NO_EXC }>(a);
            let d = _mm256_sub_pd(a, t);
            let up = _mm256_and_pd(_mm256_cmp_pd::<_CMP_GE_OQ>(d, half), one);
            let down = _mm256_and_pd(_mm256_cmp_pd::<_CMP_LE_OQ>(d, mhalf), one);
            let r = _mm256_add_pd(t, _mm256_sub_pd(up, down));
            let r = _mm256_cvttpd_epi32(_mm256_max_pd(_mm256_min_pd(r, hi), lo));
            // SAFETY: `o` is 8 i32s; this writes i32s 4q..4q+4 of it.
            unsafe { _mm_storeu_si128(o.as_mut_ptr().add(4 * q).cast(), r) };
        }
    }
}

fn transform_avx512_entry(
    input: &[i32; 64],
    m: &[[f64; 8]; 8],
    out: &mut [i32; 64],
    lo: i32,
    hi: i32,
) {
    // SAFETY: this entry is only reachable through the AVX-512 rung, which
    // rungs() installs after is_x86_feature_detected!("avx512f").
    unsafe { transform_avx512(input, m, out, lo, hi) }
}

#[target_feature(enable = "avx512f")]
fn transform_avx512(input: &[i32; 64], m: &[[f64; 8]; 8], out: &mut [i32; 64], lo: i32, hi: i32) {
    match live_extent(input) {
        (0, false) => tr_avx512::<0, false>(input, m, out, lo, hi),
        (1, false) => tr_avx512::<1, false>(input, m, out, lo, hi),
        (2, false) => tr_avx512::<2, false>(input, m, out, lo, hi),
        (3, false) => tr_avx512::<3, false>(input, m, out, lo, hi),
        (4, false) => tr_avx512::<4, false>(input, m, out, lo, hi),
        (5, false) => tr_avx512::<5, false>(input, m, out, lo, hi),
        (6, false) => tr_avx512::<6, false>(input, m, out, lo, hi),
        (7, false) => tr_avx512::<7, false>(input, m, out, lo, hi),
        (0, true) => tr_avx512::<0, true>(input, m, out, lo, hi),
        (1, true) => tr_avx512::<1, true>(input, m, out, lo, hi),
        (2, true) => tr_avx512::<2, true>(input, m, out, lo, hi),
        (3, true) => tr_avx512::<3, true>(input, m, out, lo, hi),
        (4, true) => tr_avx512::<4, true>(input, m, out, lo, hi),
        (5, true) => tr_avx512::<5, true>(input, m, out, lo, hi),
        (6, true) => tr_avx512::<6, true>(input, m, out, lo, hi),
        _ => tr_avx512::<7, true>(input, m, out, lo, hi),
    }
}

/// The transform for blocks whose non-zero coefficients all lie in rows
/// `0..K` and, if `R7`, row 7: the other rows would sum to +0 in the row
/// pass and add ±0 — nothing — in the column pass, so they are left out.
/// Rows below `K` are computed whether zero or not (exactly +0 if zero, the
/// same nothing); row 7, the last in every column sum, is added last.
/// `K` and `R7` are constants, so every loop unrolls and the sums stay in
/// registers.
#[target_feature(enable = "avx512f")]
#[inline]
fn tr_avx512<const K: usize, const R7: bool>(
    input: &[i32; 64],
    m: &[[f64; 8]; 8],
    out: &mut [i32; 64],
    lo: i32,
    hi: i32,
) {
    // SAFETY: each load reads the 8 f64s of a row.
    let mrows: [__m512d; 8] = std::array::from_fn(|u| unsafe { _mm512_loadu_pd(m[u].as_ptr()) });
    let mut ind = [0.0f64; 64];
    for (d, &c) in ind[..K * 8].iter_mut().zip(&input[..K * 8]) {
        *d = f64::from(c);
    }
    if R7 {
        for (d, &c) in ind[56..].iter_mut().zip(&input[56..]) {
            *d = f64::from(c);
        }
    }
    let mut tmp = [_mm512_setzero_pd(); K];
    let mut tmp7 = _mm512_setzero_pd();
    for u in 0..8 {
        for v in 0..K {
            tmp[v] = _mm512_add_pd(
                tmp[v],
                _mm512_mul_pd(_mm512_set1_pd(ind[v * 8 + u]), mrows[u]),
            );
        }
        if R7 {
            tmp7 = _mm512_add_pd(tmp7, _mm512_mul_pd(_mm512_set1_pd(ind[56 + u]), mrows[u]));
        }
    }
    let (lo, hi) = (_mm512_set1_pd(f64::from(lo)), _mm512_set1_pd(f64::from(hi)));
    let half = _mm512_set1_pd(0.5);
    let mhalf = _mm512_set1_pd(-0.5);
    let one = _mm512_set1_pd(1.0);
    for y in 0..8 {
        let mut acc = _mm512_setzero_pd();
        for v in 0..K {
            acc = _mm512_add_pd(acc, _mm512_mul_pd(tmp[v], _mm512_set1_pd(m[v][y])));
        }
        if R7 {
            acc = _mm512_add_pd(acc, _mm512_mul_pd(tmp7, _mm512_set1_pd(m[7][y])));
        }
        let t = _mm512_roundscale_pd::<{ _MM_FROUND_TO_ZERO | _MM_FROUND_NO_EXC }>(acc);
        let d = _mm512_sub_pd(acc, t);
        let r = _mm512_mask_add_pd(t, _mm512_cmp_pd_mask::<_CMP_GE_OQ>(d, half), t, one);
        let r = _mm512_mask_sub_pd(r, _mm512_cmp_pd_mask::<_CMP_LE_OQ>(d, mhalf), r, one);
        let r = _mm512_cvttpd_epi32(_mm512_max_pd(_mm512_min_pd(r, hi), lo));
        // SAFETY: row y of `out` is i32s 8y..8y+8 (32 bytes), inside the
        // 64-element array.
        unsafe { _mm256_storeu_si256(out.as_mut_ptr().add(8 * y).cast(), r) };
    }
}

// ------------------------------------------------------- motion compensation

fn mc_sse2(s: &McSrc, dst: &mut [u8], off: usize, stride: usize, avg: bool) {
    // SAFETY: SSE2 is part of the x86-64 baseline: every x86-64 processor
    // has it.
    unsafe { mc_sse2_impl(s, dst, off, stride, avg) }
}

#[target_feature(enable = "sse2")]
fn mc_sse2_impl(s: &McSrc, dst: &mut [u8], off: usize, stride: usize, avg: bool) {
    s.check();
    assert!(
        off + (s.h - 1) * stride + s.w <= dst.len(),
        "prediction outside its buffer"
    );
    macro_rules! go {
        ($($w:literal),*) => {
            match (s.w, s.hx, s.hy, avg) {
                $(
                    ($w, false, false, false) => mc_rows::<$w, false, false, false>(s, dst, off, stride),
                    ($w, true, false, false) => mc_rows::<$w, true, false, false>(s, dst, off, stride),
                    ($w, false, true, false) => mc_rows::<$w, false, true, false>(s, dst, off, stride),
                    ($w, true, true, false) => mc_rows::<$w, true, true, false>(s, dst, off, stride),
                    ($w, false, false, true) => mc_rows::<$w, false, false, true>(s, dst, off, stride),
                    ($w, true, false, true) => mc_rows::<$w, true, false, true>(s, dst, off, stride),
                    ($w, false, true, true) => mc_rows::<$w, false, true, true>(s, dst, off, stride),
                    ($w, true, true, true) => mc_rows::<$w, true, true, true>(s, dst, off, stride),
                )*
                _ => unreachable!("checked width"),
            }
        };
    }
    go!(8, 16)
}

/// Loads `W` (8 or 16) bytes at `p`.
///
/// # Safety
/// `p..p + W` must be readable.
#[target_feature(enable = "sse2")]
#[inline]
unsafe fn load<const W: usize>(p: *const u8) -> __m128i {
    // SAFETY: the caller guarantees W readable bytes at p.
    unsafe {
        if W == 16 {
            _mm_loadu_si128(p.cast())
        } else {
            _mm_loadl_epi64(p.cast())
        }
    }
}

/// The interpolated row at `p` (and, with `HY`, the row `stride` below).
///
/// # Safety
/// `p..p + W + HX` (and the same `stride` further on with `HY`) must be
/// readable.
#[target_feature(enable = "sse2")]
#[inline]
unsafe fn interp<const W: usize, const HX: bool, const HY: bool>(
    p: *const u8,
    stride: usize,
) -> __m128i {
    // SAFETY: every load is of W bytes at p, p + 1 (HX), p + stride (HY) or
    // p + stride + 1 (both), which the caller guarantees readable.
    unsafe {
        let a = load::<W>(p);
        match (HX, HY) {
            (false, false) => a,
            (true, false) => _mm_avg_epu8(a, load::<W>(p.add(1))),
            (false, true) => _mm_avg_epu8(a, load::<W>(p.add(stride))),
            (true, true) => {
                let z = _mm_setzero_si128();
                let (a1, b, b1) = (
                    load::<W>(p.add(1)),
                    load::<W>(p.add(stride)),
                    load::<W>(p.add(stride + 1)),
                );
                let two = _mm_set1_epi16(2);
                let lo = _mm_add_epi16(
                    _mm_add_epi16(_mm_unpacklo_epi8(a, z), _mm_unpacklo_epi8(a1, z)),
                    _mm_add_epi16(_mm_unpacklo_epi8(b, z), _mm_unpacklo_epi8(b1, z)),
                );
                let lo = _mm_srli_epi16::<2>(_mm_add_epi16(lo, two));
                let hi = if W == 16 {
                    let hi = _mm_add_epi16(
                        _mm_add_epi16(_mm_unpackhi_epi8(a, z), _mm_unpackhi_epi8(a1, z)),
                        _mm_add_epi16(_mm_unpackhi_epi8(b, z), _mm_unpackhi_epi8(b1, z)),
                    );
                    _mm_srli_epi16::<2>(_mm_add_epi16(hi, two))
                } else {
                    z
                };
                _mm_packus_epi16(lo, hi)
            }
        }
    }
}

#[target_feature(enable = "sse2")]
#[inline]
fn mc_rows<const W: usize, const HX: bool, const HY: bool, const AVG: bool>(
    s: &McSrc,
    dst: &mut [u8],
    off: usize,
    stride: usize,
) {
    for j in 0..s.h {
        // SAFETY: s.check() (in mc_sse2) proved every source byte of the
        // block — rows 0..h (+1 with HY), columns 0..W (+1 with HX) — lies in
        // s.src, and the assert there that rows 0..h of W bytes at `off`,
        // `stride` apart, lie in `dst`.
        unsafe {
            let p = interp::<W, HX, HY>(s.src.as_ptr().add(s.off + j * s.stride), s.stride);
            let d = dst.as_mut_ptr().add(off + j * stride);
            let p = if AVG {
                _mm_avg_epu8(p, load::<W>(d))
            } else {
                p
            };
            if W == 16 {
                _mm_storeu_si128(d.cast(), p);
            } else {
                _mm_storel_epi64(d.cast(), p);
            }
        }
    }
}

// ------------------------------------------------------ residual and SAD

/// Row `y` of a residual block as eight saturated i16s.
#[target_feature(enable = "sse2")]
#[inline]
fn residual_row(res: &[i32; 64], y: usize) -> __m128i {
    let r = &res[y * 8..y * 8 + 8];
    // SAFETY: `r` is 8 i32s; the loads read i32s 0..4 and 4..8.
    unsafe {
        _mm_packs_epi32(
            _mm_loadu_si128(r.as_ptr().cast()),
            _mm_loadu_si128(r.as_ptr().add(4).cast()),
        )
    }
}

fn add_block_sse2(dst: &mut [u8], off: usize, stride: usize, res: &[i32; 64]) {
    // SAFETY: SSE2 is part of the x86-64 baseline: every x86-64 processor
    // has it.
    unsafe { add_block_sse2_impl(dst, off, stride, res) }
}

#[target_feature(enable = "sse2")]
fn add_block_sse2_impl(dst: &mut [u8], off: usize, stride: usize, res: &[i32; 64]) {
    check_block8(dst.len(), off, stride);
    let z = _mm_setzero_si128();
    for y in 0..8 {
        let r = residual_row(res, y);
        // SAFETY: check_block8 proved 8 bytes at off + y * stride are in dst.
        unsafe {
            let p = dst.as_mut_ptr().add(off + y * stride);
            // Saturating: an i16 sum past 32767 still clamps to 255.
            let s = _mm_adds_epi16(_mm_unpacklo_epi8(_mm_loadl_epi64(p.cast()), z), r);
            _mm_storel_epi64(p.cast(), _mm_packus_epi16(s, z));
        }
    }
}

fn put_block_sse2(dst: &mut [u8], off: usize, stride: usize, res: &[i32; 64]) {
    // SAFETY: SSE2 is part of the x86-64 baseline: every x86-64 processor
    // has it.
    unsafe { put_block_sse2_impl(dst, off, stride, res) }
}

#[target_feature(enable = "sse2")]
fn put_block_sse2_impl(dst: &mut [u8], off: usize, stride: usize, res: &[i32; 64]) {
    check_block8(dst.len(), off, stride);
    let z = _mm_setzero_si128();
    for y in 0..8 {
        let r = residual_row(res, y);
        // SAFETY: check_block8 proved 8 bytes at off + y * stride are in dst.
        unsafe {
            _mm_storel_epi64(
                dst.as_mut_ptr().add(off + y * stride).cast(),
                _mm_packus_epi16(r, z),
            )
        };
    }
}

fn sad16_sse2(a: Blk, b: Blk, limit: u32) -> u32 {
    // SAFETY: SSE2 is part of the x86-64 baseline.
    unsafe { sad16_sse2_impl(a, b, limit) }
}

#[target_feature(enable = "sse2")]
fn sad16_sse2_impl(a: Blk, b: Blk, limit: u32) -> u32 {
    a.check16();
    b.check16();
    let rows = |r: std::ops::Range<usize>| {
        let mut acc = _mm_setzero_si128();
        for j in r {
            // SAFETY: check16 proved 16 bytes at each row's start are in a
            // and in b.
            unsafe {
                let ra = _mm_loadu_si128(a.buf.as_ptr().add(a.off + j * a.stride).cast());
                let rb = _mm_loadu_si128(b.buf.as_ptr().add(b.off + j * b.stride).cast());
                acc = _mm_add_epi64(acc, _mm_sad_epu8(ra, rb));
            }
        }
        (_mm_cvtsi128_si64(acc) + _mm_cvtsi128_si64(_mm_unpackhi_epi64(acc, acc))) as u32
    };
    let top = rows(0..8);
    if top >= limit { top } else { top + rows(8..16) }
}
