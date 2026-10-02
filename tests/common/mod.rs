//! Shared helpers: synthetic test video and quality measures.
#![allow(dead_code)]

use mpeg2::{ChromaFormat, Decoder, Encoder, EncoderConfig, Frame};

/// A deterministic pseudo-random generator (xorshift).
pub struct Rng(pub u64);
impl Rng {
    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// Frame `t` of a synthetic sequence: a smooth background drifting right,
/// a textured square moving diagonally, a disc moving left, and a little
/// noise; chroma gradients that move with the background.
pub fn synthetic(width: u32, height: u32, t: u32) -> Frame {
    let mut f = Frame::new(width, height, ChromaFormat::Yuv420);
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15 ^ u64::from(t));
    let (w, h) = (width as i32, height as i32);
    let t = t as i32;
    {
        let y = f.plane_mut(0);
        for j in 0..h {
            for i in 0..w {
                let bx = i + 2 * t;
                let mut v = 60 + ((bx * 3 + j * 2) % 120) / 2 + ((bx / 8 + j / 8) % 2) * 20;
                // textured square
                let sx = i - (8 + 3 * t) % (w.max(40) - 24);
                let sy = j - (6 + 2 * t) % (h.max(40) - 24);
                if (0..24).contains(&sx) && (0..24).contains(&sy) {
                    v = 180 + ((sx ^ sy) & 7) * 8;
                }
                // disc
                let cx = w - 1 - (t * 4) % w.max(1);
                let (dx, dy) = (i - cx, j - h / 2);
                if dx * dx + dy * dy < 100 {
                    v = 30 + (dx + dy).abs();
                }
                v += (rng.next() % 5) as i32 - 2;
                y[(j * w + i) as usize] = v.clamp(0, 255) as u8;
            }
        }
    }
    let (cw, ch) = ChromaFormat::Yuv420.chroma_size(width, height);
    for c in 1..3 {
        let p = f.plane_mut(c);
        for j in 0..ch as i32 {
            for i in 0..cw as i32 {
                let v = 128 + ((i + t + c as i32 * 7) % 40) - 20 + (j % 16) - 8;
                p[(j * cw as i32 + i) as usize] = v.clamp(0, 255) as u8;
            }
        }
    }
    f
}

/// PSNR in dB of `b` against `a` over plane `i`.
pub fn psnr(a: &Frame, b: &Frame, i: usize) -> f64 {
    let (pa, pb) = (a.plane(i), b.plane(i));
    assert_eq!(pa.len(), pb.len());
    let mse: f64 =
        pa.iter().zip(pb).map(|(&x, &y)| (f64::from(x) - f64::from(y)).powi(2)).sum::<f64>() / pa.len() as f64;
    if mse == 0.0 { 99.0 } else { 10.0 * (255.0f64 * 255.0 / mse).log10() }
}

/// Encodes `frames`, returning the stream.
pub fn encode(cfg: EncoderConfig, frames: &[Frame]) -> Vec<u8> {
    let mut enc = Encoder::new(cfg).expect("config");
    let mut out = Vec::new();
    for f in frames {
        out.extend(enc.encode(f).expect("encode"));
    }
    out.extend(enc.finish().expect("finish"));
    out
}

/// Decodes a whole stream fed in `chunk`-byte pieces.
pub fn decode(stream: &[u8], chunk: usize) -> Vec<Frame> {
    let mut dec = Decoder::new();
    let mut out = Vec::new();
    for c in stream.chunks(chunk.max(1)) {
        out.extend(dec.decode(c).expect("decode"));
    }
    out.extend(dec.flush().expect("flush"));
    out
}
