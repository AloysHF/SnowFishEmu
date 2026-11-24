# SnowFishEmu

<p align="center">
  <img src="res/logo-banner.png" alt="SnowFishEmu" width="600">
</p>

<p align="center">
  <a href="https://aloyshf.github.io/SnowFishEmu/"><img src="https://img.shields.io/badge/Website-SnowFishEmu-1F9FD0?logo=githubpages&logoColor=white" alt="Website"></a>
  <a href="https://github.com/AloysHF/SnowFishEmu/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/AloysHF/SnowFishEmu/ci.yml?branch=master&label=CI&logo=github" alt="CI"></a>
  <a href="https://git.libretro.com/libretro/snowfishemu/-/pipelines"><img src="https://img.shields.io/badge/dynamic/json?url=https%3A%2F%2Fgit.libretro.com%2Fapi%2Fv4%2Fprojects%2Flibretro%252Fsnowfishemu%2Fpipelines%3Fref%3Dmaster%26per_page%3D1&query=%24%5B0%5D.status&label=Pipeline%20Status&logo=gitlab&cacheSeconds=300" alt="Gitlab Pipeline Status" ></a>
  <a href="https://github.com/AloysHF/SnowFishEmu/releases/latest"><img src="https://img.shields.io/github/v/release/AloysHF/SnowFishEmu" alt="Release"></a>
  <a href="https://github.com/AloysHF/SnowFishEmu/releases"><img src="https://img.shields.io/github/downloads/AloysHF/SnowFishEmu/total" alt="Downloads"></a>
  <a href="https://sonarcloud.io/dashboard?id=AloysHF_SnowFishEmu"><img src="https://sonarcloud.io/api/project_badges/measure?project=AloysHF_SnowFishEmu&metric=alert_status" alt="Quality Gate Status"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-BSD%203--Clause-blue.svg" alt="License: BSD 3-Clause"></a>
  <a href="https://discord.gg/E4SQjXUkZy"><img src="https://img.shields.io/badge/Discord-Join%20Us-5865F2?logo=discord&logoColor=white" alt="Discord"></a>
  <a href="https://qm.qq.com/q/LAO7DKAWUC"><img src="https://img.shields.io/badge/QQ%E7%BE%A4-Join%20Us-12B7F5?logo=tencent-qq&logoColor=white" alt="QQ Group"></a>
</p>

SnowFishEmu is a Rust emulator for SnowFish (SFA) applications on feature
phones. It loads `.app` executables directly, runs guest code through a
pure-Rust ARM core, and bridges the phone services the applications rely on.

SnowFishEmu uses the `snowfishemu` command, crates and libretro core
identifiers. Save states use the `SFEMSTAT` signature.

## Features

- **SFA package format support** — header parsing, code relocation,
  package inspector (`sfa_info`), optional sibling resource file serving
- **ARM/Thumb CPU emulation** — ARMv4T core with ARMv5TE completion
  (BLX interworking, DSP multiplies)
- **Service bridge** — platform API table with wrapper-based dispatch:
  logging, guest file I/O over the package container, and the LZ stream
  decompressor (bit-exact with the platform's decoder)
- **GDI display bridge** — RGB565 framebuffer (240x320) with region
  fills, single-pixel lines, destination clipping, palette/RGB bitmap blits
  with source cropping, mirroring and color-key transparency. CJK text
  uses an embedded 16-pixel bitmap font. The height, glyph-width, and
  string-width queries use the same metrics as drawing.
- **UI timer service** — the repeating arm/callback pair every title
  registers during create, pumped on a millisecond frame clock so the
  guest's own 60-120 ms screen pumps fire at their designed rates
- **Offline startup** — network/pay service queries are answered
  locally so applications reach their menus without retired servers;
  see [docs/Game-Compatibility.md](docs/Game-Compatibility.md)
- **App-activate frame pump** — repeated selector 3 (`EVT_APP_RESUME`)
  drives repaints for SDK builds that rely on it. The legacy SDK uses its
  boot resume and polls queued input on its UI timer thereafter.
- **Offline HTTP responder** — the platform answers the SDK's
  carrier-facing HTTP transaction locally with a payload-free `200 OK`,
  runs the SDK's own result parser and notifies its registered completion
  callback when the response has been read. Recognized SDK tariff requests
  complete with local success through their registered callbacks.
  `set_offline_http(bool)` is the library
  equivalent, `SF_OFFLINE_HTTP=0` / `--no-offline-http` the opt-outs
- **Free play** — the platform answers the purchase service as a
  registered, billing-capable handset by default: carrier billing present,
  subscriber identity found, and the `__sfssuid`/`ssu<id>` account record
  backed by the service store. `set_free_play(bool)` is the library
  equivalent, `SF_FREE_PLAY=0` / `--no-free-play` the opt-outs
- **MIDI music playback** — the titles' music engines hand standard
  MIDI tunes to the platform through the +0x038 slot; the core renders
  them into a 44.1 kHz stereo stream the frontends drain per frame
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

## Architecture

```
crates/
├── snowfishemu-core/         # Platform-independent emulator engine (library)
│   └── src/
│       ├── lib.rs            # Crate root and public re-exports
│       ├── sfa.rs            # .app container: header parse, image, relocations
│       ├── mem.rs            # Checked sparse guest memory
│       ├── cpu.rs            # ARMv4T interpreter + ARMv5TE completion
│       ├── consts.rs         # Guest address constants
│       ├── plat.rs           # Root API table layout and stubs
│       ├── machine.rs        # SnowFishMachine: boot, frame loop, dispatch, files, timers
│       ├── gdi.rs            # GDI display bridge state (pen, canvas, blits)
│       ├── lz.rs             # Bit-exact LZ stream decompressor port
│       ├── font.rs           # Embedded Unicode bitmap font rasterizer
│       ├── save.rs           # SFEMSTAT save-state envelope and codec
│       └── sdk.rs            # Guest SDK service adapters
├── snowfishemu/              # Standalone binary (→ snowfishemu)
├── snowfishemu-tools/        # sfa_info package inspector, sfa_headless harness
└── snowfishemu-libretro/     # Libretro core (→ snowfishemu_libretro.{dll,so,dylib})
```

See [Architecture](docs/architecture.md) for implementation details.

## Running

See the [Standalone Emulator](docs/Standalone-Emulator.md) guide for
installation, keyboard controls, offline startup switches, resource files,
the headless harness, and all command-line options.

Standalone (240x320 window, keyboard controls):

```sh
snowfishemu <path/to/game.app> [--scale N]
```

The offline startup answers (registered-handset purchase service, local
HTTP transactions) are on by default, so every title launches with the
same command. `--no-free-play` / `--no-offline-http` turn one off for
diagnostics; `SF_FREE_PLAY=0` / `SF_OFFLINE_HTTP=0` are the environment
opt-outs.

| Key | Phone control |
|---|---|
| Arrow keys | D-pad |
| Number keys | Phone keypad |
| Enter | OK |
| Z | Left soft key |
| X | Right soft key |
| M | Menu |

In the supported 永恒魔塔 QVGA build, press **Z** at the centered music
prompt to reach the title screen. The title logo and selected menu
option are centered. When the trial expires, the game's original
registration callback unlocks continued play in the tower.

For 胭脂戏游龙, keep the companion resource container beside the game.
The startup screen asks whether to enable music. Press **Z** for Yes
or **X** for No. Press **Enter** at the title to read the opening story,
then press **Enter** again to enter 西门庆居. The room displays the
dialogue cue and movement tutorial in a centered panel.

In the supported 新鬼吹灯 QVGA build, press **X** at the title and **Z**
to skip the intro. In the cave, **M** opens the pause menu and **Enter**
or **Z** confirms Continue. Press **Enter** next to the first NPC to
talk, advance the dialogue with **Enter**, and use the arrows to walk
away after the last page.

In the supported StarBall (新星际撞球) QVGA build, use **Enter** to
confirm and serve. Use **Up** and **Down** to highlight a menu item,
the arrows to move the red outline to a level, and **Left** and **Right**
to move the paddle during play.
After a miss, use **Enter** to serve the next ball.
When the last ball is lost offline, the game replenishes the three
paddles. Use **Enter** to serve again in the same round.

In the supported Gold Miner (黄金矿工) QVGA build, press **M** on the
title screen, or use **Z** to open Options and choose Start Game.
Press **Enter** on New Game to enter the first level.

In the supported Universe Storm (宇宙风暴) QVGA build, press **Z** at the
music prompt, then **Enter** at the title and again on **New Game**.
Use the arrow keys to steer. The standalone window continues processing
input and close requests while a loading screen is static. The emulator
loads embedded music and bitmaps through the shared LZ service.
Press **Z** at the revive prompt and again on the game's result screen
to respawn and continue the current level.

Optional resource file: a companion container is discovered next to the
`.app` automatically — an exact `<stem>.pak`/`<stem>.res` sibling wins,
otherwise every sibling whose name mentions `.pak` or `res` is served
(the rule the libretro core follows). Both legacy and indexed resource
packs can be read from companion containers.

Headless bring-up harness (used by the test tooling; per-frame
framebuffer dumps for offline analysis):

```sh
sfa_headless <path/to/game.app> --frames 30 [--dump-fb <dir>] [--keys 10 --keys 5] [--press frame:key] [--no-free-play] [--no-offline-http] [--no-app-pump] [--no-keys]
```

`--no-free-play` / `--no-offline-http` turn a provisioning switch off
for diagnostics (`SF_FREE_PLAY=0` / `SF_OFFLINE_HTTP=0` are the
environment equivalents). A download request naming a pack is
answered with that pack's own bytes. `--no-keys` skips the
automatic bring-up key replay so an explicit `--keys` sequence is the
only input. `--press frame:key` fires a single key at an exact frame,
which is how the SDK's pay/session screens get walked deterministically.
`--dump-mem addr:len:path` (hex) writes a guest memory range after the run.

A purchase-free session that reaches a title's own play screen can be
driven entirely headlessly (six left-soft-key presses spread over the run
reach the gobang board; a directional key then moves the cursor and OK
places a stone — see docs/Game-Compatibility.md for each title's recipe):

```sh
sfa_headless <game.app> --frames 200 --no-keys --press 30:10 --press 60:10 --press 90:10 --press 120:10 --press 150:10 --press 180:10
```

Package inspector:

```sh
sfa_info <path/to/game.app>
```

### Screenshots

The standalone frontend also runs headlessly and saves a 240x320 PNG of
the presented frame: `-S <path.png>` runs `--screenshot-frames` frames
with scripted `--press frame:key` phone-key presses, prints
`frame nonzero=N colors=N` statistics, and exits without opening a
window:

```sh
snowfishemu <game.app> -S shot.png --screenshot-frames 200 \
  --press 30:10 --press 60:10 --press 90:10 --press 120:10 --press 150:10 --press 180:10
```

`scripts/batch-screenshots.ps1 -GameDirectory <corpus-dir>` captures the
corpus in `docs/images`, naming each PNG after its game's directory.
The script uses title-specific recipes where available and runs the other
titles for 200 frames without input by default.
See [Game Compatibility](docs/Game-Compatibility.md) for the observed results,
including the Super Monopoly (超级大富翁) board, the Dance (热舞旋风) menu,
and the title of 明星模仿秀-超级斗地主 after the first-launch audio prompt.

## Browser Mode (EmulatorJS)

The project site hosts a browser player at
<https://AloysHF.github.io/SnowFishEmu/play.html>: pick a local `.app`
application (or drop it onto the page) and the WebAssembly core runs it via
[EmulatorJS](https://emulatorjs.org). Files are processed entirely in the
browser and never uploaded.

The core bundle (`snowfishemu-emulatorjs.zip`) is published on every nightly
and release. To embed the core in your own site, see
[docs/EmulatorJS-Core.md](docs/EmulatorJS-Core.md).

## RetroArch

See the [RetroArch Core](docs/RetroArch-Core.md) guide for installation,
supported platforms, keyboard and RetroPad mapping, and features.

The core is produced at `target/release/snowfishemu.dll`
(`libsnowfishemu.so` on Linux, `libsnowfishemu.dylib` on macOS). Rename it
to `snowfishemu_libretro.<ext>` before placing it in the RetroArch `cores`
directory, put the info file (`snowfishemu_libretro.info`) in the `info`
directory, then load a `.app` as content.

- Video: 240x320 XRGB8888. The core reports a 60 Hz pacing rate; each
  presented frame advances the guest's millisecond UI clock by 33 ms.
- Audio: the titles' MIDI music renders through the platform's music
  slot into a 44.1 kHz stereo stream; a quiet tune still submits a full
  silent batch so frontends keep pacing on the audio clock.
- Save states use the versioned `SFEMSTAT` format and require matching
  content. Version 2 restores MIDI playback and its position. Version 1
  states load with music initially silent.
- Core options: CPU/HLE debug logging.
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

## Testing

Run the unit tests:

```bash
cargo test --workspace
```

Game files are not included. Supply legally obtained `.app` applications
separately; see [Game Compatibility](docs/Game-Compatibility.md) for what
runs and how it is verified.

## Contributing

Contributions are welcome! Whether you're interested in fixing bugs, adding
features, improving documentation, or testing game compatibility, we'd love your
help. See [CONTRIBUTING.md](docs/CONTRIBUTING.md) for details.

## License

SnowFishEmu is distributed under the [BSD 3-Clause license](LICENSE).
SnowFishEmu does not include any game data. You must supply your own `.app`
files obtained from a device you own.
