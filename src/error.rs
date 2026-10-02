//! The crate's error type.

/// Errors the decoder and the encoder report.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The bitstream breaks the syntax or the semantics of H.262: a field out
    /// of range, a codeword that matches no table entry, a macroblock address
    /// past the end of the picture, a slice before any sequence header.
    #[error("invalid MPEG-2 video data: {0}")]
    Invalid(String),
    /// Valid MPEG-2 (or MPEG-1) video this crate does not implement, named:
    /// the scalable extensions, 4:4:4 chroma, MPEG-1 D-pictures.
    #[error("unsupported MPEG-2 video feature: {0}")]
    Unsupported(String),
    /// An encoder configuration, or an input frame, that cannot be coded: a
    /// size of zero or beyond the syntax, a frame whose size or chroma format
    /// differs from the configuration.
    #[error("invalid MPEG-2 encoder configuration: {0}")]
    Config(String),
}

#[cold]
#[inline(never)]
pub(crate) fn invalid(msg: impl Into<String>) -> Error {
    Error::Invalid(msg.into())
}

#[cold]
#[inline(never)]
pub(crate) fn unsupported(msg: impl Into<String>) -> Error {
    Error::Unsupported(msg.into())
}

#[cold]
#[inline(never)]
pub(crate) fn config(msg: impl Into<String>) -> Error {
    Error::Config(msg.into())
}

/// `Result` with this crate's [`Error`].
pub type Result<T> = std::result::Result<T, Error>;
