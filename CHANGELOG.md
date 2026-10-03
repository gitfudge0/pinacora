# Changelog

## 0.4.0 — Wallpaper rotation

- Add entire-gallery and ordered selected-artwork rotation with minute intervals, pause/resume, immediate changes, and retry controls.
- Prepare the first wallpaper using gallery pagination, cache complete catalogue metadata, and retain shown history across restarts and source changes.
- Run rotation in a per-user background service after the GUI closes, with graphical-login registration on Linux and macOS.
- Pause rotation for manual wallpaper application and retain the current wallpaper when preparation fails.

- Simplify rotation settings with a compact status header, grouped source and interval controls, contextual save actions, and expandable help.

See [rotation behavior](docs/rotation.md) for scheduling, bounded retries, storage side effects, and platform details.

## 0.3.0 — Pinacora

- Rename the app to Pinacora, with updated launcher and transparent in-app artwork icons.
- Use Pinacora packaging and new-install storage names while retaining existing Reframed storage directories in place to preserve downloaded originals and walkthrough preferences.

- Simplify first launch to a single welcome screen.
- Add a prebuilt Linux x86_64 release with a local installer and application launcher icon.

## 0.2.0 — Gallery search and all Desktops

- Set wallpaper applies the original to all Desktops on macOS 14 or newer, with a local wallpaper-store backup and background application. macOS 12–13 retains connected-display application in the current Desktop.
- Show separate download and application progress, with wallpaper scope and results matching the macOS version.
- Search the live gallery by title or artist, including matching artworks beyond loaded recent pages, with standard text editing, clipboard, Unicode, and input-method support.
- Browse a vertical artwork grid, return from a selected preview to the previous results position, and load or retry pages without losing browsing position.
- Improve keyboard navigation and accessibility for artwork, search, and browsing controls.
- Add a local installation script that builds and installs the app in Applications.
- Sign release archives with a self-signed gitfudge certificate.

Known limitations: the gitfudge certificate is not an Apple Developer ID certificate; release archives remain unnotarized and may require Open Anyway. All-Desktops application relies on macOS’s undocumented local wallpaper-store format and retains a backup for each application.

## 0.1.0 — Initial public release

- Native Rust/GPUI macOS wallpaper browser with a dark hero and scrolling artwork shelves.
- Live recent-gallery browsing, pagination, local title/artist search, attribution, and source links.
- Validated full resolution original downloads and wallpaper application to all currently connected displays.
- Persistent originals and preview caches with bounded downloads and atomic file publication.
- First-launch walkthrough, replayable Guide, keyboard controls, and an explicit Motion toggle.
- App icon and macOS app bundle, with separate Apple Silicon and Intel release archives.

Known limitations: ad-hoc signing without Apple notarization; live-gallery format/access dependence; no automatic updater, wallpaper rotation, launch at login, or automatic cache-size cap. Other Spaces and disconnected displays are outside the supported wallpaper behavior. Motion must be disabled in-app because the current GPUI version does not expose the system reduced-motion preference.
