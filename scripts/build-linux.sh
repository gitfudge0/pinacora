#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
[ "$(uname -s)" = Linux ] || { echo 'This build requires Linux.' >&2; exit 1; }
profile=release
case "${1:-}" in
  --debug) profile=debug; shift ;;
  '') ;;
  *) echo "Usage: $0 [--debug]" >&2; exit 1 ;;
esac
[ "$#" -eq 0 ] || { echo "Usage: $0 [--debug]" >&2; exit 1; }
if [ "$profile" = release ]; then cargo build --release --locked; else cargo build --locked; fi
mkdir -p dist/linux
install -m 755 "${CARGO_TARGET_DIR:-target}/$profile/pinacora" dist/linux/pinacora
install -m 644 resources/app-icon.png dist/linux/pinacora.png
printf 'Built dist/linux/pinacora\n'
