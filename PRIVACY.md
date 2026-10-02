# Privacy

Reframed has no account system, analytics SDK, advertising, telemetry, or automatic crash reporting in its application code.

## Network requests

The app requests public gallery catalogue and artwork pages from `www.reframed.gallery`, and preview/original images from `cdn.reframed.gallery`, over HTTPS. Requests include a User-Agent and gallery Referer. Those services receive ordinary connection information, including your IP address, and the URLs you request. Their operators control their own logging and privacy practices. The app does not operate a server or send data to a project-maintainer service.

Search sends trimmed queries of at least two characters to Reframed’s gallery search endpoint after a short pause in typing. Refresh and Load more contact the gallery. Selecting artwork can fetch metadata and previews; Set wallpaper downloads its original if it is not already valid in the local cache. View source and the gallery link open your browser, which has its own history and privacy settings. The `--check-source` developer diagnostic also contacts the gallery and downloads one original.

## Local data

The app stores preview images in `~/Library/Caches/Reframed/previews`, originals in `~/Library/Application Support/Reframed/originals`, and a versioned walkthrough completion/skip preference in `~/Library/Application Support/Reframed/walkthrough.json`. Image filenames are hashes of source URLs. There is no automatic cache-size cap or expiry. Search text and the Motion setting are not persisted by the app.

On macOS 14 or newer, wallpaper application writes the local original’s file URL into macOS’s local wallpaper store to apply it to all Desktops, then restarts the wallpaper service. The app preserves a separate backup before each change at `~/Library/Application Support/com.apple.wallpaper/Store/Index.plist.reframed-backup-*`. These backups retain the previous local wallpaper configuration, including existing file paths, and are not automatically removed. On macOS 12–13 it passes the file URL to the public macOS API for connected displays in the current Desktop. Wallpaper application makes no additional network requests. Removing an original currently in use may affect your wallpaper.

Quit the app and use Finder to remove its cache and preference folders if desired. Deleting the app alone does not remove those files. Bug reports, screenshots, and logs that you choose to post on GitHub are subject to GitHub's policies; remove personal information before sharing them.

On Linux, originals and walkthrough preferences use `$XDG_DATA_HOME/reframed` (default `~/.local/share/reframed`); previews use `$XDG_CACHE_HOME/reframed/previews` (default `~/.cache/reframed/previews`). Relative or empty XDG values are ignored. Wallpaper application passes the retained original's path to local hyprpaper IPC on Hyprland or its file URI to GNOME background settings, without additional network requests. Hyprland changes are limited to the current session and do not edit startup configuration. The local installer writes a binary in `~/.local/bin`, plus a launcher and icon in the user data directory. Remove these files and the storage folders to uninstall and clear local data.
