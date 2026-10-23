#!/usr/bin/env bash
# Wraps a release binary into Glyph.app and puts it into a disk image with an
# Applications shortcut, so it installs by dragging like any Mac app.
# Usage: packaging/macos/build-dmg.sh <glyph binary> <version> <output.dmg>
set -euo pipefail

BIN="$1"
VERSION="$2"
OUT="$3"
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$HERE/../.."

STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
APP="$STAGE/Glyph.app"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "$BIN" "$APP/Contents/MacOS/glyph"
chmod +x "$APP/Contents/MacOS/glyph"
sed "s/@VERSION@/$VERSION/g" "$HERE/Info.plist" > "$APP/Contents/Info.plist"
cp "$ROOT/assets/icons/glyph.icns" "$APP/Contents/Resources/glyph.icns"
# Ad-hoc signature: no Apple developer ID yet, so the first start goes through
# "Open anyway" in System Settings, but the bundle is not reported as damaged.
codesign --force --deep --sign - "$APP"
ln -s /Applications "$STAGE/Applications"

mkdir -p "$(dirname "$OUT")"
hdiutil create -volname Glyph -srcfolder "$STAGE" -format UDZO -ov "$OUT"
