# Contributing

## How to Contribute

1. **Fork** this repository
2. **Create** a feature branch (`git checkout -b feature/your-feature`)
3. **Commit** your changes (`git commit -m 'Add your feature'`)
4. **Push** to the branch (`git push origin feature/your-feature`)
5. **Open** a Pull Request

## Code Style

- Use English for all comments and documentation
- Use `snake_case` for functions and variables
- Use `PascalCase` for types and structs
- Use `thiserror` error enums in the core and `anyhow::Result` at the
  frontend boundaries
- Use the `log` crate for logging (not `println!`)
- Keep host filesystem, window, audio, and libretro details out of the core
- Avoid `unsafe` outside the libretro ABI boundary
- Follow [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)

The core must stay platform-independent and drivable headlessly. Do not make
it depend on a desktop window, RetroArch, or application-specific paths. Do
not silently ignore unknown platform API slots or CPU instructions — record
the slot index or opcode and log it.

## Areas That Need Help

- **Game compatibility testing** — test more `.app` applications and report
  issues with the offline recipes that reach them
- **SFA package format support** — parsing for header/variant differences
- **Platform API bridge** — implementing further root-table slots and their
  wrapper semantics
- **GDI display bridge** — bitmap blits, text rendering, and canvas state
- **LZ stream decoder** — edge cases in the bit-exact decompressor port
- **Offline startup** — the purchase/HTTP service answers that let titles
  reach their own menus without retired servers
- **ARM CPU emulation** — instruction accuracy for edge cases
- **Documentation** — improve docs and code comments
- **Bug reports** — if you find an application that doesn't work correctly,
  please open an issue

## Getting Started

Check the [open issues](https://github.com/AloysHF/SnowFishEmu/issues) for
tasks labeled `good first issue` or `help wanted`. If you have questions, feel
free to open a discussion issue.

To understand the `.app` package format, see
[Game File Formats](Game-File-Formats.md).

## Testing

Run the unit tests:

```bash
cargo test --workspace
```

A pre-commit hook (installed by `cargo-husky` when the workspace builds) runs
`cargo fmt` and `cargo clippy -D warnings` before every commit.

Do not claim application compatibility without a reproducible test: fix
runtime bugs by adding a deterministic regression test that asserts the
semantic outcome, and verify the test catches the bug by temporarily
reverting the fix.

For game compatibility testing, see [Game Compatibility](Game-Compatibility.md).

Game files are not included. Supply legally obtained `.app` applications
separately; never commit game packages, extracted assets, or save data.
