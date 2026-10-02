//! The decoder: start-code parsing, the headers, picture management and
//! output in display order.

pub(crate) mod mc;
pub(crate) mod slice;

use crate::bits::BitReader;
use crate::error::{Result, invalid, unsupported};
use crate::frame::{ChromaFormat, Frame, PictureType, Plane};
use crate::headers::*;
use crate::tables::{DEFAULT_INTRA_MATRIX, DEFAULT_NON_INTRA_MATRIX, frame_rate_value};
use mc::PicBuf;
use slice::Params;

/// The largest picture the decoder allocates for, in luma samples.
const MAX_SAMPLES: u64 = 4096 * 4096;

/// What the active sequence header and extensions say about the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SequenceInfo {
    /// `horizontal_size`: the width of the frames the decoder returns.
    pub width: u32,
    /// `vertical_size`: their height.
    pub height: u32,
    /// Chroma sampling.
    pub chroma: ChromaFormat,
    /// The frame rate as a fraction `(numerator, denominator)` (Table 6-4 with
    /// frame_rate_extension_n / _d), or `None` for a reserved code.
    pub frame_rate: Option<(u32, u32)>,
    /// `aspect_ratio_information` (Table 6-3 for MPEG-2: 1 square samples,
    /// 2 4:3, 3 16:9, 4 2.21:1 display; for MPEG-1, pel_aspect_ratio).
    pub aspect_ratio_information: u8,
    /// `bit_rate` in bits per second (the 400 bit/s units multiplied out);
    /// 0x3FFFF units — "variable" in MPEG-1 — come back as they are.
    pub bit_rate: u64,
    /// `vbv_buffer_size` in bits.
    pub vbv_buffer_size: u64,
    /// `profile_and_level_indication` (0 for MPEG-1).
    pub profile_and_level_indication: u8,
    /// `progressive_sequence` (true for MPEG-1).
    pub progressive_sequence: bool,
    /// `low_delay`: the stream has no B-pictures.
    pub low_delay: bool,
    /// The stream is ISO/IEC 11172-2 (MPEG-1) video: no sequence extension.
    pub mpeg1: bool,
    /// From a sequence display extension: `(display_horizontal_size,
    /// display_vertical_size)`.
    pub display_size: Option<(u32, u32)>,
    /// From a sequence display extension: `(colour_primaries,
    /// transfer_characteristics, matrix_coefficients)`.
    pub colour_description: Option<(u8, u8, u8)>,
    /// From a sequence display extension: `video_format` (0 component,
    /// 1 PAL, 2 NTSC, 3 SECAM, 4 MAC, 5 unspecified).
    pub video_format: Option<u8>,
}

/// Where extension start codes belong (6.2.2.2).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    None,
    Sequence,
    Gop,
    Picture,
}

/// Display metadata a decoded frame carries until it is output.
#[derive(Clone, Copy)]
struct Meta {
    picture_type: PictureType,
    temporal_reference: u16,
    progressive_frame: bool,
    top_field_first: bool,
    repeat_first_field: bool,
    field_pictures: bool,
    decode_index: u64,
}

/// The picture whose slices are being decoded.
struct Current {
    params: Params,
    meta: Meta,
    anchor: bool,
    /// A field picture that is the first of its frame.
    first_field: bool,
}

/// A first field waiting for its second.
struct Pending {
    buf: usize,
    parity: u8,
    anchor: bool,
    meta: Meta,
}

/// An MPEG-2 (and MPEG-1) video elementary stream decoder.
///
/// Feed it bytes in any chunking with [`decode`](Self::decode); it returns
/// the frames that became ready, in display order. [`flush`](Self::flush)
/// at the end of the stream returns the last one, which a decoder holds
/// until the next reference picture (or the end) says it may be shown.
pub struct Decoder {
    /// Bytes not yet parsed, starting at a start code (or the stream start).
    input: Vec<u8>,
    seq: Option<SeqState>,
    /// A sequence header whose sequence extension (MPEG-2) or absence of one
    /// (MPEG-1) has not been seen yet.
    pending_seq: Option<SequenceHeader>,
    level: Level,
    pending_header: Option<PictureHeader>,
    pending_ext: Option<PictureCodingExtension>,
    cur: Option<Current>,
    /// The picture header was seen but the picture cannot be decoded (a
    /// reference is missing); its slices are dropped.
    skipping: bool,
    first_field: Option<Pending>,
    bufs: Vec<PicBuf>,
    /// The older and the newer reference frame.
    older: Option<usize>,
    newer: Option<usize>,
    /// The newer reference frame has not been output yet.
    newer_meta: Option<Meta>,
    closed_gop: bool,
    broken_link: bool,
    decode_index: u64,
    out: Vec<Frame>,
}

struct SeqState {
    info: SequenceInfo,
    /// Rows of macroblocks in a frame.
    mb_width: usize,
    mb_height: usize,
    /// W[0..4]: intra, non-intra, chroma intra, chroma non-intra (raster).
    qmat: [[u8; 64]; 4],
    header: SequenceHeader,
}

impl Default for Decoder {
    fn default() -> Self {
        Self::new()
    }
}

impl Decoder {
    /// A decoder waiting for a sequence header.
    pub fn new() -> Decoder {
        Decoder {
            input: Vec::new(),
            seq: None,
            pending_seq: None,
            level: Level::None,
            pending_header: None,
            pending_ext: None,
            cur: None,
            skipping: false,
            first_field: None,
            bufs: Vec::new(),
            older: None,
            newer: None,
            newer_meta: None,
            closed_gop: false,
            broken_link: false,
            decode_index: 0,
            out: Vec::new(),
        }
    }

    /// The active sequence's parameters, once a sequence header has been
    /// decoded.
    pub fn sequence(&self) -> Option<&SequenceInfo> {
        self.seq.as_ref().map(|s| &s.info)
    }

    /// Decodes elementary-stream bytes (any chunking: a unit split across
    /// calls is completed by the next one) and returns the frames that are
    /// ready, in display order.
    ///
    /// On an error the offending start-code unit is dropped and the error
    /// returned; the decoder stays usable, frames already decoded are
    /// returned by the next call, and decoding resumes at the next start
    /// code.
    pub fn decode(&mut self, data: &[u8]) -> Result<Vec<Frame>> {
        self.input.extend_from_slice(data);
        let mut pos = match find_start_code(&self.input, 0) {
            Some(p) => p,
            None => {
                // Keep the last three bytes: they may begin a start code.
                let keep = self.input.len().saturating_sub(3);
                self.input.drain(..keep);
                return Ok(std::mem::take(&mut self.out));
            }
        };
        let mut result = Ok(());
        while let Some(next) = find_start_code(&self.input, pos + 4) {
            let code = self.input[pos + 3];
            let body = self.input[pos + 4..next].to_vec();
            pos = next;
            if let Err(e) = self.unit(code, &body) {
                result = Err(e);
                break;
            }
        }
        self.input.drain(..pos);
        result?;
        Ok(std::mem::take(&mut self.out))
    }

    /// Ends the stream: decodes what is buffered and returns the remaining
    /// frames in display order. The decoder is then ready for a new stream.
    pub fn flush(&mut self) -> Result<Vec<Frame>> {
        let mut result = Ok(());
        if let Some(pos) = find_start_code(&self.input, 0) {
            let code = self.input[pos + 3];
            let body = self.input[pos + 4..].to_vec();
            result = self.unit(code, &body);
        }
        self.input.clear();
        self.end_picture();
        self.end_first_field();
        self.output_newer();
        self.older = None;
        self.newer = None;
        result?;
        Ok(std::mem::take(&mut self.out))
    }

    fn unit(&mut self, code: u8, body: &[u8]) -> Result<()> {
        // A sequence header not followed by a sequence extension is
        // ISO/IEC 11172-2's (6.2.2: the extension follows immediately).
        if self.pending_seq.is_some() {
            let ext_follows = code == EXTENSION && BitReader::new(body).peek(4) == EXT_SEQUENCE;
            if !ext_follows {
                let h = self.pending_seq.take().expect("pending");
                self.install_mpeg1(h)?;
            }
        }
        if (SLICE_MIN..=SLICE_MAX).contains(&code) {
            return self.slice(code, body);
        }
        // Anything but a slice ends the picture being decoded (extensions
        // and user data between the picture header and its first slice do
        // not: no picture is open then).
        if code != EXTENSION && code != USER_DATA {
            self.end_picture();
        }
        let mut r = BitReader::new(body);
        match code {
            SEQUENCE_HEADER => {
                let h = SequenceHeader::parse(&mut r)?;
                if h.horizontal_size_value == 0 || h.vertical_size_value == 0 {
                    return Err(invalid("zero picture size"));
                }
                self.pending_seq = Some(h);
                self.level = Level::Sequence;
            }
            EXTENSION => {
                let id = r.read(4);
                self.extension(id, &mut r)?;
            }
            GROUP_START => {
                let g = GopHeader::parse(&mut r)?;
                self.closed_gop = g.closed_gop;
                self.broken_link = g.broken_link;
                self.level = Level::Gop;
            }
            PICTURE_START => {
                let h = PictureHeader::parse(&mut r)?;
                if !(1..=4).contains(&h.picture_coding_type) {
                    self.pending_header = None;
                    return Err(invalid(format!("picture_coding_type {}", h.picture_coding_type)));
                }
                self.pending_header = Some(h);
                self.pending_ext = None;
                self.skipping = false;
                self.level = Level::Picture;
            }
            SEQUENCE_END => {
                self.end_first_field();
                self.output_newer();
                self.level = Level::None;
            }
            USER_DATA | SEQUENCE_ERROR => {}
            _ => {} // reserved and system start codes: ignored
        }
        Ok(())
    }

    /// The quantiser matrices a sequence header sets (6.3.11): its own or
    /// the defaults, luma and chroma alike.
    fn header_matrices(h: &SequenceHeader) -> [[u8; 64]; 4] {
        let intra = h.intra_quantiser_matrix.unwrap_or(DEFAULT_INTRA_MATRIX);
        let non_intra = h.non_intra_quantiser_matrix.unwrap_or(DEFAULT_NON_INTRA_MATRIX);
        [intra, non_intra, intra, non_intra]
    }

    /// Activates an ISO/IEC 11172-2 sequence header.
    fn install_mpeg1(&mut self, h: SequenceHeader) -> Result<()> {
        let info = SequenceInfo {
            width: h.horizontal_size_value,
            height: h.vertical_size_value,
            chroma: ChromaFormat::Yuv420,
            frame_rate: frame_rate_value(h.frame_rate_code),
            aspect_ratio_information: h.aspect_ratio_information,
            bit_rate: u64::from(h.bit_rate_value) * 400,
            vbv_buffer_size: u64::from(h.vbv_buffer_size_value) * 16 * 1024,
            profile_and_level_indication: 0,
            progressive_sequence: true,
            low_delay: false,
            mpeg1: true,
            display_size: None,
            colour_description: None,
            video_format: None,
        };
        let qmat = Self::header_matrices(&h);
        self.install_sequence(SeqState { mb_width: 0, mb_height: 0, info, qmat, header: h })
    }

    /// Makes `s` the active sequence, sizing it; a change of picture size or
    /// chroma format ends the previous sequence's pictures first.
    fn install_sequence(&mut self, mut s: SeqState) -> Result<()> {
        // The syntax reaches 16383 x 16383; High Level stops at 1920 x 1152.
        // Anything beyond 4096 x 4096 is refused rather than allocated.
        if u64::from(s.info.width) * u64::from(s.info.height) > MAX_SAMPLES {
            // The previous sequence ends; this one's pictures are dropped
            // (their slices find no sequence).
            self.end_first_field();
            self.output_newer();
            self.older = None;
            self.newer = None;
            self.bufs.clear();
            self.seq = None;
            return Err(unsupported(format!("picture size {}x{}", s.info.width, s.info.height)));
        }
        s.mb_width = s.info.width.div_ceil(16) as usize;
        s.mb_height = if s.info.progressive_sequence {
            s.info.height.div_ceil(16) as usize
        } else {
            2 * s.info.height.div_ceil(32) as usize
        };
        let changed = match &self.seq {
            Some(old) => {
                old.info.width != s.info.width
                    || old.info.height != s.info.height
                    || old.info.chroma != s.info.chroma
                    || old.mb_height != s.mb_height
            }
            None => true,
        };
        if changed {
            self.end_first_field();
            self.output_newer();
            self.older = None;
            self.newer = None;
            self.bufs.clear();
        }
        self.seq = Some(s);
        Ok(())
    }

    fn extension(&mut self, id: u32, r: &mut BitReader) -> Result<()> {
        match (id, self.level) {
            (EXT_SEQUENCE, _) => {
                let e = SequenceExtension::parse(r)?;
                let h = match (self.pending_seq.take(), &self.seq) {
                    (Some(h), _) => h,
                    (None, Some(s)) => s.header.clone(),
                    (None, None) => return Err(invalid("sequence extension without a sequence header")),
                };
                let chroma = ChromaFormat::from_code(u32::from(e.chroma_format))
                    .ok_or_else(|| invalid("chroma_format 0"))?;
                slice::check_supported(chroma)?;
                let qmat = Self::header_matrices(&h);
                let fr = frame_rate_value(h.frame_rate_code).map(|(n, d)| {
                    let n = n * (u32::from(e.frame_rate_extension_n) + 1);
                    let d = d * (u32::from(e.frame_rate_extension_d) + 1);
                    let g = gcd(n, d);
                    (n / g, d / g)
                });
                let info = SequenceInfo {
                    width: h.horizontal_size_value | (e.horizontal_size_extension << 12),
                    height: h.vertical_size_value | (e.vertical_size_extension << 12),
                    chroma,
                    frame_rate: fr,
                    aspect_ratio_information: h.aspect_ratio_information,
                    bit_rate: (u64::from(h.bit_rate_value) | (u64::from(e.bit_rate_extension) << 18)) * 400,
                    vbv_buffer_size: (u64::from(h.vbv_buffer_size_value)
                        | (u64::from(e.vbv_buffer_size_extension) << 10))
                        * 16
                        * 1024,
                    profile_and_level_indication: e.profile_and_level_indication,
                    progressive_sequence: e.progressive_sequence,
                    low_delay: e.low_delay,
                    mpeg1: false,
                    display_size: None,
                    colour_description: None,
                    video_format: None,
                };
                self.install_sequence(SeqState { mb_width: 0, mb_height: 0, info, qmat, header: h })?;
            }
            (EXT_SEQUENCE_DISPLAY, Level::Sequence) => {
                let e = SequenceDisplayExtension::parse(r)?;
                if let Some(s) = &mut self.seq {
                    s.info.display_size = Some((e.display_horizontal_size, e.display_vertical_size));
                    s.info.colour_description = e.colour_description;
                    s.info.video_format = Some(e.video_format);
                }
            }
            (EXT_SEQUENCE_SCALABLE, _) => {
                return Err(unsupported("scalable extensions (sequence_scalable_extension)"));
            }
            (EXT_QUANT_MATRIX, _) => {
                let q = QuantMatrixExtension::parse(r)?;
                if let Some(s) = &mut self.seq {
                    if let Some(m) = q.intra {
                        s.qmat[0] = m;
                        s.qmat[2] = m;
                    }
                    if let Some(m) = q.non_intra {
                        s.qmat[1] = m;
                        s.qmat[3] = m;
                    }
                    if let Some(m) = q.chroma_intra {
                        s.qmat[2] = m;
                    }
                    if let Some(m) = q.chroma_non_intra {
                        s.qmat[3] = m;
                    }
                }
            }
            (EXT_PICTURE_CODING, _) => {
                self.pending_ext = Some(PictureCodingExtension::parse(r)?);
            }
            // Copyright, picture display, the spatial / temporal scalable
            // picture extensions of a base layer, camera parameters: nothing
            // a base-layer decoder needs.
            _ => {}
        }
        Ok(())
    }

    fn slice(&mut self, code: u8, body: &[u8]) -> Result<()> {
        if self.cur.is_none() {
            if self.skipping || self.pending_header.is_none() {
                return Ok(());
            }
            self.begin_picture()?;
            if self.cur.is_none() {
                return Ok(());
            }
        }
        let seq = self.seq.as_ref().ok_or_else(|| invalid("slice without a sequence header"))?;
        let cur = self.cur.as_ref().expect("picture open");
        slice::decode_slice(&cur.params, &seq.qmat, &mut self.bufs, code, body)
    }

    /// Sets up the picture whose header was just read, at its first slice.
    fn begin_picture(&mut self) -> Result<()> {
        let h = self.pending_header.take().expect("picture header");
        let seq = self.seq.as_ref().ok_or_else(|| invalid("picture without a sequence header"))?;
        let (mpeg1, chroma, seq_mb_width, seq_mb_height, seq_height) =
            (seq.info.mpeg1, seq.info.chroma, seq.mb_width, seq.mb_height, seq.info.height);
        let ext = match (self.pending_ext.take(), mpeg1) {
            (Some(e), _) => e,
            (None, true) => PictureCodingExtension::mpeg1(&h),
            (None, false) => return Err(invalid("picture without a picture coding extension")),
        };
        let (fw, fh) = (seq_mb_width * 16, seq_mb_height * 16);
        let frame_pic = ext.picture_structure == FRAME_PICTURE;
        let parity = u8::from(ext.picture_structure == BOTTOM_FIELD);
        let anchor = h.picture_coding_type != 3;
        let ptype = match h.picture_coding_type {
            1 => PictureType::I,
            2 => PictureType::P,
            3 => PictureType::B,
            _ => PictureType::D,
        };
        if seq_mb_height % 2 != 0 && !frame_pic {
            return Err(invalid("field picture in a progressive sequence of odd macroblock rows"));
        }
        if self.bufs.is_empty() {
            self.bufs = (0..3).map(|_| PicBuf::new(fw, fh, chroma)).collect();
        }

        // Is this the second field of the frame whose first field we hold?
        let second = match &self.first_field {
            Some(f) if !frame_pic && f.parity != parity && f.anchor == anchor => true,
            Some(_) => {
                self.end_first_field();
                false
            }
            None => false,
        };

        let meta;
        let target;
        if second {
            let f = self.first_field.as_ref().expect("first field");
            target = f.buf;
            meta = f.meta;
        } else {
            // A new frame. Its references must exist.
            let ok = match ptype {
                PictureType::I | PictureType::D => true,
                PictureType::P => self.newer.is_some(),
                PictureType::B => {
                    self.newer.is_some() && (self.older.is_some() || self.closed_gop)
                }
            };
            if !ok {
                self.skipping = true;
                return Ok(());
            }
            if anchor {
                // The previous reference frame is due for display now.
                self.output_newer();
            }
            target = (0..3).find(|&i| Some(i) != self.older && Some(i) != self.newer).expect("free buffer");
            meta = Meta {
                picture_type: ptype,
                temporal_reference: h.temporal_reference,
                progressive_frame: ext.progressive_frame,
                top_field_first: ext.top_field_first,
                repeat_first_field: ext.repeat_first_field,
                field_pictures: !frame_pic,
                decode_index: self.decode_index,
            };
            self.decode_index += 1;
        }
        let refs = if anchor {
            [self.newer, None]
        } else {
            [self.older.or(self.newer), self.newer]
        };
        let params = Params {
            mpeg1,
            chroma,
            mb_width: seq_mb_width,
            mb_height: if frame_pic { seq_mb_height } else { seq_mb_height / 2 },
            vertical_size: seq_height,
            picture_type: h.picture_coding_type,
            picture_structure: ext.picture_structure,
            second_field: second,
            top_field_first: ext.top_field_first,
            f_code: ext.f_code,
            full_pel: [h.full_pel_forward_vector && mpeg1, h.full_pel_backward_vector && mpeg1],
            intra_dc_precision: ext.intra_dc_precision,
            frame_pred_frame_dct: ext.frame_pred_frame_dct,
            concealment_motion_vectors: ext.concealment_motion_vectors,
            q_scale_type: ext.q_scale_type,
            intra_vlc_format: ext.intra_vlc_format,
            alternate_scan: ext.alternate_scan,
            cur: target,
            refs,
        };
        if second {
            self.first_field = None;
        }
        self.cur = Some(Current { params, meta, anchor, first_field: !frame_pic && !second });
        Ok(())
    }

    /// The picture's slices are over.
    fn end_picture(&mut self) {
        let Some(c) = self.cur.take() else { return };
        if c.first_field {
            self.first_field = Some(Pending {
                buf: c.params.cur,
                parity: u8::from(c.params.picture_structure == BOTTOM_FIELD),
                anchor: c.anchor,
                meta: c.meta,
            });
            return;
        }
        self.end_frame(c.params.cur, c.anchor, c.meta);
    }

    /// A first field whose second never came: the frame ends with it.
    fn end_first_field(&mut self) {
        if let Some(f) = self.first_field.take() {
            self.end_frame(f.buf, f.anchor, f.meta);
        }
    }

    fn end_frame(&mut self, buf: usize, anchor: bool, meta: Meta) {
        if anchor {
            self.older = self.newer;
            self.newer = Some(buf);
            self.newer_meta = Some(meta);
        } else {
            self.output(buf, meta);
        }
    }

    fn output_newer(&mut self) {
        if let (Some(b), Some(m)) = (self.newer, self.newer_meta.take()) {
            self.output(b, m);
        }
    }

    /// Copies a decoded frame out, cropped to the sequence's size.
    fn output(&mut self, buf: usize, m: Meta) {
        let Some(seq) = &self.seq else { return };
        let (w, h) = (seq.info.width as usize, seq.info.height as usize);
        let chroma = seq.info.chroma;
        let (cw, ch) = chroma.chroma_size(w as u32, h as u32);
        let (cw, ch) = (cw as usize, ch as usize);
        let b = &self.bufs[buf];
        let mut data = Vec::with_capacity(w * h + 2 * cw * ch);
        for y in 0..h {
            data.extend_from_slice(&b.planes[0][y * b.width..y * b.width + w]);
        }
        for c in 1..3 {
            for y in 0..ch {
                data.extend_from_slice(&b.planes[c][y * b.cwidth..y * b.cwidth + cw]);
            }
        }
        let planes = vec![
            Plane { offset: 0, width: w as u32, height: h as u32 },
            Plane { offset: w * h, width: cw as u32, height: ch as u32 },
            Plane { offset: w * h + cw * ch, width: cw as u32, height: ch as u32 },
        ];
        self.out.push(Frame {
            width: w as u32,
            height: h as u32,
            bit_depth: 8,
            chroma,
            data,
            planes,
            picture_type: m.picture_type,
            temporal_reference: m.temporal_reference,
            progressive_frame: m.progressive_frame,
            top_field_first: m.top_field_first,
            repeat_first_field: m.repeat_first_field,
            field_pictures: m.field_pictures,
            decode_index: m.decode_index,
        });
    }
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a.max(1) } else { gcd(b, a % b) }
}

/// The position of the next `00 00 01` at or after `from`.
fn find_start_code(d: &[u8], from: usize) -> Option<usize> {
    if d.len() < 4 {
        return None;
    }
    let mut i = from;
    while i + 2 < d.len() {
        // Skip quickly: a start code needs d[i+2] == 1 and two zeros before.
        let b = d[i + 2];
        if b > 1 {
            i += 3;
        } else if b == 1 && d[i] == 0 && d[i + 1] == 0 {
            return if i + 3 < d.len() { Some(i) } else { None };
        } else {
            i += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_codes_are_found() {
        let d = [0xff, 0, 0, 1, 0xb3, 1, 2, 0, 0, 0, 1, 0xb5];
        assert_eq!(find_start_code(&d, 0), Some(1));
        assert_eq!(find_start_code(&d, 5), Some(8));
        // A prefix without its value byte is not complete yet.
        assert_eq!(find_start_code(&d[..11], 5), None);
    }
}
