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

### Legacy direct Postgres

```toml
[database]
postgres_url = "postgresql://user:password@host:5432/postgres"
```

Note: Supabase's direct Postgres endpoint is IPv6-only, which may not work on all networks. The REST API (above) is IPv4 and works everywhere.

If `identity.device_id` or `identity.device_name` is missing, ETSU will generate and persist them into the config file on first launch.

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
- restarts ETSU and prints startup status

If you want to run the pieces manually:

```bash
cargo build --release
./scripts/install_app_bundle.sh
./scripts/install_launchd.sh
```

The LaunchAgent runs `~/Applications/Etsu.app/Contents/MacOS/etsu` and writes logs under `~/Library/Logs/etsu.launchd*.log`.
If you have an older local install using `com.nswanberg.etsu`, the installer will stop it first and keep a timestamped backup of the old plist.

### Repeatable local installs on macOS

macOS privacy permissions are tied to the app's code identity. Ad-hoc signatures can force you to re-grant `Input Monitoring` / `Accessibility` after reinstalls. For a stable local development loop, create a one-time local signing identity:

```bash
ETSU_CODESIGN_IDENTITY="ETSU Development" ./scripts/create_macos_dev_signing_identity.sh
```

Then rerun installs normally:

```bash
./setup_macos.sh
```

If no signing identity exists, ETSU still installs and runs, but the installer will warn that macOS permissions may not survive reinstalls.

### Installing on another Mac

Use the installer script. It stops any existing ETSU process, backs up the live config and SQLite database from the default macOS paths, reuses the current device identity, rebuilds, installs `~/Applications/Etsu.app`, and installs the LaunchAgent.

It resolves remote sync settings in this order:

1. Supabase REST via `ETSU_SUPABASE_URL` and `ETSU_SUPABASE_API_KEY`
2. existing `~/Library/Application Support/com.seatedro.etsu/config.toml`
3. `ETSU_SUPABASE_URL_FILE` and `ETSU_SUPABASE_API_KEY_FILE`
4. `~/Library/Application Support/com.seatedro.etsu/supabase_url.txt`
5. `~/Library/Application Support/com.seatedro.etsu/supabase_api_key.txt`
6. `~/Dropbox/Records/PersonalData/Etsu/supabase_url.txt`
7. `~/Dropbox/Records/PersonalData/Etsu/supabase_api_key.txt`
8. Legacy Postgres via `ETSU_POSTGRES_URL`, `ETSU_POSTGRES_URL_OP_REF`, existing config, or the `postgres_*.txt` files

From the repo root:

```bash
./setup_macos.sh
```

If you are already in `target/release`, run:

```bash
../../setup_macos.sh
```

Optional overrides:

```bash
ETSU_SUPABASE_URL="https://your-project-ref.supabase.co" ETSU_SUPABASE_API_KEY="sb_publishable_..." ./setup_macos.sh
ETSU_SUPABASE_URL_FILE="$HOME/Dropbox/Records/PersonalData/Etsu/supabase_url.txt" ETSU_SUPABASE_API_KEY_FILE="$HOME/Dropbox/Records/PersonalData/Etsu/supabase_api_key.txt" ./setup_macos.sh
ETSU_POSTGRES_URL="postgresql://user:password@host:5432/postgres" ./setup_macos.sh
ETSU_POSTGRES_URL_FILE="$HOME/Dropbox/Records/PersonalData/Etsu/postgres_dsn.txt" ./setup_macos.sh
ETSU_POSTGRES_URL_OP_REF="op://Vault/Item/postgres_url" ./setup_macos.sh
ETSU_BACKUP_DIR="$HOME/Dropbox/Records/PersonalData/Etsu/m2" ./setup_macos.sh
```

For a new Mac without an existing ETSU config, the simplest setup is to put:
- the Supabase URL on one line in `~/Dropbox/Records/PersonalData/Etsu/supabase_url.txt`
- the publishable key on one line in `~/Dropbox/Records/PersonalData/Etsu/supabase_api_key.txt`

Then run:

```bash
../../setup_macos.sh
```

### Viewing Statistics

Etsu stores metrics in a local SQLite database located at:

- **macOS**: `~/Library/Application Support/com.seatedro.etsu/etsu.db`
- **Linux**: `~/.local/share/etsu/etsu.db`
- **Windows**: `%LOCALAPPDATA%\seatedro\etsu\etsu.db`

## License

[MIT](LICENSE)
