//! GlyphNet matrix and raster decoders.

use glyphnet_core::{
    Cell, Frame, GlyphError, LayoutFamily, Result as CoreResult, SymbolMatrix, layout,
    open_authenticated_payload,
};
use glyphnet_ecc::RecoveryTelemetry;
use image::{DynamicImage, GrayImage};
use thiserror::Error;

mod autodetect;
mod recovery;

use autodetect::{
    MAX_SUSPECT_BYTES, average_module_luma, gcd_u32, header_precheck, layout_candidates,
    module_candidates, plausible_symbol_geometry, quiet_zone_candidates,
    suspect_bytes_from_bit_confidence, threshold_candidates,
};
use recovery::decode_matrix_with_suspects;

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
    /// ECC recovery telemetry for this decode attempt.
    pub recovery: RecoveryTelemetry,
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

/// Validate a sampled byte stream without constructing a matrix.
pub fn decode_wire_prefix(bytes: &[u8]) -> CoreResult<Frame> {
    Frame::decode(bytes)
}

/// Verify and unwrap an authenticated payload envelope.
pub fn decode_authenticated_payload<F>(payload: &[u8], key_lookup: F) -> Result<Vec<u8>>
where
    F: FnMut(u32) -> Option<[u8; 32]>,
{
    let (_, raw) = open_authenticated_payload(payload, key_lookup)?;
    Ok(raw)
}

#[cfg(test)]
mod tests {
    use glyphnet_core::{HEADER_LEN, ProfileId, bitstream};
    use glyphnet_ecc::RecoveryMethod;
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
        assert_eq!(decoded.recovery.method, RecoveryMethod::None);
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
    fn decode_matrix_recovers_parity_tail_corruption_with_parity_mode() {
        let encoded = Encoder::new(EncoderConfig {
            mode: glyphnet_core::TransmissionMode::Screen,
            ..EncoderConfig::default()
        })
        .encode_static(b"recover-me")
        .unwrap();
        let mut matrix = encoded.matrix.clone();
        let mut bytes = encoded.codewords.clone();
        let corrupt_index = bytes.len() - 1;
        bytes[corrupt_index] ^= 0x01;
        let bits = bitstream::bytes_to_bits(&bytes);
        matrix.write_data_bits(bits);

        let decoded = decode_matrix(&matrix).unwrap();
        assert_eq!(decoded.frame.payload, b"recover-me");
        assert_eq!(decoded.recovery.method, RecoveryMethod::ParityTailRebuild);
        assert!(decoded.recovery.recovered);
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
        assert_eq!(decoded.recovery.method, RecoveryMethod::ReedSolomonSingle);
        assert!(decoded.recovery.recovered);
    }

    #[test]
    fn decode_matrix_recovers_two_byte_corruption_with_print_rs() {
        let encoded = Encoder::new(EncoderConfig {
            mode: glyphnet_core::TransmissionMode::Print,
            ..EncoderConfig::default()
        })
        .encode_static(b"recover-print-rs-two")
        .unwrap();
        let mut matrix = encoded.matrix.clone();
        let mut bits = bitstream::bytes_to_bits(&encoded.codewords);
        let bit_a = HEADER_LEN * 8 + 9;
        let bit_b = HEADER_LEN * 8 + 21;
        bits[bit_a] = !bits[bit_a];
        bits[bit_b] = !bits[bit_b];
        matrix.write_data_bits(bits);

        let decoded = decode_matrix_with_suspects(&matrix, &[bit_a / 8, bit_b / 8]).unwrap();
        assert_eq!(decoded.frame.payload, b"recover-print-rs-two");
        assert_eq!(decoded.recovery.method, RecoveryMethod::ReedSolomonPair);
        assert!(decoded.recovery.recovered);
    }

    #[test]
    fn authenticated_payload_roundtrips_with_key_lookup() {
        let key = [0x11u8; 32];
        let encoded = Encoder::default()
            .encode_static_authenticated(b"auth-payload", &key, 77)
            .unwrap();
        let image = RasterRenderer::default().render(&encoded.matrix).unwrap();
        let decoded = RasterDecoder::default()
            .decode(&DynamicImage::ImageRgba8(image))
            .unwrap();
        let payload = decode_authenticated_payload(&decoded.frame.payload, |id| {
            if id == 77 { Some(key) } else { None }
        })
        .unwrap();
        assert_eq!(payload, b"auth-payload");
    }

    #[cfg(feature = "ldpc")]
    #[test]
    fn decode_matrix_recovers_screen_ldpc_parity_tail_corruption() {
        let encoded = Encoder::new(EncoderConfig {
            mode: glyphnet_core::TransmissionMode::Screen,
            ..EncoderConfig::default()
        })
        .encode_static(b"recover-screen-ldpc-tail")
        .unwrap();
        let mut matrix = encoded.matrix.clone();
        let mut bytes = encoded.codewords.clone();
        let corrupt_index = bytes.len() - 1;
        bytes[corrupt_index] ^= 0x01;
        let bits = bitstream::bytes_to_bits(&bytes);
        matrix.write_data_bits(bits);

        let decoded = decode_matrix(&matrix).unwrap();
        assert_eq!(decoded.frame.payload, b"recover-screen-ldpc-tail");
        assert_eq!(decoded.recovery.method, RecoveryMethod::ParityTailRebuild);
        assert!(decoded.recovery.recovered);
    }

    #[cfg(feature = "ldpc")]
    #[test]
    fn decode_matrix_rejects_unrecoverable_screen_ldpc_single_byte_corruption() {
        let encoded = Encoder::new(EncoderConfig {
            mode: glyphnet_core::TransmissionMode::Screen,
            ..EncoderConfig::default()
        })
        .encode_static(b"recover-screen-ldpc-byte")
        .unwrap();
        let mut matrix = encoded.matrix.clone();
        let mut bits = bitstream::bytes_to_bits(&encoded.codewords);
        let bit_index = HEADER_LEN * 8 + 15;
        bits[bit_index] = !bits[bit_index];
        matrix.write_data_bits(bits);

        let err = decode_matrix_with_suspects(&matrix, &[bit_index / 8]).unwrap_err();
        assert!(matches!(err, DecodeError::EccMismatch));
    }
}
