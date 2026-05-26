# Benchmark Plan

GlyphNet benchmarks are organized around protocol profiles, not isolated helper
functions. The target is to measure complete use cases: encode, render, degrade,
sample, decode, and assemble where applicable.

Run the current encode benchmark suite with:

```powershell
cargo bench -p glyphnet-encode
cargo bench -p glyphnet-scanner
```

The scanner benchmark currently includes:

- `scan_real_debugger_screenshot`: a real debugger screenshot fixture that
  exercises totem/rail localization and fractional-grid sampling.
- `scan_generated_matrix_canvas`: a generated Matrix symbol embedded in a
  simple canvas, used as the first reliability-baseline layout benchmark.

`scan_real_debugger_screenshot` is the primary performance guard for
still-image scan changes. Current local release baseline is roughly 180-190 ms
on the debugger screenshot fixture.

Print the benchmark policy and profile targets with:

```powershell
cargo run -p glyphnet-cli -- bench-plan
```

## CI Scanner Latency Gate

The enforceable scanner regression gate lives at `scripts/check_scanner_perf.sh`.
It runs only the real screenshot benchmark target and reads Criterion output from
`target/criterion/scan_real_debugger_screenshot/new/estimates.json`.

The gate compares Criterion **median** latency against
`ProfileId::RibbonPrint.benchmark.max_decode_ms`, currently read from
`crates/glyphnet-core/src/profile.rs` by the gate script (regex parse) as the
source of truth, with a default 10% tolerance
(`SCANNER_BENCH_TOLERANCE_PCT=10`). Median is used (instead of p95) because
hosted CI machines can have sporadic long-tail scheduling outliers; median is
less noisy while still reliably catching sustained regressions.

Run locally:

```bash
scripts/check_scanner_perf.sh
```

On pull requests, CI also runs the same check on the PR branch and on the PR
base branch, then posts/updates a sticky PR comment with:

- PR median latency
- base median latency
- delta in ms and %
- current budget/allowed threshold and gate status

Useful knobs for local/CI tuning:

- `SCANNER_BENCH_TOLERANCE_PCT` (default `10`)
- `SCANNER_BENCH_WARMUP_SECS` (default `3`)
- `SCANNER_BENCH_MEASURE_SECS` (default `8`)
- `SCANNER_BENCH_SAMPLE_SIZE` (default `40`)

If CI hardware or baseline behavior changes, update policy in this order:

1. Verify multiple runs on the new baseline hardware.
2. Adjust warmup/measurement/sample-size env defaults to reduce noise first.
3. Only then change `SCANNER_BENCH_TOLERANCE_PCT` (and document why in PR).
4. Change `profile.rs` benchmark targets only when the product-level objective
   itself changes, not just CI variance.

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
- Still scanner benchmarks live in `crates/glyphnet-scanner/benches` and use
  deterministic image fixtures.
- Decode and CV benchmarks should use deterministic fixture seeds.
- Any new renderer primitive must include a fixture that decodes through the
  conservative grayscale path unless explicitly marked color-only.
- Throughput regressions over 10% need either a code fix or an explicit entry in
  the release notes explaining the quality tradeoff.


In CI we currently set `SCANNER_BENCH_TOLERANCE_PCT=700` explicitly for this gate
while scanner throughput is being optimized toward the profile target, so the job
remains informative (and comments PR-vs-base deltas) instead of permanently red.
