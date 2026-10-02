//! An MPEG-2 Video (ITU-T H.262 | ISO/IEC 13818-2) decoder and encoder.
//!
//! Rust, no C, no system libraries, no build script. Written from the
//! Recommendation (the 02/2000 edition); ISO/IEC 11172-2 (MPEG-1) video
//! decodes too, by the differences H.262 Annex D.9 lists.
//!
//! - [`Decoder`] takes elementary-stream bytes in any chunking and returns
//!   8-bit planar [`Frame`]s in display order, with the timing flags of each
//!   picture (`top_field_first`, `repeat_first_field`, `progressive_frame`).
//!   Main Profile and the 4:2:2 profile: I, P and B pictures, frame and
//!   field pictures, frame / field / dual-prime / 16×8 motion compensation,
//!   both DCT coefficient tables, the alternate scan, both quantiser scales,
//!   downloaded matrices, 4:2:0 and 4:2:2.
//! - [`Encoder`] takes 4:2:0 frames and writes a Main Profile elementary
//!   stream: progressive frame pictures, I, P and B, half-sample motion
//!   search, a constant quantiser or a simple rate control.
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
//! # Ok::<(), mpeg2::Error>(())
//! ```
//!
//! # Provenance
//!
//! Written from ITU-T Rec. H.262 (02/2000) alone; no other implementation's
//! source was read. The tables are transcribed from the Recommendation.

#![warn(missing_docs)]

pub(crate) mod bits;
mod decoder;
mod encoder;
mod error;
mod frame;
pub(crate) mod headers;
pub(crate) mod idct;
pub(crate) mod tables;
pub(crate) mod vlc;

pub use decoder::{Decoder, SequenceInfo};
pub use encoder::{Encoder, EncoderConfig, RateControl};
pub use error::{Error, Result};
pub use frame::{ChromaFormat, Frame, PictureType, Plane};
