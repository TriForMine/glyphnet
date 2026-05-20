# GlyphNet Protocol v0.1

This document defines the current GlyphNet reference protocol. The Rust
implementation in `glyphnet-core` is expected to match this document exactly for
wire compatibility.

## Layer Model

GlyphNet is organized as independent layers:

1. Payload layer: application bytes, content typing, and future compression.
2. Transport layer: frame headers, stream identifiers, sequencing, CRC checks.
3. ECC layer: parity and erasure recovery, later LDPC/fountain/RaptorQ profiles.
4. Symbol layer: data bit placement, function modules, layout family.
5. Rendering layer: print/screen/burst visual profiles and color mapping.
6. Tracking layer: anchors, timing, alignment, calibration, and temporal sync.
7. Scanner layer: camera frames, thresholding, rectification, sampling, assembly.

## Binary Frame

All integer fields are big-endian.

| Offset | Size | Name | Description |
| --- | ---: | --- | --- |
| 0 | 4 | magic | ASCII `GLYN`. |
| 4 | 1 | wire_version | Current value: `1`. |
| 5 | 1 | mode | `0=print`, `1=screen`, `2=burst`. |
| 6 | 1 | ecc_level | `0=low`, `1=medium`, `2=high`, `3=adaptive`. |
| 7 | 1 | flags | Reserved, currently `0`. |
| 8 | 2 | frame_index | Zero-based frame index. |
| 10 | 2 | frame_count | Total frames in this stream. |
| 12 | 8 | stream_id | BLAKE3-derived stream identifier. |
| 20 | 4 | payload_len | Payload byte length before ECC bytes. |
| 24 | 4 | payload_crc | CRC-32 of payload bytes. |
| 28 | 4 | header_crc | CRC-32 of bytes `0..28`. |

The header is 32 bytes. Decoders must ignore trailing parity and padding bytes
after `32 + payload_len`, but they should validate known ECC profiles when
available.

## Geometry Families

GlyphNet symbols are not required to be square and do not use QR-style corner
finder boxes or dot-code fields. The primary print layout is `RibbonWeave`, a
wide optical strip with continuous ribbon strokes and mode-specific aspect
ratios:

- Print: 96x36 sampling cells minimum, growing around an 8:3 target.
- Screen: 128x36 sampling cells minimum, growing around a 32:9 target.
- Burst: 160x28 sampling cells minimum, growing around a 40:7 target.

The compatibility `Matrix` family remains available for square test vectors and
interoperability experiments, but it is not the default.

The current ribbon-family placement rules are shared by `RibbonWeave`,
`SpectralMesh`, and `PulseStream`:

- Two side totems with asymmetric vertical rhythm.
- Top and bottom chevron rails. These are reserved visual magic patterns, not
  payload data, and are intended to prevent confusing GlyphNet symbols with QR,
  Data Matrix, or DotCode-like symbols.
- A center phase trace used for synchronization and motion compensation.
- Payload placement over non-reserved sampling cells, rendered by default as
  continuous ribbon runs rather than discrete modules.
- Row-major data bit placement across all non-function modules.
- Most-significant-bit-first byte packing.

Default renderers should draw `RibbonWeave` payload as continuous rounded
strokes and signal cells as chevrons/totems. A conforming decoder must sample the
logical grid, not assume all dark regions are square modules.

Current and reserved layout families:

- RibbonWeave: current default implementation.
- SpectralMesh: color-calibrated screen profile with interleaved color lanes.
- PulseStream: animated burst profile with temporal color lanes.
- Constellation: off-corner halo-anchor layout retained as an experimental
  research family.
- FrameGrid: rectangular compatibility family for earlier v0 experiments.
- Matrix: square-compatible baseline implementation.
- Hexagonal: dense screen profile with better sampling isotropy.
- Radial: lens-distortion tolerant rings for posters, domes, and AR markers.

## Named Profiles

Profile identifiers bind mode, layout, color modulation, ECC level, frame sizing,
and benchmark targets. Encoders should prefer profiles over ad hoc option sets.

| Profile | Mode | Layout | Color | ECC | Frame payload |
| --- | --- | --- | --- | --- | ---: |
| `ribbon-print` | print | `RibbonWeave` | mono | high | 512 |
| `spectral-screen` | screen | `SpectralMesh` | RGB | medium | 1024 |
| `pulse-burst` | burst | `PulseStream` | adaptive | adaptive | 1400 |
| `constellation-print` | print | `Constellation` | limited palette | high | 512 |
| `matrix-compat` | print | `Matrix` | mono | high | 512 |

`spectral-screen` and `pulse-burst` are intentionally visually different from
the default print symbol. They use dark calibrated colors so grayscale decoders
can still sample a conservative binary path while future color decoders recover
additional channel confidence and calibration metadata.

## ECC Profiles

The v0.1 implementation uses a deterministic parity reference code:

- Low: roughly 1 parity byte per 16 data bytes.
- Medium: roughly 1 per 8.
- High: roughly 1 per 4.
- Adaptive: roughly 1 per 3.

This is not the final high-density ECC target. The crate boundary is prepared
for:

- LDPC block codes for high-density screen mode.
- Fountain and RaptorQ-like recovery for burst mode.
- Interleaving to distribute print scratches and motion blur.
- Hybrid CRC plus erasure recovery for temporal streams.

## Mode Requirements

### Print

- Must decode in monochrome.
- Should maintain at least a 4-module quiet zone.
- Should prefer high ECC.
- Should tolerate blur, uneven illumination, print scaling, and perspective.

### Screen

- May use RGB or limited palettes.
- Should support calibration frames for camera/display response.
- Should allow smaller module sizes than print mode.
- Should expose density and readability telemetry.

### Burst

- Must include stable stream ID, frame index, and frame count.
- Should tolerate duplicate and out-of-order frames.
- Should support future fountain packets beyond fixed frame counts.
- Should provide adaptive bitrate and resend policies in higher layers.

## Compatibility Rules

- Unknown wire versions must be rejected.
- Unknown mode or ECC identifiers must be rejected.
- Header CRC failure must be rejected before payload parsing.
- Payload CRC failure must be rejected after payload extraction.
- Decoders may ignore trailing bytes only after successful frame validation.
- Feature negotiation must happen outside the fixed header until v1 reserves
  an extension block.
