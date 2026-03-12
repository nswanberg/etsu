#!/bin/zsh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
LAUNCH_AGENTS_DIR="$HOME/Library/LaunchAgents"
PRIMARY_LABEL="com.seatedro.etsu"
LEGACY_LABEL="com.nswanberg.etsu"
PLIST_PATH="$LAUNCH_AGENTS_DIR/$PRIMARY_LABEL.plist"
LEGACY_PLIST_PATH="$LAUNCH_AGENTS_DIR/$LEGACY_LABEL.plist"
ETSU_BIN_PATH="${ETSU_BIN_PATH:-$REPO_ROOT/target/release/etsu}"
LOG_DIR="$HOME/Library/Logs"
APP_SUPPORT_DIR="$HOME/Library/Application Support/com.seatedro.etsu"
CONFIG_PATH="$APP_SUPPORT_DIR/config.toml"
DB_PATH="$APP_SUPPORT_DIR/etsu.db"

if [[ ! -x "$ETSU_BIN_PATH" ]]; then
  echo "Missing ETSU binary at $ETSU_BIN_PATH" >&2
  echo "Build it first with: cargo build --release" >&2
  exit 1
fi

stop_agent() {
  local label="$1"
  local plist_path="$2"

  launchctl bootout "gui/$(id -u)/$label" 2>/dev/null || true
  if [[ -f "$plist_path" ]]; then
    launchctl bootout "gui/$(id -u)" "$plist_path" 2>/dev/null || true
  fi
}

backup_legacy_plist() {
  if [[ -f "$LEGACY_PLIST_PATH" ]]; then
    local backup_path="${LEGACY_PLIST_PATH}.bak.$(date +%Y%m%d%H%M%S)"
    mv "$LEGACY_PLIST_PATH" "$backup_path"
    echo "Backed up legacy LaunchAgent to $backup_path"
  fi
}

mkdir -p "$LAUNCH_AGENTS_DIR" "$LOG_DIR" "$APP_SUPPORT_DIR"

stop_agent "$LEGACY_LABEL" "$LEGACY_PLIST_PATH"
backup_legacy_plist
stop_agent "$PRIMARY_LABEL" "$PLIST_PATH"

cat >"$PLIST_PATH" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>$PRIMARY_LABEL</string>
  <key>ProgramArguments</key>
  <array>
    <string>$ETSU_BIN_PATH</string>
  </array>
  <key>WorkingDirectory</key>
  <string>$(dirname "$ETSU_BIN_PATH")</string>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>ProcessType</key>
  <string>Interactive</string>
  <key>LimitLoadToSessionType</key>
  <array>
    <string>Aqua</string>
  </array>
  <key>StandardOutPath</key>
  <string>$LOG_DIR/etsu.launchd.log</string>
  <key>StandardErrorPath</key>
  <string>$LOG_DIR/etsu.launchd.err.log</string>
</dict>
</plist>
PLIST

launchctl bootstrap "gui/$(id -u)" "$PLIST_PATH"
launchctl enable "gui/$(id -u)/$PRIMARY_LABEL"
launchctl kickstart -k "gui/$(id -u)/$PRIMARY_LABEL"

echo "Installed $PRIMARY_LABEL"
echo "Config file: $CONFIG_PATH"
echo "Local SQLite DB: $DB_PATH"
echo "LaunchAgent: $PLIST_PATH"
echo "Logs: $LOG_DIR/etsu.launchd.log and $LOG_DIR/etsu.launchd.err.log"
