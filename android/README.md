# Mesh Storage Android Node App

Android application and background foreground service for the decentralized Mesh Storage Monitor node, integrating the Rust `mesh-core` and `android-bridge` native libraries via JNI.

## Architecture

- **`io.meshstorage.node.MeshStorageService`**: Android Foreground Service with battery/charging (`BatteryManager`), network type (`ConnectivityManager.isActiveNetworkMetered`), and storage floor guards (minimum 10% host free space).
- **`io.meshstorage.node.MeshNodeBridge`**: JNI boundary declaring `nativeInitNode`, `nativeGetStatus`, `nativePauseNode`, `nativeResumeNode`, `nativeLeaveNode`, `nativeEvaluatePolicy`, and `nativeDestroyNode`.
- **`android-bridge` crate (`libmesh_android_bridge.so`)**: Compiles the Rust node lifecycle and policy envelope directly to shared objects for target ABIs.

## Supported ABIs

- `arm64-v8a` (Primary Android smartphones and tablets)
- `armeabi-v7a` (32-bit ARM legacy devices)
- `x86_64` (Android Studio Emulator and Intel Chromebooks)

## Prerequisites

1. **Android SDK & NDK**:
   - Android SDK 35 (compileSdk 35, minSdk 26).
   - Android NDK (r25c or newer). Set `ANDROID_NDK_HOME` to your NDK path.
2. **Rust & cargo-ndk**:
   ```bash
   cargo install cargo-ndk
   rustup target add aarch64-linux-android armv7-linux-androideabi x86_64-linux-android
   ```

## Native Compilation Pipeline

### 1. Build Native Libraries (`.so`)
Run the native build script from the repository root or `android/` directory:

```bash
# On Linux / macOS:
./android/build_native.sh

# On Windows PowerShell:
.\android\build_native.ps1
```

Or run `cargo-ndk` directly:
```bash
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -o android/app/src/main/jniLibs build --release -p mesh-android-bridge
```

This places the compiled `libmesh_android_bridge.so` files into:
```text
android/app/src/main/jniLibs/
├── arm64-v8a/libmesh_android_bridge.so
├── armeabi-v7a/libmesh_android_bridge.so
└── x86_64/libmesh_android_bridge.so
```

### 2. Gradle APK Build
With `jniLibs` populated, Gradle packages the native shared libraries into the output APK:

```bash
cd android
./gradlew assembleDebug
```

The resulting APK will be located at:
`android/app/build/outputs/apk/debug/app-debug.apk`

### 3. Verify APK Native Libraries
You can inspect that the native libraries are bundled inside the APK:
```bash
unzip -l app/build/outputs/apk/debug/app-debug.apk | grep "libmesh_android_bridge.so"
```

Expected output:
```text
lib/arm64-v8a/libmesh_android_bridge.so
lib/armeabi-v7a/libmesh_android_bridge.so
lib/x86_64/libmesh_android_bridge.so
```

## Running Instrumentation Tests

Connect a physical Android device (with USB debugging enabled) or start an Android Emulator (`x86_64`), then execute:

```bash
cd android
./gradlew connectedAndroidTest
```

The test runner will execute `MeshNodeBridgeTest` to verify:
1. Native policy evaluation and battery/WiFi constraints.
2. JNI initialization of node storage and handles.
3. Node pause, resume, and lifecycle teardown.

## Device Lifecycle & Reliability Guards

- **Foreground Notification**: Ensures Android does not kill the process in background storage operations.
- **Process Death & Restart**: Node state and Ed25519 node identity keypairs are persisted in the app's private data storage directory (`context.filesDir/mesh_identity.bin`) protected by Android Keystore.
- **Charging & WiFi Envelopes**: Pauses storage transfers if disconnected from power or connected to metered cellular networks.
