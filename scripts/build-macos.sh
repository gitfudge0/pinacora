#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
[ "$(uname -s)" = Darwin ] || { echo 'This bundle requires macOS.' >&2; exit 1; }
profile=release
target=${CARGO_BUILD_TARGET:-$(rustc -vV | sed -n 's/^host: //p')}
while [ "$#" -gt 0 ]; do
  case "$1" in
    --debug) profile=debug; shift ;;
    --target) [ "$#" -ge 2 ] || { echo '--target requires a triple' >&2; exit 1; }; target=$2; shift 2 ;;
    *) echo "Usage: $0 [--debug] [--target <triple>]" >&2; exit 1 ;;
  esac
done
case "$target" in
  aarch64-apple-darwin) arch=arm64 ;;
  x86_64-apple-darwin) arch=x86_64 ;;
  *) echo "Unsupported macOS target: $target" >&2; exit 1 ;;
esac
[ -f resources/AppIcon.icns ] || { echo 'Missing resources/AppIcon.icns' >&2; exit 1; }
export MACOSX_DEPLOYMENT_TARGET=${MACOSX_DEPLOYMENT_TARGET:-12.0}
if [ "$profile" = release ]; then
  cargo build --release --locked --target "$target"
else
  cargo build --locked --target "$target"
fi
python3 scripts/collect-licenses.py --target "$target"
bundle=dist/Reframed.app
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources/licenses"
# Publish a new inode: overwriting an executable from a previously signed bundle
# can leave macOS enforcing its cached signature while verifying the new bytes.
bundle_executable_temp=$(mktemp "$bundle/Contents/MacOS/.Reframed.XXXXXX")
trap 'rm -f "$bundle_executable_temp"' 0
trap 'exit 129' HUP
trap 'exit 130' INT
trap 'exit 143' TERM
cp -p "${CARGO_TARGET_DIR:-target}/$target/$profile/reframed" "$bundle_executable_temp"
chmod +x "$bundle_executable_temp"
mv -f "$bundle_executable_temp" "$bundle/Contents/MacOS/Reframed"
cp resources/Info.plist "$bundle/Contents/Info.plist"
cp resources/AppIcon.icns "$bundle/Contents/Resources/AppIcon.icns"
cp -R dist/license-notices/. "$bundle/Contents/Resources/licenses/"
cp LICENSE THIRD_PARTY_NOTICES.md "$bundle/Contents/Resources/licenses/"
plutil -lint "$bundle/Contents/Info.plist"
lipo "$bundle/Contents/MacOS/Reframed" -verify_arch "$arch"
codesign --force --sign - "$bundle"
codesign --verify --strict --verbose=2 "$bundle"
printf 'Built %s\n' "$bundle"
