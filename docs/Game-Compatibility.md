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

Two more switches are available. The app-activate pump (selector 3) is on
by default; `SF_NO_APP_PUMP=1` / `--no-app-pump` turns it off for
diagnostics. `SF_OFFLINE_HTTP=1` / `--offline-http` answers the SDK's
carrier-facing HTTP transactions locally and runs the SDK's own result
parser. Both are described in
[Game-File-Formats.md](Game-File-Formats.md#offline-http-responder-and-the-app-activate-pump).

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
| 五子棋 (trunk) | **playable** | With `--free-play --no-keys` and six left-soft-key presses the title opens its own 開始遊戲 menu and then the wooden gobang board with a working cursor — no carrier page is left on screen |
| 新星际撞球 (StarBall) | session screen | 资费信息获取中 请稍候; the SDK session flow still owns the screen |

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
| 永恒魔塔 | black surface with 是/否 softkeys; no fault |
| 五子棋 | white session page with 确认/返回 (an explicit LSK sequence reaches the board, see below) |
| 新星际撞球 | 温馨提示 / 资费信息获取中 请稍候 |

The dialog bodies are read through the `+0x14C` text-run slot, so the
messages the SDK is showing are now legible. Two distinct waits appear:
a **tariff lookup** (黄金矿工, 新星际撞球, and 五子棋 before its session
page) and a **resource-pack download** (the three 曹操冢 builds and
胭脂戏游龙). Both are network round trips, and both stay on screen
indefinitely because nothing answers the request.

With `--free-play --offline-http` the tariff wait is no longer a dead end:
the SDK consumes the local response and moves on to its own follow-up
dialog ("资源包下载失败，是否重试？…" with 确认/退出 for the three 曹操冢
builds, 三国危城无双 and 胭脂戏游龙). Those titles still need the
resource-pack path, but the HTTP transaction itself completes. All ten
titles finish the 240-frame sweep without a guest fault, including
永恒魔塔 under free play.

### Reaching a playable screen (五子棋)

The purchase/login chain is now bypassable without patching the guest.
From a clean start the boot's selector 1 completes, the app's own screen
state machine is active and the soft keys are consumed:

```sh
sfheadless <game.app> --frames 80 --no-keys --free-play \
  --keys 10 --keys 10 --keys 10 --keys 10 --keys 10 --keys 10
```

Three left-soft-key presses show the 開始遊戲 menu; six show the gobang
board. Input then works on the board: a directional key moves the cursor
and OK places a black stone, with the 玩家 turn indicator updating.
`--no-keys` matters: the harness's automatic bring-up replay also
presses OK every ten frames and cycles the title back into the SDK
session page.

## Free-play provisioning

`SF_FREE_PLAY=1` (or `sfheadless --free-play`) makes the platform
describe a registered, billing-capable handset:

| Slot / file | Without free play | With free play |
|---|---|---|
| `+0x4C` | 1 | 1 |
| `+0x524` identity lookup | 1 (`*out` untouched) | 0, `*out` = dual-shape callable entry |
| `__sfssuid`, `ssu<id>` | refused (probe fails everywhere) | served from the in-memory service store, guest writes read back |
| `__sfnetinfo__` | refused | refused (still an open gap) |
| SDK HTTP transaction | pending forever | locally answered with `--offline-http` |

The identity entry has to be both directly callable and indexable as a
method table. A bare code stub satisfies only the first shape: the other
shape loads a neighbouring stub's literal word, executes platform data and
aborts the whole boot event, leaving the app's active flag unset and its
keys ignored. See
[Game-File-Formats.md](Game-File-Formats.md) for the stublet layout.

Nothing is forced into the guest: the titles still run their own pay
service and decide for themselves whether to open the play state. What
free play removes is the *platform's* "no" answers.

## Known gaps

- **The purchase gate behind the menus** (hjkg): the menu's OK runs the
  SDK pay check whose "paid" outcome feeds the game's own state
  machine; completing the offline receipt chain into that state is the
  next step (see the protocol notes in the repository status file).
- **The resource-pack path is still missing**: `--offline-http` now
  completes the SDK's HTTP transactions (`SFSocketEx:Read`, `Close OK`,
  `SFPaySvc_OnHttpReault`) but answers with an empty body. Titles that
  ask for a resource pack therefore advance to their "download failed,
  retry?" dialog instead of a play screen. Synthesising a valid tariff or
  pack record would cross into inventing server data, so this remains an
  explicit gap. The charge itself is SMS-based (send to `10661098`,
  reply recorded in `__dcsmsinfo_`, both reachable through the file
  service).
- **`__sfnetinfo__` is still refused**: the SDK reads that retry counter
  before posting; re-deriving its record layout would let the pay service
  skip the network round-trip entirely.
- **Free play stays opt-in**: the previously observed 永恒魔塔 fault at
  `pc 0x00000000` no longer reproduces in the 240-frame four-way matrix
  (all ten titles now finish without a guest fault), but the switch has not
  yet been validated against every title's longer interaction path.
- **Sprite resource linking** (宇宙风暴): the guest's sprite table
  entries stay null in gameplay; the resource-link pass needs further
  reverse engineering.
- **The music prompt is a dead end** (永恒魔塔): after the prompt is
  answered the title clears to black and stops painting, and its
  artwork never passes through a blit slot — decompressed buffers are
  discarded before they reach the display bridge.
- **新星际撞球 is still in the SDK session flow**: it stops at the
  资费信息获取中 notice. 五子棋 is no longer blocked — a deterministic
  soft-key sequence reaches its playable board.
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
sfheadless <game.app> --frames 30 [--res <res.app>] [--dump-fb <dir>] \
  [--free-play] [--offline-http] [--no-app-pump] [--no-keys] [--keys 10]
```

`--dump-fb` writes per-frame RGB565 dumps for offline comparison.
`--dump-mem addr:len:path` (hex) exports a guest memory range, including
the framebuffer at `540720:25800`.

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
- `SF_OFFLINE_HTTP=1` — answer the SDK's HTTP transactions locally
  (`sfheadless --offline-http` does the same)
- `SF_NO_APP_PUMP=1` — suppress the per-frame selector-3 event
  (`sfheadless --no-app-pump` does the same)
- `SF_SEED="addr:val;..."` — write guest words once after boot
- `SF_FORCE="addr:val;..."` — write guest words every frame
- `SF_VARS="addr,..."` — log guest words per frame around the tick
- `SF_PCTRACE="lo-hi"` — trace guest fetches inside an address range
- `SF_FSKIP="addr:val;..."` — function-entry skips (PC reaches `addr`
  → return with r0 = val); the SDK SMS send method uses this
- `SF_FRAME_DUMP` (libretro test) — dump the core's presented frame
