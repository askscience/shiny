#!/usr/bin/env bash
# Build a launchable app bundle for the desktop browser.
#
# Why this exists: macOS refuses microphone and camera access to a bare
# executable. `getUserMedia` in WKWebView needs the *hosting application* to
# declare `NSMicrophoneUsageDescription` in its `Info.plist` and to be a real
# `.app` bundle that TCC can key a permission grant to — otherwise the request
# is denied with no prompt, which is exactly what "it has no microphone access"
# looks like. Running the raw binary from a terminal attributes the request to
# the terminal instead, so the app never gets its own grant.
#
# On Linux there is no bundle: the equivalent is a `.desktop` entry plus the
# binary, so this script creates one when asked for it and otherwise just builds.
#
#   ./scripts/bundle-app.sh            # build + bundle (macOS .app, Linux no-op)
#   ./scripts/bundle-app.sh --run      # build, bundle, and launch
#   ./scripts/bundle-app.sh --debug    # use the debug binary
set -euo pipefail

cd "$(dirname "$0")/.."

PROFILE="release"
RUN=0
for arg in "$@"; do
  case "$arg" in
    --debug) PROFILE="debug" ;;
    --run) RUN=1 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

APP_NAME="Peakd"
BUNDLE_ID="com.askscience.peakd"
BIN="target/${PROFILE}/peakd"

echo "==> building peakd (${PROFILE})"
if [ "$PROFILE" = "release" ]; then
  cargo build --release -p peakd
else
  cargo build -p peakd
fi

if [ ! -x "$BIN" ]; then
  echo "error: $BIN was not produced" >&2
  exit 1
fi

case "$(uname -s)" in
  Darwin)
    APP="dist/${APP_NAME}.app"
    echo "==> assembling $APP"
    rm -rf "$APP"
    mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
    cp "$BIN" "$APP/Contents/MacOS/${APP_NAME}"

    # The usage strings are what macOS shows in the permission prompt, and
    # their presence is what makes the prompt appear at all. `LSUIElement` is
    # deliberately absent: this is a regular app with a Dock icon and a menu.
    cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>${APP_NAME}</string>
    <key>CFBundleDisplayName</key>
    <string>PEAK'D!</string>
    <key>CFBundleExecutable</key>
    <string>${APP_NAME}</string>
    <key>CFBundleIdentifier</key>
    <string>${BUNDLE_ID}</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <!-- Required for navigator.mediaDevices.getUserMedia (voice input). -->
    <key>NSMicrophoneUsageDescription</key>
    <string>PEAK'D! uses the microphone so you can talk to your assistant.</string>
    <key>NSCameraUsageDescription</key>
    <string>PEAK'D! uses the camera only if you enable video capture.</string>
    <!-- The app talks to its own server on loopback. -->
    <key>NSAppTransportSecurity</key>
    <dict>
        <key>NSAllowsLocalNetworking</key>
        <true/>
    </dict>
</dict>
</plist>
PLIST

    # Ad-hoc sign so the bundle has a stable identity for TCC. Without a
    # signature macOS cannot remember a microphone grant between launches and
    # re-prompts (or silently denies) every time.
    if command -v codesign >/dev/null 2>&1; then
      echo "==> ad-hoc signing"
      codesign --force --deep --sign - "$APP" 2>&1 | sed 's/^/    /' || \
        echo "    (signing failed; the app still runs, but macOS may not remember permissions)"
    fi

    echo "==> built $APP"
    if [ "$RUN" = "1" ]; then
      echo "==> launching"
      open "$APP"
    fi
    ;;

  Linux)
    # No bundle format to build. A .desktop entry is what makes the app
    # launchable from a menu and gives it a stable name; permissions for
    # microphone/camera are handled by the portal at runtime, not a manifest.
    DESKTOP="dist/peakd.desktop"
    echo "==> writing $DESKTOP"
    mkdir -p dist
    cat > "$DESKTOP" <<DESK
[Desktop Entry]
Type=Application
Name=PEAK'D!
Comment=Peak'd desktop browser
Exec=$(pwd)/${BIN}
Terminal=false
Categories=Network;WebBrowser;
DESK
    chmod +x "$DESKTOP"
    echo "==> built $DESKTOP (binary: $BIN)"
    if [ "$RUN" = "1" ]; then
      echo "==> launching"
      "$BIN" &
    fi
    ;;

  *)
    echo "==> no bundling needed on $(uname -s); binary is at $BIN"
    ;;
esac
