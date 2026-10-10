#!/usr/bin/env bash
# Build script for Android native shared libraries using cargo-ndk
set -euo pipefail

echo "==> Building Mesh Node Android JNI Native Libraries"

# Check cargo-ndk
if ! command -v cargo-ndk &> /dev/null; then
    echo "cargo-ndk could not be found. Installing via cargo..."
    cargo install cargo-ndk
fi

# Ensure Rust Android targets are installed
echo "==> Installing Rust Android targets..."
rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
JNI_LIBS_DIR="$SCRIPT_DIR/app/src/main/jniLibs"

mkdir -p "$JNI_LIBS_DIR"

echo "==> Compiling android-bridge via cargo-ndk..."
cd "$REPO_ROOT/android-bridge"
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -o "$JNI_LIBS_DIR" build --release

echo "==> Native libraries built successfully in $JNI_LIBS_DIR"
ls -la "$JNI_LIBS_DIR"/*/*.so || true
