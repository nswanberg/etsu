#!/bin/zsh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
PRIMARY_LABEL="com.seatedro.etsu"
LEGACY_LABEL="com.nswanberg.etsu"
APP_SUPPORT_DIR="$HOME/Library/Application Support/com.seatedro.etsu"
CONFIG_PATH="$APP_SUPPORT_DIR/config.toml"
EXAMPLE_CONFIG_PATH="$REPO_ROOT/config.example.toml"
LOCAL_DB_PATH="$APP_SUPPORT_DIR/etsu.db"
BACKUP_ROOT="${ETSU_BACKUP_DIR:-$APP_SUPPORT_DIR/backups}"
POSTGRES_URL="${ETSU_POSTGRES_URL:-}"
POSTGRES_URL_OP_REF="${ETSU_POSTGRES_URL_OP_REF:-}"
POSTGRES_URL_FILE="${ETSU_POSTGRES_URL_FILE:-}"
SUPABASE_URL="${ETSU_SUPABASE_URL:-}"
SUPABASE_URL_FILE="${ETSU_SUPABASE_URL_FILE:-}"
SUPABASE_API_KEY="${ETSU_SUPABASE_API_KEY:-}"
SUPABASE_API_KEY_FILE="${ETSU_SUPABASE_API_KEY_FILE:-}"
DEVICE_ID="${ETSU_DEVICE_ID:-}"
DEVICE_NAME="${ETSU_DEVICE_NAME:-}"
SKIP_BUILD="${ETSU_SKIP_BUILD:-0}"
APP_BUNDLE_PATH="${ETSU_APP_BUNDLE_PATH:-$HOME/Applications/Etsu.app}"
BUILD_BIN_PATH="$REPO_ROOT/target/release/etsu"
INSTALLED_BIN_PATH="$APP_BUNDLE_PATH/Contents/MacOS/etsu"
BACKUP_DIR=""
POSTGRES_URL_SOURCE="none"
SUPABASE_URL_SOURCE="none"
SUPABASE_API_KEY_SOURCE="none"
REMOTE_MODE="none"

note() {
  printf '%s\n' "$*"
}

warn() {
  printf '%s\n' "$*" >&2
}

read_existing_database_value() {
  local key_name="$1"

  [[ -f "$CONFIG_PATH" ]] || return 1

  python3 - "$CONFIG_PATH" "$key_name" <<'PY'
from pathlib import Path
import sys

try:
    import tomllib
except ModuleNotFoundError:
    import importlib
    tomllib = None

path = Path(sys.argv[1])
key_name = sys.argv[2]
text = path.read_text()

if tomllib is not None:
    data = tomllib.loads(text)
    value = data.get("database", {}).get(key_name, "")
    if value:
        print(value)
else:
    # Fallback for Python < 3.11: simple line-based parse
    for line in text.splitlines():
        stripped = line.strip()
        if stripped.startswith(key_name):
            _, _, val = stripped.partition("=")
            val = val.strip().strip('"')
            if val:
                print(val)
            break
PY
}

read_first_line_from_file() {
  local file_path="$1"

  [[ -f "$file_path" ]] || return 1

  python3 - "$file_path" <<'PY'
from pathlib import Path
import sys

path = Path(sys.argv[1])
for raw_line in path.read_text().splitlines():
    line = raw_line.strip()
    if line and not line.startswith("#"):
        print(line)
        break
PY
}

escape_toml_string() {
  local value="$1"
  value="${value//\\/\\\\}"
  value="${value//\"/\\\"}"
  printf '%s' "$value"
}

ensure_config_file() {
  mkdir -p "$APP_SUPPORT_DIR"
  if [[ ! -f "$CONFIG_PATH" ]]; then
    cp "$EXAMPLE_CONFIG_PATH" "$CONFIG_PATH"
  fi
}

insert_key_in_section() {
  local section="$1"
  local key="$2"
  local escaped_value="$3"
  local tmp_file
  tmp_file="$(mktemp)"

  awk -v section="[$section]" -v line="$key = \"$escaped_value\"" '
    $0 == section && inserted == 0 {
      print
      print line
      inserted = 1
      seen_section = 1
      next
    }
    {
      if ($0 == section) {
        seen_section = 1
      }
      print
    }
    END {
      if (inserted == 0) {
        if (seen_section == 0) {
          print ""
          print section
        }
        print line
      }
    }
  ' "$CONFIG_PATH" > "$tmp_file"

  mv "$tmp_file" "$CONFIG_PATH"
}

upsert_key() {
  local section="$1"
  local key="$2"
  local raw_value="$3"
  local escaped_value
  escaped_value="$(escape_toml_string "$raw_value")"

  if grep -Eq "^[[:space:]]*(#\\s*)?${key}[[:space:]]*=" "$CONFIG_PATH"; then
    KEY="$key" VALUE="$escaped_value" perl -0pi -e '
      s/^[ \t]*(?:#\s*)?\Q$ENV{KEY}\E\s*=.*$/$ENV{KEY} . " = \"" . $ENV{VALUE} . "\""/gme
    ' "$CONFIG_PATH"
  else
    insert_key_in_section "$section" "$key" "$escaped_value"
  fi
}

delete_key() {
  local key="$1"

  KEY="$key" perl -0pi -e '
    s/^[ \t]*(?:#\s*)?\Q$ENV{KEY}\E\s*=.*\n?//gme
  ' "$CONFIG_PATH"
}

resolve_postgres_url() {
  if [[ -n "$POSTGRES_URL" ]]; then
    POSTGRES_URL_SOURCE="ETSU_POSTGRES_URL"
    return
  fi

  if [[ -n "$POSTGRES_URL_OP_REF" ]]; then
    if ! command -v op >/dev/null 2>&1; then
      warn "ETSU_POSTGRES_URL_OP_REF was set, but the 1Password CLI (op) is not installed."
      exit 1
    fi
    POSTGRES_URL="$(op read "$POSTGRES_URL_OP_REF")"
    POSTGRES_URL_SOURCE="$POSTGRES_URL_OP_REF"
    return
  fi

  POSTGRES_URL="$(read_existing_database_value postgres_url || true)"
  if [[ -n "$POSTGRES_URL" ]]; then
    POSTGRES_URL_SOURCE="$CONFIG_PATH"
    return
  fi

  if [[ -n "$POSTGRES_URL_FILE" ]]; then
    POSTGRES_URL="$(read_first_line_from_file "$POSTGRES_URL_FILE" || true)"
    if [[ -n "$POSTGRES_URL" ]]; then
      POSTGRES_URL_SOURCE="$POSTGRES_URL_FILE"
      return
    fi
  fi

  local candidate_paths=(
    "$HOME/Library/Application Support/com.seatedro.etsu/postgres_dsn.txt"
    "$HOME/Library/Application Support/com.seatedro.etsu/postgres_url.txt"
    "$HOME/Dropbox/Records/PersonalData/Etsu/postgres_dsn.txt"
    "$HOME/Dropbox/Records/PersonalData/Etsu/postgres_url.txt"
    "$HOME/Dropbox/Records/PersonalData/Etsu/supabase_dsn.txt"
  )

  local candidate_path
  for candidate_path in "${candidate_paths[@]}"; do
    [[ -n "$candidate_path" ]] || continue
    POSTGRES_URL="$(read_first_line_from_file "$candidate_path" || true)"
    if [[ -n "$POSTGRES_URL" ]]; then
      POSTGRES_URL_SOURCE="$candidate_path"
      return
    fi
  done
}

resolve_supabase_url() {
  if [[ -n "$SUPABASE_URL" ]]; then
    SUPABASE_URL_SOURCE="ETSU_SUPABASE_URL"
    return
  fi

  SUPABASE_URL="$(read_existing_database_value supabase_url || true)"
  if [[ -n "$SUPABASE_URL" ]]; then
    SUPABASE_URL_SOURCE="$CONFIG_PATH"
    return
  fi

  if [[ -n "$SUPABASE_URL_FILE" ]]; then
    SUPABASE_URL="$(read_first_line_from_file "$SUPABASE_URL_FILE" || true)"
    if [[ -n "$SUPABASE_URL" ]]; then
      SUPABASE_URL_SOURCE="$SUPABASE_URL_FILE"
      return
    fi
  fi

  local candidate_paths=(
    "$HOME/Library/Application Support/com.seatedro.etsu/supabase_url.txt"
    "$HOME/Dropbox/Records/PersonalData/Etsu/supabase_url.txt"
  )

  local candidate_path
  for candidate_path in "${candidate_paths[@]}"; do
    SUPABASE_URL="$(read_first_line_from_file "$candidate_path" || true)"
    if [[ -n "$SUPABASE_URL" ]]; then
      SUPABASE_URL_SOURCE="$candidate_path"
      return
    fi
  done
}

resolve_supabase_api_key() {
  if [[ -n "$SUPABASE_API_KEY" ]]; then
    SUPABASE_API_KEY_SOURCE="ETSU_SUPABASE_API_KEY"
    return
  fi

  SUPABASE_API_KEY="$(read_existing_database_value supabase_api_key || true)"
  if [[ -n "$SUPABASE_API_KEY" ]]; then
    SUPABASE_API_KEY_SOURCE="$CONFIG_PATH"
    return
  fi

  if [[ -n "$SUPABASE_API_KEY_FILE" ]]; then
    SUPABASE_API_KEY="$(read_first_line_from_file "$SUPABASE_API_KEY_FILE" || true)"
    if [[ -n "$SUPABASE_API_KEY" ]]; then
      SUPABASE_API_KEY_SOURCE="$SUPABASE_API_KEY_FILE"
      return
    fi
  fi

  local candidate_paths=(
    "$HOME/Library/Application Support/com.seatedro.etsu/supabase_api_key.txt"
    "$HOME/Library/Application Support/com.seatedro.etsu/supabase_publishable_key.txt"
    "$HOME/Dropbox/Records/PersonalData/Etsu/supabase_api_key.txt"
    "$HOME/Dropbox/Records/PersonalData/Etsu/supabase_publishable_key.txt"
  )

  local candidate_path
  for candidate_path in "${candidate_paths[@]}"; do
    SUPABASE_API_KEY="$(read_first_line_from_file "$candidate_path" || true)"
    if [[ -n "$SUPABASE_API_KEY" ]]; then
      SUPABASE_API_KEY_SOURCE="$candidate_path"
      return
    fi
  done
}

resolve_remote_config() {
  resolve_supabase_url
  resolve_supabase_api_key

  if [[ -n "$SUPABASE_URL" || -n "$SUPABASE_API_KEY" ]]; then
    if [[ -z "$SUPABASE_URL" || -z "$SUPABASE_API_KEY" ]]; then
      warn "Supabase REST sync was partially configured."
      warn "Both supabase_url and supabase_api_key are required."
      exit 1
    fi

    REMOTE_MODE="supabase"
    return
  fi

  resolve_postgres_url
  if [[ -n "$POSTGRES_URL" ]]; then
    REMOTE_MODE="postgres"
    return
  fi
}

stop_agent() {
  local label="$1"
  local plist_path="$HOME/Library/LaunchAgents/$label.plist"

  launchctl bootout "gui/$(id -u)/$label" 2>/dev/null || true
  if [[ -f "$plist_path" ]]; then
    launchctl bootout "gui/$(id -u)" "$plist_path" 2>/dev/null || true
  fi
}

stop_manual_processes() {
  local candidate_path
  local pids

  for candidate_path in "$BUILD_BIN_PATH" "$INSTALLED_BIN_PATH"; do
    pids="$(pgrep -f "$candidate_path" || true)"

    if [[ -z "$pids" ]]; then
      continue
    fi

    note "Stopping existing ETSU process(es) for $candidate_path"
    while IFS= read -r pid; do
      [[ -n "$pid" ]] || continue
      kill "$pid" 2>/dev/null || true
    done <<< "$pids"

    sleep 1

    pids="$(pgrep -f "$candidate_path" || true)"
    if [[ -n "$pids" ]]; then
      while IFS= read -r pid; do
        [[ -n "$pid" ]] || continue
        kill -9 "$pid" 2>/dev/null || true
      done <<< "$pids"
    fi
  done
}

stop_existing_etsu() {
  stop_agent "$LEGACY_LABEL"
  stop_agent "$PRIMARY_LABEL"
  stop_manual_processes
}

init_backup_dir() {
  BACKUP_DIR="$BACKUP_ROOT/$(date +%Y%m%d-%H%M%S)"
  mkdir -p "$BACKUP_DIR"
}

backup_file_if_present() {
  local file_path="$1"

  [[ -f "$file_path" ]] || return
  cp -p "$file_path" "$BACKUP_DIR/"
}

write_backup_stats() {
  [[ -f "$LOCAL_DB_PATH" ]] || return
  command -v sqlite3 >/dev/null 2>&1 || return

  sqlite3 "$LOCAL_DB_PATH" \
    "select count(*) as rows, min(timestamp) as first_ts, max(timestamp) as last_ts, sum(keypresses) as total_keys, sum(mouse_clicks) as total_clicks, sum(scroll_steps) as total_scrolls from metrics;" \
    > "$BACKUP_DIR/etsu.db.metrics.txt"
}

backup_existing_state() {
  init_backup_dir
  backup_file_if_present "$CONFIG_PATH"
  backup_file_if_present "$LOCAL_DB_PATH"
  write_backup_stats
}

run_build() {
  if [[ "$SKIP_BUILD" == "1" ]]; then
    return
  fi
  cargo build --release --manifest-path "$REPO_ROOT/Cargo.toml"
}

install_app_bundle() {
  ETSU_BUILD_BIN_PATH="$BUILD_BIN_PATH" \
    ETSU_APP_BUNDLE_PATH="$APP_BUNDLE_PATH" \
    "$SCRIPT_DIR/install_app_bundle.sh"
}

latest_log_path() {
  find "$APP_SUPPORT_DIR" -maxdepth 1 -type f -name 'etsu.log.*' -print | sort | tail -n 1
}

print_startup_status() {
  local log_path
  local startup_lines
  local _attempt
  local remote_status="unknown"
  local capture_status="unknown"

  for _attempt in 1 2 3 4 5; do
    log_path="$(latest_log_path)"

    if [[ -f "$log_path" ]]; then
      startup_lines="$(tail -n 120 "$log_path")"

      if grep -Fq "Supabase REST API connected at" <<< "$startup_lines"; then
        remote_status="connected"
      elif grep -Fq "Remote Postgres pool created." <<< "$startup_lines"; then
        remote_status="connected"
      elif grep -Fq "Failed to connect to remote Postgres DB:" <<< "$startup_lines"; then
        remote_status="failed"
      elif grep -Fq "Supabase REST API returned HTTP" <<< "$startup_lines"; then
        remote_status="failed"
      elif grep -Fq "Failed to reach Supabase REST API at" <<< "$startup_lines"; then
        remote_status="failed"
      elif grep -Fq "No remote Postgres URL configured." <<< "$startup_lines"; then
        remote_status="disabled"
      fi

      if grep -Fq "Input Monitoring permission is not granted" <<< "$startup_lines"; then
        capture_status="input_monitoring_missing"
      elif grep -Fq "Accessibility permission is not granted" <<< "$startup_lines"; then
        capture_status="accessibility_missing"
      elif grep -Fq "Input capture confirmed: first keyboard or mouse event received." <<< "$startup_lines"; then
        capture_status="confirmed"
      fi
    fi

    sleep 1
  done

  case "$remote_status" in
    connected)
      note "Remote sync: connected"
      ;;
    disabled)
      if [[ "$REMOTE_MODE" == "none" ]]; then
        note "Remote sync: disabled"
      else
        note "Remote sync: check $log_path"
      fi
      ;;
    failed)
      warn "Remote sync: connection failed"
      grep -E "Failed to connect to remote Postgres DB:|Supabase REST API returned HTTP|Failed to reach Supabase REST API at" <<< "$startup_lines" | tail -n 1 >&2
      ;;
    *)
      if [[ -f "$log_path" ]]; then
        note "Remote sync: check $log_path"
      else
        warn "No ETSU log file found yet at $log_path"
      fi
      ;;
  esac

  case "$capture_status" in
    input_monitoring_missing)
      warn "Input capture: Input Monitoring permission missing for the ETSU binary"
      ;;
    accessibility_missing)
      warn "Input capture: Accessibility permission missing for the ETSU binary"
      ;;
    confirmed)
      note "Input capture: confirmed"
      ;;
    *)
      if [[ -f "$log_path" ]]; then
        note "Input capture: check $log_path"
      fi
      ;;
  esac
}

print_next_steps() {
  cat <<EOF
ETSU macOS setup complete.
Config file: $CONFIG_PATH
Local SQLite DB: $LOCAL_DB_PATH
Backup directory: $BACKUP_DIR
Installed app bundle: $APP_BUNDLE_PATH
Installed binary: $INSTALLED_BIN_PATH

Verify the agent:
  launchctl print "gui/$(id -u)/com.seatedro.etsu"
  tail -n 50 "$HOME/Library/Logs/etsu.launchd.err.log"

Verify local device-tagged writes:
  sqlite3 "$LOCAL_DB_PATH" "select device_id, device_name, last_updated, total_keypresses, total_mouse_clicks, total_scroll_steps from metrics_summary order by last_updated desc;"
EOF
}

stop_existing_etsu
backup_existing_state
ensure_config_file
resolve_remote_config

case "$REMOTE_MODE" in
  supabase)
    upsert_key "database" "supabase_url" "$SUPABASE_URL"
    upsert_key "database" "supabase_api_key" "$SUPABASE_API_KEY"
    delete_key "postgres_url"
    ;;
  postgres)
    upsert_key "database" "postgres_url" "$POSTGRES_URL"
    delete_key "supabase_url"
    delete_key "supabase_api_key"
    ;;
  *)
    warn "No remote sync configuration found."
    warn "Supabase REST: set ETSU_SUPABASE_URL and ETSU_SUPABASE_API_KEY, or place them in:"
    warn "  $HOME/Library/Application Support/com.seatedro.etsu/supabase_url.txt"
    warn "  $HOME/Library/Application Support/com.seatedro.etsu/supabase_api_key.txt"
    warn "  $HOME/Dropbox/Records/PersonalData/Etsu/supabase_url.txt"
    warn "  $HOME/Dropbox/Records/PersonalData/Etsu/supabase_api_key.txt"
    warn "Legacy Postgres: set ETSU_POSTGRES_URL, ETSU_POSTGRES_URL_FILE, ETSU_POSTGRES_URL_OP_REF, or place the DSN in:"
    warn "  $HOME/Library/Application Support/com.seatedro.etsu/postgres_dsn.txt"
    warn "  $HOME/Dropbox/Records/PersonalData/Etsu/postgres_dsn.txt"
    exit 1
    ;;
esac

if [[ -n "$DEVICE_ID" ]]; then
  upsert_key "identity" "device_id" "$DEVICE_ID"
fi

if [[ -n "$DEVICE_NAME" ]]; then
  upsert_key "identity" "device_name" "$DEVICE_NAME"
fi

run_build
install_app_bundle
ETSU_APP_BUNDLE_PATH="$APP_BUNDLE_PATH" \
  ETSU_BIN_PATH="$INSTALLED_BIN_PATH" \
  "$SCRIPT_DIR/install_launchd.sh"

case "$REMOTE_MODE" in
  supabase)
    note "Supabase URL source: $SUPABASE_URL_SOURCE"
    note "Supabase API key source: $SUPABASE_API_KEY_SOURCE"
    ;;
  postgres)
    note "Postgres DSN source: $POSTGRES_URL_SOURCE"
    ;;
esac

print_startup_status
print_next_steps
