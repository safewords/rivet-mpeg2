//! The headers above the slice layer (H.262 6.2.2, 6.2.3): parsing for the
//! decoder, writing for the encoder.

use crate::bits::{BitReader, BitWriter};
use crate::error::{Result, invalid};
use crate::tables::SCAN;

/// Start code values (Table 6-1).
pub(crate) const PICTURE_START: u8 = 0x00;
pub(crate) const SLICE_MIN: u8 = 0x01;
pub(crate) const SLICE_MAX: u8 = 0xaf;
pub(crate) const USER_DATA: u8 = 0xb2;
pub(crate) const SEQUENCE_HEADER: u8 = 0xb3;
pub(crate) const SEQUENCE_ERROR: u8 = 0xb4;
pub(crate) const EXTENSION: u8 = 0xb5;
pub(crate) const SEQUENCE_END: u8 = 0xb7;
pub(crate) const GROUP_START: u8 = 0xb8;

/// extension_start_code_identifier values (Table 6-2).
pub(crate) const EXT_SEQUENCE: u32 = 1;
pub(crate) const EXT_SEQUENCE_DISPLAY: u32 = 2;
pub(crate) const EXT_QUANT_MATRIX: u32 = 3;
pub(crate) const EXT_SEQUENCE_SCALABLE: u32 = 5;
pub(crate) const EXT_PICTURE_CODING: u32 = 8;

/// picture_structure values (1 is the top field).
pub(crate) const BOTTOM_FIELD: u8 = 2;
pub(crate) const FRAME_PICTURE: u8 = 3;

/// Reads a quantiser matrix: 64 8-bit values in the default zigzag order
/// (7.3.1), returned in raster order.
fn read_matrix(r: &mut BitReader) -> Result<[u8; 64]> {
    let mut m = [0u8; 64];
    for n in 0..64 {
        let v = r.read(8) as u8;
        if v == 0 {
            return Err(invalid("quantiser matrix value of zero"));
        }
        m[SCAN[0][n] as usize] = v;
    }
    Ok(m)
}

fn write_matrix(w: &mut BitWriter, m: &[u8; 64]) {
    for n in 0..64 {
        w.put(8, u32::from(m[SCAN[0][n] as usize]));
    }
}

fn marker(r: &mut BitReader, what: &str) -> Result<()> {
    // Some encoders write a zero marker bit; a strict decoder gains nothing
    // by refusing their streams. Read it and carry on.
    let _ = what;
    r.skip(1);
    Ok(())
}

/// sequence_header() (6.2.2.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SequenceHeader {
    pub horizontal_size_value: u32,
    pub vertical_size_value: u32,
    pub aspect_ratio_information: u8,
    pub frame_rate_code: u8,
    pub bit_rate_value: u32,
    pub vbv_buffer_size_value: u32,
    pub constrained_parameters_flag: bool,
    pub intra_quantiser_matrix: Option<[u8; 64]>,
    pub non_intra_quantiser_matrix: Option<[u8; 64]>,
}

impl SequenceHeader {
    pub(crate) fn parse(r: &mut BitReader) -> Result<Self> {
        let horizontal_size_value = r.read(12);
        let vertical_size_value = r.read(12);
        let aspect_ratio_information = r.read(4) as u8;
        let frame_rate_code = r.read(4) as u8;
        let bit_rate_value = r.read(18);
        marker(r, "sequence header")?;
        let vbv_buffer_size_value = r.read(10);
        let constrained_parameters_flag = r.read_bit();
        let intra_quantiser_matrix = if r.read_bit() {
            Some(read_matrix(r)?)
        } else {
            None
        };
        let non_intra_quantiser_matrix = if r.read_bit() {
            Some(read_matrix(r)?)
        } else {
            None
        };
        if r.overrun() {
            return Err(invalid("sequence header cut short"));
        }
        Ok(SequenceHeader {
            horizontal_size_value,
            vertical_size_value,
            aspect_ratio_information,
            frame_rate_code,
            bit_rate_value,
            vbv_buffer_size_value,
            constrained_parameters_flag,
            intra_quantiser_matrix,
            non_intra_quantiser_matrix,
        })
    }

    pub(crate) fn write(&self, w: &mut BitWriter) {
        w.start_code(SEQUENCE_HEADER);
        w.put(12, self.horizontal_size_value);
        w.put(12, self.vertical_size_value);
        w.put(4, u32::from(self.aspect_ratio_information));
        w.put(4, u32::from(self.frame_rate_code));
        w.put(18, self.bit_rate_value);
        w.put_bit(true);
        w.put(10, self.vbv_buffer_size_value);
        w.put_bit(self.constrained_parameters_flag);
        w.put_bit(self.intra_quantiser_matrix.is_some());
        if let Some(m) = &self.intra_quantiser_matrix {
            write_matrix(w, m);
        }
        w.put_bit(self.non_intra_quantiser_matrix.is_some());
        if let Some(m) = &self.non_intra_quantiser_matrix {
            write_matrix(w, m);
        }
    }
}

/// sequence_extension() (6.2.2.3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SequenceExtension {
    pub profile_and_level_indication: u8,
    pub progressive_sequence: bool,
    pub chroma_format: u8,
    pub horizontal_size_extension: u32,
    pub vertical_size_extension: u32,
    pub bit_rate_extension: u32,
    pub vbv_buffer_size_extension: u32,
    pub low_delay: bool,
    pub frame_rate_extension_n: u8,
    pub frame_rate_extension_d: u8,
}

impl SequenceExtension {
    /// Parses the body after extension_start_code_identifier.
    pub(crate) fn parse(r: &mut BitReader) -> Result<Self> {
        let profile_and_level_indication = r.read(8) as u8;
        let progressive_sequence = r.read_bit();
        let chroma_format = r.read(2) as u8;
        let horizontal_size_extension = r.read(2);
        let vertical_size_extension = r.read(2);
        let bit_rate_extension = r.read(12);
        marker(r, "sequence extension")?;
        let vbv_buffer_size_extension = r.read(8);
        let low_delay = r.read_bit();
        let frame_rate_extension_n = r.read(2) as u8;
        let frame_rate_extension_d = r.read(5) as u8;
        if r.overrun() {
            return Err(invalid("sequence extension cut short"));
        }
        Ok(SequenceExtension {
            profile_and_level_indication,
            progressive_sequence,
            chroma_format,
            horizontal_size_extension,
            vertical_size_extension,
            bit_rate_extension,
            vbv_buffer_size_extension,
            low_delay,
            frame_rate_extension_n,
            frame_rate_extension_d,
        })
    }

    pub(crate) fn write(&self, w: &mut BitWriter) {
        w.start_code(EXTENSION);
        w.put(4, EXT_SEQUENCE);
        w.put(8, u32::from(self.profile_and_level_indication));
        w.put_bit(self.progressive_sequence);
        w.put(2, u32::from(self.chroma_format));
        w.put(2, self.horizontal_size_extension);
        w.put(2, self.vertical_size_extension);
        w.put(12, self.bit_rate_extension);
        w.put_bit(true);
        w.put(8, self.vbv_buffer_size_extension);
        w.put_bit(self.low_delay);
        w.put(2, u32::from(self.frame_rate_extension_n));
        w.put(5, u32::from(self.frame_rate_extension_d));
    }
}

/// sequence_display_extension() (6.2.2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SequenceDisplayExtension {
    pub video_format: u8,
    pub colour_description: Option<(u8, u8, u8)>,
    pub display_horizontal_size: u32,
    pub display_vertical_size: u32,
}

impl SequenceDisplayExtension {
    pub(crate) fn parse(r: &mut BitReader) -> Result<Self> {
        let video_format = r.read(3) as u8;
        let colour_description = if r.read_bit() {
            Some((r.read(8) as u8, r.read(8) as u8, r.read(8) as u8))
        } else {
            None
        };
        let display_horizontal_size = r.read(14);
        marker(r, "sequence display extension")?;
        let display_vertical_size = r.read(14);
        if r.overrun() {
            return Err(invalid("sequence display extension cut short"));
        }
        Ok(SequenceDisplayExtension {
            video_format,
            colour_description,
            display_horizontal_size,
            display_vertical_size,
        })
    }
}

/// group_of_pictures_header() (6.2.2.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct GopHeader {
    pub time_code: u32,
    pub closed_gop: bool,
    pub broken_link: bool,
}

impl GopHeader {
    pub(crate) fn parse(r: &mut BitReader) -> Result<Self> {
        let time_code = r.read(25);
        let closed_gop = r.read_bit();
        let broken_link = r.read_bit();
        if r.overrun() {
            return Err(invalid("group of pictures header cut short"));
        }
        Ok(GopHeader {
            time_code,
            closed_gop,
            broken_link,
        })
    }

    pub(crate) fn write(&self, w: &mut BitWriter) {
        w.start_code(GROUP_START);
        w.put(25, self.time_code);
        w.put_bit(self.closed_gop);
        w.put_bit(self.broken_link);
    }
}

/// picture_header() (6.2.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PictureHeader {
    pub temporal_reference: u16,
    pub picture_coding_type: u8,
    pub vbv_delay: u16,
    pub full_pel_forward_vector: bool,
    pub forward_f_code: u8,
    pub full_pel_backward_vector: bool,
    pub backward_f_code: u8,
}

impl PictureHeader {
    pub(crate) fn parse(r: &mut BitReader) -> Result<Self> {
        let temporal_reference = r.read(10) as u16;
        let picture_coding_type = r.read(3) as u8;
        let vbv_delay = r.read(16) as u16;
        let mut h = PictureHeader {
            temporal_reference,
            picture_coding_type,
            vbv_delay,
            full_pel_forward_vector: false,
            forward_f_code: 7,
            full_pel_backward_vector: false,
            backward_f_code: 7,
        };
        if picture_coding_type == 2 || picture_coding_type == 3 {
            h.full_pel_forward_vector = r.read_bit();
            h.forward_f_code = r.read(3) as u8;
        }
        if picture_coding_type == 3 {
            h.full_pel_backward_vector = r.read_bit();
            h.backward_f_code = r.read(3) as u8;
        }
        // extra_information_picture is skipped.
        while r.read_bit() {
            r.skip(8);
            if r.overrun() {
                return Err(invalid("picture header cut short"));
            }
        }
        if r.overrun() {
            return Err(invalid("picture header cut short"));
        }
        Ok(h)
    }

    pub(crate) fn write(&self, w: &mut BitWriter) {
        w.start_code(PICTURE_START);
        w.put(10, u32::from(self.temporal_reference));
        w.put(3, u32::from(self.picture_coding_type));
        w.put(16, u32::from(self.vbv_delay));
        if self.picture_coding_type == 2 || self.picture_coding_type == 3 {
            w.put_bit(self.full_pel_forward_vector);
            w.put(3, u32::from(self.forward_f_code));
        }
        if self.picture_coding_type == 3 {
            w.put_bit(self.full_pel_backward_vector);
            w.put(3, u32::from(self.backward_f_code));
        }
        w.put_bit(false); // extra_bit_picture
    }
}

/// picture_coding_extension() (6.2.3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PictureCodingExtension {
    /// `f_code[s][t]`: s 0 forward / 1 backward, t 0 horizontal / 1 vertical.
    pub f_code: [[u8; 2]; 2],
    pub intra_dc_precision: u8,
    pub picture_structure: u8,
    pub top_field_first: bool,
    pub frame_pred_frame_dct: bool,
    pub concealment_motion_vectors: bool,
    pub q_scale_type: bool,
    pub intra_vlc_format: bool,
    pub alternate_scan: bool,
    pub repeat_first_field: bool,
    pub chroma_420_type: bool,
    pub progressive_frame: bool,
}

impl PictureCodingExtension {
    pub(crate) fn parse(r: &mut BitReader) -> Result<Self> {
        let f_code = [
            [r.read(4) as u8, r.read(4) as u8],
            [r.read(4) as u8, r.read(4) as u8],
        ];
        let e = PictureCodingExtension {
            f_code,
            intra_dc_precision: r.read(2) as u8,
            picture_structure: r.read(2) as u8,
            top_field_first: r.read_bit(),
            frame_pred_frame_dct: r.read_bit(),
            concealment_motion_vectors: r.read_bit(),
            q_scale_type: r.read_bit(),
            intra_vlc_format: r.read_bit(),
            alternate_scan: r.read_bit(),
            repeat_first_field: r.read_bit(),
            chroma_420_type: r.read_bit(),
            progressive_frame: r.read_bit(),
        };
        // composite_display_flag and its fields are not used by decoding.
        if r.overrun() {
            return Err(invalid("picture coding extension cut short"));
        }
        if e.picture_structure == 0 {
            return Err(invalid("picture_structure 0 (reserved)"));
        }
        Ok(e)
    }

    pub(crate) fn write(&self, w: &mut BitWriter) {
        w.start_code(EXTENSION);
        w.put(4, EXT_PICTURE_CODING);
        for s in 0..2 {
            for t in 0..2 {
                w.put(4, u32::from(self.f_code[s][t]));
            }
        }
        w.put(2, u32::from(self.intra_dc_precision));
        w.put(2, u32::from(self.picture_structure));
        w.put_bit(self.top_field_first);
        w.put_bit(self.frame_pred_frame_dct);
        w.put_bit(self.concealment_motion_vectors);
        w.put_bit(self.q_scale_type);
        w.put_bit(self.intra_vlc_format);
        w.put_bit(self.alternate_scan);
        w.put_bit(self.repeat_first_field);
        w.put_bit(self.chroma_420_type);
        w.put_bit(self.progressive_frame);
        w.put_bit(false); // composite_display_flag
    }

    /// The values H.262 D.9.14 gives an ISO/IEC 11172-2 picture.
    pub(crate) fn mpeg1(h: &PictureHeader) -> Self {
        PictureCodingExtension {
            f_code: [
                [h.forward_f_code, h.forward_f_code],
                [h.backward_f_code, h.backward_f_code],
            ],
            intra_dc_precision: 0,
            picture_structure: FRAME_PICTURE,
            top_field_first: true,
            frame_pred_frame_dct: true,
            concealment_motion_vectors: false,
            q_scale_type: false,
            intra_vlc_format: false,
            alternate_scan: false,
            repeat_first_field: false,
            chroma_420_type: true,
            progressive_frame: true,
        }
    }
}

/// quant_matrix_extension() (6.2.3.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct QuantMatrixExtension {
    pub intra: Option<[u8; 64]>,
    pub non_intra: Option<[u8; 64]>,
    pub chroma_intra: Option<[u8; 64]>,
    pub chroma_non_intra: Option<[u8; 64]>,
}

impl QuantMatrixExtension {
    pub(crate) fn parse(r: &mut BitReader) -> Result<Self> {
        let mut m = [None; 4];
        for slot in &mut m {
            if r.read_bit() {
                *slot = Some(read_matrix(r)?);
            }
        }
        if r.overrun() {
            return Err(invalid("quant matrix extension cut short"));
        }
        Ok(QuantMatrixExtension {
            intra: m[0],
            non_intra: m[1],
            chroma_intra: m[2],
            chroma_non_intra: m[3],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_round_trip() {
        let mut m = [0u8; 64];
        for (i, v) in m.iter_mut().enumerate() {
            *v = (i as u8) + 1;
        }
        let sh = SequenceHeader {
            horizontal_size_value: 720,
            vertical_size_value: 576,
            aspect_ratio_information: 3,
            frame_rate_code: 3,
            bit_rate_value: 12345,
            vbv_buffer_size_value: 112,
            constrained_parameters_flag: false,
            intra_quantiser_matrix: Some(m),
            non_intra_quantiser_matrix: None,
        };
        let pce = PictureCodingExtension {
            f_code: [[2, 3], [4, 5]],
            intra_dc_precision: 2,
            picture_structure: BOTTOM_FIELD,
            top_field_first: true,
            frame_pred_frame_dct: false,
            concealment_motion_vectors: true,
            q_scale_type: true,
            intra_vlc_format: true,
            alternate_scan: true,
            repeat_first_field: false,
            chroma_420_type: false,
            progressive_frame: false,
        };
        let mut w = BitWriter::new();
        sh.write(&mut w);
        pce.write(&mut w);
        let b = w.finish();
        let mut r = BitReader::new(&b[4..]);
        assert_eq!(SequenceHeader::parse(&mut r).unwrap(), sh);
        let pos = b
            .windows(4)
            .rposition(|x| x == [0, 0, 1, EXTENSION])
            .unwrap();
        let mut r = BitReader::new(&b[pos + 4..]);
        assert_eq!(r.read(4), EXT_PICTURE_CODING);
        assert_eq!(PictureCodingExtension::parse(&mut r).unwrap(), pce);
    }
}
