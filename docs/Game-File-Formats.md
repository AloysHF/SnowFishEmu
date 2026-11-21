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
| `+0x0AC` | UI timer arm(owner, period ms, callback) |
| `+0x108` | LZ decompression |
| `+0x12C` | GDI update(obj, x, y, w, h) — commit a rectangle |
| `+0x130` / `+0x140` | Long-form fill variants (dialog chrome rows) |
| `+0x134` | GDI blit(obj, bmp, x, y, flags) — bitmap at its own size |
| `+0x14C` | GDI text run(obj, u16 string, first, count, x, y, flags) |
| `+0x154` | GDI text(obj, u16 string, x, y, font) |
| `+0x15C` | GDI fill(obj, x, y, w, h) with the canvas pen |
| `+0x180` | GDI set-color(obj, rgb888) |
| `+0x184` | GDI clear / pen color |
| `+0x194` | GDI bitmap register |
| `+0x198` | GDI draw(obj, bmp, sx, sy, w, h, _, dx, dy, _) |
| `+0x4C` | Carrier-billing capability query (1 = phone can bill) |
| `+0x500` | Timer start (id, callback, context) |
| `+0x524` | Service lookup (see below) |
| `+0x0C8` | Socket select(host, event record, handler, object) |
| `+0x0CC` | Socket connect |
| `+0x0D4` / `+0x0D8` / `+0x0DC` | Socket wrapper setup helpers |
| `+0x0E0` | Socket recv(handle, buffer, length) -> bytes read |
| `+0x0E4` | Socket write(handle, buffer, length) -> bytes written |
| `+0x0E8` | Attach a socket to the asynchronous notifier |
| `+0x0EC` | Look up the socket object attached to a handle |
| `+0x0F0` | Open / select the data bearer (mode 0, 1 or 2) |
| `+0x11C` | Read the current date/time into a guest struct |
| `+0x568` | Report socket state (error code in r2) |
| `+0x56C` | Release/decrement an async request |
| `+0x570` | Register an async request descriptor |

`+0x14C` is the sibling of `+0x154` used by dialog bodies. The guest has
already computed its own line breaks — it walks the UTF-16 string,
accumulates each cell's advance and breaks when the next cell would pass
the right margin — so the call carries a *character range* rather than a
pixel pointer: `first` is the index of the line's first code unit,
`count` is its length, and the last two register arguments are the text
origin the block was laid out at. The platform only has to write the run
at that origin with the canvas pen; dropping the slot leaves the dialog
frame painted and the body blank, which is what every notice looked like
before it was served.

`+0x12C` takes five arguments; the word that follows the height is *not*
a colour. Titles leave stale caller data there (code pointers, string
pointers, the residue of an earlier call), so the slot is served as a
dirty-rectangle commit: the guest has already drawn into the surface and
is asking the platform to push that rectangle to the display.

`+0x4C` is the purchase service's first question: *can this handset bill
over the carrier channel?* The SDK asks once at boot and every title
gates its own state machine on the answer. A zero makes
`SFPaySvc_CheckSms` return before it looks anything up and leaves most of
the corpus parked on its pay page, so the platform answers 1 — it is a
phone with a SIM.

`+0x524` is the generic service lookup and takes two shapes:

| Call | Meaning | Return / out-pointer |
|---|---|---|
| small `type` (≤ 0x100) | service object request | 1, `*out` = object whose table entries are callable |
| SDK hash as `type` | subscriber-identity (ICCID) lookup | 0 = found, `*out` = a *callable* entry; 1 = missing |

The identity form matters because a 1 makes the SDK log `GETICCID err`
and drop to the registration page. The returned entry has to satisfy two
different call shapes used by different revisions of the SDK:

- `mov lr, pc; ldr pc, [entry]` — the entry itself is jumped to.
- `ldr rX, [entry_holder]; mov lr, pc; ldr pc, [rX, #0x50]` — the entry is
  treated as a method table and one of its words is jumped to.

Handing back a bare code stub works for the first shape only. Indexing the
shared stub area reads a neighbouring value trampoline's literal word
(the canvas pointer), so the guest executes platform data and the whole
boot event aborts at the stop sentinel — the title then never sets its
own active flag and its screen state machine stops accepting keys. The
platform therefore plants a dedicated stublet whose first two words are a
return-0 trampoline and whose remaining words all point back at that
trampoline, so both shapes land on real code.

### Account record files

The registration state lives in two plain files the SDK probes on every
drive root before it starts the purchase flow:

| Name | Meaning |
|---|---|
| `__sfssuid` | Primary account record |
| `ssu<digits>` | Per-title record (for example `ssu210`) |

A record is a big-endian length dword followed by the payload, and the
whole thing is byte-scrambled: each byte is nibble-swapped and then
inverted (`b -> !swap_nibbles(b)`), which is self-inverse. A decoded
payload starts with a record count and then fixed-size entries of five
big-endian halves plus a `0xFC` terminator byte.

`__sfnetinfo__` is a third platform file. Despite its name it does not
contain a cached tariff: it is the purchase service's retry counter. Like
the account records it is a byte-scrambled big-endian record. After
decoding, the leading dword becomes the retry counter (clamped to zero if
greater than 3) and the first payload byte is the "result callback is
registered" flag. The SDK increments the counter on each post attempt and
takes its own give-up branch once it passes two. The file also carries a
`%4d-%2d-%2d %2d:%2d:%2d` date string used by the write side.

### Network and socket slots

The purchase service reaches the network through these slots in order.
The shapes below are the ones the SDK actually calls; the generic
return-0 stubs currently answer most of them.

| Slot | Shape | Notes |
|---|---|---|
| `+0x0F0` | `bearer(mode)` | mode 1 or 2 from the pay retry state, 0/1/2 from the connection setup |
| `+0x570` | `register(request_descriptor)` | r0 points at `[status, callback, context]`; status starts at `-2` (pending) |
| `+0x0C8` | `select(host, event_record, handler, object)` | see below |
| `+0x0CC` | `connect(..., &err)` | a zero result is treated as success |
| `+0x0E8` | `attach(handle, object)` | registers the socket for the async notifier |
| `+0x0EC` | `lookup(handle)` | returns the object registered by `+0xE8`; returning 0 drops every async socket event |
| `+0x0E4` | `write(handle, buffer, length)` | **returns the byte count**; -1 is the error sentinel |
| `+0x0E0` | `recv(handle, buffer, length)` | **returns the byte count** and fills the guest buffer; 0 is EOF |
| `+0x568` | `state(object, 0, err)` | `0x80001005` connected, `0x80001002` failed |
| `+0x11C` | `get_date_time(struct)` | polled every frame |

`+0x570` is the async completion registration, not a plain callback
setter. The SDK reuses the same descriptor address for each new request,
so a platform that stands in for the network has to treat a fresh
registration after a consumed response as a new transaction, serve a full
response through `+0xE0`, mark the descriptor's status done and then call
the descriptor's callback. Calling the callback before the response has
been consumed is invalid: the guest fetches a completed transaction
object through the HTTP manager's vtable and jumps through a null entry.

`+0x0C8` is a *synchronous* notifier. When the slot returns zero the SDK
immediately calls `handler(object, event_record)`; the handler reads a
signed halfword at the start of the event record, where `1` means
"connected" (the request is then built and written) and any other value
routes to the socket-failure/retry path. A real platform re-enters that
handler for each state change, so the event record is the delivery
mechanism, not the slot's return value.

The request the SDK writes is a normal HTTP/1.0 POST:

```
POST http://service2.appgames.cn/getmt.do?method=sendmt HTTP/1.0
Host: service2.appgames.cn:80
Content-Length: 52
User-Agent: Nokia6600/... Series60/2.0 Profile/MIDP-2.0 Configuration/CLDC-1.0
Connection: Keep-Alive
Content-Type: Application/octet-stream
```

The registered completion callback fetches a *completed transaction
object* through the HTTP manager's vtable, then checks its status word:
200..299 enters the body parser, anything else closes the transaction and
runs the failure handler. Calling that callback without a completed
transaction is not valid — the guest jumps through a null vtable. Without
a local answer the request is written but no response object ever
arrives, which is what historically parked titles on their wait screen.

### Offline HTTP responder and the app-activate pump

Two optional platform facilities close that loop:

- The **offline HTTP responder** (`SF_OFFLINE_HTTP=1`,
  `sfheadless --offline-http`, `set_offline_http(true)`) hooks only the
  socket/notifier slots that are otherwise answered by the generic
  return-0 stub. `+0xC8` reports a connected event record, `+0xE4`
  reports the byte count, `+0xE8`/`+0xEC` keep the attached socket object,
  and `+0xE0` serves a payload-free `HTTP/1.0 200 OK` followed by EOF.
  Once the response has been fully read the frame loop marks the `+0x570`
  descriptor done and re-enters the SDK's completion callback, which runs
  its own result parser (`SFPaySvc_OnHttpReault`). An SDK session issues
  more than one request and reuses the same descriptor address, so each
  fresh registration starts a new transaction. The responder is disabled by
  default and never synthesises server-side tariff data.
- The **app-activate pump** posts selector 3 (`EVT_APP_RESUME`) every
  frame. That selector re-arms the guest's 120 ms UI timer, refreshes its
  date/time record and drives the screen state machine; without it some
  titles stay on their loading screen even after the purchase flow has
  completed. It is on by default and can be turned off with
  `SF_NO_APP_PUMP=1`, `--no-app-pump` or `set_app_pump(false)`.

`+0x134` and `+0x198` consume the same bitmap descriptor. `+0x198` takes
the source origin `(sx, sy)` in r2/r3 and the crop dimensions `(w, h)`
on the stack. `+0x134` draws the bitmap at its own size.

`+0x0AC` returns a handle and arms a repeating callback. The callback is
a trampoline that treats r0 as the SDK's delayed-message record
`(app block, message id, delay, flag)` and re-enters the app handler with
selector 10; the platform therefore passes the record it finds in the
image data segment, not the caller's context object.

### GDI bitmap descriptor

Draw calls take a descriptor at r1:

| Offset | Size | Field |
|---|---|---|
| 0x00 | 4 | Source width (`sw`) |
| 0x04 | 4 | Source height (`sh`) |
| 0x08 | 4 | Color key or opaque sentinel |
| 0x0C | 4 | Palette pointer (a pointer selects palette mode) |
| 0x10 | 4 | Pixel data pointer (fallback: `*(desc+0x14)`) |

Palette mode stores one byte per pixel indexing a 256-entry RGB565
palette. Direct mode stores RGB565 words. Universe Storm's palette
descriptors use index 0 as a valid color key and `0x7fffffff` for an
opaque bitmap.

## LZ stream format

The LZ service reads ordinary inputs from guest memory. For the supported
Universe Storm package, it reads the embedded, unrelocated music and
bitmap archives from the original package bytes. Guest memory remains the
destination for decompressed data.

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
