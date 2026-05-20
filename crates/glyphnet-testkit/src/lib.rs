//! Test utilities and conformance fixtures for GlyphNet.

use glyphnet_core::Frame;
use glyphnet_decode::RasterDecoder;
use glyphnet_encode::{EncodeError, Encoder};
use glyphnet_render::RasterRenderer;
use image::{DynamicImage, RgbaImage};
use rand::Rng;
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

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;

    #[test]
    fn fixtures_roundtrip() {
        for payload in fixture_payloads() {
            let frame = render_roundtrip(&payload).unwrap();
            assert_eq!(frame.payload, payload);
        }
    }

    proptest! {
        #[test]
        fn short_payloads_roundtrip(payload in proptest::collection::vec(any::<u8>(), 0..256)) {
            let frame = render_roundtrip(&payload).unwrap();
            prop_assert_eq!(frame.payload, payload);
        }
    }
}
