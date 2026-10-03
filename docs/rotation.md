# Rotation in the current checkout

Rotation is unreleased source-build behavior. This document records the current implementation, which is still under development. Released v0.3.0 behavior is recorded separately in [Changelog](../CHANGELOG.md). See [the README](../README.md#use) for controls and [Privacy](../PRIVACY.md) for network requests, storage, and retention.

## Sources and catalogue

Rotation is independent of gallery search. Entire gallery chooses artwork across the gallery and shuffles each known cycle. Selected artworks follows the saved collection order. Both require at least two distinct artworks. Intervals are whole minutes from 1 to 1,440.

A cold entire-gallery start uses pagination metadata to choose an initial candidate, normally reading the first, last, and selected pages before downloading its original. Inconsistent pagination or an exhausted partial pool falls back to a complete catalogue fetch. Start applies the prepared first original without waiting for the normal background catalogue load.

`rotation-catalogue.json` retains confirmed complete metadata, successfully shown artwork IDs, and the last shown ID. Before completion it can retain shown history without an inventory. A valid complete cache supplies the starting queue immediately on later starts. Resume reuses the saved queue when available. Cached metadata does not guarantee offline application; the next original must also be cached.

The full background load extends the initial partial queue while preserving its next candidate and consumed history. Refreshes of an already complete queue supply the next cycle. Shown history survives restarts and source changes, then resets at known full-cycle exhaustion. Invalid metadata becomes a cache miss with a diagnostic. Background catalogue failures keep the usable inventory, with up to three attempts in an engine generation and increasing retry delays. Cycle boundaries can start another refresh.

## Editing and control decisions

Opening settings copies saved preferences into a draft. Closing settings discards that draft. Save changes appears only after an edit; the close action becomes Cancel while edits are unsaved. Resume, Pause, Stop, and retry actions take effect immediately and are not undone by Cancel. The compact modal keeps status, source, and interval visible, with background operation and download details under How rotation works. Add to rotation saves an artwork to the collection; changing its order, source, or interval in settings takes effect when saved.

Start prepares and applies the first candidate immediately. An interval-only save preserves the current queue and prepared original, and resets the running countdown immediately. Changing a selected collection rebuilds preparation; if rotation was running, successful preparation starts a fresh countdown without changing the current wallpaper. Saving a source switch prepares the replacement paused and keeps the current wallpaper until Resume or Change now. A failed setup keeps the saved configuration and wallpaper rather than committing the failed replacement.

Pause and Stop cancel future scheduling and invalidate stale completions. They retain the saved collection. Resume starts a fresh interval. Change now requests the next change immediately. After sleep or an overdue deadline, the worker makes at most one change, then schedules a new interval.

The worker prepares one next original at a time. If it is not ready when due, the current wallpaper stays. Ordinary next-original failures retry after 30 and 60 seconds, then pause after the third failed attempt. Retry now resets those attempts. Entire-gallery originals returning HTTP 404 or 410 are skipped with a visible retry option; selected-artwork failures retain the candidate. A wallpaper application failure pauses rotation and Retry change uses the same original. Initial setup failures require an explicit retry.

Manual Set wallpaper reserves the wallpaper operation, starts or reconnects to the worker, and pauses rotation before downloading or applying. If pausing fails, the manual operation keeps the wallpaper. Successful manual application records the current artwork with the worker. An already running HTTP request may finish and retain a file after Pause or Stop, but a stale completion cannot apply it.

## GUI, worker, and login

Closing the GUI leaves the background worker running. Its runtime file retains active intent, deadline, queue, and current wallpaper metadata for restart. Opening the GUI polls status; it does not register or start the worker by itself. Explicit rotation commands and manual Set wallpaper call `ensure_running`. Registration uses the invoking executable, so source builds can register a Cargo output path. Registration alone does not enable rotation.

Linux uses the per-user `pinacora-rotation.service` and an XDG autostart bootstrap to capture the current graphical session. GNOME and other sessions supporting XDG autostart can restore the worker at login. Bare Hyprland needs the `exec-once` bootstrap shown in the README; uwsm-managed sessions can use XDG autostart. macOS uses the per-user Aqua LaunchAgent `com.pinacora.rotation`. See [Troubleshooting](troubleshooting.md) to inspect the exact executable and registration.

Rotation uses the same platform application paths as manual wallpaper changes. macOS 14 or newer uses the all-Desktops wallpaper-store method. Older macOS uses the main-thread NSWorkspace API for connected displays in the current Desktop. Linux supports GNOME and Hyprland with hyprpaper 0.8 or newer. These are implementation paths, not a claim of runtime verification on every supported platform.

Each change can retain an original, and macOS all-Desktops changes also retain a separate wallpaper-store backup. Neither has an automatic size cap or expiry. Removing the GUI alone does not remove service registration or stored files; stop rotation and follow [removal instructions](../README.md#remove-background-rotation).
