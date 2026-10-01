#!/bin/sh
set -eu

cd "$(dirname "$0")"
[ "$(uname -s)" = Darwin ] || { echo 'Local installation requires macOS.' >&2; exit 1; }
[ -d /Applications ] && [ -w /Applications ] || {
  echo '/Applications must be writable by the current user.' >&2
  exit 1
}

./scripts/build-macos.sh "$@"

destination=/Applications/Reframed.app
staging=$(mktemp -d /Applications/.Reframed-install.XXXXXX)
cleanup() {
  status=$?
  trap - 0 HUP INT TERM
  if { [ -e "$staging/previous.app" ] || [ -L "$staging/previous.app" ]; } &&
     [ ! -e "$destination" ] && [ ! -L "$destination" ]; then
    if ! mv "$staging/previous.app" "$destination"; then
      printf 'Could not restore the previous app; it remains at %s/previous.app\n' "$staging" >&2
      exit 1
    fi
  fi
  rm -rf "$staging"
  exit "$status"
}
trap cleanup 0
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM

ditto dist/Reframed.app "$staging/Reframed.app"
codesign --verify --strict --verbose=2 "$staging/Reframed.app"
if [ -e "$destination" ] || [ -L "$destination" ]; then
  mv "$destination" "$staging/previous.app"
fi
mv "$staging/Reframed.app" "$destination"
printf 'Installed %s\n' "$destination"
