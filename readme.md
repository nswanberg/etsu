# Etsu

![etsu](https://github.com/user-attachments/assets/014ef834-63bc-42a8-a396-e158c8012044)


An elegant personal spyware. (JK, it tracks silly metrics)

## Features

- Tracks keypresses, mouse clicks, scroll steps, and mouse distance traveled
- Local SQLite storage with optional Supabase REST API syncing (and legacy PostgreSQL support)
- Minimal resource usage
- Simple configuration
- Runs as a background service/daemon

## Installation

### Build from Source

```bash
# Clone the repository
git clone https://github.com/seatedro/etsu.git
cd etsu

# Build in release mode
cargo build --release

# The binary will be available at target/release/etsu
```

## Configuration

Etsu uses a TOML configuration file. Copy the example configuration:

```bash
cp config.example.toml config.toml
```

Edit the `config.toml` file to adjust settings.

### Supabase sync (recommended)

```toml
[database]
supabase_url = "https://your-project-ref.supabase.co"
supabase_api_key = "sb_publishable_..."

[identity]
device_id = "device-your-machine"
device_name = "Your Mac"
```

ETSU syncs all local SQLite rows to Supabase via the REST API every save interval. Unsynced rows are tracked with a `supabase_synced_at` column, so historical data is backfilled automatically on first connect.

`metrics.timestamp` is captured and synced as UTC. If you also want the machine-local wall clock context for reporting, add these columns on the Supabase side before restarting ETSU:

```sql
alter table public.metrics
  alter column "timestamp" type timestamptz
  using "timestamp" at time zone 'UTC';

alter table public.metrics
  alter column "timestamp" set default now();

alter table public.metrics
  add column if not exists timestamp_local timestamp,
  add column if not exists local_utc_offset_minutes integer;
```

ETSU will start populating `timestamp_local` and `local_utc_offset_minutes` locally right away, and it will only include those fields in Supabase REST sync once the remote `metrics` table exposes them. That makes the rollout safe to stage behind your downstream schema migration.

If you want every future Supabase row to include the extra local-time fields, apply the schema change before restarting the upgraded ETSU binary on each Mac. Rows that were already synced before the remote columns existed are not backfilled automatically.

### Legacy direct Postgres

```toml
[database]
postgres_url = "postgresql://user:password@host:5432/postgres"
```

Note: Supabase's direct Postgres endpoint is IPv6-only, which may not work on all networks. The REST API (above) is IPv4 and works everywhere.

If `identity.device_id` or `identity.device_name` is missing, ETSU will generate and persist them into the config file on first launch.
Keep those values local to each machine. Do not commit real device IDs, UUIDs, or hostnames to the public repo.

### Configuration File Locations

The configuration file is searched in these locations:

- **macOS**: `~/Library/Application Support/com.seatedro.etsu/config.toml`
- **Linux**: `~/.config/etsu/config.toml`
- **Windows**: `%APPDATA%\seatedro\etsu\config.toml`

## Usage

How in god's name do we package this as a background process cross-platform? Please raise a PR

### Running Manually

You can also run Etsu directly:

```bash
# On macOS/Linux
./etsu

# On Windows
etsu.exe
```

### Running in the background on macOS with `launchd`

Use the installer:

```bash
./setup_macos.sh
```

That script:
- backs up the live config and SQLite DB from `~/Library/Application Support/com.seatedro.etsu/`
- builds release
- installs a stable app bundle at `~/Applications/Etsu.app`
- installs the LaunchAgent at `~/Library/LaunchAgents/com.seatedro.etsu.plist`
- waits for macOS Input Monitoring and Accessibility permissions to be granted
- auto-restarts ETSU after permissions are granted (macOS requires a restart)
- confirms input capture is working before finishing

If you want to run the pieces manually:

```bash
cargo build --release
./scripts/install_app_bundle.sh
./scripts/install_launchd.sh
```

The LaunchAgent runs `~/Applications/Etsu.app/Contents/MacOS/etsu` and writes logs under `~/Library/Logs/etsu.launchd*.log`.
If you have an older local install using `com.nswanberg.etsu`, the installer will stop it first and keep a timestamped backup of the old plist.

### Stable code signing (recommended, one-time)

macOS ties Input Monitoring and Accessibility permissions to the binary's code signature. Without a stable signing identity, every rebuild produces a new ad-hoc signature and macOS silently revokes the permissions -- ETSU will appear to be running but will capture nothing.

Create a one-time local signing identity (stored in your macOS keychain):

```bash
./scripts/create_macos_dev_signing_identity.sh
```

This only needs to be done once per Mac. After that, `./setup_macos.sh` automatically finds and uses the "ETSU Development" identity. Permissions survive rebuilds.

If no signing identity exists, ETSU still installs and runs, but permissions will break on every reinstall.

### Installing on another Mac

On a fresh Mac, the full setup is:

```bash
git clone https://github.com/seatedro/etsu.git
cd etsu

# One-time: create a stable signing identity so permissions survive rebuilds
./scripts/create_macos_dev_signing_identity.sh

# Install and start
./setup_macos.sh
```

The setup script will:
1. Build the release binary
2. Sign it with the "ETSU Development" identity (or ad-hoc if none exists)
3. Install the app bundle and LaunchAgent
4. Prompt you to grant Input Monitoring and Accessibility in System Settings
5. Auto-restart ETSU after you grant permissions
6. Confirm input capture is working

If the Mac was previously running `target/release/etsu` manually from a terminal, `./setup_macos.sh` will stop that process and replace it with the managed `~/Applications/Etsu.app` + LaunchAgent install.

The setup script resolves Supabase credentials from (in order):

1. `ETSU_SUPABASE_URL` and `ETSU_SUPABASE_API_KEY` env vars
2. existing `~/Library/Application Support/com.seatedro.etsu/config.toml`

For a new Mac, pass the credentials via environment variables:

```bash
ETSU_SUPABASE_URL="https://your-project-ref.supabase.co" \
  ETSU_SUPABASE_API_KEY="sb_publishable_..." \
  ./setup_macos.sh
```

Or pull them from 1Password:

```bash
ETSU_SUPABASE_URL="$(op read 'op://Vault/Etsu/supabase_url')" \
  ETSU_SUPABASE_API_KEY="$(op read 'op://Vault/Etsu/supabase_api_key')" \
  ./setup_macos.sh
```

### Updating an existing second Mac

If the second Mac is already running ETSU and already has its own local SQLite history, the update path is intentionally short:

```bash
# From target/release
git -C ../.. pull --ff-only
../../setup_macos.sh
```

or:

```bash
# From the repo root
git pull --ff-only
./setup_macos.sh
```

That preserves the existing local database and device identity, takes a backup first, and replaces any old terminal-run ETSU process with the managed `~/Applications/Etsu.app` install.

See [SECOND_MAC_RUNBOOK.md](SECOND_MAC_RUNBOOK.md) for the full operator checklist.

### Viewing Statistics

Etsu stores metrics in a local SQLite database located at:

- **macOS**: `~/Library/Application Support/com.seatedro.etsu/etsu.db`
- **Linux**: `~/.local/share/etsu/etsu.db`
- **Windows**: `%LOCALAPPDATA%\seatedro\etsu\etsu.db`

## License

[MIT](LICENSE)
