//! An MPEG-2 Video (ITU-T H.262 | ISO/IEC 13818-2) decoder and encoder.
#![allow(dead_code)]

pub(crate) mod bits;
mod decoder;
mod error;
mod frame;
pub(crate) mod headers;
pub(crate) mod idct;
pub(crate) mod tables;
pub(crate) mod vlc;

pub use decoder::{Decoder, SequenceInfo};
pub use error::{Error, Result};
pub use frame::{ChromaFormat, Frame, PictureType, Plane};
