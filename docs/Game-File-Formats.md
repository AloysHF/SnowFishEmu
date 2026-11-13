# SnowFish/SFA Application Format

SnowFish applications use the **SFA** container format. This document
describes the file structure, the guest memory map the loader prepares,
and the LZ stream format used by the platform decompression service.

## Container layout

Each application is a single `.app` file:

```
┌──────────────────────────────────┐
│ Header (0x20 or 0x24 bytes)      │
├──────────────────────────────────┤
│ Code image (code + bss sizes)    │
├──────────────────────────────────┤
│ Relocation table (rotab_n words) │
└──────────────────────────────────┘
```

### Header

| Offset | Size | Field |
|---|---|---|
| 0x00 | 4 | Magic: `0x19414653` (`SFA\x19`) or `0x19414654` (`TFA\x19`) |
| 0x04 | 4 | Version (0 or 1) |
| 0x08 | 4 | Reserved (resource / res) |
| 0x0C | 4 | Reserved (res) |
| 0x10 | 4 | Header length (0x20, or 0x24 for the TFA variant) |
| 0x14 | 4 | Code segment size (`c0`) |
| 0x18 | 4 | BSS size (`c1`) |
| 0x1C | 4 | Relocation entry count (`rotab_n`) |

The code image starts at `hdr_len`. The relocation table follows the
image: `rotab_n` little-endian u32 offsets into the loaded image. Each
relocation adds the code base (`0x101000`) to the word at
`code_base + offset` — the image is position-independent and links
itself to the code base at load time.

Some packages carry an extra sibling resource file (matched by stem);
the guest's file service serves both the container and the resource
file.

## Guest memory map

The loader maps the image and prepares the service tables:

| Region | Base | Size | Purpose |
|---|---|---|---|
| Code | `0x00100000` | up to 2 MB | Image loaded at `0x101000` |
| Stack | `0x00300000` | 1 MB | Grows down from `SP = 0x400000` |
| App data | `0x00440000` | 1 MB | Guest globals |
| Platform | `0x00540000` | 256 KB | Root API table + service state |
| Heap | `0x00600000` | 1 MB | Guest allocations |

Platform region layout:

| Address | Purpose |
|---|---|
| `0x540000` | Root API table (52 slots, 4 bytes each) |
| `0x540600` | Platform registers |
| `0x540640` | Vtable |
| `0x540650` | Application record |
| `0x540690` | Passthrough stub area |
| `0x540710` | Canvas descriptor |
| `0x540720` | Framebuffer (240x320 RGB565) |
| `0x565F20` | Memory descriptor |
| `0x565F30` | State cell |

Loader-planted words around the image:
`*(code_base-8) = reg`, `*(code_base-4) = root`,
`*(code_base+0xC0) = root`, `*(image_end-8) = root`,
`*(code_base+0xBC) = memdesc`.

### Service dispatch

Guest code calls platform services through generated wrapper stublets:

```
ldr   rX, [rX, #-4]        ; hook address (the interception point)
mov   lr, pc
ldr   pc, [rX, #slot]      ; indirect call through the root table
add   sp, sp, #adj         ; caller stack args popped by callee
pop   {pc}                 (or equivalent return)
```

The emulator scans the image for these stublets, so each call site is
interceptable with its own stack-argument layout. Slots the emulator
implements natively are handled at the hook address; every other slot
executes through the root-table stubs unchanged.

### Root API table slots

| Slot | Service |
|---|---|
| `+0x010` | GDI flush |
| `+0x058` | File open (UTF-16 path) |
| `+0x05C` | File read |
| `+0x064` | File seek |
| `+0x068` | File close |
| `+0x07C` | Log (format string at r0) |
| `+0x108` | LZ decompression |
| `+0x12C` | GDI fill(obj, x, y, w, h, color) |
| `+0x15C` | GDI fill with canvas color |
| `+0x184` | GDI clear / pen color |
| `+0x194` | GDI bitmap register |
| `+0x198` | GDI draw (bitmap descriptor blit) |
| `+0x4C` | Returns 1 |
| `+0x500` | Timer start (id, callback, context) |
| `+0x524` | Stores the memory descriptor pointer, returns 1 |

### GDI bitmap descriptor

Draw calls take a descriptor at r1:

| Offset | Size | Field |
|---|---|---|
| 0x00 | 4 | Source width (`sw`) |
| 0x04 | 4 | Source height (`sh`) |
| 0x08 | 4 | Color key (255 = opaque) |
| 0x0C | 4 | Palette pointer (a pointer selects palette mode) |
| 0x10 | 4 | Pixel data pointer (fallback: `*(desc+0x14)`) |

Palette mode stores one byte per pixel indexing a 256-entry RGB565
palette; direct mode stores RGB565 words.

## LZ stream format

The platform decompression service takes a stream with a literal-run
backbone and five match encodings. The decoder is a two-state machine:

- **dispatch** — a token byte `F`:
  - `F > 0x3F`: short match (X)
  - `0x20..=0x3F`: medium match (A2)
  - `0x10..=0x1F`: far match (W)
  - `F <= 0xF`: near copy (MidA)
- **litcheck** — `F > 0xF` returns to dispatch; `F <= 0xF` emits a
  literal run of `F + 3` bytes (`F == 0` extends through a 0xFF chain
  then `last + 0xF`); the next `F <= 0xF` is a far copy (MidB)

Token anatomy (`ip` = index of the first byte after the token byte;
`p` = index past the last offset byte; offset bytes are read without
advancing `ip`):

| Class | Bytes | Length | Distance | Trailing literals |
|---|---|---|---|---|
| W | `F b0 b1` | `(F&7)` (0 → 0xFF chain, then `b+7`) + 2 | `0x4000*(F&8?1:0) + (b0>>2) + (b1<<6) + 0x4000` | `b0 & 3` |
| A2 | `F b0 b1` | `(F&0x1F)` (0 → chain, then `b+0x1F`) + 2 | `(b0>>2) + (b1<<6) + 1` | `b0 & 3` |
| X | `F ob` | `((F>>5)-1) + 2` | `((F&0x1C)>>2) + (ob<<3) + 1` | `F & 3` |
| MidA | `F ob` | 2 | `(F>>2) + (ob<<2) + 1` | `F & 3` |
| MidB | `F ob` | 3 | `(F>>2) + (ob<<2) + 1 + 0x801` | `F & 3` |

After each match the trailing literal count is re-read from `src[p-2]`
(the class's first offset byte, or the token byte itself for the
one-offset-byte classes), the trailing bytes are emitted from
`src[p..]`, and the next token byte is read sequentially at
`src[p+count]`. The next state is **dispatch** when trailing literals
were emitted, **litcheck** when none were. Init: `F > 0x11` emits
`F - 0x11` literals and requires the next byte `> 0xF`; `F <= 0x11`
enters litcheck directly. Matches copy byte-by-byte from the output, so
overlapping ranges replicate. The stream ends with a W token whose
computed distance cell equals the current output length; its position
after the offset bytes must equal the stream length exactly and its
length field must be 1.
