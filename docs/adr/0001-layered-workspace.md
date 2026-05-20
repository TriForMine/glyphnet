# ADR 0001: Layered Cargo Workspace

## Status

Accepted.

## Context

GlyphNet must support protocol research, production tooling, multiple platforms,
and future optimized engines without turning into a monolithic crate.

## Decision

Use a Cargo workspace with narrow crates for protocol core, ECC, encoding,
rendering, decoding, CV, scanner orchestration, WASM, CLI, and test utilities.

## Consequences

- Core protocol code stays small and auditable.
- Platform-specific code can depend inward without contaminating core crates.
- CI can test every layer independently.
- More crates require more release coordination, so workspace versioning is used
  until the protocol stabilizes.
