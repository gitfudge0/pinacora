# Changelog

## 0.1.0 — Initial public release

- Native Rust/GPUI macOS wallpaper browser with a dark hero and scrolling artwork shelves.
- Live recent-gallery browsing, pagination, local title/artist search, attribution, and source links.
- Validated full resolution original downloads and wallpaper application to all currently connected displays.
- Persistent originals and preview caches with bounded downloads and atomic file publication.
- First-launch walkthrough, replayable Guide, keyboard controls, and an explicit Motion toggle.
- App icon and macOS app bundle, with separate Apple Silicon and Intel release archives.

Known limitations: ad-hoc signing without Apple notarization; live-gallery format/access dependence; no automatic updater, wallpaper rotation, launch at login, or automatic cache-size cap. Other Spaces and disconnected displays are outside the supported wallpaper behavior. Motion must be disabled in-app because the current GPUI version does not expose the system reduced-motion preference.
