use std::{fs, path::PathBuf};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand, ValueEnum};
use glyphnet_core::{EccLevel, ProfileId, SymbolGeometry, TransmissionMode, profile_catalog};
use glyphnet_decode::RasterDecoder;
use glyphnet_encode::{Encoder, EncoderConfig};
use glyphnet_render::{RasterRenderer, RenderOptions, SvgRenderer};
use glyphnet_scanner::{CameraFrame, Scanner, ScannerConfig, scan_still};

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
    },
    /// Decode a rendered PNG/JPEG image produced by the reference renderer.
    Decode {
        /// Input image path.
        input: PathBuf,
        /// Infer module size and quiet zone automatically.
        #[arg(long)]
        auto: bool,
    },
    /// Scan an image, attempting coarse auto-crop before decoding.
    Scan {
        /// Input image path.
        input: PathBuf,
        /// Transmission mode used for CV tuning.
        #[arg(long, value_enum, default_value_t = ModeArg::Print)]
        mode: ModeArg,
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
        } => {
            let sizing = render_sizing(width_modules, height_modules, fit_width_px, fit_height_px)?;
            encode(
                data.as_bytes(),
                output,
                profile.into(),
                mode.map(Into::into),
                ecc.map(Into::into),
                format,
                sizing,
            )
        }
        Command::Decode { input, auto } => decode(input, auto),
        Command::Scan { input, mode } => scan(input, mode.into()),
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
                sizing,
            )
        }
        Command::Profiles => profiles(),
        Command::BenchPlan => bench_plan(),
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

fn encode(
    payload: &[u8],
    output: PathBuf,
    profile: ProfileId,
    mode: Option<TransmissionMode>,
    ecc_level: Option<EccLevel>,
    format: FormatArg,
    sizing: RenderSizing,
) -> Result<()> {
    let encoded = encoder(profile, mode, ecc_level, sizing.geometry)
        .encode_static(payload)
        .context("failed to encode payload")?;
    let render_options = apply_fit(
        RenderOptions::for_descriptor(&encoded.descriptor),
        encoded.descriptor.width,
        encoded.descriptor.height,
        sizing.fit,
    )?;
    match format {
        FormatArg::Png => {
            let image = RasterRenderer::new(render_options)
                .render(&encoded.matrix)
                .context("failed to render PNG")?;
            image.save(&output).with_context(|| {
                format!("failed to save rendered image to {}", output.display())
            })?;
        }
        FormatArg::Svg => {
            let svg = SvgRenderer::new(render_options)
                .render(&encoded.matrix)
                .context("failed to render SVG")?;
            fs::write(&output, svg)
                .with_context(|| format!("failed to write SVG to {}", output.display()))?;
        }
    }
    println!("{}", serde_json::to_string_pretty(&encoded.descriptor)?);
    Ok(())
}

fn decode(input: PathBuf, auto: bool) -> Result<()> {
    let image =
        image::open(&input).with_context(|| format!("failed to open image {}", input.display()))?;
    if auto {
        let auto_decoded = RasterDecoder::default()
            .decode_auto_with_info(&image)
            .context("failed to auto-decode GlyphNet image")?;
        println!("{}", decode_json(&auto_decoded, None, None, None));
    } else {
        let decoded = RasterDecoder::default()
            .decode(&image)
            .context("failed to decode GlyphNet image")?;
        println!(
            "{}",
            serde_json::json!({
                "stream_id": decoded.frame.header.stream_id,
                "frame_index": decoded.frame.header.frame_index,
                "frame_count": decoded.frame.header.frame_count,
                "mode": decoded.frame.header.mode.to_string(),
                "ecc": decoded.frame.header.ecc_level.to_string(),
                "payload_utf8_lossy": String::from_utf8_lossy(&decoded.frame.payload),
                "payload_len": decoded.frame.payload.len()
            })
        );
    }
    Ok(())
}

fn scan(input: PathBuf, mode: TransmissionMode) -> Result<()> {
    let image =
        image::open(&input).with_context(|| format!("failed to open image {}", input.display()))?;
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
    println!("{payload}");
    Ok(())
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
    sizing: RenderSizing,
) -> Result<()> {
    fs::create_dir_all(&output_dir)
        .with_context(|| format!("failed to create {}", output_dir.display()))?;
    let mut config = EncoderConfig::for_profile(profile);
    config.mode = TransmissionMode::Burst;
    config.max_frame_payload = frame_payload;
    config.geometry = sizing.geometry;
    let encoder = Encoder::new(config);
    let frames = encoder
        .encode_burst(payload)
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
