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
| 黄金矿工 (hjkg) | renders + login form | The title screen and the main menu render; with the offline receipt recipe (see below) 开始游戏 walks the pay flow through the login form (the fields fill, the submit fires) into the tariff-fetch wait. The wait ends when the SDK's own session file says a send is pending — the last undecoded layer |
| 宇宙风暴 (yzfb) | renders | The full title screen (宇宙风暴 logo, the earth, the astronaut — the palette blit now draws index-0 art). The game menu behind it draws sprite handles that are still empty: the menu's own images load only after the pay/session flow advances |

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

### The hjkg login form

黄金矿工's menu 开始游戏 runs the paid-mode check, and with the offline
receipt machinery the flow walks into the SDK's own login screen:

```sh
SF_FAKE_PAY=1 SF_4C="*:1" SF_FSKIP="118fc4:1" SF_DCSMS_LOOP=1 \
SF_DCSMS="ff,ff,ff,ef,ff,7c,ff,ff,ff,ff,ff,ff,ff,ff,ff,ff" \
SF_FORCE="12f578:5;12f5b4:1;131754:5;1317e4:38;12f5f0:1;12cf84:0" \
sfheadless hjkg.app --frames 500 --free-play --offline-http --no-keys \
  --press 40:10 --press 60:5 --press 100:10
```

`SF_DCSMS` is the encoded response record (nibble-swap + invert of the
plaintext `[BE dword len][len x BE halfwords] "8"` record — the encode
is self-inverse). `SF_FORCE` seeds the queue head (5 = the pay-flow
message type), the builder stage flag, the pay state, and the send
pattern table that the callback's match loops consume. The form reports
帐号或密码输入有误 because the injected record is the *pay* receipt; the
platform-auth success record is a separate format that has not been
decoded.

## Known gaps

- **The platform-auth record** (hjkg): the login form submits and the
  SDK parses a response, but the offline response is the pay receipt.
  Decoding the auth success record is the remaining step before the
  game loop opens.
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
- `SF_TRACE` — log every platform slot and GDI operation
- `SF_FRAME_DUMP` (libretro test) — dump the core's presented frame
