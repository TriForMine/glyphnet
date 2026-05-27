# Plan Status

This file tracks execution status for the roadmap priorities in
[`docs/roadmap.md`](roadmap.md).

## Current Priorities

- [x] Align public strategy/docs with reliability-first baseline and execution order.
- [x] Fix repository metadata and harden CI/release policy defaults.
- [x] Modularize monolithic crates, starting with `glyphnet-scanner` (phase 1).
- [ ] Keep scanner performance gate enforceable while converging to profile budgets.
- [ ] Publish a versioned fixture corpus (synthetic + real + hard negatives).
- [ ] Complete Phase 2 ECC: LDPC screen profile and scanner-facing telemetry contract.
- [ ] Deliver burst erasure transport (fountain/RaptorQ-like direction).
- [ ] Add payload authenticity envelope above transport CRC.

## In Progress

- `feat/scanner-modularization-phase1`: completed scanner extraction
  (`types`, `detectors`, `rectification`, `candidates`) without behavior
  changes.

## Next Up

1. `glyphnet-decode` module split phase 1: sampling, header precheck, recovery
   routing.
2. Fixture corpus scaffolding under `crates/glyphnet-testkit`.
3. Scanner follow-up split: isolate decode/fractional paths from `lib.rs` (done
   in `decode_paths.rs`).
