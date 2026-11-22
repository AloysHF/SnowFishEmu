# RetroArch Core

This guide covers installing and running the SnowFishEmu libretro core for
RetroArch, loading content, supported features, and controls.

## Supported Platforms

| Platform | Architecture | Standalone | Libretro |
|----------|-------------|------------|----------|
| Windows | x86_64 | ✅ | ✅ |
| macOS | x86_64, aarch64 | ✅ | ✅ |
| Linux | x86_64, aarch64 | ✅ | ✅ |
| Android | arm64-v8a, armeabi-v7a, x86, x86_64 | — | ✅ (see [Android Libretro Core](Android-Libretro-Core.md)) |
| iOS | arm64, x86_64 | — | ✅ experimental (see [iOS Libretro Core](iOS-Libretro-Core.md)) |

## Installation

### Manual Installation

Build the libretro core:

```bash
cargo build -p snowfishemu-libretro --release
```

Cargo names the cdylib after its lib target, so this produces
`snowfishemu.dll` on Windows (`libsnowfishemu.so` on Linux,
`libsnowfishemu.dylib` on macOS) under `target/release/`.

RetroArch expects the core file to be named `snowfishemu_libretro.<ext>`, so
rename it accordingly before placing it into RetroArch's `cores/` directory.
Copy `snowfishemu_libretro.info` into RetroArch's `info/` directory so the
frontend can display the core metadata and supported features.

## Loading Applications

1. Open RetroArch and select **Load Core > SnowFish (SnowFishEmu)**.
2. Select **Load Content**.
3. Choose a `.app` file.

An optional sibling resource file is served the same way as in the
standalone frontend, resolved from the content path (see
[Standalone Emulator](Standalone-Emulator.md#resource-files)).

## Supported Features

- Video output at 240x320 in the RGB888 pixel format
- A 60 Hz frontend pacing rate; each presented frame advances the guest's
  millisecond UI clock, so titles pump their screens at their designed rates
- RetroPad input handling with a number layer (see below)
- Keyboard input through `RETRO_DEVICE_KEYBOARD`, mirroring the standalone
  key map one-to-one (enable RetroArch's Game Focus so keys reach the core)
- `.app` content loading with header validation and code relocation
- Save states: states carry a version tag and a content fingerprint
  (`SFEMSTAT` envelope), so states from other packages or older builds are
  rejected instead of restored
- Input descriptors for frontend button labeling
- Audio: the titles' MIDI music renders through the platform's music slot
  into a 44.1 kHz stereo stream (8 kHz mono synth resampled); a quiet tune
  still submits a full silent batch so frontends keep pacing on the audio
  clock
- Core options for CPU/HLE debug logging

## Keyboard Input

The core polls `RETRO_DEVICE_KEYBOARD` directly, so the full phone keypad is
reachable from a keyboard with the same layout the standalone frontend uses:

| Phone input | Keyboard |
|---|---|
| Numeric keypad | `0`–`9` |
| Direction pad | Arrow keys (digits 3/5/7/9 drive the same guest indices) |
| Confirm | Enter (digit 6 drives the same guest index) |
| Left soft key | `Z` |
| Right soft key | `X` |
| Menu | `M` |

**Enable Game Focus first** (default: Scroll Lock). RetroArch binds the
keyboard to its own hotkeys and RetroPad by default, which captures keys
before the core sees them; Game Focus suspends those bindings so
keystrokes reach the core. Alternatively, unbind the conflicting
keyboard-to-RetroPad and hotkey bindings.

Keyboard and RetroPad combine as a logical OR per key, so either device can
hold a key at any time.

## RetroPad Button Mapping

The guest keys include directions, OK (confirm), the two soft keys, Menu,
and the numeric layer.

| RetroPad Button | Guest key | Action |
|---|---|---|
| D-Pad Up / Left / Right / Down | Up / Left / Right / Down | Direction pad |
| A (SNES East) / B (SNES South) / Start | OK | Confirm |
| X (SNES North) / L (left shoulder) | Left soft key | Left soft key |
| Y (SNES West) / R (right shoulder) | Right soft key | Right soft key |
| L2 | Menu | Opens the pause menu in 新鬼吹灯 |
| Select + D-Pad Up / Down / Left / Right | 1 / 2 / 3 / 4 | Number layer (Select held) |
| Select + A / B / X / Y | 5 / 6 / 7 / 8 | Number layer (Select held) |
| Select + Start | 9 | Number layer (Select held) |
| Select + L or R | 0 | Number layer (Select held) |

The last four rows are the **number layer**: hold **Select** as a modifier
and the chords above type the numeric keypad (guest keys 0–9). While Select
is held the other pad keys are suppressed; release it to restore the normal
mapping.

## Timing

The core reports a 60 Hz pacing rate; each presented frame advances the
guest's millisecond UI clock by 33 ms, which is the cadence the titles'
own 60–120 ms screen pumps expect. No user configuration is needed; do not
compensate with the fast-forward or slow-motion settings.

## Core Options

Open **Settings > Core Options** in RetroArch to configure the core.

| Option | Choices (default first) | Description |
|---|---|---|
| CPU/HLE Debug Logging | disabled, enabled | Forward debug-level core logs to the frontend log |

## Known Limitations

- Audio is not emulated (the platform exposes no audio service); the core
  submits silence so frontends pace on a steady clock
- Compatibility is limited to the titles in
  [Game Compatibility](Game-Compatibility.md)
- On Windows, RetroArch's d3d11 video driver has a first-present race when
  the window starts in the background (the frontend can fault in
  `msvcrt.dll` or present one garbled frame). Launching RetroArch with the
  `gdi` video driver avoids it.
