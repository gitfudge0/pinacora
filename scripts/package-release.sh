#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
[ "$(uname -s)" = Darwin ] || { echo 'Release packaging requires macOS.' >&2; exit 1; }
target=${1:-$(rustc -vV | sed -n 's/^host: //p')}
tag=${2:-}
[ "$#" -le 2 ] || { echo "Usage: $0 [<target-triple>] [<release-tag>]" >&2; exit 1; }
case "$target" in
  aarch64-apple-darwin) arch=arm64 ;;
  x86_64-apple-darwin) arch=x86_64 ;;
  *) echo "Unsupported macOS target: $target" >&2; exit 1 ;;
esac
version=$(cargo metadata --no-deps --format-version 1 --locked | python3 -c 'import json,sys; m=json.load(sys.stdin); print(next(p["version"] for p in m["packages"] if p["id"] in m["workspace_members"] and p["name"] == "reframed"))')
plist_version=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' resources/Info.plist)
[ "$version" = "$plist_version" ] || { echo "Cargo version $version differs from bundle version $plist_version" >&2; exit 1; }
if [ -n "$tag" ] && [ "$tag" != "v$version" ]; then
  echo "Release tag $tag must match v$version" >&2
  exit 1
fi
./scripts/build-macos.sh --target "$target"
archive="Reframed-$version-macos-$arch.zip"
mkdir -p dist/releases
ditto -c -k --sequesterRsrc --keepParent dist/Reframed.app "dist/releases/$archive"
(cd dist/releases && shasum -a 256 "$archive" > "$archive.sha256")
printf 'Packaged dist/releases/%s (ad-hoc signed, not notarized)\n' "$archive"
