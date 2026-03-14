use anyhow::Context;
use directories::ProjectDirs;
use serde::Deserialize;
use std::{path::PathBuf, process::Command, time::Duration};
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct DeviceIdentity {
    pub device_id: String,
    pub device_name: String,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct RemoteDatabaseSettings {
    pub postgres_url: Option<String>,
    pub supabase_url: Option<String>,
    pub supabase_api_key: Option<String>,
}

#[derive(Deserialize, Debug, Clone, Default)]
pub struct IdentitySettings {
    pub device_id: Option<String>,
    pub device_name: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct IntervalSettings {
    #[serde(default = "default_processing_interval")]
    pub processing: u64,
    #[serde(default = "default_saving_interval")]
    pub saving: u64,
}

#[derive(Deserialize, Debug, Clone)]
pub struct Settings {
    #[serde(default)]
    pub database: RemoteDatabaseSettings,
    #[serde(default)]
    pub identity: IdentitySettings,
    #[serde(default)]
    pub intervals_ms: IntervalSettings,
    #[serde(default = "default_log_level")]
    pub log_level: String,
}

// Default functions for serde
fn default_processing_interval() -> u64 {
    250
}
fn default_saving_interval() -> u64 {
    60000
}
fn default_log_level() -> String {
    "info".to_string()
}

impl Default for IntervalSettings {
    fn default() -> Self {
        Self {
            processing: default_processing_interval(),
            saving: default_saving_interval(),
        }
    }
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            database: RemoteDatabaseSettings {
                postgres_url: None,
                supabase_url: None,
                supabase_api_key: None,
            },
            identity: IdentitySettings::default(),
            intervals_ms: IntervalSettings {
                processing: default_processing_interval(),
                saving: default_saving_interval(),
            },
            log_level: default_log_level(),
        }
    }
}

impl Settings {
    pub fn load() -> anyhow::Result<Self> {
        let proj_dirs = ProjectDirs::from("com", "seatedro", "etsu")
            .context("Failed to get project directories")?;
        let config_dir = proj_dirs.config_dir();
        std::fs::create_dir_all(config_dir).context("Failed to create config directory")?;
        let config_file = config_dir.join("config.toml");
        ensure_identity_defaults(&config_file)?;

        let builder = config::Config::builder()
            .set_default("database.postgres_url", None::<String>)?
            .set_default("database.supabase_url", None::<String>)?
            .set_default("database.supabase_api_key", None::<String>)?
            .set_default("identity.device_id", None::<String>)?
            .set_default("identity.device_name", None::<String>)?
            .set_default("intervals_ms.processing", default_processing_interval())?
            .set_default("intervals_ms.saving", default_saving_interval())?
            .set_default("log_level", default_log_level())?
            .add_source(config::File::from(config_file).required(false))
            .add_source(config::Environment::with_prefix("ETSU").separator("__"));

        let settings = builder.build()?.try_deserialize()?;

        Ok(settings)
    }

    pub fn device_identity(&self) -> anyhow::Result<DeviceIdentity> {
        let device_id = self
            .identity
            .device_id
            .clone()
            .filter(|value| !value.trim().is_empty())
            .context("Missing identity.device_id in config or ETSU__IDENTITY__DEVICE_ID")?;
        let device_name = self
            .identity
            .device_name
            .clone()
            .filter(|value| !value.trim().is_empty())
            .context("Missing identity.device_name in config or ETSU__IDENTITY__DEVICE_NAME")?;
        Ok(DeviceIdentity {
            device_id,
            device_name,
        })
    }

    pub fn get_local_sqlite_path(&self) -> anyhow::Result<PathBuf> {
        let db_filename = "etsu.db";

        let path = PathBuf::from(db_filename);
        if path.is_absolute() {
            Ok(path)
        } else {
            let proj_dirs = ProjectDirs::from("com", "seatedro", "etsu")
                .context("Failed to get project directories for local DB path")?;
            let data_dir = proj_dirs.data_local_dir();
            std::fs::create_dir_all(data_dir).context("Failed to create local data directory")?;
            Ok(data_dir.join(path))
        }
    }

    pub fn processing_interval(&self) -> Duration {
        Duration::from_millis(self.intervals_ms.processing)
    }
    pub fn saving_interval(&self) -> Duration {
        Duration::from_millis(self.intervals_ms.saving)
    }
}

fn ensure_identity_defaults(config_file: &PathBuf) -> anyhow::Result<()> {
    let raw = if config_file.exists() {
        std::fs::read_to_string(config_file)
            .with_context(|| format!("Failed to read config file at {}", config_file.display()))?
    } else {
        String::new()
    };

    let mut value = if raw.trim().is_empty() {
        toml::Value::Table(toml::map::Map::new())
    } else {
        toml::from_str::<toml::Value>(&raw)
            .with_context(|| format!("Failed to parse config file at {}", config_file.display()))?
    };

    let root = value
        .as_table_mut()
        .context("Expected the ETSU config root to be a TOML table")?;
    let identity_value = root
        .entry("identity")
        .or_insert_with(|| toml::Value::Table(toml::map::Map::new()));
    let identity_table = identity_value
        .as_table_mut()
        .context("Expected [identity] in config.toml to be a TOML table")?;

    let mut changed = false;
    if !identity_table
        .get("device_id")
        .and_then(toml::Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
    {
        identity_table.insert(
            "device_id".to_string(),
            toml::Value::String(Uuid::new_v4().to_string()),
        );
        changed = true;
    }
    if !identity_table
        .get("device_name")
        .and_then(toml::Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
    {
        identity_table.insert(
            "device_name".to_string(),
            toml::Value::String(default_device_name()),
        );
        changed = true;
    }

    if changed {
        let rendered = toml::to_string_pretty(&value).context("Failed to render ETSU config TOML")?;
        std::fs::write(config_file, rendered)
            .with_context(|| format!("Failed to write config file at {}", config_file.display()))?;
    }

    Ok(())
}

fn default_device_name() -> String {
    if let Ok(output) = Command::new("scutil").args(["--get", "ComputerName"]).output() {
        if output.status.success() {
            let rendered = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !rendered.is_empty() {
                return rendered;
            }
        }
    }
    std::env::var("HOSTNAME")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "Etsu Mac".to_string())
}
