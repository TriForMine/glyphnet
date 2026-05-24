//! GlyphNet matrix and raster decoders.

use glyphnet_core::{
    Cell, Frame, GlyphError, HEADER_LEN, LayoutFamily, Result as CoreResult, SymbolMatrix,
    bitstream, layout,
};
use glyphnet_ecc::{BlockCode, ParityCode};
use image::{DynamicImage, GrayImage};
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
        let luma = image.to_luma8();
        let width = luma.width();
        let height = luma.height();
        if width == 0 || height == 0 {
            return Err(DecodeError::InvalidImageDimensions);
        }
        let gcd = gcd_u32(width, height);
        let mut candidates = divisors_desc(gcd);
        if candidates.is_empty() {
            return Err(DecodeError::AutoDetectFailed);
        }
        let thresholds = threshold_candidates(self.options.threshold, &luma);
        let layouts = layout_candidates(self.options.layout);
        for module_px in candidates.drain(..) {
            let width_modules = width / module_px;
            let height_modules = height / module_px;
            for quiet_zone in 0..=AUTO_QUIET_ZONE_MAX {
                if width_modules <= quiet_zone * 2 || height_modules <= quiet_zone * 2 {
                    continue;
                }
                for layout in &layouts {
                    for threshold in &thresholds {
                        let options = DecodeOptions {
                            module_px,
                            quiet_zone_modules: quiet_zone,
                            threshold: *threshold,
                            layout: *layout,
                        };
                        let matrix = match Self::sample_matrix_with_luma(&luma, &options) {
                            Ok(matrix) => matrix,
                            Err(_) => continue,
                        };
                        if let Ok(decoded) = decode_matrix(&matrix) {
                            return Ok(decoded);
                        }
                    }
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
        let luma = image.to_luma8();
        Self::sample_matrix_with_luma(&luma, options)
    }

    fn sample_matrix_with_luma(luma: &GrayImage, options: &DecodeOptions) -> Result<SymbolMatrix> {
        if options.module_px == 0 {
            return Err(DecodeError::InvalidImageDimensions);
        }

        let width_modules = luma
            .width()
            .checked_div(options.module_px)
            .ok_or(DecodeError::InvalidImageDimensions)?;
        let height_modules = luma
            .height()
            .checked_div(options.module_px)
            .ok_or(DecodeError::InvalidImageDimensions)?;
        if luma.width() % options.module_px != 0
            || luma.height() % options.module_px != 0
            || width_modules <= options.quiet_zone_modules * 2
            || height_modules <= options.quiet_zone_modules * 2
        {
            return Err(DecodeError::InvalidImageDimensions);
        }

        let symbol_width = (width_modules - options.quiet_zone_modules * 2) as u16;
        let symbol_height = (height_modules - options.quiet_zone_modules * 2) as u16;
        let mut matrix = SymbolMatrix::with_layout(symbol_width, symbol_height, options.layout);

        for y in 0..symbol_height {
            for x in 0..symbol_width {
                if let Some(cell) =
                    layout::function_cell_for(options.layout, symbol_width, symbol_height, x, y)
                {
                    matrix.set(x, y, cell)?;
                } else {
                    let avg = average_module_luma(
                        luma,
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

fn layout_candidates(primary: LayoutFamily) -> Vec<LayoutFamily> {
    let all = [
        LayoutFamily::RibbonWeave,
        LayoutFamily::SpectralMesh,
        LayoutFamily::PulseStream,
        LayoutFamily::Constellation,
        LayoutFamily::FrameGrid,
        LayoutFamily::Matrix,
        LayoutFamily::Hexagonal,
        LayoutFamily::Radial,
    ];
    let mut layouts = Vec::with_capacity(all.len());
    layouts.push(primary);
    for layout in all {
        if layout != primary {
            layouts.push(layout);
        }
    }
    layouts
}

fn threshold_candidates(configured: u8, luma: &GrayImage) -> Vec<u8> {
    let estimated = estimate_threshold_otsu(luma);
    let mut thresholds = vec![estimated];
    if estimated <= 1 || estimated >= 254 {
        thresholds.push(128);
    }
    if configured != estimated && !thresholds.contains(&configured) {
        thresholds.push(configured);
    }
    thresholds
}

fn estimate_threshold_otsu(luma: &GrayImage) -> u8 {
    let mut histogram = [0u32; 256];
    for pixel in luma.pixels() {
        histogram[pixel[0] as usize] += 1;
    }
    let total = u64::from(luma.width()).saturating_mul(u64::from(luma.height()));
    if total == 0 {
        return 0;
    }
    let mut sum_total = 0u64;
    for (i, count) in histogram.iter().enumerate() {
        sum_total += (i as u64) * u64::from(*count);
    }
    let mut sum_background = 0u64;
    let mut weight_background = 0u64;
    let mut max_variance = -1.0f64;
    let mut threshold = 0u8;
    for (i, count) in histogram.iter().enumerate() {
        weight_background += u64::from(*count);
        if weight_background == 0 {
            continue;
        }
        let weight_foreground = total - weight_background;
        if weight_foreground == 0 {
            break;
        }
        sum_background += (i as u64) * u64::from(*count);
        let mean_background = sum_background as f64 / weight_background as f64;
        let mean_foreground = (sum_total - sum_background) as f64 / weight_foreground as f64;
        let diff = mean_background - mean_foreground;
        let variance = (weight_background as f64) * (weight_foreground as f64) * diff * diff;
        if variance > max_variance {
            max_variance = variance;
            threshold = i as u8;
        }
    }
    threshold
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
    fn matrix_layout_roundtrips_with_auto_layout() {
        let encoder = Encoder::new(EncoderConfig::for_profile(ProfileId::MatrixCompat));
        let encoded = encoder.encode_static(b"matrix").unwrap();
        let image = RasterRenderer::default().render(&encoded.matrix).unwrap();
        let decoded = RasterDecoder::default()
            .decode_auto(&DynamicImage::ImageRgba8(image))
            .unwrap();
        assert_eq!(decoded.frame.payload, b"matrix");
    }

    #[test]
    fn auto_threshold_overrides_bad_config() {
        let encoded = Encoder::default().encode_static(b"threshold").unwrap();
        let image = RasterRenderer::default().render(&encoded.matrix).unwrap();
        let decoder = RasterDecoder::new(DecodeOptions {
            threshold: 0,
            ..DecodeOptions::default()
        });
        let decoded = decoder
            .decode_auto(&DynamicImage::ImageRgba8(image))
            .unwrap();
        assert_eq!(decoded.frame.payload, b"threshold");
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
