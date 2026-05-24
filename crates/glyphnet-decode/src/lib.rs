//! GlyphNet matrix and raster decoders.

use glyphnet_core::{
    Cell, Frame, GlyphError, HEADER_LEN, LayoutFamily, Result as CoreResult, SymbolMatrix,
    bitstream, layout,
};
use glyphnet_ecc::{BlockCode, ParityCode};
use image::DynamicImage;
use thiserror::Error;

/// Result type for decode operations.
pub type Result<T> = std::result::Result<T, DecodeError>;

/// Decoder errors.
#[derive(Debug, Error)]
pub enum DecodeError {
    /// Wrapped core error.
    #[error(transparent)]
    Core(#[from] GlyphError),
    /// Image dimensions did not match the configured sampling grid.
    #[error("invalid image dimensions for configured module size and quiet zone")]
    InvalidImageDimensions,
    /// ECC/parity validation failed.
    #[error("error-correction parity validation failed")]
    EccMismatch,
    /// Failed to infer module size and quiet zone automatically.
    #[error("failed to infer module size and quiet zone")]
    AutoDetectFailed,
}

/// Raster sampling options.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodeOptions {
    /// Pixels per module.
    pub module_px: u32,
    /// Quiet-zone width in modules.
    pub quiet_zone_modules: u32,
    /// Luma threshold below which a sampled module is considered dark.
    pub threshold: u8,
    /// Expected layout family.
    pub layout: LayoutFamily,
}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self {
            module_px: 8,
            quiet_zone_modules: 4,
            threshold: 192,
            layout: LayoutFamily::RibbonWeave,
        }
    }
}

/// Decoded symbol output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedSymbol {
    /// Sampled matrix.
    pub matrix: SymbolMatrix,
    /// Validated binary frame.
    pub frame: Frame,
    /// Complete sampled bytes, including parity and padding.
    pub sampled_bytes: Vec<u8>,
}

/// Decode a matrix into a binary frame.
pub fn decode_matrix(matrix: &SymbolMatrix) -> Result<DecodedSymbol> {
    let bits = matrix.read_data_bits();
    let sampled_bytes = bitstream::bits_to_bytes(&bits);
    let frame = Frame::decode(&sampled_bytes)?;
    let data_len = HEADER_LEN + frame.header.payload_len as usize;
    let parity = ParityCode::from_level(frame.header.ecc_level, data_len);
    if !parity.verify(&sampled_bytes, data_len) {
        return Err(DecodeError::EccMismatch);
    }
    Ok(DecodedSymbol {
        matrix: matrix.clone(),
        frame,
        sampled_bytes,
    })
}

/// Raster image decoder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RasterDecoder {
    options: DecodeOptions,
}

impl RasterDecoder {
    /// Create a decoder from options.
    pub const fn new(options: DecodeOptions) -> Self {
        Self { options }
    }

    /// Decode a rendered GlyphNet image.
    pub fn decode(&self, image: &DynamicImage) -> Result<DecodedSymbol> {
        let matrix = self.sample_matrix(image)?;
        decode_matrix(&matrix)
    }

    /// Decode a rendered GlyphNet image by inferring module size and quiet zone.
    pub fn decode_auto(&self, image: &DynamicImage) -> Result<DecodedSymbol> {
        let width = image.width();
        let height = image.height();
        if width == 0 || height == 0 {
            return Err(DecodeError::InvalidImageDimensions);
        }
        let gcd = gcd_u32(width, height);
        let mut candidates = divisors_desc(gcd);
        if candidates.is_empty() {
            return Err(DecodeError::AutoDetectFailed);
        }
        for module_px in candidates.drain(..) {
            let width_modules = width / module_px;
            let height_modules = height / module_px;
            for quiet_zone in 0..=AUTO_QUIET_ZONE_MAX {
                if width_modules <= quiet_zone * 2 || height_modules <= quiet_zone * 2 {
                    continue;
                }
                let options = DecodeOptions {
                    module_px,
                    quiet_zone_modules: quiet_zone,
                    threshold: self.options.threshold,
                    layout: self.options.layout,
                };
                let matrix = match Self::sample_matrix_with_options(image, &options) {
                    Ok(matrix) => matrix,
                    Err(_) => continue,
                };
                if let Ok(decoded) = decode_matrix(&matrix) {
                    return Ok(decoded);
                }
            }
        }
        Err(DecodeError::AutoDetectFailed)
    }

    /// Sample a rendered image into a symbol matrix.
    pub fn sample_matrix(&self, image: &DynamicImage) -> Result<SymbolMatrix> {
        Self::sample_matrix_with_options(image, &self.options)
    }

    /// Sample a rendered image by inferring module size and quiet zone.
    pub fn sample_matrix_auto(&self, image: &DynamicImage) -> Result<SymbolMatrix> {
        Ok(self.decode_auto(image)?.matrix)
    }

    fn sample_matrix_with_options(
        image: &DynamicImage,
        options: &DecodeOptions,
    ) -> Result<SymbolMatrix> {
        if options.module_px == 0 {
            return Err(DecodeError::InvalidImageDimensions);
        }

        let width_modules = image
            .width()
            .checked_div(options.module_px)
            .ok_or(DecodeError::InvalidImageDimensions)?;
        let height_modules = image
            .height()
            .checked_div(options.module_px)
            .ok_or(DecodeError::InvalidImageDimensions)?;
        if image.width() % options.module_px != 0
            || image.height() % options.module_px != 0
            || width_modules <= options.quiet_zone_modules * 2
            || height_modules <= options.quiet_zone_modules * 2
        {
            return Err(DecodeError::InvalidImageDimensions);
        }

        let symbol_width = (width_modules - options.quiet_zone_modules * 2) as u16;
        let symbol_height = (height_modules - options.quiet_zone_modules * 2) as u16;
        let luma = image.to_luma8();
        let mut matrix = SymbolMatrix::with_layout(symbol_width, symbol_height, options.layout);

        for y in 0..symbol_height {
            for x in 0..symbol_width {
                if let Some(cell) =
                    layout::function_cell_for(options.layout, symbol_width, symbol_height, x, y)
                {
                    matrix.set(x, y, cell)?;
                } else {
                    let avg = average_module_luma(
                        &luma,
                        u32::from(x),
                        u32::from(y),
                        options.module_px,
                        options.quiet_zone_modules,
                    );
                    matrix.set(x, y, Cell::Data(avg < options.threshold))?;
                }
            }
        }

        Ok(matrix)
    }
}

impl Default for RasterDecoder {
    fn default() -> Self {
        Self::new(DecodeOptions::default())
    }
}

const AUTO_QUIET_ZONE_MAX: u32 = 12;

fn gcd_u32(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let tmp = a % b;
        a = b;
        b = tmp;
    }
    a
}

fn divisors_desc(value: u32) -> Vec<u32> {
    if value == 0 {
        return Vec::new();
    }
    let mut small = Vec::new();
    let mut large = Vec::new();
    let mut i = 1u32;
    while i * i <= value {
        if value % i == 0 {
            small.push(i);
            let other = value / i;
            if other != i {
                large.push(other);
            }
        }
        i += 1;
    }
    large.extend(small.into_iter().rev());
    large
}

fn average_module_luma(
    image: &image::GrayImage,
    module_x: u32,
    module_y: u32,
    module_px: u32,
    quiet_zone_modules: u32,
) -> u8 {
    let start_x = (module_x + quiet_zone_modules) * module_px;
    let start_y = (module_y + quiet_zone_modules) * module_px;
    let mut sum = 0u32;
    for y in start_y..start_y + module_px {
        for x in start_x..start_x + module_px {
            sum += u32::from(image.get_pixel(x, y).0[0]);
        }
    }
    (sum / (module_px * module_px)) as u8
}

/// Validate a sampled byte stream without constructing a matrix.
pub fn decode_wire_prefix(bytes: &[u8]) -> CoreResult<Frame> {
    Frame::decode(bytes)
}

#[cfg(test)]
mod tests {
    use glyphnet_core::ProfileId;
    use glyphnet_encode::{Encoder, EncoderConfig};
    use glyphnet_render::{RasterRenderer, RenderOptions};

    use super::*;

    #[test]
    fn rendered_symbol_roundtrips() {
        let encoded = Encoder::default().encode_static(b"roundtrip").unwrap();
        let image = RasterRenderer::default().render(&encoded.matrix).unwrap();
        let decoded = RasterDecoder::default()
            .decode(&DynamicImage::ImageRgba8(image))
            .unwrap();
        assert_eq!(decoded.frame.payload, b"roundtrip");
    }

    #[test]
    fn rendered_symbol_roundtrips_with_auto_sampling() {
        let encoded = Encoder::default().encode_static(b"roundtrip").unwrap();
        let image = RasterRenderer::default().render(&encoded.matrix).unwrap();
        let decoded = RasterDecoder::default()
            .decode_auto(&DynamicImage::ImageRgba8(image))
            .unwrap();
        assert_eq!(decoded.frame.payload, b"roundtrip");
    }

    #[test]
    fn spectral_screen_rendering_roundtrips_through_luma_sampler() {
        let encoder = Encoder::new(EncoderConfig::for_profile(ProfileId::SpectralScreen));
        let encoded = encoder.encode_static(b"spectral").unwrap();
        let image = RasterRenderer::new(RenderOptions::for_descriptor(&encoded.descriptor))
            .render(&encoded.matrix)
            .unwrap();
        let decoded = RasterDecoder::default()
            .decode(&DynamicImage::ImageRgba8(image))
            .unwrap();
        assert_eq!(decoded.frame.payload, b"spectral");
    }
}
