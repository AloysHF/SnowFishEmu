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

The screenshots are emulator-rendered frame captures at each title's
documented play state; regenerate them with `scripts/batch-screenshots.ps1`.

| Title | Screenshot | Status | Notes |
|---|---|---|---|
| 五子棋 (trunk) | <img src="images/五子棋.png" width="120" alt="五子棋"> | **playable** | The wooden gobang board with a working cursor; directional keys move it and OK places a stone |
| (终身)三国危城无双 | <img src="images/三国危城无双.png" width="120" alt="三国危城无双"> | **playable** | The game's own splash (dragon art, 危城) and then the tower-defence tutorial with its menu (返回主菜单/优先完成) |
| 巨作RPG曹操冢 (Mengde) | <img src="images/巨作RPG曹操冢.png" width="120" alt="巨作RPG曹操冢"> | **playable** | Gameplay with the help dialog text readable (如当前武器为铲子时，可长按确定/5号键挖地) |
| 曹操冢超豪华版 (Mengde i) | <img src="images/曹操冢超豪华版.png" width="120" alt="曹操冢超豪华版"> | **playable** | Gameplay; the help bar renders (按数字键3可以查看大地图显示) and the sprite pages load |
| 永恒魔塔 (yonghengmota) | <img src="images/永恒魔塔.png" width="120" alt="永恒魔塔"> | **playable** | The magic-tower floor renders with the story dialog fully readable (宙斯：好女儿，乖女儿，就让我一次嘛…), the characters and the floor counter |
| 胭脂戏游龙 (yzxyl) | <img src="images/胭脂戏游龙.png" width="120" alt="胭脂戏游龙"> | **playable** | The 西门庆居 scene, the dialog box (与春梅对话) and the movement tutorial (使用8,2,4,6可以进行移动) |
| 新星际撞球 (StarBall) | <img src="images/新星际撞球.png" width="120" alt="新星际撞球"> | **playable** | The brick board, the ball and the paddle run; the top score strip draws the whole digit atlas instead of the score (cosmetic, see the known gaps) |
| 新鬼吹灯 (Mengde pe) | <img src="images/新鬼吹灯.png" width="120" alt="新鬼吹灯"> | **playable** | The full title screen; OK opens the story intro (scrolling, wrapped) and then the menu (新的游戏/继续游戏/帮助/退出) and the battle |
| 黄金矿工 (hjkg) | <img src="images/黄金矿工.png" width="120" alt="黄金矿工"> | **playable** | Start on the title screen opens the first level directly. Options → Start Game opens the local New Game / Level Select menu |
| 宇宙风暴 (yzfb) | <img src="images/宇宙风暴.png" width="120" alt="宇宙风暴"> | **playable** | The music prompt, title, and main menu render. New Game enters the scrolling first level, and directional keys move the ship |

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
sfa_headless <game.app> --frames 200 --free-play --offline-http --no-keys \
  --press 30:10 --press 60:10 --press 90:10 --press 120:10 --press 150:10 --press 180:10
```

That recipe (six left-soft-key presses over 200 frames) lands 五子棋 on
its board, 三国危城无双 on its splash/tutorial, both 曹操冢 builds and
永恒魔塔 in gameplay, and 胭脂戏游龙 on its first scene. For 新鬼吹灯 the
advancing key is OK (`--press 40:5 ...`): the title opens the story
intro, and further OK presses finish it into the menu. For 新星际撞球
three presses over 500 frames suffice — its tariff wait dismisses
itself and the game starts.

### Gold Miner (黄金矿工)

SnowFishEmu identifies the supported SnowFish V2 QVGA package by its
content fingerprint and applies the offline start fix in guest memory.
The right soft key (**X**) or **Enter** on the title screen opens the
first level directly. **Z** opens Options, where Start Game opens the
local New Game / Level Select menu. The same behavior applies to the
libretro core. Both paths work with the default launch settings.

```sh
sfa_headless <game.app> --frames 90 --press 30:25 --press 65:5
```

The right soft key at frame 30 starts the first level. OK at frame 65
releases the claw. Set `SF_GOLD_MINER_APP` to the package path and run
`cargo test -p snowfishemu-core --test gold_miner -- --ignored` to verify
both start paths.

### Universe Storm (宇宙风暴)

The supported SnowFish V2 QVGA package has two malformed bitmap headers.
SnowFishEmu identifies the package by its content fingerprint and repairs
those headers as their blocks are decompressed. It reads embedded music
and bitmap blocks from the original package bytes. SDK state writes to
overlapping guest addresses previously corrupted those blocks and could
cause repeated LZ errors during the first-level load.

The left soft key (**Z**) accepts the music prompt. **Enter** opens the
main menu from the title and confirms **New Game**. The following replay
reaches the first level and steers the ship right.

```sh
sfa_headless <game.app> --frames 132 --press 30:10 --press 60:5 --press 98:5 --press 130:6
```

The desktop replay also works with `--free-play --offline-http`. The
standalone frontend keeps servicing window messages during static frames,
including the first-level loading screen.

Set `SF_UNIVERSE_STORM_APP` to the package path and run
`cargo test -p snowfishemu-core --test universe_storm -- --ignored` to
verify the menu, ship controls, and first-level loading after an
overlapping SDK state write.

## Known gaps

- **StarBall and Gold Miner HUD digits**: the HUD can draw the entire
  digit atlas (`0123456789`) where a single digit belongs.
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
sfa_headless <game.app> --frames 30 [--dump-fb <dir>] \
  [--free-play] [--offline-http] [--no-keys] [--keys 10] [--press frame:key]
```

Sibling resource containers are discovered next to the package
automatically (the rule is documented in
[Standalone Emulator](Standalone-Emulator.md#resource-files)), and a
download request naming a pack is answered with that pack's own bytes.
`--dump-fb` writes per-frame framebuffer dumps for offline
comparison (one 32-bit-per-pixel word per pixel). `--dump-mem
addr:len:path` (hex) exports a guest memory range, including the
framebuffer at `540720:25800` (raw RGB565).

### Debug environment variables

- `SF_4C="lr:val;*:val"` — override the +0x4C billing-capability result
  per call site (hex LRs; `*` sets the base, default 1)
- `SF_FREE_PLAY=1` — answer the purchase service as a registered handset
  (`sfa_headless --free-play` does the same)
- `SF_OFFLINE_HTTP=1` — answer the SDK's HTTP transactions locally
  (`sfa_headless --offline-http` does the same)
- `SF_NO_APP_PUMP=1` — suppress the per-frame selector-3 event
  (`sfa_headless --no-app-pump` does the same)
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
