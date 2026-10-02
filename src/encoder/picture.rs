//! Coding one progressive frame picture: mode decision, transform,
//! quantisation, the macroblock layer, and the reconstruction the next
//! pictures predict from.

use super::motion;
use crate::bits::BitWriter;
use crate::decoder::mc::{MbPred, PicBuf};
use crate::idct::{fdct, idct};
use crate::tables::{
    DEFAULT_INTRA_MATRIX, DEFAULT_NON_INTRA_MATRIX, MB_BACKWARD, MB_FORWARD, MB_INTRA, MB_PATTERN, SCAN,
    quantiser_scale,
};
use crate::vlc::{Encoders, encoders};

/// The settings one picture is coded with.
pub(crate) struct PictureSettings {
    /// 1 I, 2 P, 3 B.
    pub picture_type: u8,
    pub quantiser_scale_code: u8,
    pub q_scale_type: bool,
    pub intra_vlc_format: bool,
    pub alternate_scan: bool,
    pub intra_dc_precision: u8,
    /// The f_code of every vector component.
    pub f_code: u8,
    /// Motion search range in whole samples.
    pub search_range: i32,
}

/// A macroblock's coded data.
struct Mb {
    intra: bool,
    /// Bit 0 forward, bit 1 backward (non-intra).
    dirs: u8,
    mv: [[i32; 2]; 2],
    /// Quantised coefficients of the six blocks, raster order.
    qf: [[i32; 64]; 6],
    cbp: u8,
    /// The prediction (non-intra).
    pred: MbPred,
}

struct SliceState {
    dc_pred: [i32; 3],
    pmv: [[i32; 2]; 2],
    prev_dirs: u8,
}

/// Codes the slices of one picture into `w`, reconstructing into `recon`
/// when the picture is a reference.
#[allow(clippy::needless_range_loop)] // mbx is a position, not just an index
pub(crate) fn code_picture(
    s: &PictureSettings,
    src: &PicBuf,
    fwd: Option<&PicBuf>,
    bwd: Option<&PicBuf>,
    mut recon: Option<&mut PicBuf>,
    w: &mut BitWriter,
) {
    let e = encoders();
    let mb_width = src.width / 16;
    let mb_height = src.height / 16;
    let qs = quantiser_scale(s.q_scale_type, s.quantiser_scale_code);
    let f = 1i32 << (s.f_code - 1);
    let (low, high) = (-16 * f, 16 * f - 1);
    let dc_reset = 1 << (7 + s.intra_dc_precision);
    let mut row_mvs = vec![[[0i32; 2]; 2]; mb_width];

    for mby in 0..mb_height {
        w.start_code(mby as u8 + 1);
        w.put(5, u32::from(s.quantiser_scale_code));
        w.put_bit(false); // extra_bit_slice
        let mut st = SliceState { dc_pred: [dc_reset; 3], pmv: [[0; 2]; 2], prev_dirs: 0 };
        let mut skipped = 0u32;
        let mut left_mv = [[0i32; 2]; 2];
        for mbx in 0..mb_width {
            let (x, y) = (mbx * 16, mby * 16);
            let mut mb = decide(s, src, fwd, bwd, x, y, qs, [left_mv, row_mvs[mbx]], &st, low, high);
            let interior = mbx != 0 && mbx != mb_width - 1;

            // Skipped macroblocks (7.6.6): no coefficients, and the
            // prediction the decoder will infer is the one chosen.
            if !mb.intra && mb.cbp == 0 && interior {
                let skip = match s.picture_type {
                    2 => mb.dirs == 1 && mb.mv[0] == [0, 0],
                    _ => {
                        st.prev_dirs == mb.dirs
                            && (0..2).all(|d| mb.dirs & (1 << d) == 0 || mb.mv[d] == st.pmv[d])
                    }
                };
                if skip {
                    skipped += 1;
                    st.dc_pred = [dc_reset; 3];
                    if s.picture_type == 2 {
                        st.pmv = [[0; 2]; 2];
                    }
                    if let Some(r) = recon.as_deref_mut() {
                        write_mb(r, mbx, mby, &mb.pred);
                    }
                    left_mv = [mb.mv[0], mb.mv[1]];
                    row_mvs[mbx] = left_mv;
                    continue;
                }
            }

            e.put_mb_address_increment(w, skipped + 1);
            skipped = 0;
            write_macroblock(s, e, w, &mut mb, &mut st, dc_reset, low, high);
            if let Some(r) = recon.as_deref_mut() {
                let rec = reconstruct(s, &mb, qs);
                write_mb(r, mbx, mby, &rec);
            }
            left_mv = if mb.intra { [[0; 2]; 2] } else { mb.mv };
            row_mvs[mbx] = left_mv;
        }
    }
}

/// Chooses the coding of one macroblock and quantises its residual.
#[allow(clippy::too_many_arguments)]
fn decide(
    s: &PictureSettings,
    src: &PicBuf,
    fwd: Option<&PicBuf>,
    bwd: Option<&PicBuf>,
    x: usize,
    y: usize,
    qs: i32,
    neighbours: [[[i32; 2]; 2]; 2],
    st: &SliceState,
    low: i32,
    high: i32,
) -> Mb {
    let mut mb = Mb { intra: true, dirs: 0, mv: [[0; 2]; 2], qf: [[0; 64]; 6], cbp: 0, pred: MbPred::new() };
    if s.picture_type != 1 {
        let intra_cost = intra_activity(src, x, y);
        let mut best: Option<(u32, u8, [[i32; 2]; 2])> = None;
        let mut found = [[0i32; 2]; 2];
        let mut sads = [u32::MAX; 2];
        for (d, refp) in [fwd, bwd].into_iter().enumerate() {
            let Some(refp) = refp else { continue };
            if s.picture_type == 2 && d == 1 {
                continue;
            }
            let cands = [neighbours[0][d], neighbours[1][d], st.pmv[d]];
            let (mut mv, mut sad) = motion::search(src, refp, x, y, s.search_range, low, high, &cands);
            // Prefer the zero vector (and so skipped macroblocks) unless the
            // search found clearly better; in B-pictures, prefer the
            // predictor (a skip repeats it).
            let mut p = MbPred::new();
            let pref = if s.picture_type == 2 { [0, 0] } else { st.pmv[d] };
            if mv != pref && vector_fits(pref, x, y, src, low, high) {
                motion::predict(refp, x, y, pref, false, &mut p);
                let sp = motion::sad_pred(src, x, y, &p.y);
                if sp <= sad + 64 {
                    mv = pref;
                    sad = sp;
                }
            }
            found[d] = mv;
            sads[d] = sad;
            let cost = sad + if mv == pref { 0 } else { 32 };
            if best.is_none_or(|b| cost < b.0) {
                let mut v = [[0; 2]; 2];
                v[d] = mv;
                best = Some((cost, 1 << d, v));
            }
        }
        if let (3, Some(f), Some(b)) = (s.picture_type, fwd, bwd) {
            let mut p = MbPred::new();
            motion::predict(f, x, y, found[0], false, &mut p);
            motion::predict(b, x, y, found[1], true, &mut p);
            let sad = motion::sad_pred(src, x, y, &p.y) + 48;
            if best.is_none_or(|b| sad < b.0) {
                best = Some((sad, 3, found));
            }
        }
        if let Some((cost, dirs, mv)) = best
            && cost <= intra_cost + 256
        {
            mb.intra = false;
            mb.dirs = dirs;
            mb.mv = mv;
            let mut first = true;
            for (d, (&v, refp)) in mv.iter().zip([fwd, bwd]).enumerate() {
                if dirs & (1 << d) == 0 {
                    continue;
                }
                motion::predict(refp.expect("reference"), x, y, v, !first, &mut mb.pred);
                first = false;
            }
        }
    }

    // Transform and quantise the six blocks.
    let intra_dc_mult = 8 >> s.intra_dc_precision;
    for b in 0..6 {
        let mut blk = [0i32; 64];
        let (plane, px, py, stride) = block_origin(src, b, x, y);
        for j in 0..8 {
            for i in 0..8 {
                let v = i32::from(src.planes[plane][(py + j) * stride + px + i]);
                blk[j * 8 + i] = if mb.intra { v } else { v - i32::from(pred_sample(&mb.pred, b, i, j)) };
            }
        }
        let f = fdct(&blk);
        let q = &mut mb.qf[b];
        if mb.intra {
            let dc_max = (1 << (8 + s.intra_dc_precision)) - 1;
            q[0] = ((f[0] + intra_dc_mult / 2) / intra_dc_mult).clamp(0, dc_max);
            for k in 1..64 {
                let wq = i32::from(DEFAULT_INTRA_MATRIX[k]) * qs;
                let a = (f[k].abs() * 16 + wq * 3 / 8) / wq;
                q[k] = (a * f[k].signum()).clamp(-2047, 2047);
            }
        } else {
            let mut any = false;
            for k in 0..64 {
                let wq = i32::from(DEFAULT_NON_INTRA_MATRIX[k]) * qs;
                let a = (f[k].abs() * 16) / wq;
                q[k] = (a * f[k].signum()).clamp(-2047, 2047);
                any |= q[k] != 0;
            }
            if any {
                mb.cbp |= 1 << (5 - b);
            }
        }
    }
    mb
}

fn vector_fits(mv: [i32; 2], x: usize, y: usize, src: &PicBuf, low: i32, high: i32) -> bool {
    if mv.iter().any(|&v| v < low || v > high) {
        return false;
    }
    let ix = x as i32 + (mv[0] >> 1);
    let iy = y as i32 + (mv[1] >> 1);
    ix >= 0 && iy >= 0 && ix + 16 + (mv[0] & 1) <= src.width as i32 && iy + 16 + (mv[1] & 1) <= src.height as i32
}

/// Sum of absolute deviations of the luma from its mean: the cost an intra
/// macroblock is compared with.
fn intra_activity(src: &PicBuf, x: usize, y: usize) -> u32 {
    let w = src.width;
    let mut sum = 0u32;
    for j in 0..16 {
        for i in 0..16 {
            sum += u32::from(src.planes[0][(y + j) * w + x + i]);
        }
    }
    let mean = (sum + 128) / 256;
    let mut dev = 0u32;
    for j in 0..16 {
        for i in 0..16 {
            dev += u32::from(src.planes[0][(y + j) * w + x + i]).abs_diff(mean);
        }
    }
    dev
}

/// `(plane, x, y, stride)` of block `b` of the macroblock at (`x`, `y`).
fn block_origin(src: &PicBuf, b: usize, x: usize, y: usize) -> (usize, usize, usize, usize) {
    if b < 4 {
        (0, x + (b & 1) * 8, y + (b >> 1) * 8, src.width)
    } else {
        (b - 3, x / 2, y / 2, src.cwidth)
    }
}

fn pred_sample(p: &MbPred, b: usize, i: usize, j: usize) -> u8 {
    if b < 4 { p.y[((b >> 1) * 8 + j) * 16 + (b & 1) * 8 + i] } else { p.c[b - 4][j * 8 + i] }
}

/// Writes one coded macroblock (after its address increment).
#[allow(clippy::too_many_arguments)]
fn write_macroblock(
    s: &PictureSettings,
    e: &Encoders,
    w: &mut BitWriter,
    mb: &mut Mb,
    st: &mut SliceState,
    dc_reset: i32,
    low: i32,
    high: i32,
) {
    let pic = usize::from(s.picture_type - 1);
    if mb.intra {
        e.put_mb_type(w, pic, MB_INTRA);
        st.pmv = [[0; 2]; 2];
        st.prev_dirs = 0;
        let table = usize::from(s.intra_vlc_format);
        for b in 0..6 {
            let cc = if b < 4 { 0 } else { b - 3 };
            let q = &mb.qf[b];
            e.put_dc(w, cc > 0, q[0] - st.dc_pred[cc]);
            st.dc_pred[cc] = q[0];
            write_coefficients(s, e, w, q, 1, table, false);
        }
        return;
    }
    st.dc_pred = [dc_reset; 3];
    let pattern = if mb.cbp != 0 { MB_PATTERN } else { 0 };
    // A P macroblock with a zero vector and coefficients is "No MC, coded".
    let no_mc = s.picture_type == 2 && mb.mv[0] == [0, 0] && mb.cbp != 0;
    let mut flags = pattern;
    if !no_mc {
        if mb.dirs & 1 != 0 {
            flags |= MB_FORWARD;
        }
        if mb.dirs & 2 != 0 {
            flags |= MB_BACKWARD;
        }
    }
    e.put_mb_type(w, pic, flags);
    if no_mc {
        st.pmv = [[0; 2]; 2];
    } else {
        for d in 0..2 {
            if mb.dirs & (1 << d) == 0 {
                continue;
            }
            for t in 0..2 {
                put_vector_component(e, w, mb.mv[d][t] - st.pmv[d][t], s.f_code, low, high);
            }
            st.pmv[d] = mb.mv[d];
        }
    }
    st.prev_dirs = mb.dirs;
    if mb.cbp != 0 {
        e.put_cbp(w, mb.cbp);
        for b in 0..6 {
            if mb.cbp & (1 << (5 - b)) != 0 {
                write_coefficients(s, e, w, &mb.qf[b], 0, 0, true);
            }
        }
    }
}

/// motion_code and motion_residual for one component (the inverse of
/// 7.6.3.1).
fn put_vector_component(e: &Encoders, w: &mut BitWriter, delta: i32, f_code: u8, low: i32, high: i32) {
    let r_size = u32::from(f_code - 1);
    let f = 1i32 << r_size;
    let range = 32 * f;
    let mut d = delta;
    if d < low {
        d += range;
    }
    if d > high {
        d -= range;
    }
    if d == 0 || f == 1 {
        e.put_motion_code(w, d);
        return;
    }
    let a = d.abs() - 1;
    let code = a / f + 1;
    let residual = a % f;
    e.put_motion_code(w, if d < 0 { -code } else { code });
    w.put(r_size, residual as u32);
}

/// The run/level codes of one block from scan position `start`, and the
/// end of block.
fn write_coefficients(
    s: &PictureSettings,
    e: &Encoders,
    w: &mut BitWriter,
    q: &[i32; 64],
    start: usize,
    table: usize,
    non_intra: bool,
) {
    let scan = &SCAN[usize::from(s.alternate_scan)];
    let mut run = 0;
    let mut first = non_intra;
    for &pos in &scan[start..] {
        let v = q[pos as usize];
        if v == 0 {
            run += 1;
            continue;
        }
        e.put_dct(w, table, run, v, first);
        first = false;
        run = 0;
    }
    e.put_eob(w, table);
}

/// The macroblock as a decoder will reconstruct it: dequantisation with
/// mismatch control (7.4), the IDCT, prediction plus residual (7.6.8).
fn reconstruct(s: &PictureSettings, mb: &Mb, qs: i32) -> MbPred {
    let mut out = MbPred::new();
    let intra_dc_mult = 8 >> s.intra_dc_precision;
    for b in 0..6 {
        let coded = mb.intra || mb.cbp & (1 << (5 - b)) != 0;
        let mut f = [0i32; 64];
        if coded {
            let q = &mb.qf[b];
            let mut sum = 0;
            for k in 0..64 {
                let v = if mb.intra && k == 0 {
                    q[0] * intra_dc_mult
                } else if mb.intra {
                    (2 * q[k] * i32::from(DEFAULT_INTRA_MATRIX[k]) * qs / 32).clamp(-2048, 2047)
                } else {
                    ((2 * q[k] + q[k].signum()) * i32::from(DEFAULT_NON_INTRA_MATRIX[k]) * qs / 32)
                        .clamp(-2048, 2047)
                };
                f[k] = v;
                sum += v;
            }
            if sum & 1 == 0 {
                f[63] ^= 1;
            }
            idct(&mut f);
        }
        for j in 0..8 {
            for i in 0..8 {
                let p = if mb.intra { 0 } else { i32::from(pred_sample(&mb.pred, b, i, j)) };
                let v = (p + f[j * 8 + i]).clamp(0, 255) as u8;
                if b < 4 {
                    out.y[((b >> 1) * 8 + j) * 16 + (b & 1) * 8 + i] = v;
                } else {
                    out.c[b - 4][j * 8 + i] = v;
                }
            }
        }
    }
    out
}

fn write_mb(r: &mut PicBuf, mbx: usize, mby: usize, mb: &MbPred) {
    let w = r.width;
    for j in 0..16 {
        let o = (mby * 16 + j) * w + mbx * 16;
        r.planes[0][o..o + 16].copy_from_slice(&mb.y[j * 16..j * 16 + 16]);
    }
    let cw = r.cwidth;
    for c in 0..2 {
        for j in 0..8 {
            let o = (mby * 8 + j) * cw + mbx * 8;
            r.planes[c + 1][o..o + 8].copy_from_slice(&mb.c[c][j * 8..j * 8 + 8]);
        }
    }
}
