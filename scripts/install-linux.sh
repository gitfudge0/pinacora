#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
./scripts/build-linux.sh "$@"
case "${HOME:-}" in /*) ;; *) echo 'HOME must be absolute.' >&2; exit 1 ;; esac
case "${XDG_DATA_HOME:-}" in /*) data_dir=$XDG_DATA_HOME ;; *) data_dir=$HOME/.local/share ;; esac
bin_dir=$HOME/.local/bin
mkdir -p "$bin_dir" "$data_dir/applications" "$data_dir/icons/hicolor/1024x1024/apps"
# Rename a new inode so an already running executable can be replaced safely.
binary_temp=$(mktemp "$bin_dir/.pinacora.XXXXXX")
trap 'rm -f "$binary_temp"' 0
install -m 755 dist/linux/pinacora "$binary_temp"
mv -f "$binary_temp" "$bin_dir/pinacora"
install -m 644 dist/linux/pinacora.png "$data_dir/icons/hicolor/1024x1024/apps/pinacora.png"
python3 - "$bin_dir/pinacora" "$data_dir/applications/pinacora.desktop" <<'PY'
import sys
from pathlib import Path
# Desktop Entry quoting has two escape layers for reserved characters.
executable = sys.argv[1].replace('\\', '\\\\\\\\').replace('"', '\\\\"').replace('`', '\\\\`').replace('$', '\\\\$').replace('%', '%%')
Path(sys.argv[2]).write_text('[Desktop Entry]\nType=Application\nName=Pinacora\nComment=Browse artwork and set your wallpaper\nExec="' + executable + '"\nIcon=pinacora\nTerminal=false\nCategories=Graphics;\nStartupWMClass=pinacora\n')
PY
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$data_dir/applications" || :
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -f -t "$data_dir/icons/hicolor" >/dev/null 2>&1 || :
printf 'Installed %s and the Pinacora application launcher.\n' "$bin_dir/pinacora"
