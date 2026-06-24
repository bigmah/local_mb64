#!/usr/bin/env bash
#
# Mario Builder 64 — macOS installer.
#
#   curl -fsSL https://raw.githubusercontent.com/bigmah/mb64_mac/main/install.sh | bash
#
# Downloads the prebuilt launcher and installs it as an app. NOTHING is compiled
# here and no ROM or game data is downloaded — you supply your own ROM inside the
# app, and it builds the game locally the first time you run it.
#
# No code signing, no notarization, no Apple account: the binaries carry only the
# toolchain's anonymous ad-hoc signature, and curl downloads don't get macOS's
# quarantine flag, so the app just opens — no Gatekeeper "unidentified developer"
# prompt.
set -euo pipefail

REPO="bigmah/mb64_mac"
ASSET="mb64-macos-arm64.tar.gz"
APP_NAME="Mario Builder 64 Launcher"

log()  { printf '\033[1;36m==>\033[0m %s\n' "$*"; }
warn() { printf '\033[1;33mNote:\033[0m %s\n' "$*"; }
die()  { printf '\033[1;31mError:\033[0m %s\n' "$*" >&2; exit 1; }

[ "$(uname -s)" = "Darwin" ] || die "This installer is for macOS."
[ "$(uname -m)" = "arm64" ] || warn "Built for Apple Silicon; Intel Macs are untested."

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# 1. Get the prebuilt binaries (a local tarball, a pinned version, or the latest release).
if [ -n "${MB64_TARBALL:-}" ]; then
  log "Using local tarball: $MB64_TARBALL"
  cp "$MB64_TARBALL" "$WORK/$ASSET"
else
  if [ -n "${MB64_VERSION:-}" ]; then
    URL="https://github.com/$REPO/releases/download/$MB64_VERSION/$ASSET"
  else
    log "Finding the latest release…"
    URL="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
            | grep -o "https://github.com/$REPO/releases/download/[^\"]*/$ASSET" | head -1)"
    [ -n "$URL" ] || die "No '$ASSET' found in the latest release — has one been published yet?"
  fi
  log "Downloading $URL"
  curl -fL --progress-bar "$URL" -o "$WORK/$ASSET" || die "download failed"
fi

# 2. Unpack and sanity-check.
log "Unpacking…"
tar -C "$WORK" -xzf "$WORK/$ASSET"
[ -f "$WORK/mb64-launcher" ] && [ -f "$WORK/mb64-build" ] || die "tarball is missing the expected binaries"

# 3. Pick a writable install location.
APP_DIR="${MB64_APP_DIR:-/Applications}"
mkdir -p "$APP_DIR" 2>/dev/null || true
if [ ! -w "$APP_DIR" ]; then
  APP_DIR="$HOME/Applications"
  warn "/Applications isn't writable; installing to $APP_DIR instead."
  mkdir -p "$APP_DIR"
fi
APP="$APP_DIR/$APP_NAME.app"

# 4. Assemble the .app bundle (both binaries side by side so the launcher finds
#    its orchestrator as a sibling).
log "Installing → $APP"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
cp "$WORK/mb64-launcher" "$APP/Contents/MacOS/mb64-launcher"
cp "$WORK/mb64-build"    "$APP/Contents/MacOS/mb64-build"
chmod +x "$APP/Contents/MacOS/mb64-launcher" "$APP/Contents/MacOS/mb64-build"

VER="${MB64_VERSION#v}"; VER="${VER:-1.0}"
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
  <key>CFBundleShortVersionString</key><string>$VER</string>
  <key>CFBundleVersion</key><string>$VER</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
PLIST

# Belt-and-suspenders: curl downloads aren't quarantined, but clear it anyway.
xattr -dr com.apple.quarantine "$APP" 2>/dev/null || true

log "Installed."
echo
echo "    Opening $APP_NAME — set it up, add your Super Mario 64 ROM, and play."
echo
open "$APP" 2>/dev/null || warn "Open it yourself from $APP_DIR → $APP_NAME"
