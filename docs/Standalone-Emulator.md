# Standalone Emulator

This guide covers building and running the standalone `snowfishemu` binary,
loading applications, keyboard controls, the offline startup switches, the
headless harness, and all command-line options.

## Supported Platforms

| Platform | Architecture | Status |
|----------|-------------|--------|
| Windows | x86_64 | ✅ |
| macOS | x86_64, aarch64 | ✅ |
| Linux | x86_64, aarch64 | ✅ |

## Installation

Download the latest standalone binary for your platform from the
[Releases](https://github.com/AloysHF/SnowFishEmu/releases) page, or the
rolling `nightly-build` prerelease for the current master.

You can also build it from source:

```bash
cargo build -p snowfishemu --release
```

The binary is produced at `target/release/snowfishemu` (`.exe` on Windows).

## Synopsis

```text
snowfishemu [OPTIONS] <PATH>
```

## Options

| Option | Value | Default | Description |
|---|---|---|---|
| `<PATH>` | path | *required* | Path to the `.app` package. |
| `--free-play` | flag | off | Answer the purchase service as a registered, billing-capable handset (the same switch as `sfa_headless --free-play` / `SF_FREE_PLAY=1`). |
| `--offline-http` | flag | off | Answer the SDK's carrier-facing HTTP transactions locally (the same switch as `sfa_headless --offline-http` / `SF_OFFLINE_HTTP=1`). |
| `--scale <SCALE>` | integer ≥ 1 | `2` | Integer window scale factor over the 240x320 surface. |
| `--headless` | flag | off | Run without opening a window. |
| `--frames <COUNT>` | integer | `60` | Frames to run in headless mode. |
| `--instruction-limit <COUNT>` | integer | `200000000` | Maximum guest instructions per callback. |
| `-S, --screenshot <PATH>` | path | — | Run headlessly, save a PNG screenshot, then exit. |
| `--screenshot-frames <COUNT>` | integer | `30` | Frames to run before capturing. |
| `--press <FRAME:KEY>` | `FRAME:KEY` (repeatable) | — | Scripted phone-key press at an exact frame, using the core's key indices (10 = left soft key, 5 = OK, 25 = right soft key). Headless and screenshot runs only. |
| `-v, --verbose` | flag | off | Enable debug logging. |

The flags only ever turn the switches on: the `SF_FREE_PLAY` /
`SF_OFFLINE_HTTP` environment variables are honoured by the core at load, and
a flag adds the switch on top rather than overriding an environment variable
back off.

## Loading Applications

The standalone emulator accepts `.app` executables:

```bash
# Load an application directly
snowfishemu path/to/game.app

# Open a 4x window (960x1280)
snowfishemu path/to/game.app --scale 4

# Offline startup: purchase service + local HTTP answers
snowfishemu path/to/game.app --free-play --offline-http
```

### Resource files

Titles that ship a companion resource container keep it next to the `.app`,
and the emulator discovers it automatically — the same rule the libretro
core follows:

1. **Exact sibling**: `<content-stem>.pak` or `<content-stem>.res` next to
   the content file wins outright.
2. **Fuzzy siblings**: otherwise every sibling file whose name mentions
   `.pak` or `res` (case-insensitive) is served, so a title that ships
   more than one container takes all of them.

A download request naming a pack is answered with that pack's own bytes.
See [Game Compatibility](Game-Compatibility.md) for the per-title resource
notes.

## Default Key Mappings

| Phone Input | Keyboard |
| --- | --- |
| Direction pad | Arrow keys |
| Confirm (OK) | Enter |
| Left soft key | `Z` |
| Right soft key | `X` |
| Numeric keypad | `1`–`9`, `0` |
| Exit | Escape |

On this phone the arrow directions share keys with digits 3/5/7/9 and OK
with digit 6, so the digit keys drive the same guest indices as the arrows.

## Offline Startup

The SnowFish SDK gates its titles behind carrier-billing initialization, and
the original servers are retired. Two switches answer those queries locally
so titles reach their own menus; nothing is forced into the guest's state —
the guest still runs its own flow:

- `--free-play` makes the platform answer the purchase service as a
  registered, billing-capable handset: carrier billing present, subscriber
  identity found, and the account record backed by the service store.
- `--offline-http` answers the SDK's carrier-facing HTTP transaction locally
  with a payload-free `200 OK`, runs the SDK's own result parser, and
  notifies its registered completion callback. It is off by default so
  titles that never reach the network are untouched.

Which titles need which switch (and the key recipes that reach their play
state) are documented per title in
[Game Compatibility](Game-Compatibility.md).

## Logging

Logs go to stderr through the `log`/`env_logger` facade; the default filter
is `warn`. Raise it for diagnostics:

```bash
RUST_LOG=debug snowfishemu path/to/game.app
RUST_LOG=trace snowfishemu path/to/game.app   # includes per-slot traces
```

## Headless Mode and Screenshots

Run the emulator without a window — useful for automated testing and batch
processing:

```bash
# Run 120 frames silently and exit
snowfishemu path/to/game.app --headless --frames 120

# Take a PNG screenshot after 120 frames, with scripted key presses
snowfishemu path/to/game.app -S shot.png --screenshot-frames 120 \
  --press 30:10 --press 60:5
```

`--press frame:key` schedules scripted phone-key presses at exact frames
during headless and screenshot runs, so the offline bring-up recipes in
[Game Compatibility](Game-Compatibility.md) drive the capture. A screenshot
only writes a PNG when the guest actually rendered a frame within the
allotted frames; blank runs are reported as failures. Screenshots always
use the native 240x320 framebuffer resolution.

## Headless Harness

`sfa_headless` drives the same machine core without a window — the
deterministic path for compatibility work and regression testing:

```bash
sfa_headless <path/to/game.app> --frames 30 \
  [--dump-fb <dir>] [--keys 10 --keys 5] [--press frame:key] \
  [--free-play] [--offline-http] [--no-app-pump] [--no-keys]
```

| Option | Description |
|---|---|
| `--frames <COUNT>` | Frames to pump after boot (default 5). |
| `--keys <INDEX>` | Queue a few key presses after boot (indices into the key map). |
| `--press <FRAME:KEY>` | Press a key at an exact frame; repeatable. A press holds the key for one frame and releases it on the next, so a scripted run advances two frames per press. |
| `--no-keys` | Skip the automatic bring-up key replay so an explicit `--keys`/`--press` sequence is the only input. |
| `--no-app-pump` | Suppress the per-frame app-activate/resume event (diagnostic only). |
| `--dump-fb <DIR>` | Write per-frame framebuffer dumps (one 32-bit word per pixel). |
| `--dump-mem <ADDR:LEN:PATH>` | Dump a guest memory range after the run, hex `addr`/`len`; repeatable. |
| `--serve-asset <PATH>` | Side-load a local file to answer download requests. |
| `--serve-match <SUBSTR>` | Substring a request path must contain for `--serve-asset` (default `.pak`). |
| `--free-play` / `--offline-http` | The same switches as the standalone frontend. |
| `--instruction-limit <COUNT>` | Maximum guest instructions per callback (default 200000000). |

## Package Inspector

`sfa_info` prints a package's header fields, segment layout, and relocation
summary:

```bash
sfa_info <path/to/game.app>
```

## Corpus Screenshots

The compatibility documentation and the site gallery use
emulator-rendered captures, one per corpus title, produced by:

```powershell
scripts/batch-screenshots.ps1 -GameDirectory <corpus-dir>
```

The script drives the standalone emulator's headless screenshot mode
(`-S <path>` with `--screenshot-frames` and scripted `--press frame:key`
key presses) through each title's documented bring-up recipe from
[Game Compatibility](Game-Compatibility.md), discovers the sibling
resource containers next to each package, and writes `<display-name>.png`
into `docs/images`. No game data is distributed.

## Examples

```bash
# Basic usage
snowfishemu path/to/game.app

# 4x window with the offline startup switches
snowfishemu path/to/game.app --scale 4 --free-play --offline-http

# Headless bring-up replay, then dump the framebuffer frames
sfa_headless path/to/game.app --frames 200 --no-keys --free-play --offline-http \
  --press 30:10 --press 60:10 --press 90:10

# Inspect a package
sfa_info path/to/game.app
```
