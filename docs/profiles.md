# Profile Catalog

GlyphNet profiles are stable named bundles of mode, geometry, color, ECC, and
benchmark expectations. They are exposed by `glyphnet-core::profile_catalog()`
and by the CLI:

```powershell
cargo run -p glyphnet-cli -- profiles
```

## `ribbon-print`

- Layout: `RibbonWeave`
- Mode: print
- Color: monochrome
- ECC: high
- Use case: paper, stickers, cards, posters, packaging
- Identity: a wide woven strip with side totems, chevron rails, and a center
  phase trace

This is the default profile. It is deliberately not a QR-like square and should
remain legible after print blur, uneven lighting, perspective distortion, and
low-cost camera sampling.

## `spectral-screen`

- Layout: `SpectralMesh`
- Mode: screen
- Color: calibrated RGB
- ECC: medium
- Use case: display-to-camera transfer from phones, monitors, kiosks, and web
  views
- Identity: interleaved dark blue, teal, and violet ribbon lanes

The first reference renderer keeps all colors dark enough for conservative luma
decoding. Future screen decoders can use per-channel sampling, display gamma
calibration, and channel confidence scoring to increase density.

## `pulse-burst`

- Layout: `PulseStream`
- Mode: burst
- Color: adaptive
- ECC: adaptive
- Use case: animated transfer of larger payloads
- Identity: wide temporal lanes with frame-indexed payload slices

This profile is the home for temporal preambles, adaptive bitrate, fountain
packets, RaptorQ-like recovery, optical-flow tracking, and rolling-shutter
mitigation.

## `constellation-print`

- Layout: `Constellation`
- Mode: print
- Color: limited palette
- ECC: high
- Use case: experimental robust print where off-corner anchors are useful
- Identity: halo anchors and diagonal timing spines

This profile explores another non-QR visual family without changing the binary
frame format.

## `matrix-compat`

- Layout: `Matrix`
- Mode: print
- Color: monochrome
- ECC: high
- Use case: compatibility fixtures and comparative benchmarks
- Identity: square baseline

This profile should not be the public default. It exists so benchmark reports
can compare GlyphNet profiles against a familiar dense matrix baseline.
