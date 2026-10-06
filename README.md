<p align="center"><img src="resources/app-icon.png" alt="Pinacora icon" width="96" height="96"></p>

# Pinacora

Classic art for your desktop. Browse paintings and photography, then set your wallpaper.

The gallery collection comes from [Reframed Gallery](https://www.reframed.gallery/). Pinacora is an independent, unofficial app, not affiliated with or endorsed by Reframed.

## Install

Download a [release](https://github.com/gitfudge0/pinacora/releases): macOS Apple Silicon or Intel, or Linux x86_64. On Linux, extract the archive and run its `./install.sh` to add Pinacora to your launcher; Rust is not required. Linux builds target Ubuntu 24.04 or newer compatible distributions and need a Vulkan driver, Fontconfig, FreeType, Wayland/X11, libxkbcommon, OpenSSL, and ALSA runtime libraries.

To build from source:

Install [Rust through rustup](https://rustup.rs/). This repository pins Rust 1.95.0. Native build dependencies are also required.

```sh
git clone https://github.com/gitfudge0/pinacora.git
cd pinacora
./install.sh
```

On Linux, the installer adds Pinacora to your application launcher and installs the executable in `~/.local/bin`. On macOS, it installs `/Applications/Pinacora.app`. Use `./install.sh --debug` for a faster development build.

### Linux

Requires Wayland or X11 and a Vulkan-capable graphics driver. Setting wallpaper supports Hyprland with hyprpaper 0.8 or newer running with IPC enabled, and GNOME. Other desktops can browse and download artwork.

On Arch Linux:

```sh
sudo pacman -S --needed base-devel pkgconf fontconfig freetype2 libxkbcommon libxkbcommon-x11 wayland libxcb vulkan-icd-loader openssl hyprpaper
```

Install the Vulkan driver for your GPU and start hyprpaper in your Hyprland session.

### macOS

Requires macOS 12 or newer and Apple's Command Line Tools:

```sh
xcode-select --install
```

Older releases retain the Reframed name.

## Use

Search by title or artist, preview an artwork, and choose **Set wallpaper**. **View source** opens its gallery page with attribution. Browsing and downloads need internet access.

Wallpaper rotation is available in Pinacora 0.4.0 and newer.

Choose **Rotation** to change wallpapers every 1 to 1,440 minutes. **Entire gallery** starts with a wallpaper chosen across the gallery, independently of search, while the full catalogue loads quietly. Once cached, catalogue metadata is available immediately on later starts and resumes, including offline when the next original is cached. Pinacora saves artwork history to avoid repeating successfully shown artwork within the known catalogue cycle, even after reopening. The first background load extends the starting pool; later refreshes add artwork for the next cycle. **Selected artworks** rotates your saved collection in order; use **Add to rotation** while browsing, then save the collection and interval. At least two artworks are required.

**Start rotation** prepares and applies the first original without waiting for every catalogue page. Later starts use the saved catalogue immediately and refresh it in the background. **Pause**, **Resume**, **Change now**, and **Stop rotation** control the schedule. Saving an interval resets the countdown. A source switch prepares the replacement paused and keeps the current wallpaper. Rotation continues after closing the app. Its per-user background service restores active rotation at login; reopen Pinacora to pause, resume, or stop it. The service runs during your graphical login session. GNOME and other desktops that support XDG autostart start it at login. For bare Hyprland, add `exec-once = /absolute/path/to/pinacora --rotation-service-bootstrap` to your Hyprland configuration; replace the path with your installed executable. Hyprland sessions managed by uwsm can use its XDG autostart support. Setting a wallpaper manually pauses rotation. After sleep or a missed deadline, the app makes at most one change, then starts a fresh interval.

Pinacora prepares one next original at a time. If it is not ready when due, the current wallpaper stays in place while the app retries; **Retry now** retries preparation. A failed wallpaper application pauses rotation and **Retry change** retries that same artwork. Originals accumulate in local storage, and macOS 14 or newer also keeps a backup for each wallpaper change. These files have no automatic size cap or expiry.

On macOS 14 or newer, wallpaper changes apply to all Desktops. On macOS 12 and 13, they apply to connected displays in the current Desktop. On Hyprland, they apply to connected monitors for the current session.

The macOS all-Desktops method uses an undocumented wallpaper store, saves a backup, and restarts the wallpaper service. See [Privacy](PRIVACY.md) for details and local storage information.

## Artwork and credits

Reframed Gallery's [FAQ](https://www.reframed.gallery/faq) permits personal wallpaper use. Respect the rights and terms that apply to each artwork. Artwork belongs to its original rights holders, and no gallery artwork is bundled with Pinacora.

The software and original project assets use the [MIT license](LICENSE). This license does not grant rights to gallery images.

[Privacy](PRIVACY.md) · [Third-party notices](THIRD_PARTY_NOTICES.md) · [Contributing](CONTRIBUTING.md) · [Changelog](CHANGELOG.md)

Developer docs: [Code map](AGENTS.md) · [Troubleshooting](docs/troubleshooting.md) · [Rotation](docs/rotation.md) · [Updates](docs/updates.md)

## Remove background rotation

Stop rotation in Pinacora before removing it. On Linux, stop the user service with `systemctl --user stop pinacora-rotation.service`, then remove `$XDG_CONFIG_HOME/systemd/user/pinacora-rotation.service` and `$XDG_CONFIG_HOME/autostart/pinacora-rotation.desktop`, using `~/.config` when `XDG_CONFIG_HOME` is unset. Run `systemctl --user daemon-reload` afterward. On macOS, run `launchctl bootout gui/$(id -u)/com.pinacora.rotation`, then remove `~/Library/LaunchAgents/com.pinacora.rotation.plist`. You can then remove the app and its local storage as described in [Privacy](PRIVACY.md).
