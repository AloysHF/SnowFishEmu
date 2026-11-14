# Game Compatibility

Status of the SnowFish application corpus against the current build.
All titles boot and execute; the notes describe what renders and where
each title currently stops.

The billing service reports "present" by default (the +0x4C slot
returns 0): the SDK's boot pay check exits at once and the titles reach
their own menus instead of the billing wait screen.

## Corpus status

| Title | Status | Notes |
|---|---|---|
| 黄金矿工 (hjkg) | menu+payflow | Title screen and the main menu render and respond to keys; the pay-flow pages (screens 0/1/2/3) walk with the offline receipt recipe (see the debug environment reference); the last link is the dialog widget's 确认 action firing the send trigger |
| 宇宙风暴 (yzfb) | boots | Title screen call-verified; gameplay sprites pending resource-link RE |
| (终身)三国危城无双 | boots | The SDK send flow runs end to end (request built, SMS-task object created, response inbox read); the screen chain past the dialog needs deeper protocol layers |
| 五子棋 (trunk) | boots | Single background fill, matching the platform's idle session |
| 巨作RPG曹操冢 (Mengde) | boots | Waiting screen |
| 曹操冢超豪华版 (Mengde i) | boots | Waiting screen (needs both res files served) |
| 新星际撞球 (StarBall) | boots | Waiting screen |
| 新鬼吹灯 (Mengde pe) | boots | Runs clean |
| 胭脂戏游龙 (yzxyl) | boots | Waiting screen |
| 永恒魔塔 (yonghengmota) | boots* | Faults at the same guest fetch as a bare platform session (title-side issue, not emulator) |

## Known gaps

- **The purchase gate behind the menus** (hjkg): the menu's OK runs the
  SDK pay check whose "paid" outcome feeds the game's own state
  machine; completing the offline receipt chain into that state is the
  next step (see the protocol notes in the repository status file).
- **Sprite resource linking** (宇宙风暴): the guest's sprite table
  entries stay null in gameplay; the resource-link pass needs further
  reverse engineering.

## Reproducing

The headless harness drives the same machine without a window:

```sh
sfheadless <game.app> --frames 30 [--res <res.app>] [--dump-fb <dir>]
```

`--dump-fb` writes per-frame RGB565 dumps for offline comparison.

### The hjkg offline receipt recipe (screens 0 -> 1 -> 2 -> 3)

```sh
SF_FAKE_PAY=1 SF_4C="*:1" SF_FSKIP="118fc4:1" SF_DCSMS="ff,ff,ff,ef,ff,7c,ff,ff,ff,ff,ff,ff,ff,ff,ff,ff" SF_FORCE="12f578:5;12f5b4:1;131754:5;1317e4:38;12f5f0:1" sfheadless hjkg.app --frames 150
```

`SF_DCSMS` is the encoded response record (nibble-swap + invert of the
plaintext `[BE dword len][len x BE halfwords] "8"` record — the encode
is self-inverse). `SF_FORCE` seeds the queue head (5 = the pay-flow
message type), the builder stage flag, the pay state, and the send
pattern table that the callback's match loops consume.

### Debug environment variables

- `SF_4C="lr:val;*:val"` — override the +0x4C billing-present result
  per call site (hex LRs; `*` sets the base, default 0)
- `SF_SEED="addr:val;..."` — write guest words once after boot
- `SF_FORCE="addr:val;..."` — write guest words every frame
- `SF_VARS="addr,..."` — log guest words per frame around the tick
- `SF_PCTRACE="lo-hi"` — trace guest fetches inside an address range
- `SF_FSKIP="addr:val;..."` — function-entry skips (PC reaches `addr`
  → return with r0 = val); the SDK SMS send method uses this
- `SF_FRAME_DUMP` (libretro test) — dump the core's presented frame
