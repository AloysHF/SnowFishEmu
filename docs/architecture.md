# Architecture

SnowFishEmu executes native SFA applications instead of replacing their
logic with a scene preview. The core is platform-independent and exposes a
240x320 RGB565 framebuffer plus phone-key input to frontends; the desktop
window, the libretro ABI, and the browser (EmulatorJS) build are thin
frontends over the same machine.

The workspace has four crates:

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
│       └── compat.rs         # Content-fingerprint-keyed offline fixes
├── snowfishemu/              # Standalone binary (→ snowfishemu)
├── snowfishemu-tools/        # sfa_info package inspector, sfa_headless harness
└── snowfishemu-libretro/     # Libretro core (→ snowfishemu_libretro.{dll,so,dylib})
```

## Boot flow

1. `sfa.rs` validates the `.app` header (magic `SFA\x19`/`TFA\x19`, version,
   header length, code/BSS sizes) and applies the relocation table: each
   word at `CODE_BASE + offset` gains the image base `0x101000`.
2. `SnowFishMachine::load` maps the guest regions (see below), plants the
   loader words around the image, scans the image for wrapper stublets, and
   detects the content fingerprint (see [Compatibility fixes](#compatibility-fixes)).
3. The guest entry runs with `R0 = 0`, `R1 = app_addr + 0x1C`, `R2 = -1`
   (init selector), `SP = 0x400000`, `LR = 0x541000`. After init the guest
   writes its dispatch-block pointer to `app_addr + 0x1C`.
4. The boot sequence then posts selector 1 (start), selector 2
   (app activate — titles complete their boot state there), and selector 3
   (first tick). Event calls enter through the dispatch block: `PC =
   *(block)`, `R0 = block`.
5. Per-frame ticks use selector 10 (~50 ms pacing); key input uses selectors
   6 (press) / 5 (release) with MTK key codes (`KEY_MAP` in `machine.rs`).

The frame loop pumps the guest on a millisecond clock, presents the
framebuffer when the dirty flag fires, and keeps the UI-timer registrations
(`+0x0AC` / `+0x500`) firing at their designed rates.

## Guest memory

The machine uses checked sparse regions rather than reserving the entire
32-bit address space (see [Game File Formats](Game-File-Formats.md#guest-memory-map)
for the full table): code at `0x100000` (image at `0x101000`), a 1 MB stack
growing down from `0x400000`, guest globals at `0x440000`, the platform
region at `0x540000` (root API table, registers, vtable, app record,
passthrough stubs, canvas, framebuffer, memory descriptor), and a 1 MB heap
at `0x600000`.

Unmapped accesses are recorded for diagnostics; an unmapped instruction
fetch stops execution with an error instead of wandering.

## Service bridge

Guest code calls platform services through generated wrapper stublets
(`ldr rX, [rX, #-4]; mov lr, pc; ldr pc, [rX, #slot]`). The loader scans the
image for these stublets, so every call site is interceptable with its own
stack-argument layout:

- Slots the emulator implements natively are handled at the wrapper's hook
  address (the `ldr`): the native implementation writes `R0` (platform
  slots) or leaves it (GDI slots), and execution resumes at `hook + 0xC`.
- Every other slot executes natively through the root-table stubs unchanged,
  so unknown slots still reach the guest's own passthrough expectations.
- GDI stack arguments are read through the *call site's* own wrapper
  epilogue: multiple stublets share a slot with different `add sp, #imm`
  adjustments, so the argument offset derives from the caller.

Implemented platform slots cover logging (`+0x7C`), guest file I/O over the
package container and sibling resources (`+0x58`/`+0x5C`/`+0x64`/`+0x68`,
with the `__`-prefixed service files backed by an in-memory store), the LZ
stream decompressor (`+0x108`), the UI timer arms (`+0x0AC`, `+0x500`),
carrier-billing capability (`+0x4C`), the subscriber-identity lookup shape
of `+0x524`, and the socket/bearer slots the SDK's HTTP transactions use
(`+0x0C8`–`+0x0F0`, `+0x568`–`+0x570`). See
[Game File Formats](Game-File-Formats.md#root-api-table-slots) for the
authoritative slot table and the per-slot semantics.

The offline services are opt-in per run:

- `--free-play` (`SF_FREE_PLAY=1`) answers the purchase service as a
  registered handset and backs the `__sfssuid`/`ssu<id>` account record with
  the service store.
- `--offline-http` (`SF_OFFLINE_HTTP=1`) answers the SDK's carrier-facing
  HTTP transactions locally with a payload-free `200 OK`, runs the SDK's own
  result parser, and notifies its registered completion callback.
- The per-frame app-activate pump (selector 3) is on by default;
  `SF_NO_APP_PUMP=1` disables it for diagnostics.

Unknown slots are not silently ignored: unimplemented slots the corpus
exercises are recorded, and `SF_TRACE` logs every platform slot and GDI
operation for compatibility work.

## Rendering

The guest owns a 240x320 RGB565 surface at `0x540720`, managed by the GDI
bridge (`gdi.rs`). The GDI slots the corpus drives are served natively:
fills and long-form fill variants, the canvas pen/colour state
(`+0x180` set-color, `+0x184` clear), text runs in UTF-16 (`+0x154`, the
`+0x14C` character-range sibling), bitmap register/draw with palette and
color-key semantics (`+0x194`/`+0x198`/`+0x134`), canvas clipping
(`+0x12C`), and flush (`+0x010`). Text rasterizes from an embedded Unicode
bitmap font (`unifont`), so the core does not depend on host fonts. The
glyph-width query (`+0x018`) uses the same advances as text rendering
for the supported Three Kingdoms QVGA package.

Bitmap descriptors carry width, height, color key, palette pointer/mode and
a data pointer with a fallback read; palette mode is detected from whether
the palette word is a pointer. Colors reach the framebuffer in RGB565; the
canvas keeps the guest's RGB888 pen.

The clip bounds subsequent fills, text and bitmap draws. Guests can position
an entire sprite atlas behind a small clip to render one tile. The emulator
stores the clip in platform scratch memory so it persists across save states.

## LZ decompressor

The `+0x108` service is an exact port of the platform's `lz_decompress`
state machine (`lz.rs`): two dispatch states, four match-distance bands,
the trailing-literal rules, and the terminator conditions. It is validated
bit-exactly against the platform's recorded call sites, so every block the
guest decompresses produces the reference output length.

## Compatibility fixes

A small set of offline fixes is keyed by the package's content fingerprint
(FNV-1a over the container bytes), applied to guest memory after load:

- **Gold Miner** (黄金矿工): the title screen's right-soft-key/OK press and
  the options-menu Start Game action both reach the native level
  initialization without the legacy carrier check.
- **Universe Storm** (宇宙风暴): two malformed bitmap headers are repaired as
  their blocks decompress, and the embedded music/bitmap archives are
  decoded from the original package bytes (their image carries no relocation
  entries and SDK state writes can overlap their guest addresses).

The patches are reapplied when a snapshot is restored, so a pre-fix state
keeps the new behavior. The on-disk package and snapshot formats are
unchanged. Fixes are never applied by package name — the fingerprint must
match — and every fix ships with a local-package regression test that fails
with the original guest code restored.

## Save states

The payload is a bincode snapshot of the machine's runtime state wrapped in
a versioned envelope (`save.rs`): the `SFEMSTAT` signature, a little-endian
format version, and a content fingerprint. States from other packages or
older builds are rejected before decoding. Diagnostic counters and the
wrapper table (rebuilt identically from the image at load time) are not
serialized. Save states work in the libretro core; the standalone frontend
exposes the machine through the headless harness instead.

## Headless execution

`sfa_headless` loads the same package and machine used by the desktop
frontend. It can schedule phone-key presses at exact frames
(`--press frame:key`), pump a fixed frame count, dump per-frame framebuffers
(`--dump-fb`) and guest memory ranges (`--dump-mem`), side-load a local file
for download requests (`--serve-asset`), and run with the offline switches.
This is the preferred path for deterministic runtime regression checks. The
standalone frontend's headless screenshot mode (`-S` with `--press` and
`--screenshot-frames`) drives the same machine for the per-title captures in
`docs/images/` (`scripts/batch-screenshots.ps1`).

`sfa_info` inspects a package's header fields, segment layout, and
relocation summary without running anything.

## Testing

`cargo test --workspace` runs the unit tests plus the local-package
regression tests (Gold Miner's two start paths, Universe Storm's first-level
load), which are `#[ignore]`d by default and run when the package path is
provided through their environment variables. Every fixed runtime bug gets a
deterministic regression test that asserts the semantic outcome; the fix is
verified by temporarily reverting it and watching the test fail.
