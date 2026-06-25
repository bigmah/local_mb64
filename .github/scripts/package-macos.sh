#!/usr/bin/env bash
# Package the prebuilt launcher binaries for the install script to download.
#
# Produces a plain .tar.gz of our two binaries (mb64-launcher + mb64-build). We do
# NOT sign with a Developer ID or notarize — the binaries already carry the
# toolchain's automatic anonymous ad-hoc signature (required to run on Apple
# Silicon), and install.sh fetches them with curl, which does not set the
# com.apple.quarantine flag, so Gatekeeper never prompts. No Apple account or
# identity is involved, and nothing copyrighted is shipped.
set -euo pipefail

DIST="dist"
STAGE="$DIST/stage"
ASSET="mb64-macos-arm64.tar.gz"

rm -rf "$DIST"
mkdir -p "$STAGE"

cp target/release/mb64-launcher "$STAGE/mb64-launcher"
cp target/release/mb64-build    "$STAGE/mb64-build"
chmod +x "$STAGE/mb64-launcher" "$STAGE/mb64-build"

# App-bundle icon, placed into Contents/Resources by install.sh.
cp launcher/assets/AppIcon.icns "$STAGE/AppIcon.icns"

tar -C "$STAGE" -czf "$DIST/$ASSET" mb64-launcher mb64-build AppIcon.icns
rm -rf "$STAGE"

( cd "$DIST" && shasum -a 256 "$ASSET" > "$ASSET.sha256" )

echo "Built $DIST/$ASSET"
cat "$DIST/$ASSET.sha256"
