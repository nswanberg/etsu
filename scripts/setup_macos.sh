#!/bin/zsh

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
APP_SUPPORT_DIR="$HOME/Library/Application Support/com.seatedro.etsu"
CONFIG_PATH="$APP_SUPPORT_DIR/config.toml"
EXAMPLE_CONFIG_PATH="$REPO_ROOT/config.example.toml"
LOCAL_DB_PATH="$APP_SUPPORT_DIR/etsu.db"
POSTGRES_URL="${ETSU_POSTGRES_URL:-}"
POSTGRES_URL_OP_REF="${ETSU_POSTGRES_URL_OP_REF:-}"
DEVICE_ID="${ETSU_DEVICE_ID:-}"
DEVICE_NAME="${ETSU_DEVICE_NAME:-}"
SKIP_BUILD="${ETSU_SKIP_BUILD:-0}"

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

resolve_postgres_url() {
  if [[ -n "$POSTGRES_URL" ]]; then
    return
  fi

  if [[ -n "$POSTGRES_URL_OP_REF" ]]; then
    if ! command -v op >/dev/null 2>&1; then
      echo "ETSU_POSTGRES_URL_OP_REF was set, but the 1Password CLI (op) is not installed." >&2
      exit 1
    fi
    POSTGRES_URL="$(op read "$POSTGRES_URL_OP_REF")"
  fi
}

run_build() {
  if [[ "$SKIP_BUILD" == "1" ]]; then
    return
  fi
  cargo build --release
}

print_next_steps() {
  cat <<EOF
ETSU macOS setup complete.
Config file: $CONFIG_PATH
Local SQLite DB: $LOCAL_DB_PATH

Verify the agent:
  launchctl print "gui/$(id -u)/com.seatedro.etsu"
  tail -n 50 "$HOME/Library/Logs/etsu.launchd.err.log"

Verify local device-tagged writes:
  sqlite3 "$LOCAL_DB_PATH" "select device_id, device_name, last_updated, total_keypresses, total_mouse_clicks, total_scroll_steps from metrics_summary order by last_updated desc;"

Verify remote writes with psql once the Supabase DSN is configured:
  select device_id, device_name, count(*) as intervals, max(timestamp) as latest_interval
  from metrics
  group by device_id, device_name
  order by latest_interval desc;

  select device_id, device_name, last_updated, total_keypresses, total_mouse_clicks, total_scroll_steps
  from metrics_summary
  order by last_updated desc;
EOF
}

ensure_config_file
resolve_postgres_url

if [[ -n "$POSTGRES_URL" ]]; then
  upsert_key "database" "postgres_url" "$POSTGRES_URL"
fi

if [[ -n "$DEVICE_ID" ]]; then
  upsert_key "identity" "device_id" "$DEVICE_ID"
fi

if [[ -n "$DEVICE_NAME" ]]; then
  upsert_key "identity" "device_name" "$DEVICE_NAME"
fi

run_build
"$SCRIPT_DIR/install_launchd.sh"
print_next_steps
