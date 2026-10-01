# Third-party notices

The MIT license in this repository covers Reframed's project code, documentation, and original project assets. Dependencies retain their own licenses. Gallery artwork is not bundled, and its copyrights and applicable terms remain separate from the software license. Artist attribution and links to the original gallery pages are shown in the app. This project is not affiliated with or endorsed by the gallery.

## Direct dependencies

The following license identifiers were read from the package metadata for the direct dependency versions in `Cargo.lock` when preparing 0.1.0:

| Crate | Version | Declared license |
| --- | --- | --- |
| anyhow | 1.0.104 | MIT OR Apache-2.0 |
| plist | 1.10.1 | MIT |
| tempfile | 3.27.0 | MIT OR Apache-2.0 |
| gpui | 0.2.2 | Apache-2.0 |
| unicode-segmentation | 1.13.3 | MIT OR Apache-2.0 |
| async-channel | 2.5.0 | Apache-2.0 OR MIT |
| raw-window-handle | 0.6.2 | MIT OR Apache-2.0 |
| ureq | 2.12.1 | MIT OR Apache-2.0 |
| serde | 1.0.229 | MIT OR Apache-2.0 |
| serde_json | 1.0.151 | MIT OR Apache-2.0 |
| url | 2.5.8 | MIT OR Apache-2.0 |
| sha2 | 0.10.9 | MIT OR Apache-2.0 |
| image | 0.25.10 | MIT OR Apache-2.0 |
| objc2 | 0.6.4 | MIT |
| objc2-app-kit | 0.3.2 | Zlib OR Apache-2.0 OR MIT |
| objc2-foundation | 0.3.2 | MIT |

GPUI is developed by the [Zed project](https://github.com/zed-industries/zed) and distributed under Apache-2.0. Its upstream copyright and license notices remain applicable. Other dependencies and transitive dependencies may include additional attribution requirements; this table is a direct-dependency summary, not a complete license audit.

## Bundled dependency notices

macOS app builds run `scripts/collect-licenses.py --target <macOS-target>` against the locked dependency graph. The app bundle's `Contents/Resources/licenses` contains package-specific source license/notice files, an `index.json` with declared license identifiers, authors, repositories and file origins, and the project's license and this notice. The graph includes reachable normal and build dependencies and can be a conservative superset of code present in the final executable.

Some published crates omit license files or contain broken links to workspace-level files. Checked-in supplements in `resources/dependency-licenses` preserve available upstream notices from the crate's recorded revision. They also include canonical SPDX license texts with pinned source URLs and hashes, and unchanged registry manifests. Where an upstream text could not be obtained, the exact published `.crate` source archive is included to preserve all shipped source material and any inline notices. Canonical texts are explicitly labelled as canonical, not represented as crate-provided files; copyright placeholders are not filled with invented names. Per-package `provenance.json` records the distinction and SHA-256 hashes.

The collector fails when it finds a package without source texts or a checked-in supplement, or if supplemental hashes do not match. This is a reproducible notice collection process, not a complete legal audit or a guarantee that every redistribution requirement has been identified. Before redistributing modified binaries, review the exact dependency graph, applicable licenses, and upstream notices. `Cargo.lock` identifies pinned packages; `cargo metadata --locked --format-version 1` exposes package license and source metadata.
