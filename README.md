<p align="center"><img src="resources/app-icon.png" alt="Pinacora app icon" width="128" height="128"></p>

# Pinacora

A native macOS and Linux wallpaper browser built with Rust and GPUI. Discover artwork from the live [Reframed gallery](https://www.reframed.gallery/), browse a cinematic dark workspace, and set an untouched, full resolution original on all Desktops on macOS 14 or newer.

**Unofficial and independent:** this app is not affiliated with or endorsed by Reframed. Artwork belongs to its respective rights holders; no gallery artwork is bundled with the app.

## Download and install

Get the latest app from [GitHub Releases](https://github.com/gitfudge0/pinacora/releases). The published version 0.2.0 is a historical **Reframed** release with separate ZIP archives. Upcoming builds from this source use **Pinacora** branding and packaging; Pinacora release downloads are not yet published:

| Mac | Download |
| --- | --- |
| Apple Silicon (M series) | [Reframed-0.2.0-macos-arm64.zip](https://github.com/gitfudge0/pinacora/releases/download/v0.2.0/Reframed-0.2.0-macos-arm64.zip) |
| Intel | [Reframed-0.2.0-macos-x86_64.zip](https://github.com/gitfudge0/pinacora/releases/download/v0.2.0/Reframed-0.2.0-macos-x86_64.zip) |

Requires macOS 12 or newer and internet access for the live gallery. These are separate architecture builds, not a universal app. Check your Mac's chip in **Apple menu → About This Mac**. For these historical downloads, unzip the matching archive, drag **Reframed.app** to **Applications**, and open it.

Release archives are **self-signed with the gitfudge certificate and not notarized by Apple**. This certificate is not an Apple Developer ID certificate and does not remove Gatekeeper warnings. macOS may block the first launch. If you trust the release, attempt to open it, then use **System Settings → Privacy & Security → Open Anyway** (on macOS 12, **System Preferences → Security & Privacy → General**). If macOS reports that the app is damaged or contains malware, do not bypass that warning; report the problem with the release version and macOS version.

## Linux

Linux builds support Wayland and X11 with a Vulkan-capable graphics driver. Wallpaper application supports Hyprland with **hyprpaper 0.8 or newer** running with IPC enabled, and GNOME via `gsettings`. Other desktops can browse and download artwork but wallpaper application reports an unsupported desktop. Hyprland changes apply to every currently connected monitor for the current session; Pinacora does not rewrite your hyprpaper configuration. GNOME updates light and dark background preferences.

On Arch Linux install build dependencies and the desktop helper:

```sh
sudo pacman -S --needed base-devel pkgconf fontconfig freetype2 libxkbcommon libxkbcommon-x11 wayland libxcb vulkan-icd-loader openssl hyprpaper
```

Install the Vulkan driver appropriate to your GPU (`vulkan-intel`, `vulkan-radeon`, or the NVIDIA driver). Install Rust through rustup; this repository selects Rust 1.95.0. Start hyprpaper through your desktop session if it is not already running. See the [hyprpaper IPC documentation](https://wiki.hypr.land/hypr-ecosystem/user/hyprpaper/).

```sh
cargo run --locked
./install.sh --debug
```

Installation builds the app and writes `~/.local/bin/pinacora`, an application launcher, and an icon to your user data directory. Use `./install.sh` for an optimized release build, or `./scripts/build-linux.sh --debug` to build without installing. Launch Pinacora from your application menu or `~/.local/bin/pinacora`. Linux uses **Ctrl-F**, **Ctrl-Q**, and standard Ctrl clipboard shortcuts.

Linux storage uses `$XDG_DATA_HOME/pinacora/originals` and `$XDG_DATA_HOME/pinacora/walkthrough.json` (default `~/.local/share/pinacora`), and `$XDG_CACHE_HOME/pinacora/previews` (default `~/.cache/pinacora`). Empty or relative XDG values use the defaults. Existing `reframed` data and cache directories take precedence over the new names, independently for each location, so originals and walkthrough preferences stay in place. No files are migrated or deleted. Keep originals in place while they are your wallpaper. Linux release archives are not yet published. The native screen-reader accessibility bridge currently supports macOS only; Linux retains keyboard navigation.

## Browse, choose, apply

- A full-width cinematic preview sits above the vertical artwork grid. Selecting a card brings its artwork into view; **Back to results** restores your browsing position and search. Searching prioritizes matching results and hides unrelated previews.
- **Refresh** reloads the recent catalogue while browsing, or reruns the current site search. **Load more artwork** fetches the next page without moving your browsing position. **Show new artwork** jumps to additions; failed pages can be retried in place.
- Search Reframed by title or artist. Queries of at least two characters are sent to the site after a short pause. Results show the site’s top matching artworks, including matches beyond loaded pages; the search endpoint has no pagination. Clearing search restores the loaded recent catalogue. **Command-F** focuses search and brings results into view; standard editing, selection, clipboard, Unicode, and input methods are supported. **Escape** or **Clear** clears the query. Use **Tab/Shift-Tab** to move between controls and artwork, arrow keys to navigate the grid, and **Enter/Space** to activate. **Escape** returns from a selected preview to results; in search it clears the query.
- **Set wallpaper** downloads and validates the original, then applies it to all Desktops on macOS 14 or newer. On macOS 12–13 it applies to connected displays in the current Desktop. **View source** opens its gallery page with attribution.
- **Motion on/off** controls all in-app animation. Artwork fades and gently settles over 460 ms, with a short stagger for its title and actions. Hovering a card eases its image closer over 200 ms. Nothing loops. The current GPUI version does not expose the system reduced-motion preference.
- **Guide** reopens the single welcome screen. Start browsing or Enter opens the gallery; Escape dismisses the welcome. The welcome never changes your wallpaper, and dismissal is remembered.
- **Command-Q** quits.

All Desktops includes other Spaces on macOS 14 or newer. The app updates macOS’s local wallpaper store and briefly restarts the wallpaper service to show the change. This store is an undocumented macOS format; unsupported configurations report an error. macOS 12–13 uses the public display API, so other Spaces and disconnected displays are outside that fallback’s scope. Automatic rotation, launch at login, and automatic updates are not included.

Version 0.2.0 includes live gallery search and all-Desktops wallpaper application on macOS 14 or newer.

## Source, artwork, and local storage

The app reads public `/recent` pages, the site’s `/api/search/nav` search endpoint, and artwork structured metadata. It preserves Unicode titles and attribution, and downloads originals directly from the gallery CDN with ordinary browser headers. It does not use private APIs or the site's bot-protected download proxy. If access is blocked or the source format changes, it reports an error; it does not replace the original with a preview. Browsing previews are resized by the source.

| Data | New-install location |
| --- | --- |
| Original images | `~/Library/Application Support/Pinacora/originals` |
| Preview images | `~/Library/Caches/Pinacora/previews` |
| Walkthrough completion or skip preference | `~/Library/Application Support/Pinacora/walkthrough.json` |
| Wallpaper-store backups (macOS 14+) | `~/Library/Application Support/com.apple.wallpaper/Store/Index.plist.pinacora-backup-*` |

Images have hashed filenames, bounded downloads, image validation, and atomic publication. Originals are kept so macOS can continue using them. You can remove these folders in Finder to reclaim storage; deleting an active original may affect your wallpaper. Cache size is not automatically capped. Each all-Desktops application keeps a separate wallpaper-store backup alongside macOS’s `Index.plist`; backups are not automatically removed. Removing the preference file replays onboarding; malformed or outdated preferences do too. A preference save failure never prevents dismissal.

Existing `Reframed` data and cache directories take precedence over `Pinacora`, independently for each location, even if both exist. No files are migrated or deleted, preserving walkthrough preferences and paths used by active wallpapers. Earlier `Index.plist.reframed-backup-*` backups remain in place.

See the gallery's [FAQ](https://www.reframed.gallery/faq) for its personal-use guidance, and respect the rights and terms applicable to each artwork. Some gallery editions may be AI-assisted. The project's MIT license does not grant rights to gallery artwork. See [Privacy](PRIVACY.md) and [Third-party notices](THIRD_PARTY_NOTICES.md).

## Build locally

Install [Rust through rustup](https://rustup.rs/) and Apple's Command Line Tools (`xcode-select --install`). The repository pins Rust **1.95.0** and GPUI **0.2.2**. Runtime Metal shaders avoid requiring the full Xcode app. Build on macOS:

```sh
git clone https://github.com/gitfudge0/pinacora.git
cd pinacora
cargo run --locked
```

Create a local app bundle:

```sh
./scripts/build-macos.sh --debug
open dist/Pinacora.app
```

Omit `--debug` for an optimized release build. Local bundles are ad-hoc signed and unnotarized by default. A locally trusted self-signed certificate can be used for local signing; it does not provide Apple notarization or remove Gatekeeper warnings for downloaded releases. The release workflow builds both architectures when a GitHub release is published and attaches ZIP archives and checksums; see [the workflow](.github/workflows/release.yml).

```sh
cargo run --locked -- --check-source
```

This optional live diagnostic checks catalogue metadata and downloads one original into the persistent cache. It needs internet access and never changes your wallpaper.

## Contribute

See [Contributing](CONTRIBUTING.md), the [Code of Conduct](CODE_OF_CONDUCT.md), and the [Security policy](SECURITY.md). [Changelog](CHANGELOG.md) tracks released changes.

Project code and original project assets are available under the [MIT license](LICENSE). Dependencies retain their own licenses; gallery artwork is separate.
