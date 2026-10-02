Pinacora brings a new name and icon, a simpler welcome screen, and prebuilt Linux downloads. Artwork comes from [Reframed Gallery](https://www.reframed.gallery/); Pinacora is an independent, unofficial app. The gallery permits personal wallpaper use; artwork remains subject to its original rights and terms.

Downloads:

- `Pinacora-0.3.0-macos-arm64.zip` — Apple Silicon, macOS 12 or newer.
- `Pinacora-0.3.0-macos-x86_64.zip` — Intel, macOS 12 or newer.
- `Pinacora-0.3.0-linux-x86_64.tar.gz` — Linux x86_64, built on Ubuntu 24.04 (glibc 2.39 baseline).

Each archive has a `.sha256` checksum file. Linux: extract the archive and run `./install.sh` to install locally and add the app icon to your launcher. No Rust toolchain is needed; the installer uses Python 3. The app needs Vulkan with a compatible graphics driver, Fontconfig, FreeType, Wayland or X11, libxkbcommon, OpenSSL, and ALSA runtime libraries. Wallpaper setting supports GNOME and Hyprland with hyprpaper 0.8 or newer running with IPC enabled; other desktops support browsing and downloading.

macOS archives use the self-signed gitfudge certificate and are not notarized; macOS may require Open Anyway. Existing Reframed downloads and preferences remain accessible.
