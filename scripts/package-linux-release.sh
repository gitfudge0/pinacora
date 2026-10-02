#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
[ "$(uname -s)" = Linux ] && [ "$(uname -m)" = x86_64 ] || { echo 'Release packaging requires Linux x86_64.' >&2; exit 1; }
[ "$#" -le 1 ] || { echo "Usage: $0 [<release-tag>]" >&2; exit 1; }
tag=${1:-}
version=$(cargo metadata --no-deps --format-version 1 --locked | python3 -c 'import json,sys; m=json.load(sys.stdin); print(next(p["version"] for p in m["packages"] if p["id"] in m["workspace_members"] and p["name"] == "pinacora"))')
if [ -n "$tag" ] && [ "$tag" != "v$version" ]; then
  echo "Release tag $tag must match v$version" >&2
  exit 1
fi
./scripts/build-linux.sh
python3 scripts/collect-licenses.py --target x86_64-unknown-linux-gnu
name="Pinacora-$version-linux-x86_64"
staging="dist/$name"
# Only this release's generated staging directory is replaced.
rm -rf "$staging"
mkdir -p "$staging" dist/releases
install -m 755 dist/linux/pinacora "$staging/pinacora"
install -m 644 dist/linux/pinacora.png "$staging/pinacora.png"
install -m 755 scripts/install-linux-release.sh "$staging/install.sh"
install -m 644 LICENSE THIRD_PARTY_NOTICES.md "$staging/"
cp -R dist/license-notices "$staging/dependency-licenses"
archive="$name.tar.gz"
tar -C dist -czf "dist/releases/$archive" "$name"
(cd dist/releases && sha256sum "$archive" > "$archive.sha256")
printf 'Packaged dist/releases/%s\n' "$archive"
