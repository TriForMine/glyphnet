# AGENTS.md

## Mission

- Keep GlyphNet reliable-first: correctness, determinism, and reproducibility come before speed claims.
- Prefer incremental, reviewable changes over broad rewrites.

## Work Mode

- Start from current branch state; do not discard user changes.
- If unexpected unrelated modifications appear, stop and ask before proceeding.
- Keep edits minimal and local to the task.
- Favor native project conventions over introducing new patterns.
- When working under `apps/expo-glyphnet`, apply local instructions first:
  - `apps/expo-glyphnet/AGENTS.md`
  - relevant Expo skills in `apps/expo-glyphnet/.agents/skills/`

## Git Rules

- Never use destructive reset/checkout commands to drop work.
- Do not amend commits unless explicitly requested.
- Use non-interactive git commands only.
- Keep commit subjects <= 72 chars.
- One logical change per commit when feasible.

## Branching

- Continue on current branch unless asked to split.
- Rebase/pull from `main` before large phase work.
- Use feature branch names tied to roadmap phases.

## Code Style

- Default to ASCII.
- Add comments only where logic is non-obvious.
- Avoid duplicate logic; extract helpers/modules when a domain grows.
- Keep public APIs explicit and typed; avoid magic values.

## Rust Quality Gates

- Must pass:
  - `cargo fmt --all`
  - `cargo clippy --workspace --all-targets --all-features -- -D warnings`
  - relevant `cargo test` targets for changed crates
- Prefer fixing root causes over suppressing lints.

## Testing Policy

- Add/adjust tests for every behavior change.
- Prefer deterministic tests (fixed seeds or deterministic generators).
- Avoid flaky time-sensitive or randomness-heavy assertions.
- Keep slow stress tests ignored by default unless explicitly required in CI.

## Performance Policy

- Preserve scanner/perf gate operability.
- Keep benchmark scripts producing machine-readable artifacts even on failures.
- If thresholds change, update all mirrored locations (tests + scripts + docs).
- Mark non-gating benchmarks explicitly and document why.

## Scanner/CV Rules

- Do not rely on robust fallback paths as default behavior unless explicitly requested.
- Prefer early rejection and cheaper candidate filters before expensive decode attempts.
- Keep debug outputs meaningful and stage-specific when touching scanner diagnostics.

## Auth/Trust Rules

- Maintain compatibility across embedded and detached auth modes.
- Keep algorithm routing explicit (`mac-blake3` vs `ed25519`).
- Use versioned key material schemas when evolving trust formats.
- Validate key lifecycle fields (`created_at`, `expires_at`) consistently.

## CLI/WASM Parity

- When adding trust/auth features to CLI, mirror equivalent behavior in wasm where practical.
- Keep JSON contracts stable and documented.
- Support backward compatibility for legacy keyring formats when introducing versioned ones.

## Documentation Rules

- Update docs whenever behavior, contracts, or gates change:
  - `docs/plan-status.md`
  - `docs/sdk/browser.md`
  - roadmap/strategy docs if phase scope changes
- Reflect actual state (done/in-progress/next) with concrete wording.
- For Expo/mobile changes, follow versioned Expo guidance from:
  - `apps/expo-glyphnet/AGENTS.md` (Expo v56 docs requirement)
  - `apps/expo-glyphnet/.agents/skills/` references used by the touched area

## Expo Skills

- For `apps/expo-glyphnet`, use and cite the applicable local skills before edits:
  - `building-native-ui` for routes, tabs, styling, responsiveness, and interaction patterns
  - `expo-module` when creating/changing native bridges or Expo modules
  - `expo-dev-client` when native code requires a dev build path
  - `expo-deployment` / `eas-update-insights` when release/update workflow is touched
  - `expo-tailwind-setup` only if styling/tooling scope explicitly requires it

## CI/Release Rules

- Keep CI green without weakening quality checks silently.
- Any non-fatal gate behavior must still emit clear status artifacts.
- Prefer explicit config over hidden defaults in scripts/workflows.

## Security Rules

- Address vulnerable dependencies promptly.
- Prefer MSRV-compatible upgrades.
- If a security tool is missing locally, install it or document that limitation clearly.

## Communication

- Report what changed, what was validated, and what remains.
- Surface blockers quickly with concrete evidence.
- Provide commit message proposals for each completed logical step.
