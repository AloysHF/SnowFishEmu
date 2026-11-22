# Release Process

This document describes how to publish a new release of SnowFishEmu.

## Prerequisites

- Push access to the `master` branch
- Permission to create tags and releases on GitHub

## Steps

### 1. Update version numbers

Version numbers must be updated in **two files**:

| File | Field | Current |
|------|-------|---------|
| `Cargo.toml` (workspace root) | `[workspace.package] version` | `"0.1.0"` |
| `crates/snowfishemu-libretro/snowfishemu_libretro.info` | `display_version` | `"0.1.0"` |

Both values must match. The `.info` file is copied directly into the
release artifacts — RetroArch reads `display_version` to display the core
version to users.

Run `cargo check --workspace` after editing the versions to regenerate the
workspace package versions in the local `Cargo.lock`. This repository ignores
`Cargo.lock`; do not force-add it for a release.

```bash
# Example: bumping to 0.2.0
# 1. Edit Cargo.toml
sed -i 's/^version = "0.1.0"/version = "0.2.0"/' Cargo.toml

# 2. Edit .info file
sed -i 's/^display_version = "0.1.0"/display_version = "0.2.0"/' \
  crates/snowfishemu-libretro/snowfishemu_libretro.info
```

### 2. Commit the version bump

```bash
git add Cargo.toml crates/snowfishemu-libretro/snowfishemu_libretro.info docs/Release-Process.md
git commit -m "chore: bump version to 0.2.0"
git push origin master
```

### 3. Create and push a tag

The release workflow triggers on tags matching `v*` (e.g. `v0.2.0`).

```bash
git tag v0.2.0
git push origin v0.2.0
```

### 4. CI builds and creates a draft release

Pushing the tag triggers `.github/workflows/release.yml`, which:

1. **Builds standalone binaries** for Linux and macOS (x86_64 + aarch64), and
   Windows (x86_64)
2. **Builds libretro cores** for the same platforms, plus Linux and Windows
   i686, Android, iOS, and webOS, using the platform-specific core filenames
   and bundling `snowfishemu_libretro.info`
3. **Builds the EmulatorJS browser core** (`snowfishemu-emulatorjs.zip`) with
   the pinned Emscripten toolchain
4. **Creates a draft GitHub Release** with:
   - Auto-generated release notes (PRs and commits since the previous tag)
   - A download table for standalone and libretro artifacts
   - All build artifacts attached

### 5. Review and publish the release

1. Go to [Releases](https://github.com/AloysHF/SnowFishEmu/releases)
2. Find the draft release created by CI
3. Review the auto-generated changelog — edit if needed (you can refer to the
   changelog of the previous official release)
4. Verify all 16 expected artifacts are attached:
   - `snowfishemu-linux-x86_64.tar.gz`
   - `snowfishemu-linux-aarch64.tar.gz`
   - `snowfishemu-macos-x86_64.tar.gz`
   - `snowfishemu-macos-aarch64.tar.gz`
   - `snowfishemu-windows-x86_64.zip`
   - `snowfishemu-linux-x86_64-libretro.tar.gz`
   - `snowfishemu-linux-aarch64-libretro.tar.gz`
   - `snowfishemu-linux-i686-libretro.tar.gz`
   - `snowfishemu-macos-x86_64-libretro.tar.gz`
   - `snowfishemu-macos-aarch64-libretro.tar.gz`
   - `snowfishemu-windows-x86_64-libretro.zip`
   - `snowfishemu-windows-i686-libretro.zip`
   - `snowfishemu-android-libretro.tar.gz`
   - `snowfishemu-ios-libretro.tar.gz`
   - `snowfishemu-webos-libretro.tar.gz`
   - `snowfishemu-emulatorjs.zip`
5. Click **Publish release**

### 6. Sync upstream libretro metadata and builds

Submit core metadata changes to
[libretro-super](https://github.com/libretro/libretro-super/tree/master/dist/info)
under `dist/info/snowfishemu_libretro.info`, using the current
`crates/snowfishemu-libretro/snowfishemu_libretro.info` from this repository.
The separate `libretro-core-info` repository is a mirror.
Coordinate new core entries with the buildbot, GitLab mirror and
documentation maintainers. Updating this repository alone does not publish
the Core Downloader entry.

### Nightly builds

Besides the tagged releases, the rolling `nightly-build` prerelease is
rebuilt on every push to `master` and daily at 20:00 UTC (04:00 Beijing) by
`.github/workflows/nightly-build.yml`, with a 7-day artifact retention. The
GitHub Pages deployment also fetches the EmulatorJS browser core from that
rolling release, so the site's play page always tracks master.

## Troubleshooting

### CI build fails

- Check the [Actions](https://github.com/AloysHF/SnowFishEmu/actions) tab
  for the failed run
- The most common failure is a missing Linux build dependency — the CI installs
  `libasound2-dev`, `libx11-dev`, `libxkbcommon-dev`, and `libudev-dev`
  automatically

### Re-triggering a release

The release workflow only runs on tag pushes. To re-trigger:

```bash
# 1. Delete the tag locally and remotely
git tag -d v0.2.0
git push origin --delete v0.2.0

# 2. Re-push the tag (CI will re-run)
git push origin v0.2.0
```

If a draft release was already created by the failed run, delete it from the
[Releases](https://github.com/AloysHF/SnowFishEmu/releases) page before
re-pushing the tag, otherwise the new run may conflict with the existing draft.

### Release artifacts missing

- Verify the tag name starts with `v` (e.g. `v0.2.0`, not `0.2.0`)
- The release workflow only triggers on tag pushes (`v*`); manually dispatching
  the workflow from the Actions tab will not create a release

### Version mismatch in RetroArch

- Ensure both files have the same version string:
  `Cargo.toml` and `snowfishemu_libretro.info`
- The `.info` file is bundled as-is into the release artifacts
