# Contributing

GlyphNet is specification-first. Protocol-facing changes should start with an
issue or design note, then a spec update, then tests, then implementation.

## Local Setup

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
```

## Development Rules

- Keep protocol compatibility changes in `docs/protocol`.
- Add regression tests for bug fixes.
- Prefer deterministic algorithms and explicit seeds.
- Keep platform-specific code out of core crates.
- Do not add unsafe code unless an ADR explains why it is necessary.
- Benchmark performance-sensitive changes.

## Pull Request Checklist

- Tests cover the behavior change.
- Fuzz targets still build when relevant.
- Public APIs have clear names and examples where needed.
- New dependencies are justified and compatible with the license policy.
- Documentation is updated for user-visible or protocol-visible changes.
