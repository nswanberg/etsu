#!/bin/zsh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
BUILD_BIN_PATH="${ETSU_BUILD_BIN_PATH:-$REPO_ROOT/target/release/etsu}"
APP_BUNDLE_PATH="${ETSU_APP_BUNDLE_PATH:-$HOME/Applications/Etsu.app}"
APP_CONTENTS_DIR="$APP_BUNDLE_PATH/Contents"
APP_MACOS_DIR="$APP_CONTENTS_DIR/MacOS"
APP_BIN_PATH="$APP_MACOS_DIR/etsu"
INFO_PLIST_PATH="$APP_CONTENTS_DIR/Info.plist"
BUNDLE_IDENTIFIER="${ETSU_BUNDLE_IDENTIFIER:-com.seatedro.etsu}"
CODESIGN_IDENTITY_NAME="${ETSU_CODESIGN_IDENTITY:-ETSU Development}"

note() {
  printf '%s\n' "$*"
}

warn() {
  printf '%s\n' "$*" >&2
}

find_codesign_identity() {
  local identity_name="$1"

  [[ -n "$identity_name" ]] || return 1

  security find-identity -v -p codesigning 2>/dev/null \
    | grep -F "\"$identity_name\"" \
    | head -n 1
}

write_info_plist() {
  cat >"$INFO_PLIST_PATH" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleDisplayName</key>
  <string>Etsu</string>
  <key>CFBundleExecutable</key>
  <string>etsu</string>
  <key>CFBundleIdentifier</key>
  <string>$BUNDLE_IDENTIFIER</string>
  <key>CFBundleName</key>
  <string>Etsu</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.0</string>
  <key>CFBundleVersion</key>
  <string>1</string>
  <key>LSUIElement</key>
  <true/>
</dict>
</plist>
EOF
}

sign_app_bundle() {
  local identity_line

  identity_line="$(find_codesign_identity "$CODESIGN_IDENTITY_NAME" || true)"
  if [[ -n "$identity_line" ]]; then
    codesign --force --sign "$CODESIGN_IDENTITY_NAME" "$APP_BUNDLE_PATH"
    note "Signed Etsu.app with local identity: $CODESIGN_IDENTITY_NAME"
    return
  fi

  codesign --force --sign - "$APP_BUNDLE_PATH"
  warn "Installed Etsu.app with ad-hoc signing."
  warn "macOS Input Monitoring / Accessibility grants may not survive reinstalls."
  warn "For repeatable local installs, create a local codesigning identity and rerun setup:"
  warn "  ETSU_CODESIGN_IDENTITY=\"ETSU Development\" ./scripts/create_macos_dev_signing_identity.sh"
}

if [[ ! -x "$BUILD_BIN_PATH" ]]; then
  echo "Missing ETSU binary at $BUILD_BIN_PATH" >&2
  echo "Build it first with: cargo build --release" >&2
  exit 1
fi

mkdir -p "$APP_MACOS_DIR"
cp "$BUILD_BIN_PATH" "$APP_BIN_PATH"
chmod +x "$APP_BIN_PATH"
write_info_plist
sign_app_bundle

note "Installed app bundle: $APP_BUNDLE_PATH"
note "Installed binary: $APP_BIN_PATH"
