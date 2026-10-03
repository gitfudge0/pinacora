# Troubleshooting a checkout

Start with executable identity. A launcher may still use an installed release while the source checkout contains unreleased changes. There is no `--version` diagnostic in the current executable.

## Which build is running?

On Linux, list both GUI and background worker processes:

```sh
pgrep -a -x pinacora
```

For each PID, inspect `/proc/PID/exe`. Substitute the numeric PID below:

```sh
readlink /proc/PID/exe
sha256sum /proc/PID/exe
sha256sum ~/.local/bin/pinacora target/debug/pinacora target/release/pinacora dist/linux/pinacora
```

Missing comparison files are expected if that profile was not built. Compare the running executable's hash with the intended artifact. A path marked `deleted` can be an old executable that the installer replaced while it was running. Reopen the GUI after installation. An existing rotation worker can still hold the previous binary even after reopening the GUI, so inspect its PID separately.

`cargo run --locked` uses the debug profile by default. Build scripts and `./install.sh` default to release; `--debug` selects debug. `CARGO_TARGET_DIR`, an explicit target triple, `CARGO_BUILD_TARGET`, and Cargo configuration can change artifact paths. On macOS, the packaging script uses `target/<target-triple>/<profile>/pinacora` by default. Compare the matching profile and target directory, not whichever file has the newest timestamp. A hash match identifies the same artifact; it does not prove that the artifact was built from today's checkout.

On macOS, use `pgrep -fl Pinacora` and `ps -p PID -o command=` to inspect the GUI, and the LaunchAgent command below for the worker. Compare the bundle executables at `/Applications/Pinacora.app/Contents/MacOS/Pinacora` and `dist/Pinacora.app/Contents/MacOS/Pinacora` with `shasum -a 256`. A source-run worker may instead point into a Cargo target directory.

## Background service identity and logs

These Linux commands only inspect the user service:

```sh
systemctl --user status pinacora-rotation.service --no-pager
systemctl --user show pinacora-rotation.service -p MainPID -p ExecStart
systemctl --user cat pinacora-rotation.service
journalctl --user -u pinacora-rotation.service -n 100 --no-pager
```

Registration lives at `$XDG_CONFIG_HOME/systemd/user/pinacora-rotation.service` and `$XDG_CONFIG_HOME/autostart/pinacora-rotation.desktop`, with `~/.config` as the fallback for unset or relative `XDG_CONFIG_HOME`. The bootstrap records graphical session variables in `$XDG_RUNTIME_DIR/pinacora/rotation-session.env`. Inspect the service command and session paths when a GUI works but the worker cannot reach Hyprland, GNOME, or the display. `ensure_running` registers the executable that invoked it, which may be a source build rather than the installed app.

On macOS:

```sh
launchctl print "gui/$(id -u)/com.pinacora.rotation"
plutil -p ~/Library/LaunchAgents/com.pinacora.rotation.plist
```

The LaunchAgent label is `com.pinacora.rotation`. Check it in macOS Console for failures; the current registration does not configure a dedicated log file. See [Rotation](rotation.md) for login differences and [the README](../README.md#remove-background-rotation) for removal instructions.

## Trace the error to its owner

| Symptom | Start here |
| --- | --- |
| Wrong gallery page or search results | `src/catalogue.rs`: `fetch_page_info`, `parse_page_info`, `fetch_search`; then `src/ui.rs`: `load`, `SiteSearch` |
| Preparation failure or unavailable original | `src/rotation_service.rs`: `launch`, `completed`; `src/rotation_catalogue.rs`: `bootstrap`; `src/download.rs`: `fetch` |
| Catalogue cache or refresh diagnostic | `src/rotation_catalogue.rs`: `load`, `Cache::validate`, `collect_with`; `src/rotation_service.rs`: `Event::Catalogue` |
| Wallpaper change failure after download | `src/rotation_service.rs`: `apply_due`, `RealEffects`; `src/platform.rs` and the platform-specific module |
| Service not ready or unable to reconnect | `src/rotation_service_manager.rs`: `ensure_running`, `wait_until_ready`; `src/rotation_service.rs`: `request`, `bind_owned` |
| Invalid preferences or restored runtime | `src/rotation.rs`: `load`, `Preferences::validate`; `src/rotation_service.rs`: `Engine::load` |

Preparation and wallpaper application are separate operations. A preparation error keeps the wallpaper; a failed application pauses rotation and retains the candidate for retry. `--check-source` contacts the live gallery and downloads one original without applying it. It can help isolate parsing/download failures, but it is not an offline check.

On macOS 14 or newer, `apply_all_desktops` changes the wallpaper store and runs without requiring the main thread. The older macOS `platform::apply` uses NSWorkspace and requires the main thread; the GUI fallback runs there. On Linux, `src/platform/linux.rs` plans and executes hyprpaper IPC or GNOME settings commands. Use mocks or command planning to test these paths without changing a developer's wallpaper.

## Find the actual storage directory

`src/storage.rs` prefers an existing legacy directory even when a new Pinacora directory also exists. It resolves data and caches independently; never assume both use the same app name.

| Platform | Data | Cache |
| --- | --- | --- |
| Linux | `$XDG_DATA_HOME/pinacora`, default `~/.local/share/pinacora`; existing `reframed` takes priority | `$XDG_CACHE_HOME/pinacora`, default `~/.cache/pinacora`; existing `reframed` takes priority |
| macOS | `~/Library/Application Support/Pinacora`; existing `Reframed` takes priority | `~/Library/Caches/Pinacora`; existing `Reframed` takes priority |

Empty or relative Linux XDG data/cache values use the HOME fallback. Data holds `originals/`, `walkthrough.json`, `rotation.json`, `rotation-catalogue.json`, `rotation-runtime.json`, and `service/`. Previews live under the cache directory. Moving or removing originals can break active wallpaper paths. See [Privacy](../PRIVACY.md) for retention and macOS wallpaper-store backups.
