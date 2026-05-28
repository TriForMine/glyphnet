use std::{collections::HashMap, fs, path::PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use glyphnet_core::{
    DetachedAuthSignature, DetachedEd25519Signature, EccLevel, ProfileId, SymbolGeometry,
    TransmissionMode, open_authenticated_payload, profile_catalog, profile_spec,
    sign_detached_payload, sign_detached_payload_ed25519, verify_detached_payload,
    verify_detached_payload_ed25519,
};
use glyphnet_decode::{RasterDecoder, decode_authenticated_payload};
use glyphnet_encode::{Encoder, EncoderConfig};
use glyphnet_render::{RasterRenderer, RenderOptions, SvgRenderer};
use glyphnet_scanner::{CameraFrame, Scanner, ScannerConfig, scan_still};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

#[derive(Debug, Parser)]
#[command(name = "glyphnet")]
#[command(about = "Generate, inspect, and decode GlyphNet optical codes")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Encode text or binary-like UTF-8 data as PNG or SVG.
    Encode {
        /// Payload data.
        #[arg(long)]
        data: String,
        /// Output file path.
        #[arg(short, long)]
        output: PathBuf,
        /// Protocol profile.
        #[arg(long, value_enum, default_value_t = ProfileArg::RibbonPrint)]
        profile: ProfileArg,
        /// Transmission mode.
        #[arg(long, value_enum)]
        mode: Option<ModeArg>,
        /// ECC level.
        #[arg(long, value_enum)]
        ecc: Option<EccArg>,
        /// Output format.
        #[arg(long, value_enum, default_value_t = FormatArg::Png)]
        format: FormatArg,
        /// Explicit symbol width in modules.
        #[arg(long, value_name = "MODULES")]
        width_modules: Option<u16>,
        /// Explicit symbol height in modules.
        #[arg(long, value_name = "MODULES")]
        height_modules: Option<u16>,
        /// Fit output width in pixels.
        #[arg(long, value_name = "PX")]
        fit_width_px: Option<u32>,
        /// Fit output height in pixels.
        #[arg(long, value_name = "PX")]
        fit_height_px: Option<u32>,
        /// Optional 32-byte authentication key in hex (64 hex chars).
        #[arg(long)]
        auth_key_hex: Option<String>,
        /// Key id attached to the authenticity envelope.
        #[arg(long, default_value_t = 1)]
        auth_key_id: u32,
    },
    /// Decode a rendered PNG/JPEG image produced by the reference renderer.
    Decode {
        /// Input image path.
        input: PathBuf,
        /// Infer module size and quiet zone automatically.
        #[arg(long)]
        auto: bool,
        /// Optional verification key in hex for authenticated payload envelopes.
        #[arg(long)]
        verify_key_hex: Option<String>,
        /// Key id for the verification key.
        #[arg(long, default_value_t = 1)]
        verify_key_id: u32,
        /// Optional verification keyring JSON file: [{ "key_id": 1, "key_hex": "..." }].
        #[arg(long)]
        verify_key_file: Option<PathBuf>,
        /// Optional detached signature JSON file.
        #[arg(long)]
        detached_auth_file: Option<PathBuf>,
    },
    /// Scan an image, attempting coarse auto-crop before decoding.
    Scan {
        /// Input image path.
        input: PathBuf,
        /// Transmission mode used for CV tuning.
        #[arg(long, value_enum, default_value_t = ModeArg::Print)]
        mode: ModeArg,
        /// Optional verification key in hex for authenticated payload envelopes.
        #[arg(long)]
        verify_key_hex: Option<String>,
        /// Key id for the verification key.
        #[arg(long, default_value_t = 1)]
        verify_key_id: u32,
        /// Optional verification keyring JSON file: [{ "key_id": 1, "key_hex": "..." }].
        #[arg(long)]
        verify_key_file: Option<PathBuf>,
        /// Optional detached signature JSON file.
        #[arg(long)]
        detached_auth_file: Option<PathBuf>,
    },
    /// Scan an ordered directory of frames as a burst session.
    ScanBurst {
        /// Input directory containing frame images.
        input_dir: PathBuf,
        /// Transmission mode used for scanner decode.
        #[arg(long, value_enum, default_value_t = ModeArg::Burst)]
        mode: ModeArg,
    },
    /// Print descriptor JSON without rendering.
    Inspect {
        /// Payload data.
        #[arg(long)]
        data: String,
        /// Protocol profile.
        #[arg(long, value_enum, default_value_t = ProfileArg::RibbonPrint)]
        profile: ProfileArg,
        /// Transmission mode.
        #[arg(long, value_enum)]
        mode: Option<ModeArg>,
        /// ECC level.
        #[arg(long, value_enum)]
        ecc: Option<EccArg>,
        /// Explicit symbol width in modules.
        #[arg(long, value_name = "MODULES")]
        width_modules: Option<u16>,
        /// Explicit symbol height in modules.
        #[arg(long, value_name = "MODULES")]
        height_modules: Option<u16>,
    },
    /// Encode an animated burst stream as numbered SVG frames.
    Burst {
        /// Payload data.
        #[arg(long)]
        data: String,
        /// Output directory.
        #[arg(short, long)]
        output_dir: PathBuf,
        /// Protocol profile.
        #[arg(long, value_enum, default_value_t = ProfileArg::PulseBurst)]
        profile: ProfileArg,
        /// Maximum payload bytes per frame.
        #[arg(long, default_value_t = 512)]
        frame_payload: usize,
        /// Data shard count for burst erasure coding.
        #[arg(long)]
        erasure_data_shards: Option<usize>,
        /// Explicit symbol width in modules.
        #[arg(long, value_name = "MODULES")]
        width_modules: Option<u16>,
        /// Explicit symbol height in modules.
        #[arg(long, value_name = "MODULES")]
        height_modules: Option<u16>,
        /// Fit output width in pixels.
        #[arg(long, value_name = "PX")]
        fit_width_px: Option<u32>,
        /// Fit output height in pixels.
        #[arg(long, value_name = "PX")]
        fit_height_px: Option<u32>,
    },
    /// Print the built-in protocol profile catalog as JSON.
    Profiles,
    /// Print benchmark targets and suggested regression commands as JSON.
    BenchPlan,
    /// Create a detached authenticity signature sidecar JSON for payload data.
    AuthSign {
        /// Payload data.
        #[arg(long)]
        data: String,
        /// Output detached signature JSON path.
        #[arg(short, long)]
        output: PathBuf,
        /// 32-byte authentication key in hex (64 hex chars).
        #[arg(long)]
        auth_key_hex: String,
        /// Key id attached to detached signature.
        #[arg(long, default_value_t = 1)]
        auth_key_id: u32,
    },
    /// Create a detached Ed25519 authenticity signature sidecar JSON for payload data.
    AuthSignEd25519 {
        /// Payload data.
        #[arg(long)]
        data: String,
        /// Output detached signature JSON path.
        #[arg(short, long)]
        output: PathBuf,
        /// 32-byte Ed25519 signing key in hex (64 hex chars).
        #[arg(long)]
        signing_key_hex: String,
        /// Key id attached to detached signature.
        #[arg(long, default_value_t = 1)]
        key_id: u32,
    },
    /// Verify a detached Ed25519 authenticity signature JSON sidecar.
    AuthVerifyEd25519 {
        /// Payload data.
        #[arg(long)]
        data: String,
        /// Detached signature JSON path.
        #[arg(long)]
        signature: PathBuf,
        /// 32-byte Ed25519 public key in hex (64 hex chars).
        #[arg(long)]
        public_key_hex: String,
        /// Key id used to resolve the verification key.
        #[arg(long, default_value_t = 1)]
        key_id: u32,
    },
    /// Inspect a versioned authenticity keyset file.
    KeysetInspect {
        /// Keyset JSON path.
        path: PathBuf,
    },
    /// Validate a versioned authenticity keyset file.
    KeysetValidate {
        /// Keyset JSON path.
        path: PathBuf,
        /// Optional trusted root public key (hex) to enforce keyset signature verification.
        #[arg(long)]
        root_pubkey_hex: Option<String>,
    },
    /// Sign a keyset JSON using detached Ed25519 signature metadata.
    KeysetSignEd25519 {
        /// Input keyset JSON path.
        input: PathBuf,
        /// Output keyset JSON path.
        #[arg(short, long)]
        output: PathBuf,
        /// 32-byte Ed25519 signing key in hex (64 hex chars).
        #[arg(long)]
        signing_key_hex: String,
        /// Signer identity label.
        #[arg(long, default_value = "root")]
        signed_by: String,
    },
    /// Verify a signed keyset JSON with trusted root public key.
    KeysetVerify {
        /// Keyset JSON path.
        path: PathBuf,
        /// 32-byte trusted root Ed25519 public key in hex (64 hex chars).
        #[arg(long)]
        root_pubkey_hex: String,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ProfileArg {
    #[value(name = "ribbon-print")]
    RibbonPrint,
    #[value(name = "spectral-screen")]
    SpectralScreen,
    #[value(name = "pulse-burst")]
    PulseBurst,
    #[value(name = "constellation-print")]
    ConstellationPrint,
    #[value(name = "matrix-compat")]
    MatrixCompat,
}

impl From<ProfileArg> for ProfileId {
    fn from(value: ProfileArg) -> Self {
        match value {
            ProfileArg::RibbonPrint => Self::RibbonPrint,
            ProfileArg::SpectralScreen => Self::SpectralScreen,
            ProfileArg::PulseBurst => Self::PulseBurst,
            ProfileArg::ConstellationPrint => Self::ConstellationPrint,
            ProfileArg::MatrixCompat => Self::MatrixCompat,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum ModeArg {
    Print,
    Screen,
    Burst,
}

impl From<ModeArg> for TransmissionMode {
    fn from(value: ModeArg) -> Self {
        match value {
            ModeArg::Print => Self::Print,
            ModeArg::Screen => Self::Screen,
            ModeArg::Burst => Self::Burst,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum EccArg {
    Low,
    Medium,
    High,
    Adaptive,
}

impl From<EccArg> for EccLevel {
    fn from(value: EccArg) -> Self {
        match value {
            EccArg::Low => Self::Low,
            EccArg::Medium => Self::Medium,
            EccArg::High => Self::High,
            EccArg::Adaptive => Self::Adaptive,
        }
    }
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum FormatArg {
    Png,
    Svg,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Encode {
            data,
            output,
            profile,
            mode,
            ecc,
            format,
            width_modules,
            height_modules,
            fit_width_px,
            fit_height_px,
            auth_key_hex,
            auth_key_id,
        } => {
            let sizing = render_sizing(width_modules, height_modules, fit_width_px, fit_height_px)?;
            encode(EncodeRequest {
                payload: data.as_bytes(),
                output,
                profile: profile.into(),
                mode: mode.map(Into::into),
                ecc_level: ecc.map(Into::into),
                format,
                sizing,
                auth_key_hex: auth_key_hex.as_deref(),
                auth_key_id,
            })
        }
        Command::Decode {
            input,
            auto,
            verify_key_hex,
            verify_key_id,
            verify_key_file,
            detached_auth_file,
        } => decode(
            input,
            auto,
            verify_key_hex.as_deref(),
            verify_key_id,
            verify_key_file,
            detached_auth_file,
        ),
        Command::Scan {
            input,
            mode,
            verify_key_hex,
            verify_key_id,
            verify_key_file,
            detached_auth_file,
        } => scan(
            input,
            mode.into(),
            verify_key_hex.as_deref(),
            verify_key_id,
            verify_key_file,
            detached_auth_file,
        ),
        Command::ScanBurst { input_dir, mode } => scan_burst(input_dir, mode.into()),
        Command::Inspect {
            data,
            profile,
            mode,
            ecc,
            width_modules,
            height_modules,
        } => {
            let geometry = geometry_override(width_modules, height_modules)?;
            inspect(
                data.as_bytes(),
                profile.into(),
                mode.map(Into::into),
                ecc.map(Into::into),
                geometry,
            )
        }
        Command::Burst {
            data,
            output_dir,
            profile,
            frame_payload,
            erasure_data_shards,
            width_modules,
            height_modules,
            fit_width_px,
            fit_height_px,
        } => {
            let sizing = render_sizing(width_modules, height_modules, fit_width_px, fit_height_px)?;
            burst(
                data.as_bytes(),
                output_dir,
                profile.into(),
                frame_payload,
                erasure_data_shards,
                sizing,
            )
        }
        Command::Profiles => profiles(),
        Command::BenchPlan => bench_plan(),
        Command::AuthSign {
            data,
            output,
            auth_key_hex,
            auth_key_id,
        } => auth_sign(data.as_bytes(), output, &auth_key_hex, auth_key_id),
        Command::AuthSignEd25519 {
            data,
            output,
            signing_key_hex,
            key_id,
        } => auth_sign_ed25519(data.as_bytes(), output, &signing_key_hex, key_id),
        Command::AuthVerifyEd25519 {
            data,
            signature,
            public_key_hex,
            key_id,
        } => auth_verify_ed25519(data.as_bytes(), signature, &public_key_hex, key_id),
        Command::KeysetInspect { path } => keyset_inspect(path),
        Command::KeysetValidate {
            path,
            root_pubkey_hex,
        } => keyset_validate(path, root_pubkey_hex.as_deref()),
        Command::KeysetSignEd25519 {
            input,
            output,
            signing_key_hex,
            signed_by,
        } => keyset_sign_ed25519(input, output, &signing_key_hex, &signed_by),
        Command::KeysetVerify {
            path,
            root_pubkey_hex,
        } => keyset_verify(path, &root_pubkey_hex),
    }
}

#[derive(Debug, Clone, Copy)]
struct FitSize {
    width_px: u32,
    height_px: u32,
}

#[derive(Debug, Clone, Copy)]
struct RenderSizing {
    geometry: Option<SymbolGeometry>,
    fit: Option<FitSize>,
}

fn geometry_override(
    width_modules: Option<u16>,
    height_modules: Option<u16>,
) -> Result<Option<SymbolGeometry>> {
    match (width_modules, height_modules) {
        (None, None) => Ok(None),
        (Some(width), Some(height)) => {
            if width == 0 || height == 0 {
                bail!("symbol geometry must be non-zero");
            }
            Ok(Some(SymbolGeometry::new(width, height)))
        }
        _ => bail!("--width-modules and --height-modules must be set together"),
    }
}

fn fit_override(fit_width_px: Option<u32>, fit_height_px: Option<u32>) -> Result<Option<FitSize>> {
    match (fit_width_px, fit_height_px) {
        (None, None) => Ok(None),
        (Some(width_px), Some(height_px)) => {
            if width_px == 0 || height_px == 0 {
                bail!("fit pixel size must be non-zero");
            }
            Ok(Some(FitSize {
                width_px,
                height_px,
            }))
        }
        _ => bail!("--fit-width-px and --fit-height-px must be set together"),
    }
}

fn render_sizing(
    width_modules: Option<u16>,
    height_modules: Option<u16>,
    fit_width_px: Option<u32>,
    fit_height_px: Option<u32>,
) -> Result<RenderSizing> {
    Ok(RenderSizing {
        geometry: geometry_override(width_modules, height_modules)?,
        fit: fit_override(fit_width_px, fit_height_px)?,
    })
}

fn apply_fit(
    options: RenderOptions,
    symbol_width: u16,
    symbol_height: u16,
    fit: Option<FitSize>,
) -> Result<RenderOptions> {
    match fit {
        Some(fit) => {
            Ok(options.fit_to_size(symbol_width, symbol_height, fit.width_px, fit.height_px)?)
        }
        None => Ok(options),
    }
}

fn layout_name(layout: glyphnet_core::LayoutFamily) -> &'static str {
    match layout {
        glyphnet_core::LayoutFamily::RibbonWeave => "ribbon-weave",
        glyphnet_core::LayoutFamily::SpectralMesh => "spectral-mesh",
        glyphnet_core::LayoutFamily::PulseStream => "pulse-stream",
        glyphnet_core::LayoutFamily::Constellation => "constellation",
        glyphnet_core::LayoutFamily::FrameGrid => "frame-grid",
        glyphnet_core::LayoutFamily::Matrix => "matrix",
        glyphnet_core::LayoutFamily::Hexagonal => "hexagonal",
        glyphnet_core::LayoutFamily::Radial => "radial",
    }
}

fn encoder(
    profile: ProfileId,
    mode_override: Option<TransmissionMode>,
    ecc_override: Option<EccLevel>,
    geometry: Option<SymbolGeometry>,
) -> Encoder {
    let mut config = EncoderConfig::for_profile(profile);
    if let Some(mode) = mode_override {
        config.mode = mode;
    }
    if let Some(ecc_level) = ecc_override {
        config.ecc_level = ecc_level;
    }
    config.geometry = geometry;
    Encoder::new(config)
}

struct EncodeRequest<'a> {
    payload: &'a [u8],
    output: PathBuf,
    profile: ProfileId,
    mode: Option<TransmissionMode>,
    ecc_level: Option<EccLevel>,
    format: FormatArg,
    sizing: RenderSizing,
    auth_key_hex: Option<&'a str>,
    auth_key_id: u32,
}

fn encode(request: EncodeRequest<'_>) -> Result<()> {
    let encoder = encoder(
        request.profile,
        request.mode,
        request.ecc_level,
        request.sizing.geometry,
    );
    let encoded = if let Some(key_hex) = request.auth_key_hex {
        let key = parse_auth_key_hex(key_hex)?;
        encoder
            .encode_static_authenticated(request.payload, &key, request.auth_key_id)
            .context("failed to encode authenticated payload")?
    } else {
        encoder
            .encode_static(request.payload)
            .context("failed to encode payload")?
    };
    let render_options = apply_fit(
        RenderOptions::for_descriptor(&encoded.descriptor),
        encoded.descriptor.width,
        encoded.descriptor.height,
        request.sizing.fit,
    )?;
    match request.format {
        FormatArg::Png => {
            let image = RasterRenderer::new(render_options)
                .render(&encoded.matrix)
                .context("failed to render PNG")?;
            image.save(&request.output).with_context(|| {
                format!(
                    "failed to save rendered image to {}",
                    request.output.display()
                )
            })?;
        }
        FormatArg::Svg => {
            let svg = SvgRenderer::new(render_options)
                .render(&encoded.matrix)
                .context("failed to render SVG")?;
            fs::write(&request.output, svg)
                .with_context(|| format!("failed to write SVG to {}", request.output.display()))?;
        }
    }
    println!("{}", serde_json::to_string_pretty(&encoded.descriptor)?);
    Ok(())
}

fn decode(
    input: PathBuf,
    auto: bool,
    verify_key_hex: Option<&str>,
    verify_key_id: u32,
    verify_key_file: Option<PathBuf>,
    detached_auth_file: Option<PathBuf>,
) -> Result<()> {
    let image =
        image::open(&input).with_context(|| format!("failed to open image {}", input.display()))?;
    let detached_signature = load_detached_verification_input(detached_auth_file.as_ref())?;
    if auto {
        let auto_decoded = RasterDecoder::default()
            .decode_auto_with_info(&image)
            .context("failed to auto-decode GlyphNet image")?;
        let mut payload = decode_json(&auto_decoded, None, None, None);
        if let Some((verified, key_id, error)) = verify_auth_payload(
            &auto_decoded.decoded.frame.payload,
            verify_key_hex,
            verify_key_id,
            verify_key_file.as_ref(),
            detached_signature.as_ref(),
        )? {
            payload["auth"] =
                serde_json::json!({ "verified": verified, "key_id": key_id, "error": error });
        }
        println!("{payload}");
    } else {
        let decoded = RasterDecoder::default()
            .decode(&image)
            .context("failed to decode GlyphNet image")?;
        let mut payload = serde_json::json!({
            "stream_id": decoded.frame.header.stream_id,
            "frame_index": decoded.frame.header.frame_index,
            "frame_count": decoded.frame.header.frame_count,
            "mode": decoded.frame.header.mode.to_string(),
            "ecc": decoded.frame.header.ecc_level.to_string(),
            "payload_utf8_lossy": String::from_utf8_lossy(&decoded.frame.payload),
            "payload_len": decoded.frame.payload.len()
        });
        if let Some((verified, key_id, error)) = verify_auth_payload(
            &decoded.frame.payload,
            verify_key_hex,
            verify_key_id,
            verify_key_file.as_ref(),
            detached_signature.as_ref(),
        )? {
            payload["auth"] =
                serde_json::json!({ "verified": verified, "key_id": key_id, "error": error });
        }
        println!("{payload}");
    }
    Ok(())
}

fn scan(
    input: PathBuf,
    mode: TransmissionMode,
    verify_key_hex: Option<&str>,
    verify_key_id: u32,
    verify_key_file: Option<PathBuf>,
    detached_auth_file: Option<PathBuf>,
) -> Result<()> {
    let image =
        image::open(&input).with_context(|| format!("failed to open image {}", input.display()))?;
    let detached_signature = load_detached_verification_input(detached_auth_file.as_ref())?;
    let scanned = scan_still(&image, mode).context("failed to scan image")?;
    let crop = scanned.crop.map(|region| {
        serde_json::json!({
            "x": region.x,
            "y": region.y,
            "width": region.width,
            "height": region.height
        })
    });
    let quad = scanned.quad.map(|quad| {
        serde_json::json!({
            "top_left": { "x": quad.top_left.x, "y": quad.top_left.y },
            "top_right": { "x": quad.top_right.x, "y": quad.top_right.y },
            "bottom_right": { "x": quad.bottom_right.x, "y": quad.bottom_right.y },
            "bottom_left": { "x": quad.bottom_left.x, "y": quad.bottom_left.y }
        })
    });
    let warp = scanned.warp_size.map(|(width, height)| {
        serde_json::json!({
            "width": width,
            "height": height
        })
    });
    let telemetry = scanned.telemetry();
    let mut payload = decode_json(&scanned.decoded, crop, quad, warp);
    payload["scan_telemetry"] = serde_json::json!({
        "candidate_count": telemetry.candidate_count,
        "failed_candidates": telemetry.failed_candidates,
        "burst_progress": {
            "frame_count": scanned.decoded.decoded.frame.header.frame_count,
            "received_frames": 1,
            "missing_frames": usize::from(scanned.decoded.decoded.frame.header.frame_count.saturating_sub(1))
        },
        "timings": {
            "total_micros": telemetry.timings.total_micros,
            "full_frame_micros": telemetry.timings.full_frame_micros,
            "grayscale_micros": telemetry.timings.grayscale_micros,
            "threshold_micros": telemetry.timings.threshold_micros,
            "quad_micros": telemetry.timings.quad_micros,
            "candidate_micros": telemetry.timings.candidate_micros,
            "decode_attempts_micros": telemetry.timings.decode_attempts_micros
        }
    });
    if let Some((verified, key_id, error)) = verify_auth_payload(
        &scanned.decoded.decoded.frame.payload,
        verify_key_hex,
        verify_key_id,
        verify_key_file.as_ref(),
        detached_signature.as_ref(),
    )? {
        payload["auth"] = serde_json::json!({
            "verified": verified,
            "key_id": key_id,
            "error": error
        });
    }
    println!("{payload}");
    Ok(())
}

fn parse_auth_key_hex(input: &str) -> Result<[u8; 32]> {
    let normalized = input.trim();
    if normalized.len() != 64 {
        bail!("auth key must be exactly 64 hex characters");
    }
    let mut key = [0u8; 32];
    for (idx, slot) in key.iter_mut().enumerate() {
        let start = idx * 2;
        let end = start + 2;
        *slot = u8::from_str_radix(&normalized[start..end], 16)
            .with_context(|| format!("invalid hex at bytes {}..{}", start, end))?;
    }
    Ok(key)
}

fn parse_public_key_hex(input: &str) -> Result<[u8; 32]> {
    parse_auth_key_hex(input)
}

fn verify_auth_payload(
    payload: &[u8],
    verify_key_hex: Option<&str>,
    verify_key_id: u32,
    verify_key_file: Option<&PathBuf>,
    detached_signature: Option<&DetachedVerificationInput>,
) -> Result<Option<(bool, u32, Option<String>)>> {
    let keyring = verification_keyring(verify_key_hex, verify_key_id, verify_key_file)?;
    if let Some(signature) = detached_signature {
        match signature {
            DetachedVerificationInput::Mac(signature) => {
                match verify_detached_payload(payload, signature, |id| {
                    keyring.mac_keys.get(&id).copied()
                }) {
                    Ok(_) => return Ok(Some((true, signature.key_id, None))),
                    Err(error) => {
                        return Ok(Some((false, signature.key_id, Some(error.to_string()))));
                    }
                }
            }
            DetachedVerificationInput::Ed25519(signature) => {
                match verify_detached_payload_ed25519(payload, signature, |id| {
                    keyring.ed25519_public_keys.get(&id).copied()
                }) {
                    Ok(_) => return Ok(Some((true, signature.key_id, None))),
                    Err(error) => {
                        return Ok(Some((false, signature.key_id, Some(error.to_string()))));
                    }
                }
            }
        }
    }
    if open_authenticated_payload(payload, |_id| None).is_err() {
        return Ok(None);
    }
    if keyring.mac_keys.is_empty() {
        return Ok(Some((
            false,
            verify_key_id,
            Some("missing verification key".to_string()),
        )));
    }
    match decode_authenticated_payload(payload, |id| keyring.mac_keys.get(&id).copied()) {
        Ok(_) => Ok(Some((true, verify_key_id, None))),
        Err(error) => Ok(Some((false, verify_key_id, Some(error.to_string())))),
    }
}

#[derive(Default)]
struct VerificationKeyring {
    mac_keys: HashMap<u32, [u8; 32]>,
    ed25519_public_keys: HashMap<u32, [u8; 32]>,
}

fn verification_keyring(
    verify_key_hex: Option<&str>,
    verify_key_id: u32,
    verify_key_file: Option<&PathBuf>,
) -> Result<VerificationKeyring> {
    let mut keys = VerificationKeyring::default();
    if let Some(path) = verify_key_file {
        let loaded = load_verify_keys(path)?;
        keys.mac_keys.extend(loaded.mac_keys);
        keys.ed25519_public_keys.extend(loaded.ed25519_public_keys);
    }
    if let Some(hex) = verify_key_hex {
        keys.mac_keys
            .insert(verify_key_id, parse_auth_key_hex(hex)?);
    }
    Ok(keys)
}

fn load_verify_keys(path: &PathBuf) -> Result<VerificationKeyring> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read verify key file {}", path.display()))?;
    let json: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("invalid JSON in verify key file {}", path.display()))?;
    let mut out = VerificationKeyring::default();
    if let Some(arr) = json.as_array() {
        for item in arr {
            let key_id = item
                .get("key_id")
                .and_then(serde_json::Value::as_u64)
                .context("verify key item missing numeric key_id")? as u32;
            let key_hex = item
                .get("key_hex")
                .and_then(serde_json::Value::as_str)
                .context("verify key item missing string key_hex")?;
            out.mac_keys.insert(key_id, parse_auth_key_hex(key_hex)?);
        }
        return Ok(out);
    }
    let version = json
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .context("verify key file missing numeric version")?;
    if version != 1 {
        bail!("unsupported verify key file version {version}");
    }
    let keys = json
        .get("keys")
        .and_then(serde_json::Value::as_array)
        .context("verify key file missing keys array")?;
    for item in keys {
        let key_id = item
            .get("key_id")
            .and_then(serde_json::Value::as_u64)
            .context("verify key item missing numeric key_id")? as u32;
        let alg = item
            .get("alg")
            .and_then(serde_json::Value::as_str)
            .context("verify key item missing string alg")?;
        match alg {
            "mac-blake3" => {
                let key_hex = item
                    .get("key_hex")
                    .and_then(serde_json::Value::as_str)
                    .context("mac-blake3 key missing string key_hex")?;
                out.mac_keys.insert(key_id, parse_auth_key_hex(key_hex)?);
            }
            "ed25519" => {
                let key_hex = item
                    .get("public_key_hex")
                    .and_then(serde_json::Value::as_str)
                    .context("ed25519 key missing string public_key_hex")?;
                out.ed25519_public_keys
                    .insert(key_id, parse_public_key_hex(key_hex)?);
            }
            _ => bail!("unsupported key algorithm {alg}"),
        }
    }
    Ok(out)
}

fn load_keyset_json(path: &PathBuf) -> Result<serde_json::Value> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read keyset file {}", path.display()))?;
    serde_json::from_str(&content)
        .with_context(|| format!("invalid JSON in keyset file {}", path.display()))
}

fn validate_keyset_json(json: &serde_json::Value) -> Result<()> {
    let version = json
        .get("version")
        .and_then(serde_json::Value::as_u64)
        .context("keyset missing numeric version")?;
    if version != 1 {
        bail!("unsupported keyset version {version}");
    }
    json.get("issuer")
        .and_then(serde_json::Value::as_str)
        .context("keyset missing string issuer")?;
    let created_at = json
        .get("created_at")
        .and_then(serde_json::Value::as_str)
        .context("keyset missing string created_at")?;
    let expires_at = json
        .get("expires_at")
        .and_then(serde_json::Value::as_str)
        .context("keyset missing string expires_at")?;
    let created = OffsetDateTime::parse(created_at, &Rfc3339)
        .context("keyset created_at must be RFC3339 UTC")?;
    let expires = OffsetDateTime::parse(expires_at, &Rfc3339)
        .context("keyset expires_at must be RFC3339 UTC")?;
    if expires <= created {
        bail!("keyset expires_at must be after created_at");
    }
    let keys = json
        .get("keys")
        .and_then(serde_json::Value::as_array)
        .context("keyset missing keys array")?;
    if keys.is_empty() {
        bail!("keyset keys array must not be empty");
    }
    for item in keys {
        item.get("key_id")
            .and_then(serde_json::Value::as_u64)
            .context("keyset key missing numeric key_id")?;
        let alg = item
            .get("alg")
            .and_then(serde_json::Value::as_str)
            .context("keyset key missing string alg")?;
        match alg {
            "mac-blake3" => {
                let key_hex = item
                    .get("key_hex")
                    .and_then(serde_json::Value::as_str)
                    .context("mac-blake3 key missing string key_hex")?;
                let _ = parse_auth_key_hex(key_hex)?;
            }
            "ed25519" => {
                let key_hex = item
                    .get("public_key_hex")
                    .and_then(serde_json::Value::as_str)
                    .context("ed25519 key missing string public_key_hex")?;
                let _ = parse_public_key_hex(key_hex)?;
            }
            _ => bail!("unsupported key algorithm {alg}"),
        }
    }
    Ok(())
}

fn keyset_payload_for_signature(json: &serde_json::Value) -> Result<Vec<u8>> {
    let version = json
        .get("version")
        .cloned()
        .context("keyset missing version")?;
    let issuer = json
        .get("issuer")
        .cloned()
        .context("keyset missing issuer")?;
    let created_at = json
        .get("created_at")
        .cloned()
        .context("keyset missing created_at")?;
    let expires_at = json
        .get("expires_at")
        .cloned()
        .context("keyset missing expires_at")?;
    let keys = json.get("keys").cloned().context("keyset missing keys")?;
    serde_json::to_vec(&serde_json::json!({
        "version": version,
        "issuer": issuer,
        "created_at": created_at,
        "expires_at": expires_at,
        "keys": keys
    }))
    .context("failed to serialize keyset payload for signature")
}

fn parse_signature_hex(signature_hex: &str) -> Result<[u8; 64]> {
    if signature_hex.len() != 128 {
        bail!("keyset signature_hex must be 128 hex chars");
    }
    let mut signature = [0u8; 64];
    for (idx, slot) in signature.iter_mut().enumerate() {
        let start = idx * 2;
        let end = start + 2;
        *slot = u8::from_str_radix(&signature_hex[start..end], 16)
            .with_context(|| format!("invalid signature hex at bytes {}..{}", start, end))?;
    }
    Ok(signature)
}

fn verify_keyset_signature(json: &serde_json::Value, root_pubkey_hex: &str) -> Result<()> {
    let sig = json
        .get("signature")
        .and_then(serde_json::Value::as_object)
        .context("keyset missing signature object")?;
    let signature_alg = sig
        .get("signature_alg")
        .and_then(serde_json::Value::as_str)
        .context("keyset signature missing signature_alg")?;
    if signature_alg != "ed25519" {
        bail!("unsupported keyset signature_alg {signature_alg}");
    }
    let signature_hex = sig
        .get("signature_hex")
        .and_then(serde_json::Value::as_str)
        .context("keyset signature missing signature_hex")?;
    let signature = parse_signature_hex(signature_hex)?;
    let public_key = parse_public_key_hex(root_pubkey_hex)?;
    let payload = keyset_payload_for_signature(json)?;
    let detached = DetachedEd25519Signature {
        key_id: 0,
        payload_len: payload.len() as u32,
        signature,
    };
    verify_detached_payload_ed25519(&payload, &detached, |_id| Some(public_key))
        .map_err(anyhow::Error::from)?;
    Ok(())
}

enum DetachedVerificationInput {
    Mac(DetachedAuthSignature),
    Ed25519(DetachedEd25519Signature),
}

fn load_detached_verification_input(
    path: Option<&PathBuf>,
) -> Result<Option<DetachedVerificationInput>> {
    let Some(path) = path else {
        return Ok(None);
    };
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read detached signature file {}", path.display()))?;
    let json: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("invalid JSON in detached signature file {}", path.display()))?;
    if json.get("tag_hex").is_some() {
        let key_id = json
            .get("key_id")
            .and_then(serde_json::Value::as_u64)
            .context("detached signature missing numeric key_id")? as u32;
        let payload_len =
            json.get("payload_len")
                .and_then(serde_json::Value::as_u64)
                .context("detached signature missing numeric payload_len")? as u32;
        let tag_hex = json
            .get("tag_hex")
            .and_then(serde_json::Value::as_str)
            .context("detached signature missing string tag_hex")?;
        if tag_hex.len() != 32 {
            bail!("detached signature tag_hex must be 32 hex chars");
        }
        let mut tag = [0u8; 16];
        for (idx, slot) in tag.iter_mut().enumerate() {
            let start = idx * 2;
            let end = start + 2;
            *slot = u8::from_str_radix(&tag_hex[start..end], 16)
                .with_context(|| format!("invalid tag hex at bytes {}..{}", start, end))?;
        }
        return Ok(Some(DetachedVerificationInput::Mac(
            DetachedAuthSignature {
                key_id,
                payload_len,
                tag,
            },
        )));
    }
    if json.get("signature_hex").is_some() {
        let signature = load_detached_ed25519_signature(path)?;
        return Ok(Some(DetachedVerificationInput::Ed25519(signature)));
    }
    bail!("detached signature must contain tag_hex or signature_hex");
}

fn load_detached_ed25519_signature(path: &PathBuf) -> Result<DetachedEd25519Signature> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("failed to read detached signature file {}", path.display()))?;
    let json: serde_json::Value = serde_json::from_str(&content)
        .with_context(|| format!("invalid JSON in detached signature file {}", path.display()))?;
    let key_id = json
        .get("key_id")
        .and_then(serde_json::Value::as_u64)
        .context("detached signature missing numeric key_id")? as u32;
    let payload_len = json
        .get("payload_len")
        .and_then(serde_json::Value::as_u64)
        .context("detached signature missing numeric payload_len")? as u32;
    let signature_hex = json
        .get("signature_hex")
        .and_then(serde_json::Value::as_str)
        .context("detached signature missing string signature_hex")?;
    if signature_hex.len() != 128 {
        bail!("detached signature signature_hex must be 128 hex chars");
    }
    let mut signature = [0u8; 64];
    for (idx, slot) in signature.iter_mut().enumerate() {
        let start = idx * 2;
        let end = start + 2;
        *slot = u8::from_str_radix(&signature_hex[start..end], 16)
            .with_context(|| format!("invalid signature hex at bytes {}..{}", start, end))?;
    }
    Ok(DetachedEd25519Signature {
        key_id,
        payload_len,
        signature,
    })
}

fn scan_burst(input_dir: PathBuf, mode: TransmissionMode) -> Result<()> {
    let mut scanner = Scanner::new(ScannerConfig {
        mode,
        ..ScannerConfig::default()
    });
    let mut entries = Vec::new();
    for entry in fs::read_dir(&input_dir)
        .with_context(|| format!("failed to read directory {}", input_dir.display()))?
    {
        let path = entry?.path();
        if !path.is_file() {
            continue;
        }
        let Some(ext) = path.extension().and_then(|e| e.to_str()) else {
            continue;
        };
        let ext = ext.to_ascii_lowercase();
        if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "webp" | "bmp") {
            entries.push(path);
        }
    }
    entries.sort();
    if entries.is_empty() {
        bail!("no image files found in {}", input_dir.display());
    }

    let mut events = Vec::with_capacity(entries.len());
    for (i, path) in entries.iter().enumerate() {
        let image = image::open(path)
            .with_context(|| format!("failed to open image {}", path.display()))?;
        let event = scanner
            .scan_frame(CameraFrame {
                image,
                timestamp_micros: i as u64,
            })
            .with_context(|| format!("failed to scan frame {}", path.display()))?;
        events.push(serde_json::json!({
            "file": path.file_name().and_then(|name| name.to_str()).unwrap_or_default(),
            "stream_id": event.frame.header.stream_id,
            "frame_index": event.frame.header.frame_index,
            "frame_count": event.frame.header.frame_count,
            "complete": event.complete_payload.is_some(),
            "burst_progress": {
                "frame_count": event.burst_progress.frame_count,
                "received_frames": event.burst_progress.received_frames,
                "missing_frames": event.burst_progress.missing_frames
            }
        }));
        if let Some(payload) = event.complete_payload {
            println!(
                "{}",
                serde_json::json!({
                    "ok": true,
                    "event_count": events.len(),
                    "events": events,
                    "payload_utf8_lossy": String::from_utf8_lossy(&payload),
                    "payload_len": payload.len()
                })
            );
            return Ok(());
        }
    }

    println!(
        "{}",
        serde_json::json!({
            "ok": false,
            "event_count": events.len(),
            "events": events,
            "error": "incomplete burst stream"
        })
    );
    Ok(())
}

fn decode_json(
    auto_decoded: &glyphnet_decode::AutoDecodedSymbol,
    crop: Option<serde_json::Value>,
    quad: Option<serde_json::Value>,
    warp: Option<serde_json::Value>,
) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "stream_id": auto_decoded.decoded.frame.header.stream_id,
        "frame_index": auto_decoded.decoded.frame.header.frame_index,
        "frame_count": auto_decoded.decoded.frame.header.frame_count,
        "mode": auto_decoded.decoded.frame.header.mode.to_string(),
        "ecc": auto_decoded.decoded.frame.header.ecc_level.to_string(),
        "payload_utf8_lossy": String::from_utf8_lossy(&auto_decoded.decoded.frame.payload),
        "payload_len": auto_decoded.decoded.frame.payload.len(),
        "auto": {
            "module_px": auto_decoded.info.module_px,
            "quiet_zone_modules": auto_decoded.info.quiet_zone_modules,
            "threshold": auto_decoded.info.threshold,
            "layout": layout_name(auto_decoded.info.layout)
        },
        "recovery": {
            "attempted": auto_decoded.decoded.recovery.attempted,
            "recovered": auto_decoded.decoded.recovery.recovered,
            "attempts": auto_decoded.decoded.recovery.attempts,
            "method": format!("{:?}", auto_decoded.decoded.recovery.method),
            "suspect_count": auto_decoded.decoded.recovery.suspect_count,
            "max_attempts_exceeded": auto_decoded.decoded.recovery.max_attempts_exceeded
        }
    });
    if let Some(crop) = crop {
        payload["crop"] = crop;
    }
    if let Some(quad) = quad {
        payload["quad"] = quad;
    }
    if let Some(warp) = warp {
        payload["warp"] = warp;
    }
    payload
}

fn inspect(
    payload: &[u8],
    profile: ProfileId,
    mode: Option<TransmissionMode>,
    ecc_level: Option<EccLevel>,
    geometry: Option<SymbolGeometry>,
) -> Result<()> {
    let encoded = encoder(profile, mode, ecc_level, geometry)
        .encode_static(payload)
        .context("failed to encode descriptor")?;
    println!("{}", serde_json::to_string_pretty(&encoded.descriptor)?);
    Ok(())
}

fn burst(
    payload: &[u8],
    output_dir: PathBuf,
    profile: ProfileId,
    frame_payload: usize,
    erasure_data_shards: Option<usize>,
    sizing: RenderSizing,
) -> Result<()> {
    fs::create_dir_all(&output_dir)
        .with_context(|| format!("failed to create {}", output_dir.display()))?;
    let mut config = EncoderConfig::for_profile(profile);
    config.mode = TransmissionMode::Burst;
    config.max_frame_payload = frame_payload;
    config.geometry = sizing.geometry;
    let encoder = Encoder::new(config);
    let default_shards = profile_spec(profile)
        .burst_data_shards
        .map(usize::from)
        .unwrap_or(12);
    let frames = encoder
        .encode_burst_erasure(payload, erasure_data_shards.unwrap_or(default_shards))
        .context("failed to encode burst")?;
    for frame in frames {
        let render_options = apply_fit(
            RenderOptions::for_descriptor(&frame.descriptor),
            frame.descriptor.width,
            frame.descriptor.height,
            sizing.fit,
        )?;
        let svg = SvgRenderer::new(render_options)
            .render(&frame.matrix)
            .context("failed to render burst frame")?;
        let path = output_dir.join(format!("frame_{:04}.svg", frame.descriptor.frame_index));
        fs::write(&path, svg).with_context(|| format!("failed to write {}", path.display()))?;
    }
    Ok(())
}

fn profiles() -> Result<()> {
    println!("{}", serde_json::to_string_pretty(profile_catalog())?);
    Ok(())
}

fn bench_plan() -> Result<()> {
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "profiles": profile_catalog(),
            "commands": [
                "cargo bench -p glyphnet-encode",
                "cargo test -p glyphnet-testkit",
                "cargo fuzz run frame_decode"
            ],
            "regression_policy": {
                "decode_success_rate": "profile-specific targets in profiles[].benchmark.decode_success_rate",
                "decode_budget_ms": "fail investigation when median scanner decode exceeds profiles[].benchmark.max_decode_ms",
                "throughput": "screen and burst profiles track bytes per second under frame-loss and blur suites"
            }
        }))?
    );
    Ok(())
}

fn auth_sign(payload: &[u8], output: PathBuf, auth_key_hex: &str, auth_key_id: u32) -> Result<()> {
    let key = parse_auth_key_hex(auth_key_hex)?;
    let sig = sign_detached_payload(payload, &key, auth_key_id);
    let tag_hex = sig
        .tag
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    fs::write(
        &output,
        serde_json::to_string_pretty(&serde_json::json!({
            "key_id": sig.key_id,
            "payload_len": sig.payload_len,
            "tag_hex": tag_hex
        }))?,
    )
    .with_context(|| format!("failed to write detached signature to {}", output.display()))?;
    Ok(())
}

fn auth_sign_ed25519(
    payload: &[u8],
    output: PathBuf,
    signing_key_hex: &str,
    key_id: u32,
) -> Result<()> {
    let key = parse_auth_key_hex(signing_key_hex)?;
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&key);
    let sig = sign_detached_payload_ed25519(payload, &signing_key, key_id);
    let signature_hex = sig
        .signature
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    fs::write(
        &output,
        serde_json::to_string_pretty(&serde_json::json!({
            "key_id": sig.key_id,
            "payload_len": sig.payload_len,
            "signature_hex": signature_hex
        }))?,
    )
    .with_context(|| format!("failed to write detached signature to {}", output.display()))?;
    Ok(())
}

fn auth_verify_ed25519(
    payload: &[u8],
    signature_path: PathBuf,
    public_key_hex: &str,
    key_id: u32,
) -> Result<()> {
    let signature = load_detached_ed25519_signature(&signature_path)?;
    let pubkey = parse_public_key_hex(public_key_hex)?;
    let mut result = serde_json::json!({
        "verified": false,
        "key_id": signature.key_id,
        "error": serde_json::Value::Null
    });
    match verify_detached_payload_ed25519(payload, &signature, |id| {
        if id == key_id { Some(pubkey) } else { None }
    }) {
        Ok(()) => {
            result["verified"] = serde_json::json!(true);
            result["error"] = serde_json::Value::Null;
        }
        Err(error) => {
            result["verified"] = serde_json::json!(false);
            result["error"] = serde_json::json!(error.to_string());
        }
    }
    println!("{result}");
    Ok(())
}

fn keyset_inspect(path: PathBuf) -> Result<()> {
    let json = load_keyset_json(&path)?;
    validate_keyset_json(&json)?;
    let issuer = json
        .get("issuer")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let created_at = json
        .get("created_at")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let expires_at = json
        .get("expires_at")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let keys = json
        .get("keys")
        .and_then(serde_json::Value::as_array)
        .map(std::vec::Vec::len)
        .unwrap_or(0);
    println!(
        "{}",
        serde_json::json!({
            "ok": true,
            "version": 1,
            "issuer": issuer,
            "created_at": created_at,
            "expires_at": expires_at,
            "key_count": keys
        })
    );
    Ok(())
}

fn keyset_validate(path: PathBuf, root_pubkey_hex: Option<&str>) -> Result<()> {
    let json = load_keyset_json(&path)?;
    validate_keyset_json(&json)?;
    let now = OffsetDateTime::now_utc();
    let created = OffsetDateTime::parse(
        json.get("created_at")
            .and_then(serde_json::Value::as_str)
            .context("keyset missing string created_at")?,
        &Rfc3339,
    )
    .context("keyset created_at must be RFC3339 UTC")?;
    let expires = OffsetDateTime::parse(
        json.get("expires_at")
            .and_then(serde_json::Value::as_str)
            .context("keyset missing string expires_at")?,
        &Rfc3339,
    )
    .context("keyset expires_at must be RFC3339 UTC")?;
    if now < created {
        bail!("keyset not yet valid (created_at is in the future)");
    }
    if now > expires {
        bail!("keyset expired");
    }
    if let Some(root_pubkey_hex) = root_pubkey_hex {
        verify_keyset_signature(&json, root_pubkey_hex)?;
    }
    println!("{}", serde_json::json!({ "ok": true, "valid": true }));
    Ok(())
}

fn keyset_sign_ed25519(
    input: PathBuf,
    output: PathBuf,
    signing_key_hex: &str,
    signed_by: &str,
) -> Result<()> {
    let mut json = load_keyset_json(&input)?;
    validate_keyset_json(&json)?;
    let payload = keyset_payload_for_signature(&json)?;
    let signing_key = ed25519_dalek::SigningKey::from_bytes(&parse_auth_key_hex(signing_key_hex)?);
    let signature = sign_detached_payload_ed25519(&payload, &signing_key, 0);
    let signature_hex = signature
        .signature
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    json["signature"] = serde_json::json!({
        "signature_alg": "ed25519",
        "signed_by": signed_by,
        "signature_hex": signature_hex
    });
    fs::write(&output, serde_json::to_string_pretty(&json)?)
        .with_context(|| format!("failed to write signed keyset {}", output.display()))?;
    Ok(())
}

fn keyset_verify(path: PathBuf, root_pubkey_hex: &str) -> Result<()> {
    let json = load_keyset_json(&path)?;
    validate_keyset_json(&json)?;
    verify_keyset_signature(&json, root_pubkey_hex)?;
    println!("{}", serde_json::json!({ "ok": true, "verified": true }));
    Ok(())
}
