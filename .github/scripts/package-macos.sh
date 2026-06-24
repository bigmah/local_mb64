#!/usr/bin/env bash
# Assemble the launcher .app bundle and wrap it in a .dmg.
#
# Bundles BOTH our binaries side by side in Contents/MacOS/ so the launcher finds
# its mb64-build orchestrator as a sibling at runtime (see bootstrap::orchestrator_path).
# No game code or ROM is involved.
set -euo pipefail

APP_NAME="Mario Builder 64 Launcher"
DIST="dist"
APP="$DIST/$APP_NAME.app"
MACOS="$APP/Contents/MacOS"
RES="$APP/Contents/Resources"

# Version from the tag (vX.Y.Z -> X.Y.Z); fall back to 0.0.0 for manual runs.
REF="${GITHUB_REF_NAME:-dev}"
VERSION="${REF#v}"
case "$VERSION" in
  [0-9]*) : ;;
  *) VERSION="0.0.0" ;;
esac

rm -rf "$DIST"
mkdir -p "$MACOS" "$RES"

cp target/release/mb64-launcher "$MACOS/mb64-launcher"
cp target/release/mb64-build "$MACOS/mb64-build"
chmod +x "$MACOS/mb64-launcher" "$MACOS/mb64-build"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>$APP_NAME</string>
  <key>CFBundleDisplayName</key><string>$APP_NAME</string>
  <key>CFBundleIdentifier</key><string>com.bigmah.mb64launcher</string>
  <key>CFBundleExecutable</key><string>mb64-launcher</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Ad-hoc sign (reduces "app is damaged" Gatekeeper errors; still un-notarized,
# so first launch needs right-click -> Open).
codesign --force --deep --sign - "$APP" || echo "warning: ad-hoc codesign failed (continuing)"

# Build a .dmg with a drag-to-Applications layout.
STAGE="$DIST/dmg-stage"
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
DMG="$DIST/MarioBuilder64-Launcher-$VERSION-arm64.dmg"
hdiutil create -volname "$APP_NAME" -srcfolder "$STAGE" -ov -format UDZO "$DMG"
rm -rf "$STAGE"

echo "Built $DMG"
