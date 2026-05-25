# GlyphNet

GlyphNet is a next-generation visual data transmission system designed as a
modern, open alternative to QR codes and linear barcodes. The default symbol is
not a square grid, dot code, or QR-like matrix: it uses a wide ribbon-weave
optical layout with continuous strokes, side totems, chevron rails, and phase
traces. The protocol also ships color screen and animated burst profiles, plus
reserved constellation, hexagonal, and radial families for future engines. It is
built as a Rust workspace with a specification-first protocol, deterministic
reference implementation, test tooling, fuzz targets, SDK scaffolds, and
production-style CI. It is not a QR superset and is not intended to be decoded
by QR scanners.

The repository is intentionally practical: the current implementation provides
an end-to-end static encode-render-decode path, burst frame construction,
scanner orchestration, parity-based reference ECC, and protocol validation
hooks. The architecture leaves clear extension points for LDPC, fountain codes,
RaptorQ-like erasure recovery, color modulation, GPU sampling, radial and
hexagonal layouts, AR/VR scanners, and adaptive bitrate burst transfers.

## Status

This is an early reference implementation. The APIs and wire format are
versioned, tested, and deterministic, but the advanced CV and ECC engines are
still scaffolded behind stable crate boundaries. Treat `docs/protocol` as the
source of truth for compatibility.

## Goals

- Distinct visual identity: no QR-style square corner finders, dot fields, or
  square module mosaic by default.
- Static visual code generation for print and screen.
- Animated optical burst transmission for larger payloads.
- Binary-safe payload encoding with deterministic framing.
- Strong error detection now, stronger correction engines over time.
- Offline operation with no service dependency.
- Browser, mobile, desktop, CLI, and embedded-friendly Rust APIs.
- High-confidence engineering workflow: tests, property tests, fuzzing,
  benchmarks, formatting, linting, coverage, audit, and release automation.

## Visual Identity

GlyphNet's default `RibbonWeave` profile is designed to be visually distinct:

- payload bits render as continuous horizontal ribbon strokes;
- side totems replace QR corner boxes and barcode guard bars;
- chevron rails provide a recognizable optical signature;
- phase traces provide synchronization without row/column finder lines;
- the rendered artifact looks closer to a woven signal strip than a 2D barcode.

The square `Matrix` layout exists only as a compatibility and benchmarking
baseline. User-facing tools default to `RibbonWeave`.

## Protocol Profiles

GlyphNet has named profiles instead of one universal square symbol:

| Profile | Layout | Use case | Visual identity |
| --- | --- | --- | --- |
| `ribbon-print` | `RibbonWeave` | paper, stickers, cards, packaging | monochrome woven ribbon strip with side totems |
| `spectral-screen` | `SpectralMesh` | phone/webcam scanning from displays | interleaved dark blue, teal, and violet lanes |
| `pulse-burst` | `PulseStream` | animated high-speed optical transfer | temporal color lanes and wide pulse strips |
| `constellation-print` | `Constellation` | experimental robust print | off-corner halo anchors |
| `matrix-compat` | `Matrix` | benchmarks only | square baseline for comparison |

The first three are the main product shapes. `matrix-compat` is deliberately a
baseline so benchmark reports can say how much density, robustness, or visual
distinctiveness is gained over a conventional matrix approach.

## Workspace

| Crate | Responsibility |
| --- | --- |
| `glyphnet-core` | Protocol types, frame wire format, layout rules, matrix storage. |
| `glyphnet-ecc` | ECC traits, parity reference code, shard recovery, interleaving. |
| `glyphnet-encode` | Static and burst encoders. |
| `glyphnet-render` | Raster and SVG rendering. |
| `glyphnet-decode` | Layout-aware matrix and raster decoding. |
| `glyphnet-cv` | CV primitives for thresholding, anchor candidates, geometry. |
| `glyphnet-scanner` | Real-time frame source, scanner, and burst assembly orchestration. |
| `glyphnet-wasm` | WebAssembly entry points and browser-safe wrappers. |
| `glyphnet-cli` | `glyphnet` command-line tool. |
| `glyphnet-testkit` | Fixtures, degradation helpers, and property-test utilities. |

## Protocol Modes

### Print Mode

Print mode prioritizes robustness over density. The default print geometry is a
wide ribbon-weave strip suitable for cards, labels, posters, and packaging. It
uses monochrome or limited palette output, side totems, chevron rails, generous
quiet zones, high ECC overhead, and sampling profiles tuned for blur, lighting
changes, print dot gain, paper texture, perspective distortion, and low-cost
cameras.

### Screen Mode

Screen mode increases density for emissive displays. Its default geometry is a
16:9-style frame that better matches phones, monitors, kiosks, and video
surfaces. It is designed for smaller modules, color-capable renderers,
calibration frames, higher-resolution sampling, and optional micro-patterns that
can be recovered by smartphones and webcams.

### Burst Mode

Burst mode transmits animated frame sequences. Its default geometry is a
wide temporal strip optimized for video display and camera tracking. It adds
frame indexes, stream identifiers, temporal synchronization, burst assembly, and
future hooks for adaptive bitrate, temporal modulation, optical-flow tracking, and
fountain/RaptorQ-like recovery under dropped frames.

## Quick Start

```powershell
cargo test --workspace
cargo run -p glyphnet-cli -- profiles
cargo run -p glyphnet-cli -- encode --data "hello" --output hello.png
cargo run -p glyphnet-cli -- encode --data "hello" --output hello-fit.png --fit-width-px 1200 --fit-height-px 400
cargo run -p glyphnet-cli -- encode --profile spectral-screen --data "hello" --output hello-screen.png
cargo run -p glyphnet-cli -- decode hello.png
cargo run -p glyphnet-cli -- decode --auto hello.png
cargo run -p glyphnet-cli -- scan --mode print hello.png
cargo run -p glyphnet-cli -- burst --profile pulse-burst --data "large payload" --output-dir burst_frames
```

`decode --auto` infers module size, quiet zone, layout family, and threshold from the image and reports them in the JSON output. `scan` attempts a coarse auto-crop and perspective rectification before decoding.

## Engineering Workflow

The CI setup is designed to keep the project maintainable:

- `cargo fmt --all --check`
- `cargo clippy --workspace --all-targets --all-features -- -D warnings`
- `cargo test --workspace --all-features`
- `cargo doc --workspace --all-features --no-deps`
- `cargo llvm-cov` coverage workflow
- `cargo audit` and `cargo deny`
- scheduled fuzzing with `cargo fuzz`
- Criterion benchmarks for performance regression tracking
- release workflow with packaging checks and ordered crate publishing

## Design Principles

- The spec owns compatibility; code follows the spec.
- Every protocol byte is deterministic and covered by tests.
- Core crates avoid camera, OS, UI, and network assumptions.
- Advanced engines plug in through narrow trait boundaries.
- Burst transport, payload format, ECC, rendering, and CV stay separately
  testable.
- Reference algorithms are simple enough to audit before optimized variants are
  introduced.

## Documentation

- [Protocol specification](docs/protocol/glyphnet-v0.md)
- [Profile catalog](docs/profiles.md)
- [Benchmark plan](docs/benchmarks.md)
- [Architecture](docs/architecture.md)
- [Testing strategy](docs/testing.md)
- [Roadmap](docs/roadmap.md)
- [Browser SDK notes](docs/sdk/browser.md)
- [Mobile SDK notes](docs/sdk/mobile.md)
- [Scan debugger demo](demos/debug/README.md)

## License

GlyphNet is licensed under the Apache-2.0 license.
