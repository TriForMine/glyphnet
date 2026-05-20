# GlyphNet Mobile SDK Scaffold

This directory is reserved for platform wrappers around the Rust implementation.

Planned structure:

- `android/`: Kotlin CameraX wrapper and Rust library build scripts.
- `ios/`: Swift Package wrapper around a static Rust library.
- `shared/`: UniFFI interface definitions if selected.

The SDK should keep platform code thin: camera acquisition, permissions, preview
UI, and lifecycle management live in Swift/Kotlin; protocol, ECC, rendering,
decoding, and scanner state stay in Rust.
