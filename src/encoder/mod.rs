//! The encoder: a Main Profile 4:2:0 elementary stream of progressive frame
//! pictures, I, P and B.

mod motion;
mod picture;

use crate::bits::BitWriter;
use crate::decoder::mc::PicBuf;
use crate::error::{Result, config};
use crate::frame::{ChromaFormat, Frame};
use crate::headers::*;
use crate::tables::frame_rate_value;
use picture::{PictureSettings, code_picture};

/// How the encoder picks its quantiser.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RateControl {
    /// One quantiser_scale_code (1–31) for every picture: constant quality,
    /// whatever the size.
    ConstantQuantiser(u8),
    /// A target bit rate in bits per second: the quantiser of each picture
    /// is set from the bits the previous pictures of its type spent (the
    /// picture-level bit allocation of the MPEG-2 Test Model), so the
    /// average rate tracks the target over a group of pictures. No VBV
    /// model: the stream signals variable bit rate (vbv_delay 0xFFFF).
    Bitrate(u32),
}

/// Encoder settings. [`EncoderConfig::new`] gives defaults for everything
/// but the size.
#[derive(Debug, Clone, PartialEq)]
pub struct EncoderConfig {
    /// Luma width (1–4095; frames are padded to whole macroblocks
    /// internally).
    pub width: u32,
    /// Luma height (1–4095).
    pub height: u32,
    /// Frame rate `(numerator, denominator)`: one of H.262 Table 6-4's
    /// (24000/1001, 24, 25, 30000/1001, 30, 50, 60000/1001, 60) or one
    /// reachable from them with frame_rate_extension_n / _d.
    pub frame_rate: (u32, u32),
    /// `aspect_ratio_information`: 1 square samples, 2 4:3, 3 16:9,
    /// 4 2.21:1 display aspect ratio.
    pub aspect_ratio_information: u8,
    /// Frames per group of pictures (an I-picture starts each).
    pub gop_size: u32,
    /// B-pictures between reference pictures (0 for I/P only).
    pub b_frames: u32,
    /// The quantiser.
    pub rate_control: RateControl,
    /// Motion search range in whole samples (sets f_code).
    pub search_range: u32,
    /// Use Table B.15 for intra blocks (intra_vlc_format).
    pub intra_vlc_format: bool,
    /// Use the alternate scan (alternate_scan).
    pub alternate_scan: bool,
    /// Use the non-linear quantiser scale (q_scale_type).
    pub q_scale_type: bool,
    /// intra_dc_precision: 0–2 (8 to 10 bits) in Main Profile.
    pub intra_dc_precision: u8,
    /// Threads coding a picture's macroblock rows (0: one per available
    /// core). They are started once, with the encoder, and kept. The stream
    /// is the same, bit for bit, for every setting.
    pub threads: usize,
}

impl EncoderConfig {
    /// Defaults: 25 frames/s, square samples, 12-frame GOPs with two
    /// B-pictures between references, quantiser_scale_code 4, ±32 samples
    /// of motion search, Table B.15 for intra blocks, zigzag scan, linear
    /// quantiser scale, 8-bit intra DC, one thread per core.
    pub fn new(width: u32, height: u32) -> EncoderConfig {
        EncoderConfig {
            width,
            height,
            frame_rate: (25, 1),
            aspect_ratio_information: 1,
            gop_size: 12,
            b_frames: 2,
            rate_control: RateControl::ConstantQuantiser(4),
            search_range: 32,
            intra_vlc_format: true,
            alternate_scan: false,
            q_scale_type: false,
            intra_dc_precision: 0,
            threads: 0,
        }
    }
}

/// Picture-level rate control (Test Model 5's bit allocation).
struct Rate {
    picture_bits: f64,
    /// Complexity (bits × quantiser_scale) of the last I, P and B picture.
    x: [f64; 3],
    /// Bits left for the current group of pictures.
    remaining: f64,
    np: f64,
    nb: f64,
}

/// An MPEG-2 video encoder.
///
/// [`encode`](Self::encode) takes frames in display order and returns the
/// stream bytes that are ready (with B-pictures, a reference picture is
/// coded before the frames that precede it, so output lags input by up to
/// `b_frames` frames); [`finish`](Self::finish) codes what is held and ends
/// the sequence.
pub struct Encoder {
    cfg: EncoderConfig,
    frame_rate_code: u8,
    frame_rate_ext: (u8, u8),
    mb_width: usize,
    mb_height: usize,
    f_code: u8,
    /// Frames waiting for their following reference picture, with their
    /// display index.
    pending: Vec<(u64, PicBuf)>,
    older: Option<PicBuf>,
    newer: Option<PicBuf>,
    frames_in: u64,
    /// Display index of the first picture of the current GOP.
    gop_start: u64,
    /// Display index of the last reference picture.
    last_anchor: Option<u64>,
    first_gop: bool,
    rate: Option<Rate>,
    qcode: u8,
    finished: bool,
    /// The reconstruction of every picture coded, by display index, when
    /// asked for (`keep_reconstructions`).
    recons: Option<Vec<(u64, PicBuf)>>,
    /// The row threads (all but the caller's), kept between pictures.
    pool: Option<crate::pool::Pool>,
}

impl Encoder {
    /// An encoder for `cfg`, or [`crate::Error::Config`] if it cannot be
    /// coded.
    pub fn new(cfg: EncoderConfig) -> Result<Encoder> {
        if cfg.width == 0 || cfg.height == 0 || cfg.width > 4095 || cfg.height > 2800 {
            return Err(config(format!("size {}x{} (1–4095 × 1–2800)", cfg.width, cfg.height)));
        }
        if cfg.gop_size == 0 {
            return Err(config("gop_size 0"));
        }
        if cfg.b_frames > 7 {
            return Err(config("more than 7 B-pictures between references"));
        }
        if cfg.intra_dc_precision > 2 {
            return Err(config("intra_dc_precision above 2 (10 bits) is not Main Profile"));
        }
        if !(1..=4).contains(&cfg.aspect_ratio_information) {
            return Err(config("aspect_ratio_information must be 1–4"));
        }
        let (frame_rate_code, frame_rate_ext) = find_frame_rate(cfg.frame_rate)
            .ok_or_else(|| config(format!("frame rate {}/{}", cfg.frame_rate.0, cfg.frame_rate.1)))?;
        let qcode = match cfg.rate_control {
            RateControl::ConstantQuantiser(q) if (1..=31).contains(&q) => q,
            RateControl::ConstantQuantiser(q) => return Err(config(format!("quantiser_scale_code {q}"))),
            RateControl::Bitrate(b) if b >= 10_000 => 8,
            RateControl::Bitrate(b) => return Err(config(format!("bit rate {b}"))),
        };
        let range = cfg.search_range.clamp(1, 1023) as i32;
        // f_code: vectors span [−8f, 8f − 0.5] samples, f = 2^(f_code − 1).
        let f_code = (1..=9u8).find(|&fc| 8 * (1i32 << (fc - 1)) > range).unwrap_or(9);
        let fps = f64::from(cfg.frame_rate.0) / f64::from(cfg.frame_rate.1);
        let rate = match cfg.rate_control {
            RateControl::Bitrate(b) => {
                let b = f64::from(b);
                Some(Rate {
                    picture_bits: b / fps,
                    x: [160.0 * b / 115.0, 60.0 * b / 115.0, 42.0 * b / 115.0],
                    remaining: 0.0,
                    np: 0.0,
                    nb: 0.0,
                })
            }
            RateControl::ConstantQuantiser(_) => None,
        };
        Ok(Encoder {
            mb_width: cfg.width.div_ceil(16) as usize,
            mb_height: cfg.height.div_ceil(16) as usize,
            frame_rate_code,
            frame_rate_ext,
            f_code,
            cfg,
            pending: Vec::new(),
            older: None,
            newer: None,
            frames_in: 0,
            gop_start: 0,
            last_anchor: None,
            first_gop: true,
            rate,
            qcode,
            finished: false,
            recons: None,
            pool: None,
        })
    }

    /// Takes the next frame (display order; 4:2:0, the configured size) and
    /// returns the bytes coded so far.
    pub fn encode(&mut self, frame: &Frame) -> Result<Vec<u8>> {
        if self.finished {
            return Err(config("encode after finish"));
        }
        if frame.width != self.cfg.width || frame.height != self.cfg.height {
            return Err(config(format!(
                "frame is {}x{}, the encoder {}x{}",
                frame.width, frame.height, self.cfg.width, self.cfg.height
            )));
        }
        if frame.chroma != ChromaFormat::Yuv420 || frame.planes.len() != 3 {
            return Err(config("the encoder takes 4:2:0 frames"));
        }
        let (cw, ch) = ChromaFormat::Yuv420.chroma_size(frame.width, frame.height);
        for (i, p) in frame.planes.iter().enumerate() {
            let want = if i == 0 { (frame.width, frame.height) } else { (cw, ch) };
            if (p.width, p.height) != want || p.offset.checked_add(p.len()).is_none_or(|end| end > frame.data.len()) {
                return Err(config(format!("plane {i} does not fit the frame's size or data")));
            }
        }
        let buf = self.pad(frame)?;
        let idx = self.frames_in;
        self.frames_in += 1;
        self.pending.push((idx, buf));
        let mut w = BitWriter::new();
        if self.last_anchor.is_none() || self.pending.len() as u32 > self.cfg.b_frames {
            self.flush_pending(&mut w);
        }
        Ok(w.finish())
    }

    /// Codes the frames still held (the last as a reference picture, those
    /// before it as B-pictures) and ends the sequence.
    pub fn finish(&mut self) -> Result<Vec<u8>> {
        let mut w = BitWriter::new();
        if !self.finished {
            if !self.pending.is_empty() {
                self.flush_pending(&mut w);
            }
            if self.frames_in > 0 {
                w.start_code(SEQUENCE_END);
            }
            self.finished = true;
        }
        Ok(w.finish())
    }

    /// From now on, keep the reconstruction of every picture coded — the
    /// pictures a decoder will reconstruct from the stream, B-pictures too —
    /// for [`take_reconstructions`](Self::take_reconstructions). For tests:
    /// they check that a decoder's output equals them, sample for sample.
    #[doc(hidden)]
    pub fn keep_reconstructions(&mut self) {
        self.recons.get_or_insert_with(Vec::new);
    }

    /// The reconstructions kept since the last call, in display order,
    /// cropped to the frame size.
    #[doc(hidden)]
    pub fn take_reconstructions(&mut self) -> Vec<Frame> {
        let mut v = self.recons.as_mut().map(std::mem::take).unwrap_or_default();
        v.sort_by_key(|r| r.0);
        let (w, h) = (self.cfg.width, self.cfg.height);
        v.into_iter()
            .map(|(_, b)| {
                let mut f = Frame::new(w, h, ChromaFormat::Yuv420);
                for c in 0..3 {
                    let pw = f.planes[c].width as usize;
                    let bw = b.dims(c).0;
                    let rows = f.planes[c].height as usize;
                    let dst = f.plane_mut(c);
                    for y in 0..rows {
                        dst[y * pw..y * pw + pw].copy_from_slice(&b.planes[c][y * bw..y * bw + pw]);
                    }
                }
                f
            })
            .collect()
    }

    /// Copies a frame into a macroblock-aligned buffer, repeating the edge
    /// samples into the padding.
    fn pad(&self, frame: &Frame) -> Result<PicBuf> {
        let mut b = PicBuf::new(self.mb_width * 16, self.mb_height * 16, ChromaFormat::Yuv420);
        for c in 0..3 {
            let p = frame.planes[c];
            let src = frame.plane(c);
            if src.len() < p.width as usize * p.height as usize || p.width == 0 || p.height == 0 {
                return Err(config("frame plane smaller than its size"));
            }
            let (bw, bh) = b.dims(c);
            let (pw, ph) = (p.width as usize, p.height as usize);
            for y in 0..bh {
                let sy = y.min(ph - 1);
                let row = &src[sy * pw..sy * pw + pw];
                let dst = &mut b.planes[c][y * bw..y * bw + bw];
                for (x, d) in dst.iter_mut().enumerate() {
                    *d = row[x.min(pw - 1)];
                }
            }
        }
        Ok(b)
    }

    /// Codes the held frames: the last as I or P, the others as B.
    fn flush_pending(&mut self, w: &mut BitWriter) {
        let mut frames = std::mem::take(&mut self.pending);
        let (anchor_idx, anchor) = frames.pop().expect("a frame");
        let n_b = frames.len() as u64;
        let new_gop = match self.last_anchor {
            None => true,
            Some(_) => anchor_idx - self.gop_start >= u64::from(self.cfg.gop_size),
        };
        if new_gop {
            self.gop_start = anchor_idx - n_b;
            self.write_sequence_header(w);
            let closed = self.first_gop || n_b == 0;
            self.write_gop_header(w, self.gop_start, closed);
            self.first_gop = false;
            if let Some(r) = &mut self.rate {
                let g = f64::from(self.cfg.gop_size);
                let refs = (g / f64::from(self.cfg.b_frames + 1)).ceil();
                r.remaining += r.picture_bits * g;
                r.np = (refs - 1.0).max(0.0);
                r.nb = (g - refs).max(0.0);
            }
        }
        let ptype = if new_gop { 1 } else { 2 };
        let tr = (anchor_idx - self.gop_start) as u16;
        let mut recon = PicBuf::new(self.mb_width * 16, self.mb_height * 16, ChromaFormat::Yuv420);
        let previous = self.newer.take();
        let refp = if ptype == 2 { previous.as_ref() } else { None };
        self.code(w, ptype, tr, &anchor, refp, None, Some(&mut recon));
        if let Some(v) = &mut self.recons {
            v.push((anchor_idx, recon.clone()));
        }
        self.older = previous;
        self.newer = Some(recon);
        self.last_anchor = Some(anchor_idx);
        for (idx, f) in frames {
            let tr = (idx - self.gop_start) as u16;
            let (older, newer) = (self.older.take(), self.newer.take());
            // A B-picture is reconstructed only to be checked.
            let mut recon = self.recons.is_some().then(|| PicBuf::new(self.mb_width * 16, self.mb_height * 16, ChromaFormat::Yuv420));
            self.code(w, 3, tr, &f, older.as_ref(), newer.as_ref(), recon.as_mut());
            if let (Some(v), Some(r)) = (&mut self.recons, recon) {
                v.push((idx, r));
            }
            self.older = older;
            self.newer = newer;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn code(
        &mut self,
        w: &mut BitWriter,
        ptype: u8,
        temporal_reference: u16,
        src: &PicBuf,
        fwd: Option<&PicBuf>,
        bwd: Option<&PicBuf>,
        recon: Option<&mut PicBuf>,
    ) {
        let start = w.bit_len();
        let t = usize::from(ptype - 1);
        let qcode = match &self.rate {
            Some(r) => {
                let (xi, xp, xb) = (r.x[0], r.x[1], r.x[2]);
                let (kp, kb) = (1.0, 1.4);
                let target = match ptype {
                    1 => r.remaining / (1.0 + r.np * xp / (xi * kp) + r.nb * xb / (xi * kb)),
                    2 => r.remaining / (r.np + r.nb * kp * xb / (kb * xp)),
                    _ => r.remaining / (r.nb + r.np * kb * xp / (kp * xb)),
                }
                .max(r.picture_bits / 8.0);
                let qs = r.x[t] / target;
                let code = if self.cfg.q_scale_type {
                    crate::tables::NON_LINEAR_QSCALE[1..]
                        .iter()
                        .position(|&v| f64::from(v) >= qs)
                        .map_or(31, |p| p + 1) as u8
                } else {
                    (qs / 2.0).round().clamp(1.0, 31.0) as u8
                };
                code.clamp(1, 31)
            }
            None => self.qcode,
        };
        let header = PictureHeader {
            temporal_reference: temporal_reference & 1023,
            picture_coding_type: ptype,
            vbv_delay: 0xffff,
            full_pel_forward_vector: false,
            forward_f_code: 7,
            full_pel_backward_vector: false,
            backward_f_code: 7,
        };
        header.write(w);
        let fc = self.f_code;
        let ext = PictureCodingExtension {
            f_code: match ptype {
                1 => [[15, 15], [15, 15]],
                2 => [[fc, fc], [15, 15]],
                _ => [[fc, fc], [fc, fc]],
            },
            intra_dc_precision: self.cfg.intra_dc_precision,
            picture_structure: FRAME_PICTURE,
            top_field_first: false,
            frame_pred_frame_dct: true,
            concealment_motion_vectors: false,
            q_scale_type: self.cfg.q_scale_type,
            intra_vlc_format: self.cfg.intra_vlc_format,
            alternate_scan: self.cfg.alternate_scan,
            repeat_first_field: false,
            chroma_420_type: true,
            progressive_frame: true,
        };
        ext.write(w);
        let settings = PictureSettings {
            picture_type: ptype,
            quantiser_scale_code: qcode,
            q_scale_type: self.cfg.q_scale_type,
            intra_vlc_format: self.cfg.intra_vlc_format,
            alternate_scan: self.cfg.alternate_scan,
            intra_dc_precision: self.cfg.intra_dc_precision,
            f_code: fc,
            search_range: self.cfg.search_range.clamp(1, 1023) as i32,
        };
        let threads = match self.cfg.threads {
            0 => std::thread::available_parallelism().map_or(1, |n| n.get()),
            n => n,
        };
        let pool = crate::pool::ensure(&mut self.pool, threads);
        code_picture(&settings, src, fwd, bwd, recon, w, threads, pool);
        let bits = (w.bit_len() - start) as f64;
        if let Some(r) = &mut self.rate {
            let qs = f64::from(crate::tables::quantiser_scale(self.cfg.q_scale_type, qcode));
            r.x[t] = (bits * qs).max(1.0);
            r.remaining -= bits;
            match ptype {
                2 => r.np = (r.np - 1.0).max(0.0),
                3 => r.nb = (r.nb - 1.0).max(0.0),
                _ => {}
            }
            // Keep the allocation sane after a scene the model mispredicted.
            let gop_bits = r.picture_bits * f64::from(self.cfg.gop_size);
            r.remaining = r.remaining.clamp(-gop_bits, 2.0 * gop_bits);
            if r.remaining < r.picture_bits {
                r.remaining = r.picture_bits * (1.0 + r.np + r.nb).max(1.0) / 2.0;
            }
        }
    }

    fn write_sequence_header(&self, w: &mut BitWriter) {
        let (width, height) = (self.cfg.width, self.cfg.height);
        let fps = f64::from(self.cfg.frame_rate.0) / f64::from(self.cfg.frame_rate.1);
        // Main Profile at the lowest level whose limits the size and rate
        // fit (Table 8-10): Main, High-1440, High.
        let (level, max_rate, vbv) = if width <= 720 && height <= 576 && fps <= 30.0 {
            (8u8, 15_000_000u64, 112u32)
        } else if width <= 1440 && height <= 1152 && fps <= 60.0 {
            (6, 60_000_000, 448)
        } else {
            (4, 80_000_000, 597)
        };
        let bit_rate = match self.cfg.rate_control {
            RateControl::Bitrate(b) => u64::from(b),
            RateControl::ConstantQuantiser(_) => max_rate,
        };
        let units = bit_rate.div_ceil(400).clamp(1, (1 << 30) - 1);
        SequenceHeader {
            horizontal_size_value: width & 0xfff,
            vertical_size_value: height & 0xfff,
            aspect_ratio_information: self.cfg.aspect_ratio_information,
            frame_rate_code: self.frame_rate_code,
            bit_rate_value: (units & 0x3ffff) as u32,
            vbv_buffer_size_value: vbv & 0x3ff,
            constrained_parameters_flag: false,
            intra_quantiser_matrix: None,
            non_intra_quantiser_matrix: None,
        }
        .write(w);
        SequenceExtension {
            profile_and_level_indication: 0x40 | level,
            progressive_sequence: true,
            chroma_format: 1,
            horizontal_size_extension: width >> 12,
            vertical_size_extension: height >> 12,
            bit_rate_extension: (units >> 18) as u32,
            vbv_buffer_size_extension: vbv >> 10,
            low_delay: self.cfg.b_frames == 0,
            frame_rate_extension_n: self.frame_rate_ext.0,
            frame_rate_extension_d: self.frame_rate_ext.1,
        }
        .write(w);
    }

    fn write_gop_header(&self, w: &mut BitWriter, display_index: u64, closed: bool) {
        // time_code (6.3.8): hours, minutes, seconds, pictures; no drop-frame.
        let (n, d) = self.cfg.frame_rate;
        let per_sec = u64::from(n.div_ceil(d)).max(1);
        let secs_total = display_index / per_sec;
        let pictures = display_index % per_sec;
        let (h, m, s) = ((secs_total / 3600) % 24, (secs_total / 60) % 60, secs_total % 60);
        let time_code =
            ((h as u32) << 19) | ((m as u32) << 13) | (1 << 12) | ((s as u32) << 6) | (pictures as u32 & 63);
        GopHeader { time_code, closed_gop: closed, broken_link: false }.write(w);
    }
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// The frame_rate_code and (frame_rate_extension_n, _d) for a rate.
fn find_frame_rate((n, d): (u32, u32)) -> Option<(u8, (u8, u8))> {
    if n == 0 || d == 0 {
        return None;
    }
    // A rate Table 6-4 lists directly shall be coded with zero extensions
    // (6.3.3), so those are tried first.
    for code in 1..=8u8 {
        let (vn, vd) = frame_rate_value(code)?;
        if u64::from(vn) * u64::from(d) == u64::from(n) * u64::from(vd) {
            return Some((code, (0, 0)));
        }
    }
    for code in 1..=8u8 {
        let (vn, vd) = frame_rate_value(code)?;
        for en in 0..4u64 {
            for ed in 0..32u64 {
                if gcd(en + 1, ed + 1) != 1 {
                    continue;
                }
                // vn/vd × (en+1)/(ed+1) == n/d
                if u64::from(vn) * (en + 1) * u64::from(d) == u64::from(n) * u64::from(vd) * (ed + 1) {
                    return Some((code, (en as u8, ed as u8)));
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_rates_map_to_codes() {
        assert_eq!(find_frame_rate((25, 1)), Some((3, (0, 0))));
        assert_eq!(find_frame_rate((30000, 1001)), Some((4, (0, 0))));
        assert_eq!(find_frame_rate((50, 1)), Some((6, (0, 0))));
        // 15 = 25 × 3/5 (frame_rate_extension_n 2, _d 4).
        assert_eq!(find_frame_rate((15, 1)), Some((3, (2, 4))));
        assert_eq!(find_frame_rate((1, 1000)), None);
    }
}
