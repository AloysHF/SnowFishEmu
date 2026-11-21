# SnowFishEmu

<p align="center">
  <a href="https://github.com/AloysHF/SnowFishEmu/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/AloysHF/SnowFishEmu/ci.yml?branch=master&label=CI&logo=github" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-BSD%203--Clause-blue.svg" alt="License: BSD 3-Clause"></a>
</p>

SnowFishEmu is a Rust emulator for SnowFish (SFA) applications on feature
phones. It loads `.app` executables directly, runs guest code through a
pure-Rust ARM core, and bridges the phone services the applications rely on.

SnowFishEmu uses the `snowfishemu` command, crates and libretro core
identifiers. Save states use the `SFEMSTAT` signature.

> **Status: early development.** The package loader, guest execution, and
> service bridges are under active construction. See
> [docs/Game-Compatibility.md](docs/Game-Compatibility.md) for what runs.

## Features

- **SFA package format support** — header parsing, code relocation,
  package inspector (`sfainfo`), optional sibling resource file serving
- **ARM/Thumb CPU emulation** — ARMv4T core with ARMv5TE completion
  (BLX interworking, DSP multiplies)
- **Service bridge** — platform API table with wrapper-based dispatch:
  logging, guest file I/O over the package container, and the LZ stream
  decompressor (bit-exact with the platform's decoder)
- **GDI display bridge** — RGB565 framebuffer (240x320) with region
  fills, palette/RGB bitmap blits with source cropping and color-key
  transparency, CJK text rendering, and the pen/colour state the titles
  drive through the canvas object
- **UI timer service** — the repeating arm/callback pair every title
  registers during create, pumped on a millisecond frame clock so the
  guest's own 60-120 ms screen pumps fire at their designed rates
- **Offline startup** — network/pay service queries are answered
  locally so applications reach their menus without retired servers;
  see [docs/Game-Compatibility.md](docs/Game-Compatibility.md)
- **App-activate frame pump** — selector 3 (`EVT_APP_RESUME`) is posted
  every frame by default; without it several titles never leave their
  loading screen. `set_app_pump(false)` / `--no-app-pump` /
  `SF_NO_APP_PUMP=1` disables it for diagnostics
- **Offline HTTP responder** — `SF_OFFLINE_HTTP=1` (or `--offline-http`)
  answers the SDK's carrier-facing HTTP transaction locally with a
  payload-free `200 OK`, runs the SDK's own result parser and notifies its
  registered completion callback for every transaction. It is off by
  default so titles that never reach the network are untouched;
  `set_offline_http(bool)` is the library equivalent
- **Free play** — `SF_FREE_PLAY=1` (or `--free-play`) makes the platform
  answer the purchase service as a registered, billing-capable handset:
  carrier billing present, subscriber identity found, and the
  `__sfssuid`/`ssu<id>` account record backed by the service store. The
  guest still runs its own flow; nothing is forced into its state
- **RetroArch core** — the `snowfishemu_libretro` core runs the same
  machine under RetroArch and other libretro frontends

## Building

```sh
cargo build --release -p snowfishemu
```

The libretro core builds standalone (this is what the buildbot does):

```sh
cargo build --release
```

## Running

Standalone (240x320 window, keyboard controls):

```sh
snowfishemu <path/to/game.app> [--free-play] [--offline-http] [--res <res.app>] [--scale N]
```

`--free-play` and `--offline-http` are the same switches as sfheadless's
(they turn on what `SF_FREE_PLAY=1` / `SF_OFFLINE_HTTP=1` turn on; those
environment variables are honoured too).

| Key | Phone control |
|---|---|
| Arrow keys | D-pad |
| Enter | OK |
| Z | Left soft key |
| X | Right soft key |

In the supported Gold Miner (黄金矿工) QVGA build, press **X** or
**Enter** on the title screen to start the first level offline. Use **Z**
to open Options and choose Start Game to reach the level selection menu.

In the supported Universe Storm (宇宙风暴) QVGA build, press **Z** at the
music prompt, then **Enter** at the title and again on **New Game**.
Use the arrow keys to steer. The standalone window continues processing
input and close requests while a loading screen is static. The emulator
loads embedded music and bitmaps directly from the package during play.

Optional resource file: place it next to the `.app` under the same stem
with `Res`/`res` in the name (see the compatibility notes).

Headless bring-up harness (used by the test tooling; per-frame
framebuffer dumps for offline analysis):

```sh
sfheadless <path/to/game.app> --frames 30 [--res <res.app>] [--dump-fb <dir>] [--keys 10 --keys 5] [--press frame:key] [--free-play] [--offline-http] [--no-app-pump] [--no-keys]
```

`--free-play` is the same switch as `SF_FREE_PLAY=1`; `--offline-http` is
the same as `SF_OFFLINE_HTTP=1`. `--res` is repeatable — a title that
ships two resource containers takes both, and a download request naming a
pack is answered with that pack's own bytes. `--no-keys` skips the
automatic bring-up key replay so an explicit `--keys` sequence is the
only input. `--press frame:key` fires a single key at an exact frame,
which is how the SDK's pay/session screens get walked deterministically.
`--dump-mem addr:len:path` (hex) writes a guest memory range after the run.

A purchase-free session that reaches a title's own play screen can be
driven entirely headlessly (six left-soft-key presses spread over the run
reach the gobang board; a directional key then moves the cursor and OK
places a stone — see docs/Game-Compatibility.md for each title's recipe):

```sh
sfheadless <game.app> --frames 200 --no-keys --free-play --offline-http   --press 30:10 --press 60:10 --press 90:10 --press 120:10 --press 150:10 --press 180:10
```

Package inspector:

```sh
sfainfo <path/to/game.app>
```

## RetroArch

Deploy the built core (`target/release/snowfishemu_libretro.dll`, or
`libsnowfishemu_libretro.so` on Linux) to the RetroArch `cores` directory
and the info file (`snowfishemu_libretro.info`) to the `info` directory,
then load a `.app` as content.

- Video: 240x320 XRGB8888. The core reports a 60 Hz pacing rate; each
  presented frame advances the guest's millisecond UI clock by 33 ms.
- Audio: none (the platform exposes no audio service; the core submits
  silence so frontends pace on a steady clock).
- Save states: supported. States carry a version tag and a content
  fingerprint (`SFEMSTAT` envelope), so states from other packages or
  older builds are rejected instead of restored.
- Core options: CPU/HLE debug logging.
- Gold Miner (黄金矿工): press the right soft key or OK to start the first
  level from the title screen.
- Resource file: the core serves the same optional sibling resource file
  as the standalone frontend (see above), resolved from the content path.
- Note: on Windows, RetroArch's d3d11 driver has a first-present race
  when the window starts in the background (the frontend can fault in
  `msvcrt.dll` or present one garbled frame). Launching with the `gdi`
  video driver avoids it.

RetroPad controls (in addition to the keyboard controls above):

| RetroPad | Phone control |
|---|---|
| D-pad | D-pad (left stick also works) |
| A / B / Start | OK |
| X / L | Left soft key |
| Y / R | Right soft key |
| Select (hold) + D-pad | Number layer: digits 1-4 |
| Select (hold) + A / B / X / Y | Number layer: digits 5-8 |
| Select (hold) + Start | Number layer: digit 9 |
| Select (hold) + L / R | Number layer: digit 0 |

## License

SnowFishEmu is distributed under the [BSD 3-Clause license](LICENSE).
SnowFishEmu does not include any game data. You must supply your own `.app`
files obtained from a device you own.
