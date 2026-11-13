# Game Compatibility

Status of the SnowFish application corpus against the current build.
All titles boot and execute; the notes describe what renders and where
each title currently stops.

The emulator services are derived from the platform contract (package
format, guest memory map, wrapper dispatch, LZ stream format). Where a
service is absent the guest observes the same behavior as on a device
without network coverage: the title stays on its own waiting screen.

## Corpus status

| Title | Status | Notes |
|---|---|---|
| 黄金矿工 (hjkg) | boots | Service dialog renders; pay/SMS wait screen (see below) |
| 宇宙风暴 (yzfb) | boots | Title screen call-verified; gameplay sprites pending resource-link RE |
| (终身)三国危城无双 | boots | Waiting screen |
| 五子棋 (trunk) | boots | Single background fill, matching the platform's idle session |
| 巨作RPG曹操冢 (Mengde) | boots | Waiting screen |
| 曹操冢超豪华版 (Mengde i) | boots | Waiting screen (needs both res files served) |
| 新星际撞球 (StarBall) | boots | Waiting screen |
| 新鬼吹灯 (Mengde pe) | boots | Runs clean |
| 胭脂戏游龙 (yzxyl) | boots | Waiting screen |
| 永恒魔塔 (yonghengmota) | boots* | Faults at the same guest fetch as a bare platform session (title-side issue, not emulator) |

## Known gaps

- **Pay/SMS wait screens**: titles that query the billing service
  before their menus park on the service dialog. The platform-side
  service would complete a registered timer callback with the response;
  the offline billing simulation (local receipts) is planned.
- **Sprite resource linking** (宇宙风暴): the guest's sprite table
  entries stay null in gameplay; the resource-link pass needs further
  reverse engineering.

## Reproducing

The headless harness drives the same machine without a window:

```sh
sfheadless <game.app> --frames 30 [--res <res.app>] [--dump-fb <dir>]
```

`--dump-fb` writes per-frame RGB565 dumps for offline comparison.
