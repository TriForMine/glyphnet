//! Test utilities and conformance fixtures for GlyphNet.

use glyphnet_core::Frame;
use glyphnet_decode::RasterDecoder;
use glyphnet_encode::{EncodeError, Encoder};
use glyphnet_render::RasterRenderer;
use image::{DynamicImage, Rgba, RgbaImage};
use rand::Rng;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors returned by reusable testkit workflows.
#[derive(Debug, Error)]
pub enum TestkitError {
    /// Encoder failed.
    #[error(transparent)]
    Encode(#[from] EncodeError),
    /// Renderer failed.
    #[error(transparent)]
    Render(#[from] glyphnet_render::RenderError),
    /// Decoder failed.
    #[error(transparent)]
    Decode(#[from] glyphnet_decode::DecodeError),
    /// Fixture corpus manifest parse failure.
    #[error(transparent)]
    Manifest(#[from] serde_json::Error),
    /// Fixture corpus manifest semantic validation error.
    #[error("invalid fixture corpus: {0}")]
    InvalidCorpus(&'static str),
}

/// Supported fixture corpus categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureCategory {
    /// Deterministic generated fixtures.
    Synthetic,
    /// Captured real-world fixtures.
    Real,
    /// Inputs that should not decode.
    HardNegative,
}

/// One fixture entry in the corpus manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixtureManifestEntry {
    /// Stable fixture identifier.
    pub id: String,
    /// Fixture category.
    pub category: FixtureCategory,
    /// Relative path under the corpus root.
    pub path: String,
    /// Human-readable short description.
    pub description: String,
}

/// Versioned fixture corpus manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FixtureCorpusManifest {
    /// Manifest schema version.
    pub schema_version: u32,
    /// Corpus semantic version string.
    pub corpus_version: String,
    /// Fixture entries.
    pub entries: Vec<FixtureManifestEntry>,
}

impl FixtureCorpusManifest {
    /// Parse and validate a fixture corpus manifest from JSON.
    pub fn from_json(json: &str) -> Result<Self, TestkitError> {
        let manifest: Self = serde_json::from_str(json)?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Validate basic manifest invariants.
    pub fn validate(&self) -> Result<(), TestkitError> {
        if self.schema_version == 0 {
            return Err(TestkitError::InvalidCorpus("schema_version must be >= 1"));
        }
        if self.corpus_version.trim().is_empty() {
            return Err(TestkitError::InvalidCorpus(
                "corpus_version must not be empty",
            ));
        }
        if self.entries.is_empty() {
            return Err(TestkitError::InvalidCorpus("entries must not be empty"));
        }

        let mut seen_ids = std::collections::BTreeSet::new();
        for entry in &self.entries {
            if entry.id.trim().is_empty() {
                return Err(TestkitError::InvalidCorpus("entry id must not be empty"));
            }
            if !seen_ids.insert(entry.id.as_str()) {
                return Err(TestkitError::InvalidCorpus("entry id must be unique"));
            }
            if entry.path.trim().is_empty() {
                return Err(TestkitError::InvalidCorpus("entry path must not be empty"));
            }
            if entry.path.contains("..") {
                return Err(TestkitError::InvalidCorpus(
                    "entry path must stay within corpus root",
                ));
            }
            if entry.description.trim().is_empty() {
                return Err(TestkitError::InvalidCorpus(
                    "entry description must not be empty",
                ));
            }
        }
        Ok(())
    }
}

/// Canonical payload fixtures for protocol tests.
pub fn fixture_payloads() -> Vec<Vec<u8>> {
    vec![
        Vec::new(),
        b"hello glyphnet".to_vec(),
        (0u8..=255).collect(),
        vec![0x55; 1024],
    ]
}

/// Encode, render, decode, and return the validated frame.
pub fn render_roundtrip(payload: &[u8]) -> Result<Frame, TestkitError> {
    let encoded = Encoder::default().encode_static(payload)?;
    let image = RasterRenderer::default().render(&encoded.matrix)?;
    let decoded = RasterDecoder::default().decode(&DynamicImage::ImageRgba8(image))?;
    Ok(decoded.frame)
}

/// Apply salt-and-pepper noise to an RGBA image.
pub fn add_salt_pepper_noise<R: Rng>(
    image: &mut RgbaImage,
    probability_per_pixel: f32,
    rng: &mut R,
) {
    for pixel in image.pixels_mut() {
        if rng.r#gen::<f32>() < probability_per_pixel {
            let value = if rng.r#gen::<bool>() { 0 } else { 255 };
            *pixel = image::Rgba([value, value, value, 255]);
        }
    }
}

/// Place an image on a solid RGBA canvas.
pub fn place_on_canvas(
    image: &RgbaImage,
    padding_x: u32,
    padding_y: u32,
    background: Rgba<u8>,
) -> RgbaImage {
    let mut canvas = RgbaImage::from_pixel(
        image.width() + padding_x * 2,
        image.height() + padding_y * 2,
        background,
    );
    image::imageops::overlay(
        &mut canvas,
        image,
        i64::from(padding_x),
        i64::from(padding_y),
    );
    canvas
}

/// Apply a uniform exposure delta to an image.
pub fn adjust_exposure(image: &mut RgbaImage, delta: i16) {
    for pixel in image.pixels_mut() {
        for channel in &mut pixel.0[..3] {
            *channel = (i16::from(*channel) + delta).clamp(0, 255) as u8;
        }
    }
}

/// Resize an image with a high-quality filter.
pub fn resize(image: &RgbaImage, width: u32, height: u32) -> RgbaImage {
    image::imageops::resize(
        image,
        width.max(1),
        height.max(1),
        image::imageops::FilterType::Lanczos3,
    )
}

/// Blur an image with a Gaussian kernel.
pub fn blur(image: &RgbaImage, sigma: f32) -> RgbaImage {
    image::imageops::blur(image, sigma)
}

/// Apply a simple horizontal perspective skew onto a larger white canvas.
pub fn skew_x_on_white(image: &RgbaImage, top_shift_px: i32, bottom_shift_px: i32) -> RgbaImage {
    let extra = top_shift_px
        .unsigned_abs()
        .max(bottom_shift_px.unsigned_abs());
    let width = image.width() + extra;
    let mut output = RgbaImage::from_pixel(width, image.height(), Rgba([255, 255, 255, 255]));
    let height_denom = image.height().saturating_sub(1).max(1) as f32;
    let base_offset = -top_shift_px.min(bottom_shift_px).min(0);

    for y in 0..image.height() {
        let t = y as f32 / height_denom;
        let shift = (top_shift_px as f32 * (1.0 - t) + bottom_shift_px as f32 * t).round() as i32;
        let dest_x = base_offset + shift;
        for x in 0..image.width() {
            let target_x = dest_x + x as i32;
            if target_x >= 0 {
                let target_x = target_x as u32;
                if target_x < output.width() {
                    output.put_pixel(target_x, y, *image.get_pixel(x, y));
                }
            }
        }
    }

    output
}

#[cfg(test)]
mod tests {
    use rand::{RngCore, SeedableRng, rngs::StdRng};

    use super::*;

    #[test]
    fn fixtures_roundtrip() {
        for payload in fixture_payloads() {
            let frame = render_roundtrip(&payload).unwrap();
            assert_eq!(frame.payload, payload);
        }
    }

    #[test]
    fn short_payloads_roundtrip() {
        let mut rng = StdRng::seed_from_u64(0x6759_7068_6e65_7473);
        let lengths = [0usize, 1, 2, 3, 7, 15, 31, 63, 95, 127, 191, 255];
        for len in lengths {
            let mut payload = vec![0u8; len];
            rng.fill_bytes(&mut payload);
            let frame = render_roundtrip(&payload).unwrap();
            assert_eq!(frame.payload, payload);
        }
    }

    #[test]
    fn fixture_manifest_parses_and_validates() {
        let json = r#"{
            "schema_version": 1,
            "corpus_version": "0.1.0",
            "entries": [
                {
                    "id": "synthetic-ribbon-clean-001",
                    "category": "synthetic",
                    "path": "synthetic/ribbon/clean-001.png",
                    "description": "Generated clean ribbon fixture."
                },
                {
                    "id": "real-debugger-screenshot-001",
                    "category": "real",
                    "path": "real/debugger/screenshot-001.png",
                    "description": "Captured debugger screenshot fixture."
                },
                {
                    "id": "negative-ui-clutter-001",
                    "category": "hard_negative",
                    "path": "hard_negative/ui/clutter-001.png",
                    "description": "No symbol present; used for false-positive checks."
                }
            ]
        }"#;
        let manifest = FixtureCorpusManifest::from_json(json).unwrap();
        assert_eq!(manifest.schema_version, 1);
        assert_eq!(manifest.entries.len(), 3);
    }

    #[test]
    fn fixture_manifest_rejects_duplicate_ids() {
        let json = r#"{
            "schema_version": 1,
            "corpus_version": "0.1.0",
            "entries": [
                {
                    "id": "dup-id",
                    "category": "synthetic",
                    "path": "synthetic/a.png",
                    "description": "A"
                },
                {
                    "id": "dup-id",
                    "category": "real",
                    "path": "real/b.png",
                    "description": "B"
                }
            ]
        }"#;
        let error = FixtureCorpusManifest::from_json(json).unwrap_err();
        assert!(matches!(
            error,
            TestkitError::InvalidCorpus("entry id must be unique")
        ));
    }
}
