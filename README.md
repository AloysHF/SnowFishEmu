# SnowFishEmu

<p align="center">
  <a href="https://github.com/AloysHF/SnowFishEmu/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/AloysHF/SnowFishEmu/ci.yml?branch=master&label=CI&logo=github" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/License-BSD%203--Clause-blue.svg" alt="License: BSD 3-Clause"></a>
</p>

SnowFishEmu is a Rust emulator for SnowFish (SFA) applications on feature
phones. It loads `.app` executables directly, runs guest code through a
pure-Rust ARM core, and bridges the phone services the applications rely on.

SnowFishEmu uses the `snowfishemu` command, crates and libretro core
identifiers. Save states use the `SFEMSTAT` signature.

> **Status: early development.** The package loader, guest execution, and
> service bridges are under active construction. See
> [docs/Game-Compatibility.md](docs/Game-Compatibility.md) for what runs.

## Building

```sh
cargo build --release -p snowfishemu
```

## Running

```sh
snowfishemu <path/to/game.app>
```

## License

SnowFishEmu is distributed under the [BSD 3-Clause license](LICENSE).
SnowFishEmu does not include any game data. You must supply your own `.app`
files obtained from a device you own.
