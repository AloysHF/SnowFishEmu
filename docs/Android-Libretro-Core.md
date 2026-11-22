# Android Libretro Core

The SnowFishEmu libretro core runs on Android, so it can be used by most
Android RetroArch-based frontends.

## Install in RetroArch on Android

### Manual installation

The renamed core requires coordinated upstream buildbot and core-info updates.
Until those updates are published, install a SnowFishEmu release manually;
availability under the new name in Core Downloader is not assumed.

1. **Download** `snowfishemu-android-libretro.tar.gz` from the
   [Releases](https://github.com/AloysHF/SnowFishEmu/releases) page. It
   contains per-ABI `snowfishemu_libretro_android.so` files for `arm64-v8a`,
   `armeabi-v7a`, `x86` and `x86_64`.
2. **Install the core**: copy the `snowfishemu_libretro_android.so` matching
   your device's ABI (most modern devices are `arm64-v8a`) into RetroArch's
   `cores/` directory (typically
   `/storage/emulated/0/RetroArch/cores/` or the app's internal `cores/` path),
   and copy `snowfishemu_libretro.info` into RetroArch's `info/` directory.
3. **Load** the core and content the same way as on desktop.

## Building the Android core locally

Building for Android requires the [Android NDK](https://developer.android.com/ndk)
and [`cargo-ndk`](https://github.com/bbqsrc/cargo-ndk):

```bash
cargo install cargo-ndk
rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android
export ANDROID_NDK_HOME=/path/to/android-ndk
# Android 15 devices require 16 KB page alignment for native libraries.
export RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384"

# Build all four ABIs (artifacts land in target/<triple>/release/)
cargo ndk -t arm64-v8a -t armeabi-v7a -t x86 -t x86_64 --platform 21 \
  build -p snowfishemu-libretro --release
```

Each ABI produces `libsnowfishemu.so`; rename it to
`snowfishemu_libretro_android.so` when installing into RetroArch on Android.

## Android host rules

The core ships `android.json`, the system profile used by Android hosts in
this emulator family: display names, content extensions, the
virtual-controller preset, and orientation.

Rules the core follows for any Android host:

- The optional sibling resource file (`.pak`/`.res` next to the content, see
  [Standalone Emulator](Standalone-Emulator.md#resource-files)) is resolved
  from the content path, the same way as on desktop.
- Applications in this core have no battery-backed save RAM, so there is no
  save file to place next to the content; libretro save states are handled by
  the frontend.
- Load failures are reported with `RETRO_ENVIRONMENT_SET_MESSAGE` and the log,
  never as a silent `false`.
- Panics are caught at the C ABI boundary, so a bad frame cannot kill the host.
- Save states carry the `SFEMSTAT` magic header plus a format version and a
  content fingerprint; states from another version or package are rejected
  instead of being misread.
- Cores are linked with 16 KB page alignment for Android 15 devices.
