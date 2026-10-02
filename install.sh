#!/bin/sh
set -eu

cd "$(dirname "$0")"
case "$(uname -s)" in
  Linux) exec ./scripts/install-linux.sh "$@" ;;
  Darwin) ;;
  *) echo 'Local installation supports macOS and Linux.' >&2; exit 1 ;;
esac
[ -d /Applications ] && [ -w /Applications ] || {
  echo '/Applications must be writable by the current user.' >&2
  exit 1
}

./scripts/build-macos.sh "$@"

destination=/Applications/Pinacora.app
staging=$(mktemp -d /Applications/.Pinacora-install.XXXXXX)
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

ditto dist/Pinacora.app "$staging/Pinacora.app"
codesign --verify --strict --verbose=2 "$staging/Pinacora.app"
if [ -e "$destination" ] || [ -L "$destination" ]; then
  mv "$destination" "$staging/previous.app"
fi
mv "$staging/Pinacora.app" "$destination"
printf 'Installed %s\n' "$destination"
