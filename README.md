<p align="center"><img src="resources/app-icon.png" alt="Reframed app icon" width="128" height="128"></p>

# Reframed

A native macOS wallpaper browser built with Rust and GPUI. Discover artwork from the live [Reframed gallery](https://www.reframed.gallery/), browse a cinematic dark workspace, and set an untouched, full resolution original on every currently connected display.

**Unofficial and independent:** this app is not affiliated with or endorsed by Reframed. Artwork belongs to its respective rights holders; no gallery artwork is bundled with the app.

## Download and install

Get the latest app from [GitHub Releases](https://github.com/gitfudge0/reframed/releases). Version 0.1.0 provides separate ZIP archives:

| Mac | Download |
| --- | --- |
| Apple Silicon (M series) | [Reframed-0.1.0-macos-arm64.zip](https://github.com/gitfudge0/reframed/releases/download/v0.1.0/Reframed-0.1.0-macos-arm64.zip) |
| Intel | [Reframed-0.1.0-macos-x86_64.zip](https://github.com/gitfudge0/reframed/releases/download/v0.1.0/Reframed-0.1.0-macos-x86_64.zip) |

Requires macOS 12 or newer and internet access for the live gallery. These are separate architecture builds, not a universal app. Check your Mac's chip in **Apple menu → About This Mac**. Unzip the matching archive, drag **Reframed.app** to **Applications**, and open it.

The app is **ad-hoc signed and not notarized by Apple**. macOS may block the first launch. If you trust the release, attempt to open it, then use **System Settings → Privacy & Security → Open Anyway** (on macOS 12, **System Preferences → Security & Privacy → General**). If macOS reports that the app is damaged or contains malware, do not bypass that warning; report the problem with the release version and macOS version.

## Browse, choose, apply

- A full-width cinematic preview sits above the vertical artwork grid. Selecting a card brings its artwork into view; **Back to results** restores your browsing position and search. Searching prioritizes matching results and hides unrelated previews.
- **Refresh** reloads the recent catalogue while browsing, or reruns the current site search. **Load more artwork** fetches the next page without moving your browsing position. **Show new artwork** jumps to additions; failed pages can be retried in place.
- Search Reframed by title or artist. Queries of at least two characters are sent to the site after a short pause. Results show the site’s top matching artworks, including matches beyond loaded pages; the search endpoint has no pagination. Clearing search restores the loaded recent catalogue. **Command-F** focuses search and brings results into view; standard editing, selection, clipboard, Unicode, and input methods are supported. **Escape** or **Clear** clears the query. Use **Tab/Shift-Tab** to move between controls and artwork, arrow keys to navigate the grid, and **Enter/Space** to activate. **Escape** returns from a selected preview to results; in search it clears the query.
- **Set wallpaper** downloads and validates the original, then applies it through macOS to all currently connected displays. **View source** opens its gallery page with attribution.
- **Motion on/off** controls all in-app animation, including the walkthrough backdrop. The current GPUI version does not expose the system reduced-motion preference.
- **Guide** replays the welcome walkthrough. Continue, Next, and Back are manual; Skip or Escape dismisses it. The walkthrough never changes your wallpaper.
- **Command-Q** quits.

Other Spaces and disconnected displays are outside this version's wallpaper behavior. Automatic rotation, launch at login, and automatic updates are not included.

## Source, artwork, and local storage

The app reads public `/recent` pages, the site’s `/api/search/nav` search endpoint, and artwork structured metadata. It preserves Unicode titles and attribution, and downloads originals directly from the gallery CDN with ordinary browser headers. It does not use private APIs or the site's bot-protected download proxy. If access is blocked or the source format changes, it reports an error; it does not replace the original with a preview. Browsing previews are resized by the source.

| Data | Location |
| --- | --- |
| Original images | `~/Library/Application Support/Reframed/originals` |
| Preview images | `~/Library/Caches/Reframed/previews` |
| Walkthrough completion or skip preference | `~/Library/Application Support/Reframed/walkthrough.json` |

Images have hashed filenames, bounded downloads, image validation, and atomic publication. Originals are kept so macOS can continue using them. You can remove these folders in Finder to reclaim storage; deleting an active original may affect your wallpaper. Cache size is not automatically capped. Removing the preference file replays onboarding; malformed or outdated preferences do too. A preference save failure never prevents dismissal.

See the gallery's [FAQ](https://www.reframed.gallery/faq) for its personal-use guidance, and respect the rights and terms applicable to each artwork. Some gallery editions may be AI-assisted. The project's MIT license does not grant rights to gallery artwork. See [Privacy](PRIVACY.md) and [Third-party notices](THIRD_PARTY_NOTICES.md).

## Build locally

Install [Rust through rustup](https://rustup.rs/) and Apple's Command Line Tools (`xcode-select --install`). The repository pins Rust **1.95.0** and GPUI **0.2.2**. Runtime Metal shaders avoid requiring the full Xcode app. Build on macOS:

```sh
git clone https://github.com/gitfudge0/reframed.git
cd reframed
cargo run --locked
```

Create a local app bundle:

```sh
./scripts/build-macos.sh --debug
open dist/Reframed.app
```

Omit `--debug` for an optimized release build. Local bundles are ad-hoc signed and unnotarized. The release workflow builds both architectures when a GitHub release is published and attaches ZIP archives and checksums; see [the workflow](.github/workflows/release.yml).

```sh
cargo run --locked -- --check-source
```

This optional live diagnostic checks catalogue metadata and downloads one original into the persistent cache. It needs internet access and never changes your wallpaper.

## Contribute

See [Contributing](CONTRIBUTING.md), the [Code of Conduct](CODE_OF_CONDUCT.md), and the [Security policy](SECURITY.md). [Changelog](CHANGELOG.md) tracks released changes.

Project code and original project assets are available under the [MIT license](LICENSE). Dependencies retain their own licenses; gallery artwork is separate.
