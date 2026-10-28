use std::fmt;

/// Everything that can go wrong while building or parsing protocol data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// An animation needs at least one frame.
    EmptyAnimation,
    /// More frames than the one-byte header counter allows.
    TooManyFrames(usize),
    /// A frame buffer was not exactly [`crate::FRAME_BYTES`] long.
    BadFrameSize { index: usize, got: usize },
    /// The payload is shorter than the 256-byte header.
    PayloadTooShort(usize),
    /// The payload claims more frames than it actually contains.
    Truncated { declared: usize, available: usize },
    /// Declared frame count is zero.
    ZeroFrames,
    /// Report size too small to carry the 8-byte header plus data.
    ReportTooSmall(usize),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::EmptyAnimation => f.write_str("animation needs at least one frame"),
            Error::TooManyFrames(n) => {
                write!(f, "{n} frames exceeds the 255-frame protocol limit")
            }
            Error::BadFrameSize { index, got } => write!(
                f,
                "frame {index} is {got} bytes, expected {} (128×128 RGB565)",
                crate::FRAME_BYTES
            ),
            Error::PayloadTooShort(len) => {
                write!(
                    f,
                    "payload is {len} bytes, header alone is {}",
                    crate::PAYLOAD_HEADER
                )
            }
            Error::Truncated {
                declared,
                available,
            } => write!(
                f,
                "payload declares {declared} frames but only holds {available}"
            ),
            Error::ZeroFrames => f.write_str("payload declares zero frames"),
            Error::ReportTooSmall(n) => write!(f, "output report of {n} bytes is too small"),
        }
    }
}

impl std::error::Error for Error {}
