//! Error types for media processing operations

use std::fmt;
use std::io;

/// Result type for media operations
pub type Result<T> = std::result::Result<T, Error>;

/// Media processing errors
#[derive(Debug)]
pub enum Error {
    /// I/O error
    Io(io::Error),

    /// Invalid data format
    InvalidData(String),

    /// Unsupported format or codec
    Unsupported(String),

    /// End of stream reached
    EndOfStream,

    /// Not enough data available
    NotEnoughData,

    /// Decoder needs more data to produce a frame
    NeedMoreData,

    /// Configuration error
    Config(String),

    /// Resource allocation failed
    AllocationFailed(String),

    /// Invalid state for the operation
    InvalidState(String),

    /// Feature not implemented
    NotImplemented(String),

    /// Generic error with message
    Other(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Io(e) => write!(f, "I/O error: {}", e),
            Error::InvalidData(msg) => write!(f, "Invalid data: {}", msg),
            Error::Unsupported(msg) => write!(f, "Unsupported: {}", msg),
            Error::EndOfStream => write!(f, "End of stream"),
            Error::NotEnoughData => write!(f, "Not enough data"),
            Error::NeedMoreData => write!(f, "Need more data"),
            Error::Config(msg) => write!(f, "Configuration error: {}", msg),
            Error::AllocationFailed(msg) => write!(f, "Allocation failed: {}", msg),
            Error::InvalidState(msg) => write!(f, "Invalid state: {}", msg),
            Error::NotImplemented(msg) => write!(f, "Not implemented: {}", msg),
            Error::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Error::Io(error)
    }
}
