# Second Mac Runbook

This is the update path for an existing second Mac that is already running ETSU and already has its own local SQLite history.

Do not commit a real `device_id`, machine UUID, or hostname to git. ETSU stores machine identity in the local config at `~/Library/Application Support/com.seatedro.etsu/config.toml`, and `./setup_macos.sh` preserves the existing identity unless you explicitly override it with `ETSU_DEVICE_ID` or `ETSU_DEVICE_NAME`.

## One-command update

If you are already sitting in `target/release` on the other Mac:

```bash
git -C ../.. pull --ff-only
../../setup_macos.sh
```

If you are at the repo root:

```bash
git pull --ff-only
./setup_macos.sh
```

What this does:

1. Backs up the live config and SQLite DB into `~/Library/Application Support/com.seatedro.etsu/backups/<timestamp>/`
2. Rebuilds ETSU
3. Reinstalls `~/Applications/Etsu.app`
4. Restarts the LaunchAgent
5. Preserves the existing machine identity from the local config
6. Waits for macOS permissions and confirms capture before returning success

## If this Mac already has Supabase configured

You do not need to pass any credentials. `./setup_macos.sh` will reuse the existing values from `~/Library/Application Support/com.seatedro.etsu/config.toml`.

## If this Mac is new or missing remote config

Put the shared values in one of these local-only files:

- `~/Library/Application Support/com.seatedro.etsu/supabase_url.txt`
- `~/Library/Application Support/com.seatedro.etsu/supabase_api_key.txt`

or:

- `~/Dropbox/Records/PersonalData/Etsu/supabase_url.txt`
- `~/Dropbox/Records/PersonalData/Etsu/supabase_api_key.txt`

Then run:

```bash
./setup_macos.sh
```

## Verify after update

```bash
sqlite3 "$HOME/Library/Application Support/com.seatedro.etsu/etsu.db" \
  "SELECT version, description, success FROM _sqlx_migrations ORDER BY version;"

sqlite3 "$HOME/Library/Application Support/com.seatedro.etsu/etsu.db" \
  "PRAGMA table_info(metrics);"

sqlite3 "$HOME/Library/Application Support/com.seatedro.etsu/etsu.db" \
  "SELECT id, timestamp, timestamp_local, local_utc_offset_minutes FROM metrics ORDER BY id DESC LIMIT 5;"
```

Expected:

- migration `20260315090000` is present
- `metrics` includes `timestamp_local` and `local_utc_offset_minutes`
- new rows show `timestamp` in UTC and the local wall clock alongside it
