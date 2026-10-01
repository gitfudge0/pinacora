# Changelog

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
