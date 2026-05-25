# Testing Strategy

GlyphNet uses specification-first and test-driven practices. Protocol behavior
should be changed by updating the spec, adding failing tests, implementing the
change, and then updating conformance fixtures.

## Test Layers

- Unit tests: frame parsing, layout invariants, ECC, rendering, CV helpers.
- Integration-style crate tests: encode-render-decode roundtrips.
- Property tests: arbitrary payload roundtrips and interleaving invariants.
- Fuzz tests: frame parser and rendered-matrix decode boundaries.
- Benchmarks: encode and render throughput for representative payloads.
- Snapshot fixtures: protocol bytes and descriptor JSON once v0 stabilizes.
- Scanner regressions: clean render, embedded screenshot-style images, and
  imported real screenshots that exercise totem/rail localization plus
  fractional-grid sampling.

## Required Local Checks

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo doc --workspace --all-features --no-deps
cargo run -p glyphnet-cli -- profiles
cargo run -p glyphnet-cli -- bench-plan
```

Optional:

```powershell
cargo install cargo-llvm-cov cargo-deny cargo-audit cargo-fuzz
cargo llvm-cov --workspace --all-features
cargo deny check
cargo audit
cargo fuzz run frame_decode
```

## Regression Policy

- Every bug fix should add a focused regression test.
- Protocol changes must include compatibility notes in `docs/protocol`.
- CV/scanner improvements should include synthetic degradation tests where
  practical and a real-image fixture when the fix came from an imported
  screenshot or camera frame.
- Performance-sensitive changes should update or add a Criterion benchmark.
- Profile-sensitive changes should update `docs/profiles.md` and
  `docs/benchmarks.md`.
- Fuzz crashes should be minimized into fixtures before closing the issue.

## Determinism

The reference implementation must produce stable bytes, matrix dimensions, and
module placement for the same config and payload. Any randomized strategy must
take an explicit seed and record it in the descriptor or test output.
