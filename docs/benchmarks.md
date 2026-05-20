# Benchmark Plan

GlyphNet benchmarks are organized around protocol profiles, not isolated helper
functions. The target is to measure complete use cases: encode, render, degrade,
sample, decode, and assemble where applicable.

Run the current encode benchmark suite with:

```powershell
cargo bench -p glyphnet-encode
```

Print the benchmark policy and profile targets with:

```powershell
cargo run -p glyphnet-cli -- bench-plan
```

## Profile Targets

| Profile | Payload vector | Decode target | Decode budget | Throughput target |
| --- | ---: | ---: | ---: | ---: |
| `ribbon-print` | 256 B | 99.5% | 25 ms | static |
| `spectral-screen` | 1 KiB | 99.0% | 16 ms | 30 KB/s |
| `pulse-burst` | 64 KiB | 98.5% | 10 ms | 84 KB/s |
| `constellation-print` | 384 B | 99.7% | 30 ms | static |
| `matrix-compat` | 256 B | 99.5% | 20 ms | baseline |

These are engineering targets for the degradation suites, not claims that the
current reference scanner already reaches every target.

## Degradation Suites

- Print: blur, dot gain, perspective warp, low contrast, uneven illumination,
  partial scratches, and camera noise.
- Screen: moire, glare, white-balance shift, display gamma, subpixel blur,
  rolling exposure, and webcam compression.
- Burst: frame loss, duplicate frames, out-of-order frames, motion blur,
  rolling shutter, dropped color channels, and variable frame pacing.

## Regression Policy

- Profile encode benchmarks live in `crates/glyphnet-encode/benches`.
- Decode and CV benchmarks should use deterministic fixture seeds.
- Any new renderer primitive must include a fixture that decodes through the
  conservative grayscale path unless explicitly marked color-only.
- Throughput regressions over 10% need either a code fix or an explicit entry in
  the release notes explaining the quality tradeoff.
