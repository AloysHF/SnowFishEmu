# Game Compatibility

Status of the SnowFish application corpus against the current build.
All titles boot and execute; the notes describe what renders and where
each title currently stops.

The billing service reports "present" by default (the +0x4C slot
returns 1): the platform is a handset with a SIM, so the SDK's pay
service runs its whole SMS pass instead of exiting at the first check.
Titles that still cannot finish it fall back to their own notice dialogs
rather than the billing wait screen.

`SF_FREE_PLAY=1` (or `sfheadless --free-play`) additionally answers the
subscriber-identity lookup and backs the account record files with the
service store — i.e. the platform claims to be a *registered* handset.
It does not patch the guest: the title still has to accept the answers
and open its own play state. See [Free-play provisioning](#free-play-provisioning).

## Corpus status

| Title | Status | Notes |
|---|---|---|
| 黄金矿工 (hjkg) | renders + input | Full title screen (signboard art, 黄金矿工, the miner, 选项/开始 softkeys); keys walk the menu into the pay-flow pages |
| 新鬼吹灯 (Mengde pe) | renders | Full title screen: cave background, the 鬼吹灯 logo, 拯救美少女, the menu rows and the softkey bar |
| 宇宙风暴 (yzfb) | renders | "是否打开音乐?" prompt over the striped backdrop with the note icon and the 是 softkeys |
| (终身)三国危城无双 | renders | SDK 温馨提示 / 资源包下载中 |
| 巨作RPG曹操冢 (Mengde) | renders | SDK 温馨提示 / 资源包下载中 |
| 曹操冢超豪华版 (Mengde i) | renders | SDK 温馨提示 / 资源包下载中, 是/否 |
| 胭脂戏游龙 (yzxyl) | renders | SDK 温馨提示 / 资源包下载中 |
| 永恒魔塔 (yonghengmota) | prompt only | Clears to black, prints 是否开启音乐 with 是/否 softkeys; the artwork never reaches the bridge (the title issues no blit call at all) |
| 五子棋 (trunk) | session screen | White page with the 确认/返回 softkeys: the title sits in the SDK session flow and has not entered its own menu |
| 新星际撞球 (StarBall) | session screen | Same as 五子棋 |

Every title now paints through the platform's UI timer: all ten arm
`+0xAC` during create and drive their screens from the callback's
selector-10 re-entry. The frame loop runs them on a millisecond clock
(`FRAME_MS`), without which no title painted past its background colour.

### Where a 240-frame key sweep ends

The table above describes the first stable screen each title reaches.
Driving the bring-up key replay for 240 frames moves most of them on:

| Title | End state after 240 frames |
|---|---|
| 黄金矿工 | 温馨提示 / 资费信息获取中……请稍候 (tariff lookup) |
| 新鬼吹灯 | title screen artwork, unchanged |
| 宇宙风暴 | black surface, no slot calls |
| (终身)三国危城无双 | 温馨提示 / 资源包下载中，请稍候 (resource pack) |
| 巨作RPG曹操冢 | 温馨提示 / 资源包下载中，请稍候 |
| 曹操冢超豪华版 | 温馨提示 / 资源包下载中，请稍候, 是/否 |
| 胭脂戏游龙 | 温馨提示 / 资源包下载中，请稍候 |
| 永恒魔塔 | guest call fault at `pc 0x00000000` |
| 五子棋 | 温馨提示, empty body, 确认/返回 |
| 新星际撞球 | 温馨提示 / 资费信息获取中 请稍候 |

The dialog bodies are read through the `+0x14C` text-run slot, so the
messages the SDK is showing are now legible. Two distinct waits appear:
a **tariff lookup** (黄金矿工, 新星际撞球, and 五子棋 before its session
page) and a **resource-pack download** (the three 曹操冢 builds and
胭脂戏游龙). Both are network round trips, and both stay on screen
indefinitely because nothing answers the request.

So the corpus converges on the SDK's own notice dialog: the titles boot,
paint, take input, run the purchase service and then stop on a message
the platform never answers. With `SF_FREE_PLAY=1` the same sweep turns
those dialogs into the SDK's loading screen (五子棋: `读取中` over the
dark page) or a blank surface — the gate moved from "ask the user" to
"wait for the network", which is the next thing to answer.

## Free-play provisioning

`SF_FREE_PLAY=1` (or `sfheadless --free-play`) makes the platform
describe a registered, billing-capable handset:

| Slot / file | Without free play | With free play |
|---|---|---|
| `+0x4C` | 1 | 1 |
| `+0x524` identity lookup | 1 (`*out` untouched) | 0, `*out` = callable entry point |
| `__sfssuid`, `ssu<id>` | refused (probe fails everywhere) | served from the in-memory service store, guest writes read back |
| `__sfnetinfo__` | refused | refused (still an open gap) |

Nothing is forced into the guest: the titles still run their own pay
service and decide for themselves whether to open the play state. What
free play removes is the *platform's* three "no" answers, which are the
reason the service stops early.

## Known gaps

- **The purchase gate behind the menus** (hjkg): the menu's OK runs the
  SDK pay check whose "paid" outcome feeds the game's own state
  machine; completing the offline receipt chain into that state is the
  next step (see the protocol notes in the repository status file).
- **The SDK network stack is unanswered**: `+0xE4`/`+0xE8`/`+0xEC`/
  `+0xF0` (request post) and `+0x568`/`+0x570` (socket) run the planted
  stub, so a pay request that reaches the network layer never gets a
  response. With free play on, 五子棋 sits on `读取中` and 新星际撞球 on
  its logo because of this, not because of the platform's answers.
- **`__sfnetinfo__` is still refused**: the SDK reads the cached network
  descriptor before posting, and re-deriving its record layout is what
  would let the pay service skip the network round-trip entirely.
- **Free play is not uniformly safe yet**: with `SF_FREE_PLAY=1` and the
  240-frame key sweep, 永恒魔塔 takes a guest call fault at
  `pc 0x00000000` (it does not fault without free play), so the switch
  needs per-title validation before it can be enabled by default.
- **Sprite resource linking** (宇宙风暴): the guest's sprite table
  entries stay null in gameplay; the resource-link pass needs further
  reverse engineering.
- **The music prompt is a dead end** (永恒魔塔): after the prompt is
  answered the title clears to black and stops painting, and its
  artwork never passes through a blit slot — decompressed buffers are
  discarded before they reach the display bridge.
- **The session flow blocks 五子棋/新星际撞球**: both stop at the SDK's
  login/session screen, which paints only the softkey row.
- **Label anchoring**: the SDK anchors some labels by their centre
  column (dialog titles sit at `width / 2`) and others by their left
  edge (softkeys sit at `width - 5`). The bridge keeps the literal
  left-edge reading and slides a label back inside the surface, so
  titles land right of centre.
- **Some dialog labels are still empty**: `+0x14` hands the guest a
  label for a menu item and is not served, so titles that resolve their
  softkeys through it draw blank buttons (五子棋 shows an empty body and
  two unlabelled keys, while 曹操冢 resolves its own strings and shows
  确认/返回).

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

- `SF_4C="lr:val;*:val"` — override the +0x4C billing-capability result
  per call site (hex LRs; `*` sets the base, default 1)
- `SF_FREE_PLAY=1` — answer the purchase service as a registered handset
  (`sfheadless --free-play` does the same)
- `SF_SEED="addr:val;..."` — write guest words once after boot
- `SF_FORCE="addr:val;..."` — write guest words every frame
- `SF_VARS="addr,..."` — log guest words per frame around the tick
- `SF_PCTRACE="lo-hi"` — trace guest fetches inside an address range
- `SF_FSKIP="addr:val;..."` — function-entry skips (PC reaches `addr`
  → return with r0 = val); the SDK SMS send method uses this
- `SF_FRAME_DUMP` (libretro test) — dump the core's presented frame
