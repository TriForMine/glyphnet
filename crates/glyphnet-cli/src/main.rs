use std::{fs, path::PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use glyphnet_core::{EccLevel, ProfileId, TransmissionMode, profile_catalog};
use glyphnet_decode::RasterDecoder;
use glyphnet_encode::{Encoder, EncoderConfig};
use glyphnet_render::{RasterRenderer, RenderOptions, SvgRenderer};

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
    },
    /// Decode a rendered PNG/JPEG image produced by the reference renderer.
    Decode {
        /// Input image path.
        input: PathBuf,
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
        } => encode(
            data.as_bytes(),
            output,
            profile.into(),
            mode.map(Into::into),
            ecc.map(Into::into),
            format,
        ),
        Command::Decode { input } => decode(input),
        Command::Inspect {
            data,
            profile,
            mode,
            ecc,
        } => inspect(
            data.as_bytes(),
            profile.into(),
            mode.map(Into::into),
            ecc.map(Into::into),
        ),
        Command::Burst {
            data,
            output_dir,
            profile,
            frame_payload,
        } => burst(data.as_bytes(), output_dir, profile.into(), frame_payload),
        Command::Profiles => profiles(),
        Command::BenchPlan => bench_plan(),
    }
}

fn encoder(
    profile: ProfileId,
    mode_override: Option<TransmissionMode>,
    ecc_override: Option<EccLevel>,
) -> Encoder {
    let mut config = EncoderConfig::for_profile(profile);
    if let Some(mode) = mode_override {
        config.mode = mode;
    }
    if let Some(ecc_level) = ecc_override {
        config.ecc_level = ecc_level;
    }
    Encoder::new(config)
}

fn encode(
    payload: &[u8],
    output: PathBuf,
    profile: ProfileId,
    mode: Option<TransmissionMode>,
    ecc_level: Option<EccLevel>,
    format: FormatArg,
) -> Result<()> {
    let encoded = encoder(profile, mode, ecc_level)
        .encode_static(payload)
        .context("failed to encode payload")?;
    let render_options = RenderOptions::for_descriptor(&encoded.descriptor);
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

fn decode(input: PathBuf) -> Result<()> {
    let image =
        image::open(&input).with_context(|| format!("failed to open image {}", input.display()))?;
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
    Ok(())
}

fn inspect(
    payload: &[u8],
    profile: ProfileId,
    mode: Option<TransmissionMode>,
    ecc_level: Option<EccLevel>,
) -> Result<()> {
    let encoded = encoder(profile, mode, ecc_level)
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
) -> Result<()> {
    fs::create_dir_all(&output_dir)
        .with_context(|| format!("failed to create {}", output_dir.display()))?;
    let mut config = EncoderConfig::for_profile(profile);
    config.mode = TransmissionMode::Burst;
    config.max_frame_payload = frame_payload;
    let encoder = Encoder::new(config);
    let frames = encoder
        .encode_burst(payload)
        .context("failed to encode burst")?;
    for frame in frames {
        let svg = SvgRenderer::new(RenderOptions::for_descriptor(&frame.descriptor))
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
