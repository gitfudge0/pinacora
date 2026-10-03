# Working in Pinacora

Pinacora is a Rust/GPUI desktop app. `src/lib.rs` exports gallery, download, storage, platform, and rotation modules. `src/main.rs` owns the executable entry points and declares the binary-only UI, search input, onboarding, and accessibility modules.

## Where to start

| Task | Files and search symbols |
| --- | --- |
| Gallery parsing, pagination, live search | `src/catalogue.rs`: `fetch_page_info`, `parse_page_info`, `fetch_search` |
| Original/preview downloads and validation | `src/download.rs`: `fetch`, `validate_url` |
| Browsing, selection, manual wallpaper | `src/ui.rs`: `Gallery`, `load`, `select`, `apply`, `SiteSearch` |
| Rotation controls and settings drafts | `src/ui.rs`: `rotation_command`, `rotation_commit`, `rotation_panel` |
| Rotation preferences, queue, pure scheduling | `src/rotation.rs`: `Preferences`, `Queue`, `Schedule` |
| Catalogue discovery, cache, shown history | `src/rotation_catalogue.rs`: `bootstrap`, `collect_with`, `Cache` |
| Background scheduling, retries, local commands | `src/rotation_service.rs`: `Command`, `Engine`, `completed`, `apply_due` |
| Login registration and service startup | `src/rotation_service_manager.rs`: `ensure_running`, `linux_commands`, `macos_commands` |
| Wallpaper application | `src/platform.rs`, `src/platform/linux.rs`, `src/platform/all_desktops.rs`: `apply`, `apply_all_desktops` |
| Storage and legacy paths | `src/storage.rs`: `data_dir`, `cache_dir`, `app_directory` |
| Text editing and keyboard input | `src/search_input.rs`: `TextInput`, `bind_keys` |
| First launch and accessibility | `src/onboarding.rs`, `src/accessibility.rs` |
| Packaging and local installation | `scripts/build-linux.sh`, `scripts/build-macos.sh`, `install.sh`, `scripts/install-linux.sh` |

Find the relevant symbols before reading large sections of `src/ui.rs`. For example, use `rg -n 'fn rotation_|Command::' src/ui.rs` or `rg -n 'bootstrap|adopt_complete|apply_due' src/rotation*.rs`, then read the surrounding function. Use `rg --files src scripts docs` to locate files.

## Build and behavior checks

[Contributing](CONTRIBUTING.md) has complete platform dependencies and packaging paths. Rust is pinned in `rust-toolchain.toml`; keep Cargo operations locked. The shared checks are `cargo fmt --all -- --check`, `cargo test --locked`, and `cargo clippy --locked --all-targets -- -D warnings`. Use fixtures, command planning, or mocks for wallpaper tests. Live-source diagnostics contact the gallery and retain an original.

A source checkout, Cargo output, packaged app, installed app, and already running worker can be different builds. Identify the running executable before diagnosing behavior using [Troubleshooting](docs/troubleshooting.md). Rotation is available starting in 0.4.0; see [Rotation](docs/rotation.md) and [Changelog](CHANGELOG.md). Storage still prefers existing Reframed directories, independently for data and caches. Search both names when investigating retained files.

When another agent is writing a module, get an explicit handoff with owned files, completed changes, and checks before reviewing it. An absent symbol or partial implementation during an active edit is not evidence of a finished defect. Re-read shared files after the handoff.
