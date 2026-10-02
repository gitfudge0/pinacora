# Contributing

Thanks for helping improve Pinacora. Small, focused pull requests and reproducible bug reports are welcome. This is an unofficial macOS and Linux app; preserve gallery attribution, source links, and original-image behavior.

For a substantial feature or visual change, open an issue first so the approach can be discussed. For a bug, include the app version, operating system version, desktop environment, architecture, steps to reproduce, and expected versus actual behavior. Redact personal information from screenshots and logs. Report vulnerabilities through [the security process](SECURITY.md), rather than a public issue.

## Development

Use macOS 12 or newer, Rust via rustup, and Apple's Command Line Tools. `rust-toolchain.toml` selects Rust 1.95.0. GPUI uses runtime Metal shaders, so the full Xcode app is not required.

```sh
cargo run --locked
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
./scripts/build-macos.sh --debug
```

Use `cargo fmt --all` to format changes. Keep `Cargo.lock` committed. Add meaningful tests for changed parsing, download validation, or preference behavior; avoid tests that depend on the live gallery. Manually check UI changes on macOS, including keyboard controls and Motion off. The optional `cargo run --locked -- --check-source` contacts the live service and caches an original without applying wallpaper.

## Pull requests

Describe the problem, resulting behavior, and the checks you ran. Include screenshots for visual changes when useful. Keep unrelated cleanup separate and update documentation when behavior changes. Do not commit downloaded artwork, generated app bundles, signing credentials, or personal data. Follow the [Code of Conduct](CODE_OF_CONDUCT.md).

By contributing project code or original project assets, you agree to make them available under this repository's MIT license. Only contribute material you have permission to license; third-party material must retain its applicable notices and licenses.

## Linux development

See [Linux setup](README.md#linux) for Arch dependencies and Vulkan drivers. Ubuntu requires `build-essential`, `pkg-config`, `libfontconfig1-dev`, `libfreetype6-dev`, `libwayland-dev`, `libxkbcommon-dev`, `libxkbcommon-x11-dev`, `libx11-xcb-dev`, the XCB development packages listed in CI, `libssl-dev`, and `libvulkan-dev`. Run the same Cargo checks above; use `./scripts/build-linux.sh --debug` for packaging or `./install.sh --debug` for user-local installation. Test keyboard editing and browsing on Wayland and X11. Wallpaper tests must use command planning or mocks rather than changing a developer's wallpaper; manually apply only when intended. CI builds and checks both macOS architectures and Linux.
