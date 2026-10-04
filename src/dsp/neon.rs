//! AArch64 NEON kernels (NEON is part of every AArch64 processor).
//!
//! The transform performs the scalar code's operations one for one, two
//! columns per register, with no fused multiply-add; FRINTA rounds half
//! away from zero, as `f64::round`.

// The kernels index several arrays by the same row / column number.
#![allow(clippy::needless_range_loop)]

use super::{Blk, Dsp, McSrc, check_block8, live_extent};
use std::arch::aarch64::*;

static NEON: Dsp = Dsp {
    name: "neon",
    transform: transform_neon,
    mc: mc_neon,
    add_block: add_block_neon,
    put_block: put_block_neon,
    sad16: sad16_neon,
};

pub(super) fn rungs() -> Vec<&'static Dsp> {
    vec![&NEON]
}

fn transform_neon(input: &[i32; 64], m: &[[f64; 8]; 8], out: &mut [i32; 64], lo: i32, hi: i32) {
    // SAFETY: NEON is part of the AArch64 baseline: every AArch64 processor
    // has it.
    unsafe { transform_neon_impl(input, m, out, lo, hi) }
}

#[target_feature(enable = "neon")]
fn transform_neon_impl(
    input: &[i32; 64],
    m: &[[f64; 8]; 8],
    out: &mut [i32; 64],
    lo: i32,
    hi: i32,
) {
    match live_extent(input) {
        (0, false) => tr_neon::<0, false>(input, m, out, lo, hi),
        (1, false) => tr_neon::<1, false>(input, m, out, lo, hi),
        (2, false) => tr_neon::<2, false>(input, m, out, lo, hi),
        (3, false) => tr_neon::<3, false>(input, m, out, lo, hi),
        (4, false) => tr_neon::<4, false>(input, m, out, lo, hi),
        (5, false) => tr_neon::<5, false>(input, m, out, lo, hi),
        (6, false) => tr_neon::<6, false>(input, m, out, lo, hi),
        (7, false) => tr_neon::<7, false>(input, m, out, lo, hi),
        (0, true) => tr_neon::<0, true>(input, m, out, lo, hi),
        (1, true) => tr_neon::<1, true>(input, m, out, lo, hi),
        (2, true) => tr_neon::<2, true>(input, m, out, lo, hi),
        (3, true) => tr_neon::<3, true>(input, m, out, lo, hi),
        (4, true) => tr_neon::<4, true>(input, m, out, lo, hi),
        (5, true) => tr_neon::<5, true>(input, m, out, lo, hi),
        (6, true) => tr_neon::<6, true>(input, m, out, lo, hi),
        _ => tr_neon::<7, true>(input, m, out, lo, hi),
    }
}

/// The transform for blocks whose non-zero coefficients all lie in rows
/// `0..K` and, if `R7`, row 7: the other rows would sum to +0 in the row
/// pass and add ±0 — nothing — in the column pass, so they are left out.
/// Rows below `K` are computed whether zero or not (exactly +0 if zero, the
/// same nothing); row 7, the last in every column sum, is added last.
/// `K` and `R7` are constants, so every loop unrolls and the sums stay in
/// registers.
#[target_feature(enable = "neon")]
#[inline]
fn tr_neon<const K: usize, const R7: bool>(
    input: &[i32; 64],
    m: &[[f64; 8]; 8],
    out: &mut [i32; 64],
    lo: i32,
    hi: i32,
) {
    // SAFETY: each load reads two f64s at 2q..2q+2 of an 8-element row.
    let mrows: [[float64x2_t; 4]; 8] = std::array::from_fn(|u| {
        std::array::from_fn(|q| unsafe { vld1q_f64(m[u].as_ptr().add(2 * q)) })
    });
    // vmulq then vaddq: two roundings, as the scalar code (no vfmaq).
    let mut tmp = [[vdupq_n_f64(0.0); 4]; K];
    let mut tmp7 = [vdupq_n_f64(0.0); 4];
    for u in 0..8 {
        for v in 0..K {
            let c = vdupq_n_f64(f64::from(input[v * 8 + u]));
            for q in 0..4 {
                tmp[v][q] = vaddq_f64(tmp[v][q], vmulq_f64(c, mrows[u][q]));
            }
        }
        if R7 {
            let c = vdupq_n_f64(f64::from(input[56 + u]));
            for q in 0..4 {
                tmp7[q] = vaddq_f64(tmp7[q], vmulq_f64(c, mrows[u][q]));
            }
        }
    }
    let (lo, hi) = (vdupq_n_f64(f64::from(lo)), vdupq_n_f64(f64::from(hi)));
    for y in 0..8 {
        let mut acc = [vdupq_n_f64(0.0); 4];
        for v in 0..K {
            let k = vdupq_n_f64(m[v][y]);
            for q in 0..4 {
                acc[q] = vaddq_f64(acc[q], vmulq_f64(tmp[v][q], k));
            }
        }
        if R7 {
            let k = vdupq_n_f64(m[7][y]);
            for q in 0..4 {
                acc[q] = vaddq_f64(acc[q], vmulq_f64(tmp7[q], k));
            }
        }
        let o = &mut out[y * 8..y * 8 + 8];
        for (q, &a) in acc.iter().enumerate() {
            // FRINTA: to nearest, ties away from zero.
            let r = vminq_f64(vmaxq_f64(vrndaq_f64(a), lo), hi);
            let r = vmovn_s64(vcvtq_s64_f64(r));
            // SAFETY: `o` is 8 i32s; this writes i32s 2q..2q+2.
            unsafe { vst1_s32(o.as_mut_ptr().add(2 * q), r) };
        }
    }
}

fn mc_neon(s: &McSrc, dst: &mut [u8], off: usize, stride: usize, avg: bool) {
    // SAFETY: NEON is part of the AArch64 baseline: every AArch64 processor
    // has it.
    unsafe { mc_neon_impl(s, dst, off, stride, avg) }
}

#[target_feature(enable = "neon")]
fn mc_neon_impl(s: &McSrc, dst: &mut [u8], off: usize, stride: usize, avg: bool) {
    s.check();
    assert!(
        off + (s.h - 1) * stride + s.w <= dst.len(),
        "prediction outside its buffer"
    );
    if s.w == 16 {
        for j in 0..s.h {
            // SAFETY: s.check() proved rows 0..h (+1 with hy) of 16 (+1 with
            // hx) bytes from s.off, s.stride apart, lie in s.src; the assert
            // above that rows 0..h of 16 bytes at off, stride apart, lie in
            // dst.
            unsafe {
                let p = s.src.as_ptr().add(s.off + j * s.stride);
                let a = vld1q_u8(p);
                let v = match (s.hx, s.hy) {
                    (false, false) => a,
                    (true, false) => vrhaddq_u8(a, vld1q_u8(p.add(1))),
                    (false, true) => vrhaddq_u8(a, vld1q_u8(p.add(s.stride))),
                    (true, true) => {
                        let (a1, b, b1) = (
                            vld1q_u8(p.add(1)),
                            vld1q_u8(p.add(s.stride)),
                            vld1q_u8(p.add(s.stride + 1)),
                        );
                        let lo = vaddq_u16(
                            vaddl_u8(vget_low_u8(a), vget_low_u8(a1)),
                            vaddl_u8(vget_low_u8(b), vget_low_u8(b1)),
                        );
                        let hi = vaddq_u16(
                            vaddl_u8(vget_high_u8(a), vget_high_u8(a1)),
                            vaddl_u8(vget_high_u8(b), vget_high_u8(b1)),
                        );
                        vcombine_u8(vrshrn_n_u16::<2>(lo), vrshrn_n_u16::<2>(hi))
                    }
                };
                let d = dst.as_mut_ptr().add(off + j * stride);
                let v = if avg { vrhaddq_u8(v, vld1q_u8(d)) } else { v };
                vst1q_u8(d, v);
            }
        }
    } else {
        for j in 0..s.h {
            // SAFETY: as above, with rows of 8 (+1 with hx) bytes.
            unsafe {
                let p = s.src.as_ptr().add(s.off + j * s.stride);
                let a = vld1_u8(p);
                let v = match (s.hx, s.hy) {
                    (false, false) => a,
                    (true, false) => vrhadd_u8(a, vld1_u8(p.add(1))),
                    (false, true) => vrhadd_u8(a, vld1_u8(p.add(s.stride))),
                    (true, true) => {
                        let (a1, b, b1) = (
                            vld1_u8(p.add(1)),
                            vld1_u8(p.add(s.stride)),
                            vld1_u8(p.add(s.stride + 1)),
                        );
                        vrshrn_n_u16::<2>(vaddq_u16(vaddl_u8(a, a1), vaddl_u8(b, b1)))
                    }
                };
                let d = dst.as_mut_ptr().add(off + j * stride);
                let v = if avg { vrhadd_u8(v, vld1_u8(d)) } else { v };
                vst1_u8(d, v);
            }
        }
    }
}

/// Row `y` of a residual block as eight saturated i16s.
#[target_feature(enable = "neon")]
#[inline]
fn residual_row(res: &[i32; 64], y: usize) -> int16x8_t {
    let r = &res[y * 8..y * 8 + 8];
    // SAFETY: `r` is 8 i32s; the loads read i32s 0..4 and 4..8.
    unsafe {
        vcombine_s16(
            vqmovn_s32(vld1q_s32(r.as_ptr())),
            vqmovn_s32(vld1q_s32(r.as_ptr().add(4))),
        )
    }
}

fn add_block_neon(dst: &mut [u8], off: usize, stride: usize, res: &[i32; 64]) {
    // SAFETY: NEON is part of the AArch64 baseline: every AArch64 processor
    // has it.
    unsafe { add_block_neon_impl(dst, off, stride, res) }
}

#[target_feature(enable = "neon")]
fn add_block_neon_impl(dst: &mut [u8], off: usize, stride: usize, res: &[i32; 64]) {
    check_block8(dst.len(), off, stride);
    for y in 0..8 {
        let r = residual_row(res, y);
        // SAFETY: check_block8 proved 8 bytes at off + y * stride are in dst.
        unsafe {
            let p = dst.as_mut_ptr().add(off + y * stride);
            let s = vqaddq_s16(vreinterpretq_s16_u16(vmovl_u8(vld1_u8(p))), r);
            vst1_u8(p, vqmovun_s16(s));
        }
    }
}

fn put_block_neon(dst: &mut [u8], off: usize, stride: usize, res: &[i32; 64]) {
    // SAFETY: NEON is part of the AArch64 baseline: every AArch64 processor
    // has it.
    unsafe { put_block_neon_impl(dst, off, stride, res) }
}

#[target_feature(enable = "neon")]
fn put_block_neon_impl(dst: &mut [u8], off: usize, stride: usize, res: &[i32; 64]) {
    check_block8(dst.len(), off, stride);
    for y in 0..8 {
        let r = residual_row(res, y);
        // SAFETY: check_block8 proved 8 bytes at off + y * stride are in dst.
        unsafe { vst1_u8(dst.as_mut_ptr().add(off + y * stride), vqmovun_s16(r)) };
    }
}

fn sad16_neon(a: Blk, b: Blk, limit: u32) -> u32 {
    // SAFETY: NEON is part of the AArch64 baseline.
    unsafe { sad16_neon_impl(a, b, limit) }
}

#[target_feature(enable = "neon")]
fn sad16_neon_impl(a: Blk, b: Blk, limit: u32) -> u32 {
    a.check16();
    b.check16();
    let rows = |r: std::ops::Range<usize>| {
        let mut acc = vdupq_n_u16(0);
        for j in r {
            // SAFETY: check16 proved 16 bytes at each row's start are in a
            // and in b.
            unsafe {
                let ra = vld1q_u8(a.buf.as_ptr().add(a.off + j * a.stride));
                let rb = vld1q_u8(b.buf.as_ptr().add(b.off + j * b.stride));
                // 8 rows × 2 per lane × 255 = 4080: no u16 overflow.
                acc = vpadalq_u8(acc, vabdq_u8(ra, rb));
            }
        }
        vaddlvq_u16(acc)
    };
    let top = rows(0..8);
    if top >= limit { top } else { top + rows(8..16) }
}
