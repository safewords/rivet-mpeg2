//! Where a picture's slices read their predictions from and write their
//! macroblocks to, so that the slices of one picture can be decoded on
//! several threads at once with exactly the result of decoding them one
//! after the other.
//!
//! The picture's macroblock rows are split into bands, one per thread; a
//! thread decodes a run of consecutive slices (in stream order) and writes
//! the macroblocks that fall in its own band straight into the picture.
//! While the slices are decoded nothing reads the picture being written:
//! predictions come from the reference pictures, and a second field that
//! predicts from the first field of its own frame reads a copy of it. So
//! the order of the writes is all that can differ from serial decoding,
//! and for that each macroblock remembers which slice wrote it last: a
//! macroblock a slice writes outside its thread's band (a damaged stream,
//! or an ISO/IEC 11172-2 slice running over several rows into the next
//! thread's) is held back and written afterwards, unless a later slice —
//! in stream order — wrote that macroblock too. Every macroblock ends up
//! with the content of the last slice that wrote it, as in serial decoding.

use super::mc::{MbPred, PicBuf};
use crate::frame::ChromaFormat;

/// The pictures a slice predicts from.
pub(crate) struct Refs<'a> {
    /// Forward (0) and backward (1) reference frames.
    pub dir: [Option<&'a PicBuf>; 2],
    /// The frame being decoded, as it was before this picture's slices: the
    /// first field of a field pair, for its second field.
    pub cur: Option<&'a PicBuf>,
}

/// The geometry of the picture being written.
#[derive(Clone, Copy)]
pub(crate) struct Layout {
    pub mb_width: usize,
    /// Frame picture (else a field picture of parity `parity`).
    pub frame: bool,
    pub parity: usize,
    pub chroma: ChromaFormat,
    pub width: usize,
    pub cwidth: usize,
}

impl Layout {
    /// Chroma lines per macroblock.
    fn ch(&self) -> usize {
        if self.chroma == ChromaFormat::Yuv420 { 8 } else { 16 }
    }

    /// Frame lines spanned by macroblock row `r` (luma, chroma): its first
    /// line. A field macroblock row spans twice the lines of a frame one.
    pub(crate) fn first_lines(&self, r: usize) -> [usize; 2] {
        let k = if self.frame { 1 } else { 2 };
        [r * 16 * k, r * self.ch() * k]
    }

    fn line(&self, row: usize) -> usize {
        if self.frame { row } else { 2 * row + self.parity }
    }

    /// Writes `mb` at macroblock `(mbx, mby)` into planes whose first line is
    /// frame line `l0[0]` (luma) / `l0[1]` (chroma).
    fn write(&self, planes: &mut [&mut [u8]; 3], l0: [usize; 2], mbx: usize, mby: usize, mb: &MbPred) {
        let w = self.width;
        for y in 0..16 {
            let o = (self.line(mby * 16 + y) - l0[0]) * w + mbx * 16;
            planes[0][o..o + 16].copy_from_slice(&mb.y[y * 16..y * 16 + 16]);
        }
        let (cw, ch) = (self.cwidth, self.ch());
        for c in 0..2 {
            for y in 0..ch {
                let o = (self.line(mby * ch + y) - l0[1]) * cw + mbx * 8;
                planes[c + 1][o..o + 8].copy_from_slice(&mb.c[c][y * 8..y * 8 + 8]);
            }
        }
    }
}

/// A macroblock written outside its thread's band: (slice index + 1,
/// address, content).
pub(crate) type Held = (u32, usize, Box<MbPred>);

/// The macroblock rows `rows` of the picture being decoded, owned by one
/// thread.
pub(crate) struct Band<'a> {
    pub layout: Layout,
    pub rows: std::ops::Range<usize>,
    /// The planes from the band's first line (luma, then chroma).
    pub planes: [&'a mut [u8]; 3],
    pub l0: [usize; 2],
    /// Per macroblock of the band: the slice that last wrote it (index + 1;
    /// 0 none).
    pub writer: &'a mut [u32],
    /// The slice being decoded (index + 1).
    pub slice: u32,
    pub held: Vec<Held>,
}

impl Band<'_> {
    /// Stores the reconstructed macroblock at `addr`.
    pub(crate) fn put(&mut self, addr: usize, mb: &MbPred) {
        let mbw = self.layout.mb_width;
        let (mbx, mby) = (addr % mbw, addr / mbw);
        if self.rows.contains(&mby) {
            self.writer[addr - self.rows.start * mbw] = self.slice;
            self.layout.write(&mut self.planes, self.l0, mbx, mby, mb);
        } else {
            self.held.push((self.slice, addr, Box::new(MbPred { y: mb.y, c: mb.c })));
        }
    }
}

/// Splits `buf` (the picture being decoded, laid out as `layout`) and the
/// per-macroblock `writer` map into bands of macroblock rows, the band `i`
/// rows `bounds[i]..bounds[i + 1]`.
pub(crate) fn split<'a>(buf: &'a mut PicBuf, writer: &'a mut [u32], layout: Layout, bounds: &[usize]) -> Vec<Band<'a>> {
    let [p0, p1, p2] = &mut buf.planes;
    let (mut y, mut u, mut v) = (&mut p0[..], &mut p1[..], &mut p2[..]);
    let mut wr = writer;
    let mut bands = Vec::with_capacity(bounds.len() - 1);
    let mut taken = [0usize; 2];
    for (k, pair) in bounds.windows(2).enumerate() {
        let (r0, r1) = (pair[0], pair[1]);
        let last = k + 2 == bounds.len();
        let [l0, c0] = layout.first_lines(r0);
        let [l1, c1] = layout.first_lines(r1);
        debug_assert_eq!([l0, c0], taken);
        // The last band takes whatever is left (the planes may be taller
        // than the rows, never shorter).
        let take = |s: &mut &'a mut [u8], lines: usize, w: usize| -> &'a mut [u8] {
            let n = if last { s.len() } else { (lines * w).min(s.len()) };
            let (a, b) = std::mem::take(s).split_at_mut(n);
            *s = b;
            a
        };
        let by = take(&mut y, l1 - l0, layout.width);
        let bu = take(&mut u, c1 - c0, layout.cwidth);
        let bv = take(&mut v, c1 - c0, layout.cwidth);
        let n = if last { wr.len() } else { ((r1 - r0) * layout.mb_width).min(wr.len()) };
        let (bw, rest) = std::mem::take(&mut wr).split_at_mut(n);
        wr = rest;
        taken = [l1, c1];
        bands.push(Band { layout, rows: r0..r1, planes: [by, bu, bv], l0: [l0, c0], writer: bw, slice: 0, held: Vec::new() });
    }
    bands
}
