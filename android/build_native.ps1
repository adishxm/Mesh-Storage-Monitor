# PowerShell build script for Android native shared libraries using cargo-ndk
$ErrorActionPreference = "Stop"

Write-Host "==> Building Mesh Node Android JNI Native Libraries" -ForegroundColor Cyan

# Check cargo-ndk
if (-not (Get-Command "cargo-ndk" -ErrorAction SilentlyContinue)) {
    Write-Host "cargo-ndk could not be found. Installing via cargo..." -ForegroundColor Yellow
    cargo install cargo-ndk
}

# Ensure Rust Android targets are installed
Write-Host "==> Installing Rust Android targets..." -ForegroundColor Cyan
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = Split-Path -Parent $ScriptDir
$JniLibsDir = Join-Path $ScriptDir "app\src\main\jniLibs"

if (-not (Test-Path $JniLibsDir)) {
    New-Item -ItemType Directory -Path $JniLibsDir -Force | Out-Null
}

Write-Host "==> Compiling android-bridge via cargo-ndk..." -ForegroundColor Cyan
Push-Location (Join-Path $RepoRoot "android-bridge")
try {
    cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -o $JniLibsDir build --release
    Write-Host "==> Native libraries built successfully into $JniLibsDir" -ForegroundColor Green
} finally {
    Pop-Location
}
