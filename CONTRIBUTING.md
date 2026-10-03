# Contributing

Thanks for helping improve Pinacora. Small, focused pull requests and reproducible bug reports are welcome. This is an unofficial macOS and Linux app; preserve gallery attribution, source links, and original-image behavior.

For a substantial feature or visual change, open an issue first so the approach can be discussed. For a bug, include the app version, operating system version, desktop environment, architecture, steps to reproduce, and expected versus actual behavior. Redact personal information from screenshots and logs. Report vulnerabilities through [the security process](SECURITY.md), rather than a public issue.

## Development

Install Rust through rustup. `rust-toolchain.toml` pins Rust 1.95.0 on both platforms. Install its check components once:

```sh
rustup toolchain install 1.95.0 --profile minimal --component rustfmt --component clippy
```

After installing the native dependencies below, run the shared commands:

```sh
cargo run --locked
cargo fmt --all -- --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

Use `cargo fmt --all` to format changes. Keep `Cargo.lock` committed. Add meaningful tests for changed parsing, download validation, or preference behavior; avoid tests that depend on the live gallery. Wallpaper tests must use command planning or mocks rather than changing a developer's wallpaper; manually apply only when intended. The optional `cargo run --locked -- --check-source` contacts the live service and caches an original without applying wallpaper.

Use [the code map](AGENTS.md), [troubleshooting checklist](docs/troubleshooting.md), and [current rotation behavior](docs/rotation.md) to find the relevant implementation. Confirm which executable is running before comparing installed behavior with this checkout.

### Linux dependencies

Ubuntu build dependencies match CI. Python 3 is also required by the local installer:

```sh
sudo apt-get update
sudo apt-get install -y build-essential pkg-config libasound2-dev libfontconfig1-dev libfreetype6-dev libwayland-dev libxkbcommon-dev libxkbcommon-x11-dev libx11-xcb-dev libxcb1-dev libxcb-composite0-dev libxcb-randr0-dev libxcb-xfixes0-dev libxcb-shape0-dev libxcb-render0-dev libxcb-xkb-dev libssl-dev libvulkan-dev python3
```

On Arch Linux:

```sh
sudo pacman -S --needed base-devel pkgconf alsa-lib fontconfig freetype2 libxkbcommon libxkbcommon-x11 wayland libxcb vulkan-icd-loader openssl python
```

Install the Vulkan driver for your GPU. Hyprland wallpaper application also needs hyprpaper 0.8 or newer running with IPC enabled. See [Linux setup](README.md#linux) for supported desktops. Test keyboard editing and browsing on Wayland and X11, including Motion off.

### macOS dependencies

Use macOS 12 or newer and Apple's Command Line Tools:

```sh
xcode-select --install
```

GPUI uses runtime Metal shaders, so the full Xcode app is not required. Packaging also uses Python 3 to collect license notices. Manually check UI changes on macOS, including keyboard controls and Motion off.

### Packaging and local installation

`./scripts/build-linux.sh --debug` creates `dist/linux/pinacora`. `./scripts/build-macos.sh --debug` creates `dist/Pinacora.app`; it accepts `--target aarch64-apple-darwin` or `--target x86_64-apple-darwin`. Omit `--debug` for release packaging. CI builds and checks both macOS architectures and Linux.

`./install.sh --debug` builds and installs for the current platform. Linux installs `~/.local/bin/pinacora`, a launcher, and an icon under the user data directory. macOS installs `/Applications/Pinacora.app`, which must be writable by the current user. Reopen the installed app after replacing it, and inspect the rotation worker separately if it was already running. Cargo target directory and profile choices are described in [Troubleshooting](docs/troubleshooting.md).

## Pull requests

Describe the problem, resulting behavior, and the checks you ran. Include screenshots for visual changes when useful. Keep unrelated cleanup separate and update documentation when behavior changes. Do not commit downloaded artwork, generated app bundles, signing credentials, or personal data. Follow the [Code of Conduct](CODE_OF_CONDUCT.md).

By contributing project code or original project assets, you agree to make them available under this repository's MIT license. Only contribute material you have permission to license; third-party material must retain its applicable notices and licenses.
