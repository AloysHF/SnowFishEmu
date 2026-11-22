# EmulatorJS Core (Browser)

SnowFishEmu can run in the browser as a standard
[EmulatorJS](https://emulatorjs.org) core. The build follows EmulatorJS's own
pipeline: the Rust libretro core is compiled for `wasm32-unknown-emscripten`,
linked against the `EmulatorJS/RetroArch` fork, and packaged as the
`<core>-wasm.data` bundles that an EmulatorJS page loads at runtime.

The project site ships a ready-to-use player at
<https://AloysHF.github.io/SnowFishEmu/play.html>: visitors pick a `.app`
application from their own machine and it runs from a blob URL, so nothing is
uploaded and the site does not host any game data.

## What you get

Every nightly publishes a single asset:

| File | Contents |
|------|----------|
| `snowfishemu-emulatorjs.zip` | Both core variants, the build report `cores/reports/snowfishemu.json`, `core.json`, `build.json`, `license.txt` and a README |

The two `.data` files stay separate inside the ZIP: EmulatorJS requests
`snowfishemu-wasm.data` or `snowfishemu-legacy-wasm.data` by name depending on
WebGL2 support, so one bundle cannot serve both cases.

The bundle is laid out like this:

```
snowfishemu-emulatorjs.zip
├── cores/
│   ├── snowfishemu-wasm.data
│   ├── snowfishemu-legacy-wasm.data
│   └── reports/snowfishemu.json
├── core.json
├── build.json
├── license.txt
└── README.md
```

Each `.data` file is a 7z archive containing `snowfishemu_libretro.js`,
`snowfishemu_libretro.wasm`, `core.json`, `build.json` and `license.txt`.

## Install into a site

EmulatorJS always fetches cores from `<EJS_pathtodata>cores/`, so host the
runtime yourself (or use the pinned CDN files) and drop the core beside it:

```
data/
├── loader.js
├── emulator.min.js
├── emulator.min.css
├── compression/
│   ├── extract7z.js       # extracts the core .data archive
│   └── extractzip.js      # extracts ZIP content
└── cores/
    ├── snowfishemu-wasm.data
    ├── snowfishemu-legacy-wasm.data
    └── reports/snowfishemu.json
```

`emulator.min.zip` only contains `emulator.min.js` and `emulator.min.css`;
the `compression/` files are loaded on demand from the same EmulatorJS data
directory. Without `extract7z.js` the core download ends in a "Network Error".

To upgrade the runtime, replace the runtime files with a newer EmulatorJS
release (≥ 4.2.3) and keep this `cores/` directory.

## Use

```html
<div id="game"></div>
<script>
  EJS_player = "#game";
  EJS_core = "snowfishemu";
  EJS_gameUrl = URL.createObjectURL(gameFile);
  EJS_gameName = gameFile.name;
  EJS_pathtodata = "data/";
  EJS_startOnLoaded = false;
</script>
<script src="data/loader.js"></script>
```

`site/play.html` in this repository is a complete example that loads a single
`.app` file.

When the application comes from a blob URL, EmulatorJS writes it into the wasm
filesystem under `EJS_gameName`. Set that to the original file name, extension
included (for example `game.app`), so the frontend keeps a recognizable
content path for the core.

### Content and companion files

The built-in player loads a bare `.app` application. The tested titles carry
their resources embedded in the package; a title that opens an optional
sibling resource file (a `*Res*.app` beside the package) needs those files
shipped as a separate ZIP in `EJS_biosUrl`:

```html
EJS_biosUrl = URL.createObjectURL(companionZip);
```

EmulatorJS extracts the archive next to the content file in the wasm
filesystem, and the core indexes that directory read-only as the guest device
filesystem — the same sibling-resource lookup the desktop and RetroArch
frontends perform on the content path. Keep the companion files at the
archive root. Guest writes stay in memory and are lost when the page is
reloaded.

## Rendering

The core renders its framebuffer through libretro's software path as XRGB8888
at the application's native size: 240×320 for the QVGA titles. The play stage
scales the output to fit the page; pixel-art applications look best with
nearest-neighbor scaling.

## Controls

EmulatorJS maps its RetroPad to the core's phone-keypad layout:

| Input | Guest key |
|-------|-----------|
| Arrow keys | D-pad |
| A / B / Start | OK |
| X / L | Left soft key |
| Y / R | Right soft key |
| Select (hold) + directions/face/Start | Number layer: digits 0-9 |

The default EmulatorJS keyboard bindings drive the same layout: the arrow
keys move, `x`, `s` or Enter confirm (OK), `z` or `q` is the left soft key,
`a` or `e` is the right soft key, and `v` is Select for the number layer.
The exact keys are shown in EmulatorJS's own control settings. Touch
controls and physical gamepads are provided by EmulatorJS and arrive
through the libretro pointer and RetroPad devices.

## Saves

Applications do not have battery-backed save RAM in this core, so `core.json`
sets `"save": false` to hide EmulatorJS's save-file buttons. Libretro save
states are implemented by the core (`SFEMSTAT` envelope with a version tag and
a content fingerprint) and handled by EmulatorJS; they stay in the browser's
local storage.

## Building the bundle

The nightly workflow runs this automatically. To build locally you need an
activated [emsdk](https://emscripten.org/docs/getting_started/downloads.html)
(`emmake` in `PATH`), `p7zip`, `zip`, and the pinned nightly toolchain:

```bash
# Rust's prebuilt emscripten stdlib carries wasm exception handling, which
# Binaryen's Asyncify pass rejects. Rebuild std with panic=abort instead.
cargo +nightly build -p snowfishemu-libretro \
  --target wasm32-unknown-emscripten --profile wasm-release \
  -Zbuild-std=std,panic_abort

scripts/build-emulatorjs-core.sh
```

The script clones the pinned `EmulatorJS/RetroArch` revision, links the
staticlib with `Makefile.emulatorjs`, builds the WebGL2 and legacy variants, and
writes `out/snowfishemu-emulatorjs.zip`. `--work-dir` moves the RetroArch
checkout out of `target/`, which matters on CI because `rust-cache` restores
`target/` without the checkout's `.git`.

The Pages deployment fetches the pinned EmulatorJS runtime, downloads the
`snowfishemu-emulatorjs.zip` asset from the rolling `nightly-build` release,
and redeploys after every successful Nightly Build.
