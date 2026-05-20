//! Real-time scanner orchestration for GlyphNet.

use std::collections::{BTreeMap, HashMap};

use glyphnet_core::{Frame, TransmissionMode};
use glyphnet_cv::VisionProfile;
use glyphnet_decode::{DecodeOptions, RasterDecoder};
use image::DynamicImage;
use thiserror::Error;

/// Result type for scanner operations.
pub type Result<T> = std::result::Result<T, ScannerError>;

/// Scanner errors.
#[derive(Debug, Error)]
pub enum ScannerError {
    /// Wrapped decode error.
    #[error(transparent)]
    Decode(#[from] glyphnet_decode::DecodeError),
    /// Conflicting burst metadata was observed.
    #[error("inconsistent burst metadata for stream {0}")]
    InconsistentBurst(u64),
}

/// Camera frame passed into the scanner.
#[derive(Debug, Clone)]
pub struct CameraFrame {
    /// Image payload.
    pub image: DynamicImage,
    /// Monotonic timestamp in microseconds.
    pub timestamp_micros: u64,
}

/// Pull-based camera source abstraction.
pub trait FrameSource {
    /// Return the next frame, or `None` when the source is exhausted.
    fn next_frame(&mut self) -> Option<CameraFrame>;
}

/// Scanner configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannerConfig {
    /// Expected mode used for CV tuning.
    pub mode: TransmissionMode,
    /// Raster decode options.
    pub decode: DecodeOptions,
    /// Maximum frames to consume in a single scan loop.
    pub max_frames: usize,
}

impl Default for ScannerConfig {
    fn default() -> Self {
        Self {
            mode: TransmissionMode::Print,
            decode: DecodeOptions::default(),
            max_frames: 120,
        }
    }
}

/// Result of scanning one frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanEvent {
    /// Decoded protocol frame.
    pub frame: Frame,
    /// Complete payload when a static symbol or full burst stream is available.
    pub complete_payload: Option<Vec<u8>>,
    /// Capture timestamp for diagnostics.
    pub timestamp_micros: u64,
}

/// Stateful real-time scanner.
#[derive(Debug)]
pub struct Scanner {
    config: ScannerConfig,
    decoder: RasterDecoder,
    bursts: HashMap<u64, BurstAssembler>,
}

impl Scanner {
    /// Create a scanner from configuration.
    pub fn new(config: ScannerConfig) -> Self {
        Self {
            decoder: RasterDecoder::new(config.decode.clone()),
            config,
            bursts: HashMap::new(),
        }
    }

    /// Borrow the active vision profile.
    pub fn vision_profile(&self) -> VisionProfile {
        VisionProfile::for_mode(self.config.mode)
    }

    /// Scan a single camera frame.
    pub fn scan_frame(&mut self, frame: CameraFrame) -> Result<ScanEvent> {
        let decoded = self.decoder.decode(&frame.image)?;
        let protocol_frame = decoded.frame;
        let complete_payload = if protocol_frame.header.frame_count == 1 {
            Some(protocol_frame.payload.clone())
        } else {
            let assembler = self
                .bursts
                .entry(protocol_frame.header.stream_id)
                .or_insert_with(|| BurstAssembler::new(protocol_frame.header.frame_count));
            assembler.push(&protocol_frame)?
        };

        Ok(ScanEvent {
            frame: protocol_frame,
            complete_payload,
            timestamp_micros: frame.timestamp_micros,
        })
    }

    /// Consume frames until a payload is complete or the configured frame limit is reached.
    pub fn scan_source<S: FrameSource>(&mut self, source: &mut S) -> Result<Option<Vec<u8>>> {
        for _ in 0..self.config.max_frames {
            let Some(frame) = source.next_frame() else {
                return Ok(None);
            };
            let event = self.scan_frame(frame)?;
            if let Some(payload) = event.complete_payload {
                return Ok(Some(payload));
            }
        }
        Ok(None)
    }
}

impl Default for Scanner {
    fn default() -> Self {
        Self::new(ScannerConfig::default())
    }
}

/// Burst frame assembly state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BurstAssembler {
    frame_count: u16,
    frames: BTreeMap<u16, Vec<u8>>,
}

impl BurstAssembler {
    /// Create burst assembly state.
    pub const fn new(frame_count: u16) -> Self {
        Self {
            frame_count,
            frames: BTreeMap::new(),
        }
    }

    /// Push a frame and return the full payload once all frames are present.
    pub fn push(&mut self, frame: &Frame) -> Result<Option<Vec<u8>>> {
        if frame.header.frame_count != self.frame_count {
            return Err(ScannerError::InconsistentBurst(frame.header.stream_id));
        }
        self.frames
            .entry(frame.header.frame_index)
            .or_insert_with(|| frame.payload.clone());

        if self.frames.len() == usize::from(self.frame_count) {
            let mut payload = Vec::new();
            for index in 0..self.frame_count {
                if let Some(chunk) = self.frames.get(&index) {
                    payload.extend_from_slice(chunk);
                }
            }
            return Ok(Some(payload));
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use glyphnet_core::{EccLevel, Frame};

    use super::*;

    #[test]
    fn burst_assembler_returns_payload_when_complete() {
        let mut assembler = BurstAssembler::new(2);
        let first = Frame::new(
            TransmissionMode::Burst,
            EccLevel::Low,
            0,
            2,
            7,
            b"ab".to_vec(),
        )
        .unwrap();
        let second = Frame::new(
            TransmissionMode::Burst,
            EccLevel::Low,
            1,
            2,
            7,
            b"cd".to_vec(),
        )
        .unwrap();
        assert!(assembler.push(&second).unwrap().is_none());
        assert_eq!(assembler.push(&first).unwrap(), Some(b"abcd".to_vec()));
    }
}
