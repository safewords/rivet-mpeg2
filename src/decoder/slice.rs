//! The slice and macroblock layers (H.262 6.2.4–6.2.6, 7.1–7.6): parsing,
//! inverse quantisation, IDCT, motion vector reconstruction, prediction and
//! reconstruction of one slice.

use super::band::{Band, Refs};
use super::mc::{self, MbPred, PicBuf, Region, View};
use crate::bits::BitReader;
use crate::error::{Result, invalid, unsupported};
use crate::frame::ChromaFormat;
use crate::headers::{BOTTOM_FIELD, FRAME_PICTURE};
use crate::idct::idct;
use crate::tables::{
    MB_BACKWARD, MB_FORWARD, MB_INTRA, MB_PATTERN, MB_QUANT, SCAN, quantiser_scale,
};
use crate::vlc::{self, DCT_EOB, DCT_ESCAPE, Decoders, MB_ESCAPE, MB_STUFFING};

/// Everything a slice needs to know about its picture.
#[derive(Clone, Debug)]
pub(crate) struct Params {
    pub mpeg1: bool,
    pub chroma: ChromaFormat,
    /// Macroblocks per row and rows in this picture (field rows for a field
    /// picture).
    pub mb_width: usize,
    pub mb_height: usize,
    pub vertical_size: u32,
    /// picture_coding_type: 1 I, 2 P, 3 B, 4 D (ISO/IEC 11172-2).
    pub picture_type: u8,
    pub picture_structure: u8,
    pub second_field: bool,
    pub top_field_first: bool,
    pub f_code: [[u8; 2]; 2],
    /// ISO/IEC 11172-2 full_pel_forward_vector / full_pel_backward_vector.
    pub full_pel: [bool; 2],
    pub intra_dc_precision: u8,
    pub frame_pred_frame_dct: bool,
    pub concealment_motion_vectors: bool,
    pub q_scale_type: bool,
    pub intra_vlc_format: bool,
    pub alternate_scan: bool,
    /// The buffer being decoded into.
    pub cur: usize,
    /// Reference frames for forward (0) and backward (1) prediction.
    pub refs: [Option<usize>; 2],
}

impl Params {
    fn frame_picture(&self) -> bool {
        self.picture_structure == FRAME_PICTURE
    }
    /// Parity of a field picture (0 top, 1 bottom).
    fn parity(&self) -> u8 {
        u8::from(self.picture_structure == BOTTOM_FIELD)
    }
    fn block_count(&self) -> usize {
        if self.chroma == ChromaFormat::Yuv422 {
            8
        } else {
            6
        }
    }
}

/// The prediction modes of Tables 6-17 and 6-18.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// Frame picture, frame-based.
    Frame,
    /// Frame picture, field-based.
    FieldInFrame,
    /// Frame picture, dual-prime.
    DualFrame,
    /// Field picture, field-based.
    Field,
    /// Field picture, 16×8.
    Field16x8,
    /// Field picture, dual-prime.
    DualField,
}

impl Kind {
    /// `(motion_vector_count, mv_format == field, dmv)`.
    fn shape(self) -> (usize, bool, bool) {
        match self {
            Kind::Frame => (1, false, false),
            Kind::FieldInFrame => (2, true, false),
            Kind::DualFrame => (1, true, true),
            Kind::Field => (1, true, false),
            Kind::Field16x8 => (2, true, false),
            Kind::DualField => (1, true, true),
        }
    }
}

/// The motion of one macroblock.
#[derive(Clone, Copy, Debug)]
struct Motion {
    /// Bit 0 forward, bit 1 backward.
    dirs: u8,
    kind: Kind,
    /// `motion_vertical_field_select[r][s]`.
    sel: [[u8; 2]; 2],
    /// `vector'[r][s][t]` in half samples (MPEG-1 full-pel vectors doubled).
    mv: [[[i32; 2]; 2]; 2],
    dmv: [i32; 2],
}

/// Per-slice decoding state.
struct State<'a> {
    p: &'a Params,
    d: &'static Decoders,
    qmat: &'a [[u8; 64]; 4],
    quantiser_scale_code: u8,
    dc_pred: [i32; 3],
    /// `PMV[r][s][t]`.
    pmv: [[[i32; 2]; 2]; 2],
    /// Motion of the previous macroblock (B-picture skipped macroblocks
    /// repeat its direction).
    prev_dirs: u8,
}

impl State<'_> {
    fn reset_dc(&mut self) {
        let v = 1 << (7 + self.p.intra_dc_precision);
        self.dc_pred = [v; 3];
    }
}

/// The macroblock row a slice starts in (`vpos` its start code value,
/// `data` the bytes after the start code).
pub(crate) fn slice_row(p: &Params, vpos: u8, data: &[u8]) -> usize {
    let mut row = usize::from(vpos).saturating_sub(1);
    if p.vertical_size > 2800 {
        row += (BitReader::new(data).read(3) as usize) << 7;
    }
    row
}

/// Decodes one slice: `vpos` is the start code value, `data` the bytes after
/// the start code.
pub(crate) fn decode_slice(
    p: &Params,
    qmat: &[[u8; 64]; 4],
    refs: &Refs,
    band: &mut Band,
    vpos: u8,
    data: &[u8],
) -> Result<()> {
    let mut r = BitReader::new(data);
    let mut mb_row = usize::from(vpos) - 1;
    if p.vertical_size > 2800 {
        mb_row += (r.read(3) as usize) << 7;
    }
    let quantiser_scale_code = r.read(5) as u8;
    if quantiser_scale_code == 0 {
        return Err(invalid("quantiser_scale_code 0"));
    }
    if !p.mpeg1 && r.peek(1) == 1 {
        // slice_extension_flag, intra_slice, slice_picture_id_enable,
        // slice_picture_id.
        r.skip(9);
    }
    while r.read_bit() {
        r.skip(8); // extra_information_slice
        if r.overrun() {
            return Err(invalid("slice header cut short"));
        }
    }
    if mb_row >= p.mb_height {
        return Err(invalid(format!(
            "slice row {mb_row} beyond the picture's {}",
            p.mb_height
        )));
    }

    let mut st = State {
        p,
        d: vlc::decoders(),
        qmat,
        quantiser_scale_code,
        dc_pred: [0; 3],
        pmv: [[[0; 2]; 2]; 2],
        prev_dirs: 0,
    };
    st.reset_dc();
    let total = p.mb_width * p.mb_height;
    let mut addr: usize = 0;
    let mut first = true;
    loop {
        // macroblock_address_increment, with escapes (and MPEG-1 stuffing).
        let mut inc = 0usize;
        loop {
            match st.d.mb_address_increment.decode(&mut r) {
                Some(MB_ESCAPE) => inc += 33,
                Some(MB_STUFFING) if p.mpeg1 => {}
                Some(v) if v <= 33 => {
                    inc += v as usize;
                    break;
                }
                _ => return Err(invalid("bad macroblock_address_increment")),
            }
        }
        if first {
            addr = mb_row * p.mb_width + inc - 1;
            first = false;
        } else {
            for _ in 1..inc {
                addr += 1;
                if addr >= total {
                    return Err(invalid("skipped macroblocks run past the picture"));
                }
                skipped_mb(&mut st, refs, band, addr)?;
            }
            addr += 1;
        }
        if addr >= total {
            return Err(invalid("macroblock address beyond the picture"));
        }
        macroblock(&mut st, &mut r, refs, band, addr)?;
        if r.overrun() {
            return Err(invalid("slice data cut short"));
        }
        // The slice ends at the next start code: 23 zero bits.
        if r.bits_left() == 0 || r.peek(23) == 0 {
            break;
        }
    }
    // Only zero stuffing may follow the last macroblock (next_start_code()).
    while r.bits_left() > 0 {
        let n = r.bits_left().min(32) as u32;
        if r.read(n) != 0 {
            return Err(invalid("data after the last macroblock of a slice"));
        }
    }
    Ok(())
}

/// A skipped macroblock (7.6.6).
fn skipped_mb(st: &mut State, refs: &Refs, band: &mut Band, addr: usize) -> Result<()> {
    let p = st.p;
    st.reset_dc();
    let kind = if p.frame_picture() {
        Kind::Frame
    } else {
        Kind::Field
    };
    let parity = p.parity();
    let m = match p.picture_type {
        2 => {
            st.pmv = [[[0; 2]; 2]; 2];
            Motion {
                dirs: 1,
                kind,
                sel: [[parity; 2]; 2],
                mv: [[[0; 2]; 2]; 2],
                dmv: [0; 2],
            }
        }
        3 => {
            let dirs = st.prev_dirs;
            if dirs == 0 {
                return Err(invalid(
                    "skipped macroblock after an intra macroblock in a B-picture",
                ));
            }
            let mut mv = [[[0; 2]; 2]; 2];
            for (s, v) in mv[0].iter_mut().enumerate() {
                *v = scale_full_pel(p, s, st.pmv[0][s]);
            }
            Motion {
                dirs,
                kind,
                sel: [[parity; 2]; 2],
                mv,
                dmv: [0; 2],
            }
        }
        _ => return Err(invalid("skipped macroblock in an I-picture")),
    };
    let mut pred = MbPred::new();
    form_prediction(p, refs, addr, &m, &mut pred);
    band.put(addr, &pred);
    Ok(())
}

fn scale_full_pel(p: &Params, s: usize, v: [i32; 2]) -> [i32; 2] {
    if p.full_pel[s] {
        [v[0] * 2, v[1] * 2]
    } else {
        v
    }
}

fn macroblock(
    st: &mut State,
    r: &mut BitReader,
    refs: &Refs,
    band: &mut Band,
    addr: usize,
) -> Result<()> {
    let p = st.p;
    let d = st.d;
    // A D-picture's only macroblock_type is `1`, intra: Table B.2's code.
    let table = match p.picture_type {
        2 => 1,
        3 => 2,
        _ => 0,
    };
    let flags = d.mb_type[table]
        .decode(r)
        .ok_or_else(|| invalid("bad macroblock_type"))? as u8;
    if p.picture_type == 4 {
        return d_macroblock(st, r, band, addr, flags);
    }
    let intra = flags & MB_INTRA != 0;
    let fwd = flags & MB_FORWARD != 0;
    let bwd = flags & MB_BACKWARD != 0;
    let pattern = flags & MB_PATTERN != 0;

    let mut kind = if p.frame_picture() {
        Kind::Frame
    } else {
        Kind::Field
    };
    if fwd || bwd {
        if p.frame_picture() {
            if !p.frame_pred_frame_dct {
                kind = match r.read(2) {
                    1 => Kind::FieldInFrame,
                    2 => Kind::Frame,
                    3 => Kind::DualFrame,
                    _ => return Err(invalid("frame_motion_type 0")),
                };
            }
        } else {
            kind = match r.read(2) {
                1 => Kind::Field,
                2 => Kind::Field16x8,
                3 => Kind::DualField,
                _ => return Err(invalid("field_motion_type 0")),
            };
        }
    }
    let dct_type =
        p.frame_picture() && !p.frame_pred_frame_dct && (intra || pattern) && r.read_bit();
    if flags & MB_QUANT != 0 {
        st.quantiser_scale_code = r.read(5) as u8;
        if st.quantiser_scale_code == 0 {
            return Err(invalid("quantiser_scale_code 0"));
        }
    }

    let mut m = Motion {
        dirs: u8::from(fwd) | (u8::from(bwd) << 1),
        kind,
        sel: [[0; 2]; 2],
        mv: [[[0; 2]; 2]; 2],
        dmv: [0; 2],
    };
    if matches!(kind, Kind::DualFrame | Kind::DualField) && (bwd || p.picture_type != 2) {
        return Err(invalid("dual-prime outside a P-picture"));
    }
    if fwd || (intra && p.concealment_motion_vectors) {
        motion_vectors(st, r, &mut m, 0)?;
    }
    if bwd {
        motion_vectors(st, r, &mut m, 1)?;
    }
    if intra && p.concealment_motion_vectors {
        r.skip(1); // marker_bit
    }

    let block_count = p.block_count();
    let cbp: u32 = if pattern {
        let v = d
            .cbp
            .decode(r)
            .ok_or_else(|| invalid("bad coded_block_pattern"))? as u32;
        if p.chroma == ChromaFormat::Yuv422 {
            (v << 2) | r.read(2)
        } else {
            v
        }
    } else if intra {
        (1 << block_count) - 1
    } else {
        0
    };

    // Predictor bookkeeping (7.6.3.4, 7.2.1).
    if intra {
        if !p.concealment_motion_vectors {
            st.pmv = [[[0; 2]; 2]; 2];
        }
        st.prev_dirs = 0;
    } else {
        st.reset_dc();
        if p.picture_type == 2 && !fwd {
            // No MC in a P-picture: zero vector, same-parity field.
            st.pmv = [[[0; 2]; 2]; 2];
            m.dirs = 1;
            m.kind = if p.frame_picture() {
                Kind::Frame
            } else {
                Kind::Field
            };
            m.sel = [[p.parity(); 2]; 2];
        }
        st.prev_dirs = m.dirs;
    }

    // Prediction, then the coded blocks on top.
    let mut pred = MbPred::new();
    if !intra {
        form_prediction(p, refs, addr, &m, &mut pred);
    }
    let qs = if p.mpeg1 {
        i32::from(st.quantiser_scale_code)
    } else {
        quantiser_scale(p.q_scale_type, st.quantiser_scale_code)
    };
    for i in 0..block_count {
        if cbp & (1 << (block_count - 1 - i)) == 0 {
            continue;
        }
        let mut blk = [0i32; 64];
        let cc = if i < 4 { 0 } else { 1 + (i & 1) };
        // Table 7-5: chroma uses its own matrices only beyond 4:2:0.
        let w = match (intra, cc > 0 && p.chroma != ChromaFormat::Yuv420) {
            (true, false) => &st.qmat[0],
            (false, false) => &st.qmat[1],
            (true, true) => &st.qmat[2],
            (false, true) => &st.qmat[3],
        };
        read_block(st, r, intra, cc, qs, w, &mut blk)?;
        idct(&mut blk);
        add_block(p, &mut pred, i, dct_type, intra, &blk);
    }
    band.put(addr, &pred);
    Ok(())
}

/// A macroblock of an ISO/IEC 11172-2 D-picture: intra, each block only its
/// DC coefficient (no end of block), then end_of_macroblock, a `1`.
fn d_macroblock(
    st: &mut State,
    r: &mut BitReader,
    band: &mut Band,
    addr: usize,
    flags: u8,
) -> Result<()> {
    let p = st.p;
    if flags != MB_INTRA {
        return Err(invalid("non-intra macroblock in a D-picture"));
    }
    let mut out = MbPred::new();
    for i in 0..p.block_count() {
        let cc = if i < 4 { 0 } else { 1 + (i & 1) };
        let size = st.d.dc_size[usize::from(cc > 0)]
            .decode(r)
            .ok_or_else(|| invalid("bad dct_dc_size"))? as u32;
        let diff = if size == 0 {
            0
        } else {
            let v = r.read(size) as i32;
            if v >= 1 << (size - 1) {
                v
            } else {
                v + 1 - (1 << size)
            }
        };
        let dc = (st.dc_pred[cc] + diff).clamp(0, 255);
        st.dc_pred[cc] = dc;
        let mut blk = [0i32; 64];
        blk[0] = dc * 8;
        idct(&mut blk);
        add_block(p, &mut out, i, false, true, &blk);
    }
    if !r.read_bit() {
        return Err(invalid("end_of_macroblock is not 1"));
    }
    band.put(addr, &out);
    Ok(())
}

/// motion_vectors(s) (6.2.5.2) and the reconstruction of 7.6.3.
fn motion_vectors(st: &mut State, r: &mut BitReader, m: &mut Motion, s: usize) -> Result<()> {
    // Concealment vectors of an intra macroblock: frame-based in a frame
    // picture, field-based in a field picture (Tables 7-9, 7-10, note a).
    let (count, field_format, dmv) = m.kind.shape();
    if count == 1 {
        if field_format && !dmv {
            m.sel[0][s] = r.read(1) as u8;
        }
        motion_vector(st, r, m, 0, s, field_format, dmv)?;
        // Table 7-9 / 7-10: one vector updates both predictors.
        st.pmv[1][s] = st.pmv[0][s];
    } else {
        m.sel[0][s] = r.read(1) as u8;
        motion_vector(st, r, m, 0, s, field_format, dmv)?;
        m.sel[1][s] = r.read(1) as u8;
        motion_vector(st, r, m, 1, s, field_format, dmv)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn motion_vector(
    st: &mut State,
    r: &mut BitReader,
    m: &mut Motion,
    rr: usize,
    s: usize,
    field_format: bool,
    dmv: bool,
) -> Result<()> {
    let p = st.p;
    let d = st.d;
    for t in 0..2 {
        let code = i32::from(
            d.motion_code
                .decode(r)
                .ok_or_else(|| invalid("bad motion_code"))?,
        );
        let f_code = p.f_code[s][t];
        if !(1..=9).contains(&f_code) {
            return Err(invalid(format!("f_code {f_code} used by a motion vector")));
        }
        let r_size = u32::from(f_code - 1);
        let residual = if r_size != 0 && code != 0 {
            r.read(r_size) as i32
        } else {
            0
        };
        if dmv {
            m.dmv[t] = i32::from(
                d.dmvector
                    .decode(r)
                    .ok_or_else(|| invalid("bad dmvector"))?,
            );
        }
        let f = 1i32 << r_size;
        let high = 16 * f - 1;
        let low = -16 * f;
        let range = 32 * f;
        let delta = if f == 1 || code == 0 {
            code
        } else {
            let dlt = (code.abs() - 1) * f + residual + 1;
            if code < 0 { -dlt } else { dlt }
        };
        let field_in_frame = field_format && t == 1 && p.frame_picture();
        let mut prediction = st.pmv[rr][s][t];
        if field_in_frame {
            prediction >>= 1; // DIV 2
        }
        let mut v = prediction + delta;
        if v < low {
            v += range;
        }
        if v > high {
            v -= range;
        }
        st.pmv[rr][s][t] = if field_in_frame { v * 2 } else { v };
        m.mv[rr][s][t] = if p.full_pel[s] { v * 2 } else { v };
    }
    Ok(())
}

/// Reads one block's coefficients and inverse-quantises them (7.2–7.4),
/// leaving `F[v][u]` in raster order in `blk`.
fn read_block(
    st: &mut State,
    r: &mut BitReader,
    intra: bool,
    cc: usize,
    qs: i32,
    w: &[u8; 64],
    blk: &mut [i32; 64],
) -> Result<()> {
    let p = st.p;
    let d = st.d;
    let scan = &SCAN[usize::from(p.alternate_scan)];
    let mut sum: i32 = 0;
    let mut n: usize;
    let table;
    if intra {
        let size = d.dc_size[usize::from(cc > 0)]
            .decode(r)
            .ok_or_else(|| invalid("bad dct_dc_size"))? as u32;
        let diff = if size == 0 {
            0
        } else {
            let v = r.read(size) as i32;
            if v >= 1 << (size - 1) {
                v
            } else {
                v + 1 - (1 << size)
            }
        };
        let dc = st.dc_pred[cc] + diff;
        let max = (1 << (8 + p.intra_dc_precision)) - 1;
        if !(0..=max).contains(&dc) {
            return Err(invalid("intra DC out of range"));
        }
        st.dc_pred[cc] = dc;
        let f = dc << (3 - p.intra_dc_precision);
        blk[0] = f;
        sum = f;
        n = 1;
        table = usize::from(p.intra_vlc_format);
    } else {
        n = 0;
        table = 0;
    }
    let mut first = !intra;
    loop {
        let (run, level);
        if first && r.peek(1) == 1 {
            // Table B.14 note 3: `1s` is run 0, level ±1 for the first
            // coefficient of a non-intra block.
            r.skip(1);
            run = 0;
            level = if r.read_bit() { -1 } else { 1 };
        } else {
            let v = d.dct[table]
                .decode(r)
                .ok_or_else(|| invalid("bad DCT coefficient code"))?;
            if v == DCT_EOB {
                if first {
                    return Err(invalid("end of block as the only code of a block"));
                }
                break;
            } else if v == DCT_ESCAPE {
                run = r.read(6) as usize;
                level = if p.mpeg1 {
                    let l = r.read(8) as i32;
                    match l {
                        0 => r.read(8) as i32,
                        128 => r.read(8) as i32 - 256,
                        l if l > 128 => l - 256,
                        l => l,
                    }
                } else {
                    let l = r.read(12) as i32;
                    if l == 0 || l == 2048 {
                        return Err(invalid("forbidden escape level"));
                    }
                    if l > 2048 { l - 4096 } else { l }
                };
                if level == 0 {
                    return Err(invalid("escape level 0"));
                }
            } else {
                let (rn, lv) = vlc::split_run_level(v);
                run = rn;
                level = if r.read_bit() { -lv } else { lv };
            }
        }
        first = false;
        n += run;
        if n >= 64 {
            return Err(invalid("DCT coefficients run past the block"));
        }
        let pos = scan[n] as usize;
        let wq = i32::from(w[pos]) * qs;
        let f = if p.mpeg1 {
            // ISO/IEC 11172-2 2.4.4: odd reconstruction ("oddification")
            // instead of mismatch control (H.262 D.9.1).
            let mut f = if intra {
                (2 * level * wq) / 16
            } else {
                ((2 * level + level.signum()) * wq) / 16
            };
            if f & 1 == 0 && f != 0 {
                f -= f.signum();
            }
            f.clamp(-2048, 2047)
        } else {
            let k = if intra { 0 } else { level.signum() };
            ((2 * level + k) * wq / 32).clamp(-2048, 2047)
        };
        blk[pos] = f;
        sum += f;
        n += 1;
        if r.overrun() {
            return Err(invalid("block cut short"));
        }
    }
    // Mismatch control (7.4.4).
    if !p.mpeg1 && sum & 1 == 0 {
        blk[63] ^= 1;
    }
    Ok(())
}

/// Adds block `i`'s IDCT output to the prediction (or, intra, stores it),
/// saturating to [0, 255] (7.6.8).
fn add_block(
    p: &Params,
    pred: &mut MbPred,
    i: usize,
    dct_type: bool,
    intra: bool,
    blk: &[i32; 64],
) {
    let (plane, stride, x0, y0, step): (&mut [u8], usize, usize, usize, usize) = if i < 4 {
        let (y0, step) = if dct_type {
            (i >> 1, 2)
        } else {
            ((i >> 1) * 8, 1)
        };
        (&mut pred.y[..], 16, (i & 1) * 8, y0, step)
    } else {
        let c = (i - 4) & 1;
        let half = (i - 4) >> 1; // 4:2:2: blocks 6 and 7 are the lower half
        let (y0, step) = if dct_type && p.chroma == ChromaFormat::Yuv422 {
            (half, 2)
        } else {
            (half * 8, 1)
        };
        (&mut pred.c[c][..], 8, 0, y0, step)
    };
    let d = crate::dsp::dsp();
    let kernel = if intra { d.put_block } else { d.add_block };
    kernel(plane, y0 * stride + x0, step * stride, blk);
}

/// Forms the prediction of a non-intra macroblock (7.6.2–7.6.7).
fn form_prediction(p: &Params, refs: &Refs, addr: usize, m: &Motion, pred: &mut MbPred) {
    let mbx = (addr % p.mb_width) as i32;
    let mby = (addr / p.mb_width) as i32;
    let x = mbx * 16;
    let parity = p.parity();
    // The reference frame for direction s; in a P field picture that is the
    // second field of its frame, the field of opposite parity is the first
    // field of the frame being decoded (7.6.2.1).
    let field_ref = |s: usize, field: u8| -> Option<&PicBuf> {
        if s == 0 && p.picture_type == 2 && p.second_field && field != parity {
            refs.cur
        } else {
            refs.dir[s].or(refs.cur)
        }
    };
    let mut first = true;
    for s in 0..2 {
        if m.dirs & (1 << s) == 0 {
            continue;
        }
        let avg = !first;
        first = false;
        match m.kind {
            Kind::Frame => {
                let r = Region {
                    x,
                    y: mby * 16,
                    h: 16,
                    dst_row: 0,
                    dst_parity: 0,
                    dst_step: 1,
                    mv: m.mv[0][s],
                    avg,
                };
                if let Some(b) = field_ref(s, 0) {
                    mc::predict(b, View::Frame, p.chroma, &r, pred);
                }
            }
            Kind::FieldInFrame => {
                for f in 0..2 {
                    let sel = m.sel[f][s];
                    let r = Region {
                        x,
                        y: mby * 8,
                        h: 8,
                        dst_row: 0,
                        dst_parity: f,
                        dst_step: 2,
                        mv: m.mv[f][s],
                        avg,
                    };
                    if let Some(b) = field_ref(s, sel) {
                        mc::predict(b, View::Field(sel), p.chroma, &r, pred);
                    }
                }
            }
            Kind::DualFrame => {
                let v = m.mv[0][0];
                for f in 0..2u8 {
                    // Table 7-11: m[parity_ref][parity_pred]; Table 7-12: e.
                    let (mm, e) = match (f, p.top_field_first) {
                        (0, true) => (1, -1),
                        (0, false) => (3, -1),
                        (_, true) => (3, 1),
                        (_, false) => (1, 1),
                    };
                    let dv = [
                        mc::div2_round(v[0] * mm) + m.dmv[0],
                        mc::div2_round(v[1] * mm) + e + m.dmv[1],
                    ];
                    let refi = refs.dir[0].or(refs.cur);
                    let base = Region {
                        x,
                        y: mby * 8,
                        h: 8,
                        dst_row: 0,
                        dst_parity: f as usize,
                        dst_step: 2,
                        mv: v,
                        avg: false,
                    };
                    if let Some(b) = refi {
                        mc::predict(b, View::Field(f), p.chroma, &base, pred);
                    }
                    let opp = Region {
                        mv: dv,
                        avg: true,
                        ..base
                    };
                    if let Some(b) = refi {
                        mc::predict(b, View::Field(1 - f), p.chroma, &opp, pred);
                    }
                }
            }
            Kind::Field => {
                let sel = m.sel[0][s];
                let r = Region {
                    x,
                    y: mby * 16,
                    h: 16,
                    dst_row: 0,
                    dst_parity: 0,
                    dst_step: 1,
                    mv: m.mv[0][s],
                    avg,
                };
                if let Some(b) = field_ref(s, sel) {
                    mc::predict(b, View::Field(sel), p.chroma, &r, pred);
                }
            }
            Kind::Field16x8 => {
                for half in 0..2 {
                    let sel = m.sel[half][s];
                    let r = Region {
                        x,
                        y: mby * 16 + half as i32 * 8,
                        h: 8,
                        dst_row: half * 8,
                        dst_parity: 0,
                        dst_step: 1,
                        mv: m.mv[half][s],
                        avg,
                    };
                    if let Some(b) = field_ref(s, sel) {
                        mc::predict(b, View::Field(sel), p.chroma, &r, pred);
                    }
                }
            }
            Kind::DualField => {
                let v = m.mv[0][0];
                let e = if parity == 0 { -1 } else { 1 };
                let dv = [
                    mc::div2_round(v[0]) + m.dmv[0],
                    mc::div2_round(v[1]) + e + m.dmv[1],
                ];
                let same = Region {
                    x,
                    y: mby * 16,
                    h: 16,
                    dst_row: 0,
                    dst_parity: 0,
                    dst_step: 1,
                    mv: v,
                    avg: false,
                };
                if let Some(b) = field_ref(0, parity) {
                    mc::predict(b, View::Field(parity), p.chroma, &same, pred);
                }
                let opp = Region {
                    mv: dv,
                    avg: true,
                    ..same
                };
                if let Some(b) = field_ref(0, 1 - parity) {
                    mc::predict(b, View::Field(1 - parity), p.chroma, &opp, pred);
                }
            }
        }
    }
}

/// Refuses what the slice layer does not implement.
pub(crate) fn check_supported(chroma: ChromaFormat) -> Result<()> {
    if chroma == ChromaFormat::Yuv444 {
        return Err(unsupported("4:4:4 chroma"));
    }
    Ok(())
}
