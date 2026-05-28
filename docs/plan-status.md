# Plan Status

This file tracks execution status for the roadmap priorities in
[`docs/roadmap.md`](roadmap.md).

## Current Priorities

- [x] Align public strategy/docs with reliability-first baseline and execution order.
- [x] Fix repository metadata and harden CI/release policy defaults.
- [x] Modularize monolithic crates, starting with `glyphnet-scanner` (phase 1).
- [ ] Keep scanner performance gate enforceable while converging to profile budgets.
- [x] Publish a versioned fixture corpus (synthetic + real + hard negatives)
  scaffold.
- [x] Complete Phase 2 ECC: LDPC screen profile and scanner-facing telemetry contract.
- [ ] Lock matrix as scanner reliability baseline in CI (fixtures + perf/reliability rows).
- [x] Deliver burst erasure transport (fountain/RaptorQ-like direction) baseline.
- [x] Add payload authenticity envelope above transport CRC.

## In Progress

- `feat/testkit-corpus-integrity-check`: add default fixture-manifest loading
  and integrity checks for corpus file paths.
- `feat/phase2-ecc-telemetry`: complete Phase 2 baseline with scanner-facing
  ECC telemetry contract, feature-gated screen LDPC path, and regression/bench
  coverage.
- `feat/matrix-baseline-lock-phase1`: add matrix reliability tests, matrix
  candidate-priority ordering, and matrix visibility in scanner perf CI
  reporting. Matrix full-canvas benchmark remains non-gating until
  `feat/matrix-fastpath-phase1` lands ROI/multi-scale acceleration.
- `feat/phase4-burst-transport-phase1`: add burst packet wire format, RS shard
  erasure encode/recover baseline, stateful scanner burst progress reporting,
  `scan-burst` tooling, and non-gating burst loss-sweep CI comment table.
- `feat/phase45-auth-envelope-cli`: add embedded authenticity envelope in core,
  authenticated encode/decode APIs, detached signature primitives, CLI keyring
  verify + sidecar signing flow, wasm authenticated encode/verify bridges, and
  wasm detached sign/verify exports.

## Next Up

1. Keep scanner performance gate enforceable while converging to profile
   budgets.
2. Lock matrix as scanner reliability baseline in CI:
   add matrix-first scan path checks, real matrix fixtures, and matrix-specific
   perf/reliability gate rows.
3. Implement `feat/matrix-fastpath-phase1`:
   downscaled/ROI-first matrix detection so a realistic matrix benchmark can be
   promoted to gating.
4. Improve burst high-loss reliability (30-40%) and promote burst CI thresholds
   from non-gating baseline tracking to gated targets once stable.
5. Add public-key detached signature mode (Ed25519) and versioned key-discovery
   format for SDK integrations beyond shared-secret MAC keys.
