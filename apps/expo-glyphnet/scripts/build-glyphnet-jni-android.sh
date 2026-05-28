#!/usr/bin/env bash
set -euo pipefail

if ! command -v cargo >/dev/null 2>&1; then
  echo "WARNING: cargo not found. Skipping GlyphNet JNI build."
  exit 0
fi

if ! command -v cargo-ndk >/dev/null 2>&1; then
  echo "WARNING: cargo-ndk not found. Skipping GlyphNet JNI build."
  exit 0
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
WORKSPACE_ROOT="$(cd "${SCRIPT_DIR}/../../.." && pwd)"
OUT_ROOT="${WORKSPACE_ROOT}/apps/expo-glyphnet/modules/glyphnet-scanner/android/src/main/jniLibs"

build_one() {
  local triple="$1"
  local abi="$2"
  echo "Building glyphnet-jni for ${triple}..."
  (cd "${WORKSPACE_ROOT}" && cargo ndk -t "${triple}" build --release -p glyphnet-jni)
  local src="${WORKSPACE_ROOT}/target/${triple}/release/libglyphnet_jni.so"
  if [[ ! -f "${src}" ]]; then
    echo "Expected output not found: ${src}" >&2
    exit 1
  fi
  mkdir -p "${OUT_ROOT}/${abi}"
  cp "${src}" "${OUT_ROOT}/${abi}/libglyphnet_scanner_bridge.so"
}

build_one "aarch64-linux-android" "arm64-v8a"
build_one "armv7-linux-androideabi" "armeabi-v7a"
build_one "x86_64-linux-android" "x86_64"

echo "JNI libraries copied to ${OUT_ROOT}"
