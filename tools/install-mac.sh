#!/bin/sh
# Build the release bundle and put it where the Dock link points: /Applications/Any Two Keys.app.
#   tools/install-mac.sh            build, then install
#   tools/install-mac.sh --no-build install the bundle already built
# The Dock keeps working because the PATH stays the same; only the bundle under it is replaced.
set -eu
cd "$(dirname "$0")/.."
APP="Any Two Keys.app"
SRC="src-tauri/target/release/bundle/macos/$APP"
DST="/Applications/$APP"
[ "${1:-}" = "--no-build" ] || npm run build
[ -d "$SRC" ] || { echo "no bundle at $SRC" >&2; exit 1; }
if pgrep -fq "/Applications/$APP/Contents/MacOS/"; then
  echo "Any Two Keys is running: quitting it first"
  osascript -e 'tell application "Any Two Keys" to quit' || true
  for i in 1 2 3 4 5 6 7 8 9 10; do pgrep -fq "/Applications/$APP/Contents/MacOS/" || break; sleep 1; done
fi
rm -rf "$DST"
ditto "$SRC" "$DST"
[ "$(md5 -q "$SRC/Contents/MacOS/"*)" = "$(md5 -q "$DST/Contents/MacOS/"*)" ] || { echo "install did not match the build" >&2; exit 1; }
echo "installed $(stat -f '%Sm' "$DST/Contents/MacOS/"*) -> $DST"
