# Etsu

![etsu](https://github.com/user-attachments/assets/014ef834-63bc-42a8-a396-e158c8012044)


An elegant personal spyware. (JK, it tracks silly metrics)

## Features

- Tracks keypresses, mouse clicks, scroll steps, and mouse distance traveled
- Local SQLite storage with optional PostgreSQL syncing
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

Example Supabase-backed identity block:

```toml
[database]
postgres_url = "postgresql://user:password@host:5432/postgres"

[identity]
device_id = "device-your-machine"
device_name = "Your Mac"
```

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

Build ETSU first:

```bash
cargo build --release
```

Then install the LaunchAgent:

```bash
./scripts/install_launchd.sh
```

That installs `~/Library/LaunchAgents/com.seatedro.etsu.plist`, configures ETSU to restart on crash, and writes logs under `~/Library/Logs/etsu.launchd*.log`.
If you have an older local install using `com.nswanberg.etsu`, the installer will stop it first and keep a timestamped backup of the old plist.

### Installing on another Mac

Use the installer script. It stops any existing ETSU process, backs up the live config and SQLite database from the default macOS paths, reuses the current device identity, rebuilds, and installs the LaunchAgent.

It resolves the Supabase/Postgres DSN from the first place that has one:

1. `ETSU_POSTGRES_URL`
2. `ETSU_POSTGRES_URL_OP_REF`
3. existing `~/Library/Application Support/com.seatedro.etsu/config.toml`
4. `~/Repositories/ledger/ops/ledger.env`
5. `~/Third-party-repositories/ledger/ops/ledger.env`

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
ETSU_POSTGRES_URL="postgresql://user:password@host:5432/postgres" ./setup_macos.sh
ETSU_POSTGRES_URL_OP_REF="op://Vault/Item/postgres_url" ./setup_macos.sh
ETSU_BACKUP_DIR="$HOME/Dropbox/Records/PersonalData/Etsu/m2" ./setup_macos.sh
```

### Viewing Statistics

Etsu stores metrics in a local SQLite database located at:

- **macOS**: `~/Library/Application Support/com.seatedro.etsu/etsu.db`
- **Linux**: `~/.local/share/etsu/etsu.db`
- **Windows**: `%LOCALAPPDATA%\seatedro\etsu\etsu.db`

## License

[MIT](LICENSE)
