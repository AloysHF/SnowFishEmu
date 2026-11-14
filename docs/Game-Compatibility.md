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
| 黄金矿工 (hjkg) | renders + input | Full title screen (signboard art, 黄金矿工, the miner, 选项/开始 softkeys); keys walk the menu into the pay-flow pages |
| 新鬼吹灯 (Mengde pe) | renders | Full title screen: cave background, the 鬼吹灯 logo, 拯救美少女, the menu rows and the softkey bar |
| 宇宙风暴 (yzfb) | renders | "是否打开音乐?" prompt over the striped backdrop with the note icon and the 是 softkeys |
| (终身)三国危城无双 | renders | SDK 温馨提示 waiting dialog: title bar, body panel, 确认/退出 |
| 巨作RPG曹操冢 (Mengde) | renders | SDK 温馨提示 waiting dialog |
| 曹操冢超豪华版 (Mengde i) | renders | SDK 温馨提示 waiting dialog |
| 胭脂戏游龙 (yzxyl) | renders | SDK 温馨提示 waiting dialog |
| 永恒魔塔 (yonghengmota) | prompt only | Clears to black, prints 是否开启音乐 with 是/否 softkeys; the artwork never reaches the bridge (the title issues no blit call at all) |
| 五子棋 (trunk) | session screen | White page with the 确认/返回 softkeys: the title sits in the SDK session flow and has not entered its own menu |
| 新星际撞球 (StarBall) | session screen | Same as 五子棋 |

Every title now paints through the platform's UI timer: all ten arm
`+0xAC` during create and drive their screens from the callback's
selector-10 re-entry. The frame loop runs them on a millisecond clock
(`FRAME_MS`), without which no title painted past its background colour.

## Known gaps

- **The purchase gate behind the menus** (hjkg): the menu's OK runs the
  SDK pay check whose "paid" outcome feeds the game's own state
  machine; completing the offline receipt chain into that state is the
  next step (see the protocol notes in the repository status file).
- **Sprite resource linking** (宇宙风暴): the guest's sprite table
  entries stay null in gameplay; the resource-link pass needs further
  reverse engineering.
- **The music prompt is a dead end** (永恒魔塔): after the prompt is
  answered the title clears to black and stops painting, and its
  artwork never passes through a blit slot — decompressed buffers are
  discarded before they reach the display bridge.
- **The session flow blocks 五子棋/新星际撞球**: both stop at the SDK's
  login/session screen, which paints only the softkey row.
- **Dialog body text** is missing in the 温馨提示 dialogs: the frame and
  the title render, the message body is drawn by a path that emits no
  text slot call.
- **Label anchoring**: the SDK anchors some labels by their centre
  column (dialog titles sit at `width / 2`) and others by their left
  edge (softkeys sit at `width - 5`). The bridge keeps the literal
  left-edge reading and slides a label back inside the surface, so
  titles land right of centre.

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
