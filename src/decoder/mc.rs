//! Motion-compensated prediction (H.262 7.6): half-sample interpolation
//! from a reference frame or one of its fields, for every prediction mode,
//! into a macroblock-sized prediction buffer.

use crate::frame::ChromaFormat;

/// A decoded picture: the three planes at macroblock-aligned size.
#[derive(Clone)]
pub(crate) struct PicBuf {
    pub planes: [Vec<u8>; 3],
    /// Luma width and height (multiples of 16).
    pub width: usize,
    pub height: usize,
    /// Chroma width and height.
    pub cwidth: usize,
    pub cheight: usize,
}

impl PicBuf {
    pub(crate) fn new(width: usize, height: usize, chroma: ChromaFormat) -> PicBuf {
        let cwidth = width / 2;
        let cheight = if chroma == ChromaFormat::Yuv420 { height / 2 } else { height };
        PicBuf {
            planes: [vec![16; width * height], vec![128; cwidth * cheight], vec![128; cwidth * cheight]],
            width,
            height,
            cwidth,
            cheight,
        }
    }

    /// A buffer with no samples (a placeholder while one is borrowed out).
    pub(crate) fn empty() -> PicBuf {
        PicBuf { planes: [Vec::new(), Vec::new(), Vec::new()], width: 0, height: 0, cwidth: 0, cheight: 0 }
    }

    pub(crate) fn dims(&self, c: usize) -> (usize, usize) {
        if c == 0 { (self.width, self.height) } else { (self.cwidth, self.cheight) }
    }
}

/// Which lines of a reference picture a prediction reads.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum View {
    /// Every line.
    Frame,
    /// The field of this parity (0 top, 1 bottom): lines parity, parity + 2…
    Field(u8),
}

/// The prediction of one macroblock: Y 16×16, Cb and Cr 8×8 (4:2:0) or
/// 8×16 (4:2:2), each with a row stride of its width.
pub(crate) struct MbPred {
    pub y: [u8; 256],
    pub c: [[u8; 128]; 2],
}

impl MbPred {
    pub(crate) fn new() -> Self {
        MbPred { y: [0; 256], c: [[0; 128]; 2] }
    }
}

/// One rectangular prediction, described in luma terms and carried out for
/// all three components.
pub(crate) struct Region {
    /// Horizontal position of the region in the reference (luma samples).
    pub x: i32,
    /// Vertical position in the reference view's lines (luma).
    pub y: i32,
    /// Height in luma lines (16 or 8); the width is always 16.
    pub h: i32,
    /// First destination row in the macroblock buffer (luma), before the
    /// field parity offset.
    pub dst_row: usize,
    /// Destination field parity offset (0 or 1) — not scaled for chroma.
    pub dst_parity: usize,
    /// Destination row step (1, or 2 for a field of a frame macroblock).
    pub dst_step: usize,
    /// Motion vector in half luma samples.
    pub mv: [i32; 2],
    /// Average with what is already in the buffer (`//2`) instead of
    /// overwriting.
    pub avg: bool,
}

/// Forms `region`'s prediction from `refp` into `pred` for every component.
pub(crate) fn predict(refp: &PicBuf, view: View, chroma: ChromaFormat, region: &Region, pred: &mut MbPred) {
    // Luma.
    predict_plane(
        &refp.planes[0],
        refp.width,
        refp.height,
        view,
        region.x,
        region.y,
        16,
        region.h,
        region.mv,
        &mut pred.y,
        16,
        region.dst_row + region.dst_parity,
        region.dst_step,
        region.avg,
    );
    // Chroma vectors and sizes (7.6.3.7): horizontal halved (with "/",
    // truncation toward zero); vertical halved for 4:2:0 only.
    let v420 = chroma == ChromaFormat::Yuv420;
    let mv = [region.mv[0] / 2, if v420 { region.mv[1] / 2 } else { region.mv[1] }];
    let (cy, ch, crow) = if v420 {
        (region.y / 2, region.h / 2, region.dst_row / 2)
    } else {
        (region.y, region.h, region.dst_row)
    };
    for c in 0..2 {
        predict_plane(
            &refp.planes[c + 1],
            refp.cwidth,
            refp.cheight,
            view,
            region.x / 2,
            cy,
            8,
            ch,
            mv,
            &mut pred.c[c],
            8,
            crow + region.dst_parity,
            region.dst_step,
            region.avg,
        );
    }
}

/// Half-sample prediction of a `w` × `h` block of one plane (7.6.4).
#[allow(clippy::too_many_arguments)]
#[inline]
fn predict_plane(
    src: &[u8],
    stride: usize,
    frame_lines: usize,
    view: View,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    mv: [i32; 2],
    dst: &mut [u8],
    dst_stride: usize,
    dst_row: usize,
    dst_step: usize,
    avg: bool,
) {
    let (line0, step, lines) = match view {
        View::Frame => (0usize, 1usize, frame_lines as i32),
        View::Field(p) => (p as usize, 2, (frame_lines / 2) as i32),
    };
    // int_vec = vector DIV 2 (toward −∞), half_flag = the remainder.
    let ix = x + (mv[0] >> 1);
    let iy = y + (mv[1] >> 1);
    let hx = mv[0] & 1;
    let hy = mv[1] & 1;
    let width = stride as i32;
    let inside = ix >= 0 && iy >= 0 && ix + w + hx <= width && iy + h + hy <= lines;
    let at = |sx: i32, sy: i32| -> u32 {
        // Vectors pointing outside the reference are not allowed (7.6.3.8);
        // a damaged stream is clamped to the edge rather than trusted.
        let sx = sx.clamp(0, width - 1) as usize;
        let sy = sy.clamp(0, lines - 1) as usize;
        u32::from(src[(sy * step + line0) * stride + sx])
    };
    if inside {
        let s = crate::dsp::McSrc {
            src,
            off: (iy as usize * step + line0) * stride + ix as usize,
            stride: stride * step,
            w: w as usize,
            h: h as usize,
            hx: hx != 0,
            hy: hy != 0,
        };
        (crate::dsp::dsp().mc)(&s, dst, dst_row * dst_stride, dst_step * dst_stride, avg);
        return;
    }
    for j in 0..h {
        let drow = (dst_row + j as usize * dst_step) * dst_stride;
        let d = &mut dst[drow..drow + w as usize];
        let sy = iy + j;
        for i in 0..w {
            let sx = ix + i;
            let p = match (hx, hy) {
                (0, 0) => at(sx, sy),
                (1, 0) => (at(sx, sy) + at(sx + 1, sy) + 1) >> 1,
                (0, _) => (at(sx, sy) + at(sx, sy + 1) + 1) >> 1,
                _ => (at(sx, sy) + at(sx + 1, sy) + at(sx, sy + 1) + at(sx + 1, sy + 1) + 2) >> 2,
            };
            let di = i as usize;
            d[di] = if avg { ((u32::from(d[di]) + p + 1) >> 1) as u8 } else { p as u8 };
        }
    }
}

/// `a // 2`: division by two rounding half away from zero (H.262 4.1).
#[inline]
pub(crate) fn div2_round(a: i32) -> i32 {
    if a >= 0 { (a + 1) / 2 } else { -((-a + 1) / 2) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp() -> PicBuf {
        let mut p = PicBuf::new(32, 32, ChromaFormat::Yuv420);
        for y in 0..32 {
            for x in 0..32 {
                p.planes[0][y * 32 + x] = (x * 4 + y) as u8;
            }
        }
        p
    }

    #[test]
    fn half_sample_interpolation_matches_7_6_4() {
        let p = ramp();
        let mut pred = MbPred::new();
        let r = Region { x: 8, y: 8, h: 16, dst_row: 0, dst_parity: 0, dst_step: 1, mv: [1, 1], avg: false };
        predict(&p, View::Frame, ChromaFormat::Yuv420, &r, &mut pred);
        // Sample (0,0) of the prediction: average of (8,8) (9,8) (8,9) (9,9).
        let s = |x: usize, y: usize| u32::from(p.planes[0][y * 32 + x]);
        assert_eq!(u32::from(pred.y[0]), (s(8, 8) + s(9, 8) + s(8, 9) + s(9, 9) + 2) / 4);
        // A negative odd vector: −1 → int −1, half 1 (DIV rounds toward −∞).
        let r = Region { mv: [-1, 0], ..r };
        predict(&p, View::Frame, ChromaFormat::Yuv420, &r, &mut pred);
        assert_eq!(u32::from(pred.y[0]), (s(7, 8) + s(8, 8)).div_ceil(2));
    }

    #[test]
    fn field_view_reads_alternate_lines() {
        let p = ramp();
        let mut pred = MbPred::new();
        let r = Region { x: 0, y: 2, h: 8, dst_row: 0, dst_parity: 1, dst_step: 2, mv: [0, 0], avg: false };
        predict(&p, View::Field(1), ChromaFormat::Yuv420, &r, &mut pred);
        // Field line 2 of the bottom field is frame line 5; it lands in MB
        // row 1 (parity 1).
        assert_eq!(pred.y[16], p.planes[0][5 * 32]);
        assert_eq!(pred.y[3 * 16], p.planes[0][7 * 32]);
    }

    #[test]
    fn rounding_division() {
        assert_eq!(div2_round(3), 2);
        assert_eq!(div2_round(-3), -2);
        assert_eq!(div2_round(4), 2);
        assert_eq!(div2_round(-1), -1);
        assert_eq!(div2_round(1), 1);
    }
}
