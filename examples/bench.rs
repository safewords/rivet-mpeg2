//! End-to-end throughput: decoding a stream, and encoding natural pictures
//! scaled to a chosen size. Each measurement is repeated and the fastest
//! run reported (the least disturbed by other work on the machine).
//!
//! ```text
//! # decode a stream `reps` times on `threads` threads (0: one per core)
//! cargo run --release --example bench -- dec <stream.m2v> <reps> [threads]
//! # encode `frames` frames of <src.m2v>, decoded and scaled (bilinear) to
//! # w x h, `reps` times; with an output path, write the stream there
//! cargo run --release --example bench -- enc <src.m2v> <w> <h> <frames> <reps> [threads] [out.m2v]
//! ```
//!
//! The source used for the README's figures is tcela-7 of the conformance
//! suite (Mobile & Calendar, 720x480); see tools/bench.sh.

use mpeg2::{ChromaFormat, Decoder, Encoder, EncoderConfig, Frame};
use std::time::Instant;

fn decode_all(data: &[u8], threads: usize) -> Vec<Frame> {
    let mut dec = Decoder::new();
    dec.set_threads(threads);
    let mut frames = dec.decode(data).expect("decode");
    frames.extend(dec.flush().expect("flush"));
    frames
}

/// Bilinear scaling of one plane (centre-aligned samples).
fn scale_plane(src: &[u8], sw: usize, sh: usize, dst: &mut [u8], dw: usize, dh: usize) {
    for y in 0..dh {
        let fy = ((y as f64 + 0.5) * sh as f64 / dh as f64 - 0.5).clamp(0.0, (sh - 1) as f64);
        let y0 = fy.floor() as usize;
        let y1 = (y0 + 1).min(sh - 1);
        let wy = fy - y0 as f64;
        for x in 0..dw {
            let fx = ((x as f64 + 0.5) * sw as f64 / dw as f64 - 0.5).clamp(0.0, (sw - 1) as f64);
            let x0 = fx.floor() as usize;
            let x1 = (x0 + 1).min(sw - 1);
            let wx = fx - x0 as f64;
            let p = |xx: usize, yy: usize| f64::from(src[yy * sw + xx]);
            let v = (p(x0, y0) * (1.0 - wx) + p(x1, y0) * wx) * (1.0 - wy) + (p(x0, y1) * (1.0 - wx) + p(x1, y1) * wx) * wy;
            dst[y * dw + x] = v.round().clamp(0.0, 255.0) as u8;
        }
    }
}

fn scale(f: &Frame, w: u32, h: u32) -> Frame {
    let mut out = Frame::new(w, h, ChromaFormat::Yuv420);
    for c in 0..3 {
        let sp = f.planes[c];
        let dp = out.planes[c];
        let src = f.plane(c).to_vec();
        scale_plane(&src, sp.width as usize, sp.height as usize, out.plane_mut(c), dp.width as usize, dp.height as usize);
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let usage = || -> ! {
        eprintln!("usage: bench dec <stream> <reps> [threads] | bench enc <src> <w> <h> <frames> <reps> [threads] [out]");
        std::process::exit(2)
    };
    match args.get(1).map(String::as_str) {
        Some("dec") => {
            let data = std::fs::read(args.get(2).unwrap_or_else(|| usage())).expect("read");
            let reps: usize = args.get(3).map_or(5, |s| s.parse().expect("reps"));
            let threads: usize = args.get(4).map_or(1, |s| s.parse().expect("threads"));
            let mut best = f64::INFINITY;
            let mut n = 0;
            for _ in 0..reps {
                let t = Instant::now();
                let f = decode_all(&data, threads);
                best = best.min(t.elapsed().as_secs_f64());
                n = f.len();
            }
            println!("dec {} frames threads {threads}: {:.3} ms/frame, {:.1} fps", n, best * 1e3 / n as f64, n as f64 / best);
        }
        Some("enc") => {
            let data = std::fs::read(args.get(2).unwrap_or_else(|| usage())).expect("read");
            let w: u32 = args.get(3).unwrap_or_else(|| usage()).parse().expect("w");
            let h: u32 = args.get(4).unwrap_or_else(|| usage()).parse().expect("h");
            let n: usize = args.get(5).unwrap_or_else(|| usage()).parse().expect("frames");
            let reps: usize = args.get(6).map_or(3, |s| s.parse().expect("reps"));
            let threads: usize = args.get(7).map_or(1, |s| s.parse().expect("threads"));
            let src = decode_all(&data, 0);
            let frames: Vec<Frame> = (0..n).map(|i| scale(&src[i % src.len()], w, h)).collect();
            let mut best = f64::INFINITY;
            let mut stream = Vec::new();
            for _ in 0..reps {
                let cfg = EncoderConfig { threads, frame_rate: (30000, 1001), ..EncoderConfig::new(w, h) };
                let t = Instant::now();
                let mut enc = Encoder::new(cfg).expect("config");
                let mut s = Vec::new();
                for f in &frames {
                    s.extend(enc.encode(f).expect("encode"));
                }
                s.extend(enc.finish().expect("finish"));
                best = best.min(t.elapsed().as_secs_f64());
                stream = s;
            }
            println!(
                "enc {w}x{h} {n} frames threads {threads}: {:.3} ms/frame, {:.2} fps, {} bytes",
                best * 1e3 / n as f64,
                n as f64 / best,
                stream.len()
            );
            if let Some(out) = args.get(8) {
                std::fs::write(out, &stream).expect("write");
            }
        }
        _ => usage(),
    }
}
