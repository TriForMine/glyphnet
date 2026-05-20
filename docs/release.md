# Release Process

GlyphNet uses workspace versioning until the protocol stabilizes.

1. Update protocol docs and changelog entries.
2. Run local CI checks.
3. Run fuzz targets for parser-facing changes.
4. Tag `vX.Y.Z`.
5. Let GitHub Actions run tests and package checks.
6. Publish crates in dependency order. Do not use a workspace-wide publish
   dry-run before the internal dependency crates exist in the target registry;
   Cargo will verify dependent packages against the registry copy of internal
   crates.

Dependency order:

1. `glyphnet-core`
2. `glyphnet-ecc`
3. `glyphnet-encode`, `glyphnet-render`, `glyphnet-cv`
4. `glyphnet-decode`
5. `glyphnet-scanner`, `glyphnet-wasm`, `glyphnet-cli`, `glyphnet-testkit`
