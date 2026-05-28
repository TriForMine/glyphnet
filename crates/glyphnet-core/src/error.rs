use thiserror::Error;

/// Shared result type for protocol operations.
pub type Result<T> = std::result::Result<T, GlyphError>;

/// Errors emitted by core protocol validation and binary parsing.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum GlyphError {
    /// A binary frame did not start with the GlyphNet magic bytes.
    #[error("invalid GlyphNet frame magic")]
    InvalidMagic,
    /// The frame version is not supported by this implementation.
    #[error("unsupported GlyphNet wire version {0}")]
    UnsupportedVersion(u8),
    /// The frame header was too short or the payload ended early.
    #[error("truncated frame: needed at least {needed} bytes, got {actual}")]
    Truncated { needed: usize, actual: usize },
    /// Header checksum validation failed.
    #[error("header checksum mismatch")]
    HeaderChecksumMismatch,
    /// Payload checksum validation failed.
    #[error("payload checksum mismatch")]
    PayloadChecksumMismatch,
    /// A numeric mode identifier was not defined by the protocol.
    #[error("invalid transmission mode id {0}")]
    InvalidMode(u8),
    /// A numeric ECC identifier was not defined by the protocol.
    #[error("invalid ECC level id {0}")]
    InvalidEccLevel(u8),
    /// A frame index was outside the frame-count range.
    #[error("frame index {index} is outside frame count {count}")]
    InvalidFrameIndex { index: u16, count: u16 },
    /// A matrix coordinate was outside the symbol bounds.
    #[error("matrix coordinate ({x}, {y}) is outside {width}x{height}")]
    MatrixBounds {
        x: u16,
        y: u16,
        width: u16,
        height: u16,
    },
    /// The payload is larger than the selected symbol layout can hold.
    #[error("symbol capacity exceeded: need {needed_bits} bits, have {available_bits}")]
    CapacityExceeded {
        needed_bits: usize,
        available_bits: usize,
    },
    /// A caller provided an invalid argument.
    #[error("invalid argument: {0}")]
    InvalidArgument(&'static str),
    /// Authenticated payload envelope magic/version did not match.
    #[error("invalid authenticity envelope")]
    InvalidAuthenticityEnvelope,
    /// No verification key was available for the envelope key id.
    #[error("unknown authenticity key id {0}")]
    UnknownAuthenticityKey(u32),
    /// Authenticity tag verification failed.
    #[error("authenticity tag mismatch")]
    AuthenticityMismatch,
}
