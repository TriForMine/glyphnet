# Expo GlyphNet App (Android-first shell)

This app was generated with:

```bash
bun create expo apps/expo-glyphnet --template default@sdk-56 --yes
```

Current status:

- Expo Router + TypeScript base.
- Home screen replaced with GlyphNet scan/encode shell.
- Pluggable scanner adapter contract under `src/adapters/scanner`.
- Expo native module scaffold active under `modules/glyphnet-scanner`.
- Android JNI-ready bridge wrapper is in place (`GlyphNetNativeBridge`), with
  fallback responses until Rust `.so` is linked.

## Run

```bash
cd apps/expo-glyphnet
bun install
bun run start
```

## Next integration step

- Implement Rust JNI exports for `scanStillNative` and `encodeSvgNative` and
  package `libglyphnet_scanner_bridge.so` into Android build.
- Replace fallback responses with real Rust-backed scan/decode results.
