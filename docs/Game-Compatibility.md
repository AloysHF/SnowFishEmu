# Game Compatibility

Status of the SnowFish application corpus against the current build.
All titles boot and execute without faults; the notes describe where
each title stands and the input recipe that reaches its play state.

The billing service reports "present" by default (the +0x4C slot
returns 1): the platform is a handset with a SIM, so the SDK's pay
service runs its whole SMS pass instead of exiting at the first check.

`--free-play` additionally answers the subscriber-identity lookup and
backs the account record files with the service store — i.e. the
platform claims to be a *registered* handset. It does not patch the
guest: the title still has to accept the answers and open its own play
state. `--offline-http` answers the SDK's carrier-facing HTTP
transactions locally and runs the SDK's own result parser. The
app-activate pump (selector 3) is on by default;
`--no-app-pump` turns it off for diagnostics.

## Corpus status

| Title | Status | Notes |
|---|---|---|
| 五子棋 (trunk) | **playable** | The wooden gobang board with a working cursor; directional keys move it and OK places a stone |
| (终身)三国危城无双 | **playable** | The game's own splash (dragon art, 危城) and then the tower-defence tutorial with its menu (返回主菜单/优先完成) |
| 巨作RPG曹操冢 (Mengde) | **playable** | Gameplay with the help dialog text readable (如当前武器为铲子时，可长按确定/5号键挖地) |
| 曹操冢超豪华版 (Mengde i) | **playable** | Gameplay; the help bar renders (按数字键3可以查看大地图显示) and the sprite pages load |
| 永恒魔塔 (yonghengmota) | **playable** | The magic-tower floor renders with the story dialog fully readable (宙斯：好女儿，乖女儿，就让我一次嘛…), the characters and the floor counter |
| 胭脂戏游龙 (yzxyl) | **playable** | The 西门庆居 scene, the dialog box (与春梅对话) and the movement tutorial (使用8,2,4,6可以进行移动) |
| 新星际撞球 (StarBall) | **playable** | The brick board, the ball and the paddle run; the top score strip draws the whole digit atlas instead of the score (cosmetic, see the known gaps) |
| 新鬼吹灯 (Mengde pe) | **playable** | The full title screen; OK opens the story intro (scrolling, wrapped) and then the menu (新的游戏/继续游戏/帮助/退出) and the battle |
| 黄金矿工 (hjkg) | renders + charge screen | The title screen and the main menu render. With the offline receipt recipe (see below) 开始游戏 walks the pay flow through the login form into the tariff wait, and with the send-record seed and the dispatch-phase seed the send driver runs and the flow reaches the SDK's 付费中 charge screen — the deepest offline run. The remaining wall: the SDK validates the platform's subscriber-identity record (an XOR-0xF5-encoded, comma-separated, checksummed 8-field string) and shows 无效的sim卡 when it does not parse |
| 宇宙风暴 (yzfb) | renders | The full title screen (宇宙风暴 logo, the earth, the astronaut — the palette blit now draws index-0 art). The game menu behind it draws sprite handles that are still empty: the menu's own images load only after the pay/session flow advances, and the same session-file wall applies |

Every title now paints through the platform's UI timer: all ten arm
`+0xAC` during create and drive their screens from the callback's
selector-10 re-entry. The frame loop runs them on a millisecond clock
(`FRAME_MS`), without which no title painted past its background colour.

## Reaching the play state

The SDK's pay/session screens sit between the title and the game's own
menu in most titles. A scripted left-soft-key replay walks through them;
`--no-keys` disables the harness's automatic bring-up replay, which
presses OK every ten frames and can cycle a title back into the SDK
session page.

```sh
sfheadless <game.app> --frames 200 --free-play --offline-http --no-keys \
  --press 30:10 --press 60:10 --press 90:10 --press 120:10 --press 150:10 --press 180:10
```

That recipe (six left-soft-key presses over 200 frames) lands 五子棋 on
its board, 三国危城无双 on its splash/tutorial, both 曹操冢 builds and
永恒魔塔 in gameplay, and 胭脂戏游龙 on its first scene. For 新鬼吹灯 the
advancing key is OK (`--press 40:5 ...`): the title opens the story
intro, and further OK presses finish it into the menu. For 新星际撞球
three presses over 500 frames suffice — its tariff wait dismisses
itself and the game starts.

### The hjkg charge screen

黄金矿工's menu 开始游戏 runs the paid-mode check, and with the offline
receipt machinery the flow walks the login form into the tariff wait
and then — with the send-record seed — the SDK's own 付费中 charge
screen:

```sh
SF_FAKE_PAY=1 SF_4C="1084a0:1;1094bc:1;106fb0:1" \
SF_DCSMS="ff,ff,ff,ef,ff,7c,ff,ff,ff,ff,ff,ff,ff,ff,ff,ff" \
SF_NETINFO="ff,ff,ff,cf,ef" SF_SENDINFO="ff,ff,ff,b5" \
SF_FSKIP="118fc4:1" SF_DCSMS_LOOP=1 \
SF_FORCE="12f5a4:2;12f578:5;12f5ba:2" \
sfheadless hjkg.app --frames 900 --free-play --offline-http --no-keys \
  --press 40:10 --press 60:5 --press 200:10 --press 400:10
```

`SF_SENDINFO` seeds the send record (the encoded dword 164 — the ssu
id). `SF_FORCE` seeds the dispatch phase cell (0x12F5A4 = 2, the gate
that lets the per-tick dispatch reach the response machine), the queue
head (0x12F578 = 5, the send/parse dispatch case) and the send-state
byte (0x12F5BA = 2, what the receipt action's screen-advance block
wants). `SF_FSKIP` returns the SDK's SMS send method (0x118FC4) as
accepted — without it the send runs the full socket dance and the
response parser walks the planted transaction's garbage body pointer.

`SF_4C` per site: 0x1094bc is the paid-mode check inside CheckSms
(returning 0 there means "free" and parks the title on the menu with
the pay flow never starting — but the title's own OK path then fails
the pay attempt on the next check); 0x1084a0 and 0x106fb0 keep the
service present.

## Known gaps

- **The hjkg charge completion**: the flow reaches the 付费中 charge
  screen and waits for the carrier reply. The reply path (the charge
  POST's response parse) needs the planted async transaction to carry
  a real body — the response grammar is decoded (sections with a
  `b1*b2 == b0` check, the ssu dword, then a tag stream whose last tag
  byte lands in the queue-head cell), but the transaction completion
  plumbing that hands the body to the parser is not wired yet.
- **The identity record** (hjkg): the SDK checksums an XOR-0xF5-encoded
  comma-separated field list from the platform's identity service and
  shows 无效的sim卡 when it does not parse. The encoder and the record
  callable exist (SF_IDENTITY), but the parsed fields then feed the
  SDK's encoded-stream parsers, so the record stays behind
  SF_RECORD_LOOKUP until that path is understood.
- **The yzfb session wall**: after the game-start press the SDK's pay
  callback re-registers itself every frame and nothing paints. Its
  address set (callback 0x112374, the pay-state/queue-head pools) needs
  the same per-title decode the older corpus notes describe.
- **StarBall's score strip**: the HUD draws the whole 90x12 digit atlas
  per score panel instead of a digit slice — the score reads as
  `0123456789`. The game itself (board, ball, paddle) runs.
- **Ghosting behind sliding menus** (新鬼吹灯): menu items and softkey
  labels drawn while a widget animates leave earlier positions on the
  surface, because the bridge renders directly into the display instead
  of compositing an off-screen canvas at the commit rect. The reference
  bridge shows the same artifact.
- **`__sfnetinfo__` is still refused**: the SDK reads that retry counter
  before posting; re-deriving its record layout would let the pay
  service skip the network round-trip entirely.
- **Label anchoring**: the bridge keeps the literal left-edge reading
  the reference uses and slides a label back inside the surface when it
  would overflow, so centre-anchored prompts (x = width/2) start at the
  centre column rather than being centred around it.

## Reproducing

The headless harness drives the same machine without a window:

```sh
sfheadless <game.app> --frames 30 [--res <res.app>] [--dump-fb <dir>] \
  [--free-play] [--offline-http] [--no-keys] [--keys 10] [--press frame:key]
```

`--res` is repeatable: a title that ships two resource containers takes
both, and a download request naming a pack is answered with that pack's
own bytes. `--dump-fb` writes per-frame framebuffer dumps for offline
comparison (one 32-bit-per-pixel word per pixel). `--dump-mem
addr:len:path` (hex) exports a guest memory range, including the
framebuffer at `540720:25800` (raw RGB565).

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
- `SF_DCSMS="hex,hex,..."` — the encoded response record injected into
  the `__dcsmsinfo_` inbox
- `SF_DCSMS_LOOP=1` — re-arm the inbox injection on every empty open
- `SF_SENDINFO="hex,hex,..."` — the encoded send-record seeded into
  `__sfsmspaysendinfo_` (re-armed on every empty open); its first
  decoded dword (>0) is what makes the pay dispatch run the send
  driver
- `SF_IDENTITY="f1,f2,..."` — the plain subscriber-identity field
  list; the core XOR-0xF5-encodes it with the trailing checksum and
  installs the record callable. Experimental: also needs
  `SF_RECORD_LOOKUP=1`, because the SDK's record parse feeds plain
  text into its encoded-stream parsers
- `SF_TRACE` — log every platform slot and GDI operation
- `SF_FRAME_DUMP` (libretro test) — dump the core's presented frame
