# Expo GlyphNet App (Android-first shell)

This app was generated with:

```bash
bun create expo apps/expo-glyphnet --template default@sdk-56 --yes
```

Current status:

- Expo Router + TypeScript base.
- Home screen replaced with GlyphNet scan/encode shell.
- Pluggable scanner adapter contract under `src/adapters/scanner`.
- Mock adapter active; Phase 5.2 will replace it with native Android bridge.

## Run

```bash
cd apps/expo-glyphnet
bun install
bun run start
```

## Next integration step

- Implement adapter backed by Android native module (Expo module/JNI into Rust)
  and keep `src/features/*` UI unchanged.
