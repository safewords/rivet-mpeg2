//! Motion estimation for the encoder: frame vectors in half samples, found
//! by a coarse grid search, a small-diamond refinement in whole samples and
//! a half-sample refinement, all by luma SAD.

use crate::decoder::mc::{self, MbPred, PicBuf, Region, View};
use crate::frame::ChromaFormat;

/// SAD of the 16×16 luma block at (`x`, `y`) of `src` against `refp`
/// displaced by the whole-sample vector (`dx`, `dy`), stopping early once it
/// passes `limit`.
fn sad_full(src: &PicBuf, refp: &PicBuf, x: usize, y: usize, dx: i32, dy: i32, limit: u32) -> u32 {
    let w = src.width;
    let rx = (x as i32 + dx) as usize;
    let ry = (y as i32 + dy) as usize;
    let mut sad = 0u32;
    for j in 0..16 {
        let a = &src.planes[0][(y + j) * w + x..][..16];
        let b = &refp.planes[0][(ry + j) * w + rx..][..16];
        for i in 0..16 {
            sad += u32::from(a[i].abs_diff(b[i]));
        }
        if sad >= limit {
            return sad;
        }
    }
    sad
}

/// SAD of a source macroblock's luma against a prediction.
pub(crate) fn sad_pred(src: &PicBuf, x: usize, y: usize, pred: &[u8; 256]) -> u32 {
    let w = src.width;
    let mut sad = 0u32;
    for j in 0..16 {
        let a = &src.planes[0][(y + j) * w + x..][..16];
        for i in 0..16 {
            sad += u32::from(a[i].abs_diff(pred[j * 16 + i]));
        }
    }
    sad
}

/// Forms the frame prediction of the macroblock at (`x`, `y`) from `refp`
/// with the half-sample vector `mv` (all components).
pub(crate) fn predict(refp: &PicBuf, x: usize, y: usize, mv: [i32; 2], avg: bool, pred: &mut MbPred) {
    let r = Region { x: x as i32, y: y as i32, h: 16, dst_row: 0, dst_parity: 0, dst_step: 1, mv, avg };
    mc::predict(refp, View::Frame, ChromaFormat::Yuv420, &r, pred);
}

/// The best half-sample vector for the macroblock at (`x`, `y`) within
/// ±`range` whole samples (and within `[low, high]` half samples, the f_code
/// limit), keeping the whole prediction inside the reference picture.
/// `candidates` are starting points in half samples. Returns the vector and
/// its SAD.
#[allow(clippy::too_many_arguments)]
pub(crate) fn search(
    src: &PicBuf,
    refp: &PicBuf,
    x: usize,
    y: usize,
    range: i32,
    low: i32,
    high: i32,
    candidates: &[[i32; 2]],
) -> ([i32; 2], u32) {
    let (w, h) = (src.width as i32, src.height as i32);
    let (xi, yi) = (x as i32, y as i32);
    // Whole-sample limits: the 16×16 block (plus one column / row for a
    // half-sample offset) stays inside the picture.
    let min_dx = (-xi).max(-range).max(low.div_euclid(2));
    let max_dx = (w - 16 - xi - 1).min(range).min(high.div_euclid(2)).max(min_dx);
    let min_dy = (-yi).max(-range).max(low.div_euclid(2));
    let max_dy = (h - 16 - yi - 1).min(range).min(high.div_euclid(2)).max(min_dy);
    let ok = |dx: i32, dy: i32| (min_dx..=max_dx).contains(&dx) && (min_dy..=max_dy).contains(&dy);

    let mut best = (0i32, 0i32);
    let mut best_sad = if ok(0, 0) { sad_full(src, refp, x, y, 0, 0, u32::MAX) } else { u32::MAX };
    if !ok(0, 0) {
        best = (min_dx.max(0).min(max_dx), min_dy.max(0).min(max_dy));
        best_sad = sad_full(src, refp, x, y, best.0, best.1, u32::MAX);
    }
    let try_at = |dx: i32, dy: i32, best: &mut (i32, i32), best_sad: &mut u32| {
        if ok(dx, dy) && (dx, dy) != *best {
            let s = sad_full(src, refp, x, y, dx, dy, *best_sad);
            if s < *best_sad {
                *best_sad = s;
                *best = (dx, dy);
            }
        }
    };
    for c in candidates {
        try_at(c[0] >> 1, c[1] >> 1, &mut best, &mut best_sad);
    }
    // Coarse grid.
    let step = 4;
    let mut dy = -range;
    while dy <= range {
        let mut dx = -range;
        while dx <= range {
            try_at(dx, dy, &mut best, &mut best_sad);
            dx += step;
        }
        dy += step;
    }
    // Diamond refinement.
    for size in [2, 1] {
        loop {
            let start = best;
            for (ox, oy) in [(size, 0), (-size, 0), (0, size), (0, -size)] {
                try_at(start.0 + ox, start.1 + oy, &mut best, &mut best_sad);
            }
            if best == start {
                break;
            }
        }
    }
    // Half-sample refinement around the whole-sample best.
    let mut mv = [best.0 * 2, best.1 * 2];
    let mut pred = MbPred::new();
    let mut best_half = best_sad;
    let centre = mv;
    for oy in -1..=1 {
        for ox in -1..=1 {
            if ox == 0 && oy == 0 {
                continue;
            }
            let cand = [centre[0] + ox, centre[1] + oy];
            if cand[0] < low || cand[0] > high || cand[1] < low || cand[1] > high {
                continue;
            }
            // Inside the picture, including the extra half-sample column.
            let ix = xi + (cand[0] >> 1);
            let iy = yi + (cand[1] >> 1);
            if ix < 0 || iy < 0 || ix + 16 + (cand[0] & 1) > w || iy + 16 + (cand[1] & 1) > h {
                continue;
            }
            predict(refp, x, y, cand, false, &mut pred);
            let s = sad_pred(src, x, y, &pred.y);
            if s < best_half {
                best_half = s;
                mv = cand;
            }
        }
    }
    (mv, best_half)
}
