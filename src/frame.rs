//! The planar frame the decoder hands back and the encoder takes.

/// Chroma sampling of a frame (H.262 `chroma_format`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromaFormat {
    /// 4:2:0 (`chroma_format` 1): chroma at half the width and half the
    /// height. Every MPEG-1 stream and every Main Profile stream.
    Yuv420,
    /// 4:2:2 (`chroma_format` 2): chroma at half the width, full height.
    /// The 4:2:2 profile.
    Yuv422,
    /// 4:4:4 (`chroma_format` 3). Named so a caller can see what a stream
    /// declares; the decoder refuses it and the encoder does not write it.
    Yuv444,
}

impl ChromaFormat {
    /// `(SubWidthC, SubHeightC)` — how many luma samples one chroma sample
    /// covers in each direction.
    pub fn subsampling(self) -> (u32, u32) {
        match self {
            ChromaFormat::Yuv420 => (2, 2),
            ChromaFormat::Yuv422 => (2, 1),
            ChromaFormat::Yuv444 => (1, 1),
        }
    }

    /// From `chroma_format` (1, 2 or 3; 0 is reserved).
    pub fn from_code(code: u32) -> Option<Self> {
        Some(match code {
            1 => ChromaFormat::Yuv420,
            2 => ChromaFormat::Yuv422,
            3 => ChromaFormat::Yuv444,
            _ => return None,
        })
    }

    /// The width and height of a chroma plane of a `width` × `height` frame
    /// (rounded up, so an odd luma size keeps its last chroma column / row).
    pub fn chroma_size(self, width: u32, height: u32) -> (u32, u32) {
        let (sw, sh) = self.subsampling();
        (width.div_ceil(sw), height.div_ceil(sh))
    }
}

/// The coding type of a picture (H.262 `picture_coding_type`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PictureType {
    /// Intra-coded.
    I,
    /// Predictive-coded (forward prediction from the previous I or P frame).
    P,
    /// Bidirectionally predictive-coded.
    B,
    /// ISO/IEC 11172-2 DC-coded picture: intra, DC coefficients only (an
    /// MPEG-1 picture type H.262 decoders also decode, 8.1).
    D,
}

/// One plane of a [`Frame`]: where it sits in the frame's data. Samples are
/// one byte each and tightly packed (stride == width).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Plane {
    /// Byte offset of the plane's first sample in [`Frame::data`].
    pub offset: usize,
    /// Width in samples.
    pub width: u32,
    /// Height in samples.
    pub height: u32,
}

impl Plane {
    /// The plane's size in bytes.
    pub fn len(&self) -> usize {
        self.width as usize * self.height as usize
    }

    /// Whether the plane holds no samples.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A frame: one buffer holding the planes one after the other (Y, then Cb,
/// then Cr — the layout of a packed planar frame), 8 bits per sample, cropped
/// to the sequence's `horizontal_size` × `vertical_size`.
///
/// The decoder returns frames in display order with the picture's timing
/// flags; a frame coded as two field pictures comes back as one frame. The
/// encoder reads only the size, the chroma format and the samples.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Luma width.
    pub width: u32,
    /// Luma height.
    pub height: u32,
    /// Bits per sample: always 8 (H.262 has no other depth).
    pub bit_depth: u32,
    /// Chroma sampling.
    pub chroma: ChromaFormat,
    /// The samples of every plane, packed.
    pub data: Vec<u8>,
    /// Y, then Cb, then Cr.
    pub planes: Vec<Plane>,
    /// The coding type of the picture (of the first field, for a frame
    /// coded as two field pictures).
    pub picture_type: PictureType,
    /// `temporal_reference` of the picture (of its first field): the display
    /// position within the group of pictures, modulo 1024.
    pub temporal_reference: u16,
    /// `progressive_frame`: the two fields are from the same instant.
    pub progressive_frame: bool,
    /// `top_field_first` as coded: in an interlaced sequence, the top field
    /// is output first. In a progressive sequence it is zero unless
    /// `repeat_first_field` is set, where it means "show the frame three
    /// times" (see [`Self::repeat_first_field`]). True for MPEG-1.
    pub top_field_first: bool,
    /// `repeat_first_field`: in an interlaced sequence, the first field is
    /// shown again after the second (3:2 pulldown); in a progressive sequence
    /// the frame is shown twice, or three times when `top_field_first` is
    /// also set. Reported, not applied: the decoder returns each frame once.
    pub repeat_first_field: bool,
    /// The frame was coded as two field pictures.
    pub field_pictures: bool,
    /// Decode-order index of the frame (0 for the first decoded frame).
    pub decode_index: u64,
}

impl Frame {
    /// A black frame (Y 16, Cb and Cr 128) of the given size and chroma
    /// format, to be filled by the caller (for the encoder).
    pub fn new(width: u32, height: u32, chroma: ChromaFormat) -> Frame {
        let (cw, ch) = chroma.chroma_size(width, height);
        let luma = width as usize * height as usize;
        let c = cw as usize * ch as usize;
        let mut data = vec![16u8; luma + 2 * c];
        data[luma..].fill(128);
        let planes = vec![
            Plane { offset: 0, width, height },
            Plane { offset: luma, width: cw, height: ch },
            Plane { offset: luma + c, width: cw, height: ch },
        ];
        Frame {
            width,
            height,
            bit_depth: 8,
            chroma,
            data,
            planes,
            picture_type: PictureType::I,
            temporal_reference: 0,
            progressive_frame: true,
            top_field_first: true,
            repeat_first_field: false,
            field_pictures: false,
            decode_index: 0,
        }
    }

    /// The samples of plane `i` (0 Y, 1 Cb, 2 Cr).
    pub fn plane(&self, i: usize) -> &[u8] {
        let p = &self.planes[i];
        &self.data[p.offset..p.offset + p.len()]
    }

    /// The samples of plane `i`, writable.
    pub fn plane_mut(&mut self, i: usize) -> &mut [u8] {
        let p = self.planes[i];
        &mut self.data[p.offset..p.offset + p.len()]
    }

    /// The planes concatenated: Y then Cb then Cr.
    pub fn packed(&self) -> &[u8] {
        &self.data
    }
}
