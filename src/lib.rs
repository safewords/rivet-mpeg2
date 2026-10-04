//! An MPEG-2 Video (ITU-T H.262 | ISO/IEC 13818-2) decoder and encoder.
//!
//! Rust, no C, no system libraries, no build script. Written from the
//! Recommendation (the 02/2000 edition); ISO/IEC 11172-2 (MPEG-1) video
//! decodes too, by the differences H.262 Annex D.9 lists.
//!
//! - [`Decoder`] takes elementary-stream bytes in any chunking and returns
//!   8-bit planar [`Frame`]s in display order, with each picture's timing
//!   flags (`top_field_first`, `repeat_first_field`, `progressive_frame`).
//!   Main Profile and the 4:2:2 profile: I, P and B pictures, frame and
//!   field pictures, frame / field / dual-prime / 16×8 motion compensation,
//!   both DCT coefficient tables, the alternate scan, both quantiser scales,
//!   downloaded matrices, intra DC precision 8–11 bits, 4:2:0 and 4:2:2;
//!   MPEG-1 streams including D-pictures. The scalable extensions and 4:4:4
//!   are refused with [`Error::Unsupported`].
//! - [`Encoder`] takes 4:2:0 frames and writes a Main Profile elementary
//!   stream: progressive frame pictures, I, P and B, half-sample motion
//!   search, a constant quantiser or a picture-level rate control.
//!
//! ```
//! use mpeg2::{ChromaFormat, Decoder, Encoder, EncoderConfig, Frame};
//!
//! let mut enc = Encoder::new(EncoderConfig::new(64, 48))?;
//! let mut stream = Vec::new();
//! for i in 0..5u8 {
//!     let mut f = Frame::new(64, 48, ChromaFormat::Yuv420);
//!     f.plane_mut(0).fill(40 + i * 10);
//!     stream.extend(enc.encode(&f)?);
//! }
//! stream.extend(enc.finish()?);
//!
//! let mut dec = Decoder::new();
//! let mut frames = dec.decode(&stream)?;
//! frames.extend(dec.flush()?);
//! assert_eq!(frames.len(), 5);
//! assert_eq!(frames[4].plane(0)[0], 80);
//! # Ok::<(), mpeg2::Error>(())
//! ```
//!
//! # Speed
//!
//! The sample processing — the 8×8 transforms, motion compensation, residual
//! addition, the encoder's SAD — runs on SIMD kernels picked at run time
//! (SSE2, AVX2, AVX-512 on x86-64; NEON on AArch64; [`simd_rung`] names
//! the one in use, `MPEG2_FORCE_SCALAR=1` forces the scalar code). Each
//! returns exactly what the scalar code returns, the double-precision IDCT
//! included, so output never depends on the processor. The decoder decodes a
//! picture's slices on several threads ([`Decoder::set_threads`]) and the
//! encoder codes a picture's rows on several ([`EncoderConfig::threads`]);
//! neither changes a single sample or bit.
//!
//! # Provenance and verification
//!
//! Written from ITU-T Rec. H.262 (02/2000); no other implementation's
//! source was read. The tables are transcribed from the Recommendation and
//! checked as complete prefix codes. The IDCT passes the IEEE 1180 test
//! Annex A requires; the decoder decodes all 57 main- and 4:2:2-profile
//! conformance bitstreams of ISO/IEC 13818-4 and matches, sample for sample,
//! the reconstructions their traces carry. See the README for the figures.

#![warn(missing_docs)]

pub(crate) mod bits;
mod dsp;
mod decoder;
mod encoder;
mod error;
mod frame;
pub(crate) mod headers;
pub(crate) mod idct;
mod pool;
pub(crate) mod tables;
pub(crate) mod vlc;

pub use decoder::{Decoder, SequenceInfo};
pub use encoder::{Encoder, EncoderConfig, RateControl};
pub use error::{Error, Result};
pub use frame::{ChromaFormat, Frame, PictureType, Plane};

/// The SIMD rung the sample-processing kernels run on: `"avx512"`,
/// `"avx2"` or `"sse2"` on x86-64, `"neon"` on AArch64, else `"scalar"`
/// (also under `MPEG2_FORCE_SCALAR=1`). Output does not depend on it: every
/// rung computes exactly what the scalar code computes.
pub fn simd_rung() -> &'static str {
    dsp::active_rung()
}

/// Kernel entry points for `examples/kernels.rs` (the per-kernel
/// benchmark). Not part of the API.
#[doc(hidden)]
pub mod __bench {
    use crate::dsp::{self, Dsp, McSrc};

    fn rung(name: &str) -> &'static Dsp {
        dsp::rungs().into_iter().find(|d| d.name == name).expect("no such rung on this processor")
    }

    /// The rungs this processor runs, scalar first.
    pub fn rungs() -> Vec<&'static str> {
        dsp::rungs().iter().map(|d| d.name).collect()
    }

    /// The IDCT of each block in place (with the DC-only shortcut, as the
    /// decoder does).
    pub fn idct(name: &str, blocks: &mut [[i32; 64]]) {
        let d = rung(name);
        for b in blocks {
            if b[1..].iter().all(|&c| c == 0) {
                let v = dsp::round_away(f64::from(b[0]) / 8.0).clamp(-256, 255);
                b.fill(v);
            } else {
                let input = *b;
                (d.transform)(&input, &crate::idct::BASIS, b, -256, 255);
            }
        }
    }

    /// The forward DCT of each block; returns a checksum.
    pub fn fdct(name: &str, blocks: &[[i32; 64]]) -> i64 {
        let d = rung(name);
        let bt = crate::idct::basis_t();
        let mut sum = 0i64;
        let mut out = [0; 64];
        for b in blocks {
            (d.transform)(b, &bt, &mut out, i32::MIN, i32::MAX);
            sum += i64::from(out[0]) + i64::from(out[63]);
        }
        sum
    }

    /// One `w`-wide, `h`-high prediction per source offset, into `dst`
    /// (stride `w`).
    #[allow(clippy::too_many_arguments)]
    pub fn mc(name: &str, src: &[u8], stride: usize, offs: &[usize], w: usize, h: usize, hx: bool, hy: bool, avg: bool, dst: &mut [u8]) {
        let d = rung(name);
        for &off in offs {
            (d.mc)(&McSrc { src, off, stride, w, h, hx, hy }, dst, 0, w, avg);
        }
    }

    /// Adds (or, `intra`, puts) each residual block into `dst` (stride 16).
    pub fn add_block(name: &str, res: &[[i32; 64]], dst: &mut [u8], intra: bool) {
        let d = rung(name);
        let k = if intra { d.put_block } else { d.add_block };
        for (i, r) in res.iter().enumerate() {
            k(dst, (i & 1) * 8, 16, r);
        }
    }

    /// The 16x16 SAD of `a` at offset 0 against `b` at each offset (both
    /// strides `stride`); returns their sum.
    pub fn sad16(name: &str, a: &[u8], b: &[u8], stride: usize, offs: &[usize]) -> u64 {
        let d = rung(name);
        let a = dsp::Blk { buf: a, off: 0, stride };
        offs.iter().map(|&o| u64::from((d.sad16)(a, dsp::Blk { buf: b, off: o, stride }, u32::MAX))).sum()
    }
}
