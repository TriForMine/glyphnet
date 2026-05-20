use std::{fmt, str::FromStr};

use serde::{Deserialize, Serialize};

use crate::{GlyphError, Result};

/// Optical transmission profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum TransmissionMode {
    /// Robust static symbols for physical print, packaging, stickers, and paper.
    Print = 0,
    /// Higher-density static symbols optimized for emissive displays.
    Screen = 1,
    /// Animated frame sequences for high-throughput optical transfer.
    Burst = 2,
}

impl TransmissionMode {
    /// Convert a compact wire identifier into a mode.
    pub fn from_wire(value: u8) -> Result<Self> {
        match value {
            0 => Ok(Self::Print),
            1 => Ok(Self::Screen),
            2 => Ok(Self::Burst),
            other => Err(GlyphError::InvalidMode(other)),
        }
    }

    /// Return the compact wire identifier.
    pub const fn wire_id(self) -> u8 {
        self as u8
    }

    /// The minimum static module dimension for this mode.
    pub const fn minimum_dimension(self) -> u16 {
        match self {
            Self::Print => 45,
            Self::Screen => 57,
            Self::Burst => 37,
        }
    }
}

impl fmt::Display for TransmissionMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Print => f.write_str("print"),
            Self::Screen => f.write_str("screen"),
            Self::Burst => f.write_str("burst"),
        }
    }
}

impl FromStr for TransmissionMode {
    type Err = GlyphError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "print" => Ok(Self::Print),
            "screen" => Ok(Self::Screen),
            "burst" => Ok(Self::Burst),
            _ => Err(GlyphError::InvalidArgument("unknown transmission mode")),
        }
    }
}

/// Error-correction profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum EccLevel {
    /// Low overhead for clean screen-to-camera transfers.
    Low = 0,
    /// Balanced correction for general use.
    Medium = 1,
    /// Strong correction for print and rough camera conditions.
    High = 2,
    /// Experimental adaptive profile reserved for LDPC/fountain hybrids.
    Adaptive = 3,
}

impl EccLevel {
    /// Convert a compact wire identifier into an ECC profile.
    pub fn from_wire(value: u8) -> Result<Self> {
        match value {
            0 => Ok(Self::Low),
            1 => Ok(Self::Medium),
            2 => Ok(Self::High),
            3 => Ok(Self::Adaptive),
            other => Err(GlyphError::InvalidEccLevel(other)),
        }
    }

    /// Return the compact wire identifier.
    pub const fn wire_id(self) -> u8 {
        self as u8
    }

    /// Parity overhead used by the reference encoder.
    pub const fn parity_ratio(self) -> (usize, usize) {
        match self {
            Self::Low => (1, 16),
            Self::Medium => (1, 8),
            Self::High => (1, 4),
            Self::Adaptive => (1, 3),
        }
    }
}

impl fmt::Display for EccLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Low => f.write_str("low"),
            Self::Medium => f.write_str("medium"),
            Self::High => f.write_str("high"),
            Self::Adaptive => f.write_str("adaptive"),
        }
    }
}

impl FromStr for EccLevel {
    type Err = GlyphError;

    fn from_str(value: &str) -> Result<Self> {
        match value {
            "low" => Ok(Self::Low),
            "medium" => Ok(Self::Medium),
            "high" => Ok(Self::High),
            "adaptive" => Ok(Self::Adaptive),
            _ => Err(GlyphError::InvalidArgument("unknown ECC level")),
        }
    }
}
