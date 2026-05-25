# GlyphNet Strategy

GlyphNet is not trying to be "QR but custom." QR is the baseline to beat for
general scan reliability, speed, tooling, and ecosystem support. GlyphNet should
only diverge where it can offer a concrete advantage:

- non-square and layout-flexible symbols;
- better visual integration into products, posters, UI, packaging, and themed
  designs;
- higher screen-to-camera throughput through color, temporal frames, and burst
  recovery;
- profile-specific error correction tuned for print damage, screenshots,
  displays, and video;
- scanner diagnostics that expose why a symbol did or did not decode.

The project should support multiple layouts because one visual format will not
cover every use case well. The shared protocol, frame header, ECC layer,
renderer options, scanner diagnostics, and benchmark harness should be common;
layout detection and sampling should be layout-specific.

## Product Profiles

### Matrix Primary

Purpose: reliable static code for general scanning.

Shape: square or rectangular matrix, allowed to be non-square when the content
or surface benefits from it.

Use cases:

- URLs, short text, credentials, pairing codes, labels;
- camera scanning from print and screens;
- baseline comparison against QR/Data Matrix style scanners.

Why it matters: this should become the reliability-first profile. It can use
strong, obvious finder/timing patterns, dynamic geometry inference, and
conservative monochrome decoding. If GlyphNet cannot beat QR here, it should at
least be close enough to act as a compatibility and benchmark baseline.

### Ribbon Static

Purpose: wide, visually integrated codes.

Shape: horizontal ribbon, ticket strip, UI banner, product label edge, embedded
stroke ornament.

Use cases:

- screenshots and browser/UI embeds;
- packaging bands;
- wristbands, tickets, receipts;
- decorative horizontal marks where QR would look awkward.

Why it matters: this is where GlyphNet can be meaningfully different from QR.
The tradeoff is harder detection, especially in cluttered UI and perspective
views. It should remain a first-class profile, but not the only scanner target.

### Spectral Screen

Purpose: high-density static screen transfer.

Shape: usually rectangular, with calibrated RGB lanes or color confidence
signals.

Use cases:

- screen-to-phone transfer;
- device pairing;
- web app to mobile app handoff;
- short binary payloads larger than typical QR comfort zones.

Why it matters: screen capture gives control over brightness, color, and
animation timing. Color should improve confidence and density only when the
scanner can still fall back to a robust grayscale path.

### Pulse Burst

Purpose: large data transfer over animated/video optical frames.

Shape: animated sequence using PulseStream or another burst layout.

Use cases:

- tens of KB to MB transfer without network;
- air-gapped transfer;
- TV/display to camera transfer;
- resilient transfer over dropped frames.

Why it matters: this is the strongest place to beat static QR. The goal is not
one huge static symbol; it is a stream with temporal sync, frame IDs, fountain
packets, interleaving, and adaptive bitrate.

### Decorative / Magic Circle

Purpose: visually expressive symbols that can still decode.

Shape: radial, circular, constellation, ornamental, themed.

Use cases:

- games, events, posters, collectibles;
- branded codes where aesthetics matter more than maximum speed;
- AR markers and large physical installations.

Why it matters: this is a differentiator, but it must be honest about tradeoffs.
Decorative profiles should ship with lower capacity/speed targets and strong
diagnostics, not pretend to be the reliability default.

## Scanner Architecture

The scanner should be a modular pipeline:

```text
image/frame
  -> cheap preprocessing
  -> detector families produce candidates
  -> geometry inference per candidate
  -> sampler per layout/profile
  -> header precheck
  -> ECC/decode validation
  -> diagnostics and telemetry
```

Detector families should be explicit:

- generated/simple-background detector;
- matrix finder/timing detector;
- ribbon rail/totem detector;
- color/spectral detector;
- burst temporal detector;
- decorative/radial detector;
- generic fallback detector for debugging only.

Every candidate should report detector family, layout hint, crop, geometry
confidence, timings, and failure reason. This keeps the project extensible and
prevents RibbonWeave-specific heuristics from becoming the whole scanner.

## Benchmark Contract

Claims must be benchmarked against fixtures, not intuition.

Core metrics:

- detection time: time to find candidate regions;
- decode time: total time to validated payload;
- success rate: decoded payload matches expected bytes;
- false-positive rate: random/clutter fixtures must not decode;
- payload density: payload bytes per square centimeter or screen pixel area;
- damage tolerance: success by blur, noise, contrast, occlusion, perspective;
- burst throughput: useful payload bytes per second after recovery;
- user-facing latency: time to first valid decode in browser/mobile demos.

Minimum benchmark suites:

- clean generated images for every layout;
- screenshot/UI clutter fixtures;
- camera-like perspective and blur fixtures;
- print degradation fixtures;
- screen moire/glare/white-balance fixtures;
- burst video fixtures with frame drops and motion blur;
- negative fixtures with no GlyphNet symbol.

## Near-Term Plan

### Milestone 1: Honest Static Baseline

- Make `Matrix` a real scanner target, not only an encoder/render baseline.
- Add a layout selector to the playground generator.
- Add clean encode/render/scan tests for Matrix, RibbonWeave, SpectralMesh,
  PulseStream, Constellation, and Radial where supported.
- Add scanner diagnostics for detector family, layout hint, geometry, and
  confidence.
- Keep QR/Data Matrix comparison language conservative until benchmarks exist.

### Milestone 2: Scanner-First Matrix Format

- Design strong finder/timing/alignment patterns for Matrix/rectangular Matrix.
- Implement fast matrix detector before expensive generic crop attempts.
- Add dynamic module-size and quiet-zone inference.
- Benchmark against clean, screenshot, and degraded fixtures.
- Decide whether Matrix should become the default reliability profile.

### Milestone 3: RibbonWeave as Wide-Format Profile

- Remove hardcoded 96x36 assumptions from fractional sampling.
- Infer RibbonWeave geometry from rails/totems and frame header validation.
- Add perspective and low-resolution tests.
- Define where RibbonWeave is better than Matrix/QR: wide surfaces, UI embeds,
  decorative strips.

### Milestone 4: Real ECC Upgrade

- Replace the current parity reference with profile-specific ECC:
  Reed-Solomon/BCH for static print, LDPC or similar for dense screen, fountain
  recovery for burst.
- Add interleaving policies per profile.
- Expose erasure/confidence telemetry from the scanner.

### Milestone 5: Burst Transfer

- Define temporal sync/preamble.
- Implement fixed-frame burst first, then fountain packets.
- Build browser sender/receiver demos.
- Benchmark useful KB/s at different frame rates, camera resolutions, and frame
  loss rates.

### Milestone 6: Decorative Profiles

- Treat radial/constellation/magic-circle layouts as lower-speed profiles with
  explicit visual constraints.
- Add design-safe anchors that survive styling.
- Benchmark them separately from reliability-first Matrix and wide Ribbon.

## Known Limits

- QR has an enormous ecosystem and decades of optimization. GlyphNet should not
  claim to be generally better until independent benchmarks support it.
- More visual freedom usually reduces scan reliability unless the style system
  reserves strong anchors and timing marks.
- More data in one static symbol increases module density and hurts camera
  tolerance. Large payloads should usually use burst mode.
- Color can improve density on screens but can hurt print and low-quality camera
  capture. Every color profile needs a grayscale fallback or clear limitation.
- Decorative layouts are valuable, but they should not be the default for
  critical data transfer.

## Definition of Better Than QR

GlyphNet can claim improvement only per profile:

- Matrix Primary: equal or near-equal reliability with non-square flexibility.
- Ribbon Static: better visual integration and surface fit than QR at similar
  payload sizes.
- Spectral Screen: higher screen-to-camera density or speed than monochrome QR
  under controlled screen conditions.
- Pulse Burst: much higher payload transfer than any static QR workflow.
- Decorative: better aesthetics with documented capacity and reliability limits.

The project should make these claims measurable in CI and benchmark reports.
