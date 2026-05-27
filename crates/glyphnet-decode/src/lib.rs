//! GlyphNet matrix and raster decoders.

use glyphnet_core::{
    Cell, Frame, FrameHeader, GlyphError, HEADER_LEN, LayoutFamily, Result as CoreResult,
    SymbolMatrix, bitstream, layout,
};
use glyphnet_ecc::{try_recover_for_mode, try_recover_for_mode_with_suspects, verify_for_mode};
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

/// Auto-detected decode parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoDecodeInfo {
    /// Pixels per module.
    pub module_px: u32,
    /// Quiet-zone width in modules.
    pub quiet_zone_modules: u32,
    /// Luma threshold below which a sampled module is considered dark.
    pub threshold: u8,
    /// Inferred layout family.
    pub layout: LayoutFamily,
}

/// Auto-decoded symbol and inference metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AutoDecodedSymbol {
    /// Decoded symbol payload and matrix.
    pub decoded: DecodedSymbol,
    /// Auto-detected parameters.
    pub info: AutoDecodeInfo,
}

/// Decode a matrix into a binary frame.
pub fn decode_matrix(matrix: &SymbolMatrix) -> Result<DecodedSymbol> {
    decode_matrix_with_suspects(matrix, &[])
}

fn decode_matrix_with_suspects(
    matrix: &SymbolMatrix,
    suspect_bytes: &[usize],
) -> Result<DecodedSymbol> {
    let bits = matrix.read_data_bits();
    let sampled_bytes = bitstream::bits_to_bytes(&bits);
    let header = FrameHeader::decode(&sampled_bytes)?;
    let data_len = HEADER_LEN + header.payload_len as usize;
    if verify_for_mode(header.mode, header.ecc_level, &sampled_bytes, data_len) {
        let frame = Frame::decode(&sampled_bytes)?;
        return Ok(DecodedSymbol {
            matrix: matrix.clone(),
            frame,
            sampled_bytes,
        });
    }

    let recovered = if suspect_bytes.is_empty() {
        try_recover_for_mode(header.mode, header.ecc_level, &sampled_bytes, data_len)
    } else {
        try_recover_for_mode_with_suspects(
            header.mode,
            header.ecc_level,
            &sampled_bytes,
            data_len,
            suspect_bytes,
            RECOVERY_MAX_ATTEMPTS,
        )
    };
    if let Some(recovered_bytes) = recovered {
        if verify_for_mode(header.mode, header.ecc_level, &recovered_bytes, data_len) {
            let recovered_frame = Frame::decode(&recovered_bytes)?;
            return Ok(DecodedSymbol {
                matrix: matrix.clone(),
                frame: recovered_frame,
                sampled_bytes: recovered_bytes,
            });
        }
    }

    Err(DecodeError::EccMismatch)
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
        let luma = image.to_luma8();
        let (matrix, bit_confidence) = Self::sample_matrix_with_luma_asymmetric_and_confidence(
            &luma,
            &self.options,
            self.options.quiet_zone_modules,
            self.options.quiet_zone_modules,
        )?;
        let suspect_bytes =
            suspect_bytes_from_bit_confidence(&matrix, &bit_confidence, MAX_SUSPECT_BYTES);
        decode_matrix_with_suspects(&matrix, &suspect_bytes)
    }

    /// Decode a rendered GlyphNet image by inferring module size and quiet zone.
    pub fn decode_auto(&self, image: &DynamicImage) -> Result<DecodedSymbol> {
        Ok(self.decode_auto_with_info(image)?.decoded)
    }

    /// Decode a rendered GlyphNet image and return the inferred parameters.
    pub fn decode_auto_with_info(&self, image: &DynamicImage) -> Result<AutoDecodedSymbol> {
        let luma = image.to_luma8();
        let width = luma.width();
        let height = luma.height();
        if width == 0 || height == 0 {
            return Err(DecodeError::InvalidImageDimensions);
        }
        let gcd = gcd_u32(width, height);
        let mut candidates = module_candidates(gcd, self.options.module_px);
        if candidates.is_empty() {
            return Err(DecodeError::AutoDetectFailed);
        }
        let thresholds = threshold_candidates(self.options.threshold, &luma);
        let layouts = layout_candidates(self.options.layout);
        for module_px in candidates.drain(..) {
            let width_modules = width / module_px;
            let height_modules = height / module_px;
            for (quiet_zone_x, quiet_zone_y) in quiet_zone_candidates(width_modules, height_modules)
            {
                if width_modules <= quiet_zone_x * 2 || height_modules <= quiet_zone_y * 2 {
                    continue;
                }
                let symbol_width = width_modules - quiet_zone_x * 2;
                let symbol_height = height_modules - quiet_zone_y * 2;
                if !plausible_symbol_geometry(symbol_width, symbol_height) {
                    continue;
                }
                for layout in &layouts {
                    for threshold in &thresholds {
                        let options = DecodeOptions {
                            module_px,
                            quiet_zone_modules: quiet_zone_x.min(quiet_zone_y),
                            threshold: *threshold,
                            layout: *layout,
                        };
                        if !header_precheck(
                            &luma,
                            &options,
                            quiet_zone_x,
                            quiet_zone_y,
                            symbol_width as u16,
                            symbol_height as u16,
                        ) {
                            continue;
                        }
                        let (matrix, bit_confidence) =
                            match Self::sample_matrix_with_luma_asymmetric_and_confidence(
                                &luma,
                                &options,
                                quiet_zone_x,
                                quiet_zone_y,
                            ) {
                                Ok(matrix) => matrix,
                                Err(_) => continue,
                            };
                        let suspect_bytes = suspect_bytes_from_bit_confidence(
                            &matrix,
                            &bit_confidence,
                            MAX_SUSPECT_BYTES,
                        );
                        if let Ok(decoded) = decode_matrix_with_suspects(&matrix, &suspect_bytes) {
                            return Ok(AutoDecodedSymbol {
                                decoded,
                                info: AutoDecodeInfo {
                                    module_px,
                                    quiet_zone_modules: quiet_zone_x.min(quiet_zone_y),
                                    threshold: *threshold,
                                    layout: *layout,
                                },
                            });
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

        Self::sample_matrix_with_luma_asymmetric(
            luma,
            options,
            options.quiet_zone_modules,
            options.quiet_zone_modules,
        )
    }

    fn sample_matrix_with_luma_asymmetric(
        luma: &GrayImage,
        options: &DecodeOptions,
        quiet_zone_x_modules: u32,
        quiet_zone_y_modules: u32,
    ) -> Result<SymbolMatrix> {
        Ok(Self::sample_matrix_with_luma_asymmetric_and_confidence(
            luma,
            options,
            quiet_zone_x_modules,
            quiet_zone_y_modules,
        )?
        .0)
    }

    fn sample_matrix_with_luma_asymmetric_and_confidence(
        luma: &GrayImage,
        options: &DecodeOptions,
        quiet_zone_x_modules: u32,
        quiet_zone_y_modules: u32,
    ) -> Result<(SymbolMatrix, Vec<u8>)> {
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
        if width_modules <= quiet_zone_x_modules * 2 || height_modules <= quiet_zone_y_modules * 2 {
            return Err(DecodeError::InvalidImageDimensions);
        }

        let symbol_width = (width_modules - quiet_zone_x_modules * 2) as u16;
        let symbol_height = (height_modules - quiet_zone_y_modules * 2) as u16;
        let mut matrix = SymbolMatrix::with_layout(symbol_width, symbol_height, options.layout);
        let mut bit_confidence = Vec::new();

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
                        quiet_zone_x_modules,
                        quiet_zone_y_modules,
                    );
                    matrix.set(x, y, Cell::Data(avg < options.threshold))?;
                    bit_confidence.push(avg.abs_diff(options.threshold));
                }
            }
        }

        Ok((matrix, bit_confidence))
    }
}

impl Default for RasterDecoder {
    fn default() -> Self {
        Self::new(DecodeOptions::default())
    }
}

const AUTO_QUIET_ZONE_MAX: u32 = 32;
const AUTO_MIN_SYMBOL_MODULES: u32 = 20;
const AUTO_MAX_SYMBOL_WIDTH_MODULES: u32 = 512;
const AUTO_MAX_SYMBOL_HEIGHT_MODULES: u32 = 256;
const AUTO_MAX_SYMBOL_AREA_MODULES: u32 = 65_536;
const MAX_SUSPECT_BYTES: usize = 16;
const RECOVERY_MAX_ATTEMPTS: usize = 4096;

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

fn module_candidates(gcd: u32, preferred: u32) -> Vec<u32> {
    let mut candidates = Vec::new();
    if preferred != 0 && gcd % preferred == 0 {
        candidates.push(preferred);
    }
    for candidate in divisors_desc(gcd) {
        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }
    candidates
}

fn quiet_zone_candidates(width_modules: u32, height_modules: u32) -> Vec<(u32, u32)> {
    let mut candidates = Vec::new();
    for quiet in 0..=AUTO_QUIET_ZONE_MAX {
        if width_modules > quiet * 2 && height_modules > quiet * 2 {
            let symbol_width = width_modules - quiet * 2;
            let symbol_height = height_modules - quiet * 2;
            if reference_sized_geometry(symbol_width, symbol_height) {
                candidates.push((quiet, quiet));
            }
        }
    }
    for quiet_x in 0..=AUTO_QUIET_ZONE_MAX {
        for quiet_y in 0..=AUTO_QUIET_ZONE_MAX {
            if quiet_x == quiet_y {
                continue;
            }
            if width_modules > quiet_x * 2 && height_modules > quiet_y * 2 {
                let symbol_width = width_modules - quiet_x * 2;
                let symbol_height = height_modules - quiet_y * 2;
                if reference_sized_geometry(symbol_width, symbol_height) {
                    candidates.push((quiet_x, quiet_y));
                }
            }
        }
    }
    candidates
}

fn reference_sized_geometry(width: u32, height: u32) -> bool {
    if width == height && width >= 29 {
        return true;
    }
    if width >= 96 && height >= 24 && width % 4 == 0 && height % 4 == 0 {
        let aspect = width as f32 / height.max(1) as f32;
        return (2.0..=6.5).contains(&aspect);
    }
    if width >= 48 && height >= 28 && width % 2 == 0 && height % 2 == 0 {
        let aspect = width as f32 / height.max(1) as f32;
        return (1.0..=3.2).contains(&aspect);
    }
    false
}

fn plausible_symbol_geometry(width: u32, height: u32) -> bool {
    if width < AUTO_MIN_SYMBOL_MODULES || height < AUTO_MIN_SYMBOL_MODULES {
        return false;
    }
    if width > AUTO_MAX_SYMBOL_WIDTH_MODULES || height > AUTO_MAX_SYMBOL_HEIGHT_MODULES {
        return false;
    }
    if width.saturating_mul(height) > AUTO_MAX_SYMBOL_AREA_MODULES {
        return false;
    }
    let aspect = width as f32 / height.max(1) as f32;
    (0.5..=8.0).contains(&aspect)
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
    quiet_zone_x_modules: u32,
    quiet_zone_y_modules: u32,
) -> u8 {
    let start_x = (module_x + quiet_zone_x_modules) * module_px;
    let start_y = (module_y + quiet_zone_y_modules) * module_px;
    let mut sum = 0u32;
    for y in start_y..start_y + module_px {
        for x in start_x..start_x + module_px {
            sum += u32::from(image.get_pixel(x, y).0[0]);
        }
    }
    (sum / (module_px * module_px)) as u8
}

fn header_precheck(
    luma: &GrayImage,
    options: &DecodeOptions,
    quiet_zone_x_modules: u32,
    quiet_zone_y_modules: u32,
    symbol_width: u16,
    symbol_height: u16,
) -> bool {
    let mut bits = Vec::with_capacity(HEADER_LEN * 8);
    'rows: for y in 0..symbol_height {
        for x in 0..symbol_width {
            if !layout::is_data_module_for(options.layout, symbol_width, symbol_height, x, y) {
                continue;
            }
            let avg = average_module_luma(
                luma,
                u32::from(x),
                u32::from(y),
                options.module_px,
                quiet_zone_x_modules,
                quiet_zone_y_modules,
            );
            bits.push(avg < options.threshold);
            if bits.len() == HEADER_LEN * 8 {
                break 'rows;
            }
        }
    }
    if bits.len() < HEADER_LEN * 8 {
        return false;
    }
    let bytes = bitstream::bits_to_bytes(&bits);
    FrameHeader::decode(&bytes).is_ok()
}

fn suspect_bytes_from_bit_confidence(
    matrix: &SymbolMatrix,
    bit_confidence: &[u8],
    limit: usize,
) -> Vec<usize> {
    if bit_confidence.is_empty() || limit == 0 {
        return Vec::new();
    }
    let total_data_bits = matrix.read_data_bits().len();
    let total_bytes = total_data_bits.div_ceil(8);
    let mut scores = Vec::with_capacity(total_bytes);
    for byte_index in 0..total_bytes {
        let start = byte_index * 8;
        let end = (start + 8).min(bit_confidence.len());
        if start >= end {
            break;
        }
        let score = *bit_confidence[start..end].iter().min().unwrap_or(&u8::MAX);
        scores.push((byte_index, score));
    }
    scores.sort_by_key(|(_, score)| *score);
    scores.into_iter().take(limit).map(|(idx, _)| idx).collect()
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

    #[test]
    fn decode_matrix_recovers_single_byte_corruption_with_parity() {
        let encoded = Encoder::new(EncoderConfig {
            mode: glyphnet_core::TransmissionMode::Screen,
            ..EncoderConfig::default()
        })
        .encode_static(b"recover-me")
        .unwrap();
        let mut matrix = encoded.matrix.clone();
        let mut bits = bitstream::bytes_to_bits(&encoded.codewords);
        bits[HEADER_LEN * 8 + 11] = !bits[HEADER_LEN * 8 + 11];
        matrix.write_data_bits(bits);

        let decoded = decode_matrix(&matrix).unwrap();
        assert_eq!(decoded.frame.payload, b"recover-me");
    }

    #[test]
    fn decode_matrix_recovers_single_byte_corruption_with_print_rs() {
        let encoded = Encoder::new(EncoderConfig {
            mode: glyphnet_core::TransmissionMode::Print,
            ..EncoderConfig::default()
        })
        .encode_static(b"recover-print-rs")
        .unwrap();
        let mut matrix = encoded.matrix.clone();
        let mut bits = bitstream::bytes_to_bits(&encoded.codewords);
        bits[HEADER_LEN * 8 + 13] = !bits[HEADER_LEN * 8 + 13];
        matrix.write_data_bits(bits);

        let decoded = decode_matrix(&matrix).unwrap();
        assert_eq!(decoded.frame.payload, b"recover-print-rs");
    }
}
