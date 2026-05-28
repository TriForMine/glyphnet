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

## Android JNI build

Native scanner bridge can be built on demand during Android prebuild/run:

```bash
# macOS/Linux
cd apps/expo-glyphnet
GLYPHNET_BUILD_JNI=1 npx expo run:android
```

```powershell
# Windows PowerShell
cd apps/expo-glyphnet
$env:GLYPHNET_BUILD_JNI = "1"
npx expo run:android
```

Notes:

- JNI build uses `cargo ndk` and writes `.so` files to
  `modules/glyphnet-scanner/android/src/main/jniLibs/*/libglyphnet_scanner_bridge.so`.
- EAS profiles default to `GLYPHNET_BUILD_JNI=0` to keep cloud builds stable.
- Set `GLYPHNET_BUILD_JNI=1` only in environments where Rust + cargo-ndk are present.
- `.easignore` excludes local Android build artifacts so EAS never uploads
  machine-specific generated autolinking paths.

## JNI CI artifacts

GitHub Actions workflow `android-jni.yml` builds Android JNI `.so` files from
`crates/glyphnet-jni` for:

- `arm64-v8a`
- `armeabi-v7a`
- `x86_64`

It uploads an artifact named `glyphnet-android-jni-libs` with the `jniLibs`
folder structure ready to copy into
`modules/glyphnet-scanner/android/src/main/jniLibs`.
