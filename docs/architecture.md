# Architecture

GlyphNet is a protocol ecosystem, not a single renderer. Each crate owns one
layer with minimal dependencies so implementations can be reused across CLI,
desktop, mobile, browser, embedded, and industrial camera contexts.

## Data Flow

```text
payload bytes
  -> glyphnet-encode
  -> glyphnet-core Frame
  -> glyphnet-ecc parity/interleaving
  -> glyphnet-core SymbolGeometry + SymbolMatrix
  -> glyphnet-render raster/SVG/video frames
  -> camera/display/print medium
  -> glyphnet-cv threshold/locate/rectify/sample
  -> glyphnet-decode Frame
  -> glyphnet-scanner burst assembly
  -> application bytes
```

## Core Boundaries

- `glyphnet-core` must stay deterministic and free of camera/rendering concerns.
- `glyphnet-ecc` owns recovery algorithms behind `BlockCode` and shard APIs.
- `glyphnet-render` turns matrices into pixels or vectors, but does not invent
  protocol bytes.
- `glyphnet-cv` owns image analysis primitives, not payload parsing.
- `glyphnet-scanner` is orchestration state for real-time streams.

## Synchronization Strategy

Static symbols use spatial synchronization:

- side totems establish orientation without QR-like corner boxes;
- chevron rails estimate pitch, skew, and capture phase;
- the center phase trace estimates timing drift and motion;
- quiet zones isolate symbol boundaries.

Burst streams add temporal synchronization:

- stream ID groups frames;
- frame index and frame count restore ordering;
- duplicate frames are idempotent;
- future fountain packets will allow completion without every original frame;
- adaptive senders can change density and color profile based on scanner
  telemetry.

## Capability Negotiation

The initial code has a `CapabilitySet` descriptor. Long term, a sender can
advertise:

- mode support: print, screen, burst;
- color and calibration support;
- ECC families and required overhead;
- frame rate and resolution budget;
- GPU/SIMD acceleration;
- camera type and rolling-shutter constraints.

## Profile System

Profiles are the user-facing protocol products. `glyphnet-core` owns the static
catalog, `glyphnet-encode` converts a profile into `EncoderConfig`, and
`glyphnet-render` maps the resulting descriptor into visual primitives.

- `RibbonPrint` targets physical print with monochrome ribbon strokes.
- `SpectralScreen` targets displays with interleaved dark RGB lanes.
- `PulseBurst` targets animated transfer with adaptive color and larger frames.
- `ConstellationPrint` remains an experimental robust print family.
- `MatrixCompat` is a benchmark baseline, not the default experience.

This split lets benchmarks compare concrete use cases instead of comparing
unnamed flag combinations.

## Rendering Pipeline

Reference rendering is intentionally simple:

1. Choose mode and ECC.
2. Build a binary frame.
3. Append parity bytes.
4. Choose the smallest mode/layout geometry that fits.
5. Place bits in canonical order.
6. Render with explicit module size, quiet zone, and visual primitive profile.

Geometry (module width/height) is chosen by the encoder. Pixel size is chosen
by the renderer. If you need a symbol to fit within a specific pixel box while
keeping the geometry fixed, use `glyphnet-render::RenderOptions::fit_to_size`
which selects the largest integer module size that fits inside the target
bounds without distortion.

The default primitive profile is intentionally non-QR:

- continuous ribbon payload strokes;
- side totems;
- chevron rails and phase traces;
- no square finder boxes in the default path.

Future renderers can add:

- calibrated RGB palettes and profile-specific color correction;
- display gamma compensation;
- print dot-gain compensation;
- non-rectangular physical projection from the same logical bitstream;
- shape-coded module primitives for display/camera calibration;
- animated burst frame pacing;
- GPU texture generation;
- vector, PNG, video, and AR surface outputs.

## Decoding Pipeline

The current decoder supports rendered reference images. The scanner roadmap is:

1. Convert to grayscale or calibrated color channels.
2. Adaptive threshold and denoise.
3. Detect anchor candidates.
4. Estimate perspective transform.
5. (Optional) auto-infer module size and quiet zone from rendered images.
6. Sample modules using timing and alignment markers.
7. Decode matrix bits.
8. Validate header, payload CRC, and ECC.
9. Assemble burst streams and surface telemetry.

## Performance Plan

- Keep core encoding allocation patterns visible and benchmarked.
- Use Criterion for encode/render/decode regression tracking.
- Add SIMD thresholding and module sampling behind feature flags.
- Add GPU sampling/rendering through optional crates, never in core.
- Use rayon only where deterministic outputs and measurable wins are proven.
- Preserve no-unsafe defaults; unsafe optimizations require isolated review.
