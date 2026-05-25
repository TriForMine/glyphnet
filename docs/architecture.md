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

The decoder has two paths:

1. `glyphnet-decode` decodes already-isolated rendered reference images. It can
   infer integer module size, quiet zone, layout family, and threshold.
2. `glyphnet-scanner` handles still images, screenshots, and camera-like inputs
   where the symbol may be embedded in UI chrome and modules may land on
   fractional pixels.

The still scanner follows the same broad structure used by modern QR scanners:
cheap signature localization first, then grid sampling and ECC validation.

Current still-image flow:

1. Convert to grayscale and adaptive threshold.
2. Try CV anchor/quad estimation when anchors are available.
3. Collect candidate regions from detector families:
   - `generated-content` for clean rendered symbols on simple backgrounds;
   - `generic-binary` for layout-agnostic dark bands and components;
   - `ribbon-weave` for RibbonWeave-specific signatures.
4. Attach detector family and layout hints to each candidate, then route it to
   the appropriate sampler/decoder.
5. Locate RibbonWeave candidates from layout-specific signatures:
   - dashed side totems;
   - horizontal chevron rails;
   - dark-bounds fallback for small/simple images.
6. Estimate a candidate symbol box from detected signature geometry.
7. Try exact integer-grid decoding for clean reference crops.
8. For screenshot/camera-style crops, run fractional-grid sampling:
   - try small phase offsets;
   - try small scale corrections;
   - run a frame-header precheck before full matrix decode;
   - accept only when full ECC/header validation passes.
9. Return the decoded payload plus crop/quad/attempt diagnostics.

Large still images avoid the old generic crop crawl after signature detection,
so failures stay bounded instead of scanning arbitrary UI regions.

The detector split is intentional. RibbonWeave is the most complete detector
today, but it is not the scanner architecture. Matrix-like, constellation, and
future camera-specific formats should be added as their own detector families
with their own geometry inference and sampling strategy, then share the same
candidate diagnostics and decode validation path.

## Performance Plan

- Keep core encoding allocation patterns visible and benchmarked.
- Use Criterion for encode/render/decode regression tracking.
- Add SIMD thresholding and fractional module sampling behind feature flags.
- Keep candidate generation signature-driven; avoid brute-force crop search.
- Add GPU sampling/rendering through optional crates, never in core.
- Use rayon only where deterministic outputs and measurable wins are proven.
- Preserve no-unsafe defaults; unsafe optimizations require isolated review.
