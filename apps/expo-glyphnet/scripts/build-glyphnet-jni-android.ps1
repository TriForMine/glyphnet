$ErrorActionPreference = "Stop"

function Require-Command($name) {
  if (-not (Get-Command $name -ErrorAction SilentlyContinue)) {
    return $false
  }
  return $true
}

if (-not (Require-Command "cargo")) {
  Write-Host "WARNING: cargo not found. Skipping GlyphNet JNI build."
  exit 0
}

$workspaceRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..\..")
$outRoot = Join-Path $workspaceRoot "apps\expo-glyphnet\modules\glyphnet-scanner\android\src\main\jniLibs"

if (-not (Get-Command "cargo-ndk" -ErrorAction SilentlyContinue)) {
  Write-Host "WARNING: cargo-ndk not found. Skipping GlyphNet JNI build."
  exit 0
}

$targets = @(
  @{ Triple = "aarch64-linux-android"; Abi = "arm64-v8a" },
  @{ Triple = "armv7-linux-androideabi"; Abi = "armeabi-v7a" },
  @{ Triple = "x86_64-linux-android"; Abi = "x86_64" }
)

foreach ($target in $targets) {
  Write-Host "Building glyphnet-jni for $($target.Triple)..."
  Push-Location $workspaceRoot
  try {
    cargo ndk -t $target.Triple build --release -p glyphnet-jni
  } finally {
    Pop-Location
  }

  $srcSo = Join-Path $workspaceRoot "target\$($target.Triple)\release\libglyphnet_jni.so"
  if (-not (Test-Path $srcSo)) {
    throw "Expected output not found: $srcSo"
  }

  $destDir = Join-Path $outRoot $target.Abi
  New-Item -ItemType Directory -Force -Path $destDir | Out-Null
  Copy-Item $srcSo (Join-Path $destDir "libglyphnet_scanner_bridge.so") -Force
}

Write-Host "JNI libraries copied to $outRoot"
