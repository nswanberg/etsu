#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod db;
mod distance;
mod error;
mod input;
mod persistence;
mod platform;
mod processing;
mod state;

use crate::error::Result;
use directories::ProjectDirs;
use error::AppError;
use state::MetricsState;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{broadcast, mpsc};
use tokio::time;
use tracing::{debug, error, info, warn};
use tracing_appender::rolling;
use tracing_subscriber::EnvFilter;

use futures::stream::StreamExt;
use signal_hook::consts::signal::*;
use signal_hook_tokio::Signals;

#[tokio::main]
async fn main() -> Result<()> {
    info!("Etsu starting...");

    let settings = config::Settings::load().map_err(|e| {
        eprintln!("FATAL: Failed to load configuration: {}", e);
        e
    })?;

    let proj_dirs = ProjectDirs::from("com", "seatedro", "etsu")
        .ok_or_else(|| AppError::Initialization("Failed to get project dirs for logging".into()))?;
    let log_dir = proj_dirs.data_local_dir();
    std::fs::create_dir_all(log_dir)
        .map_err(|e| AppError::Initialization(format!("Failed to create log directory: {}", e)))?;
    let _log_file = rolling::daily(log_dir, "etsu.log");
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_new(&settings.log_level).unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_writer(_log_file)
        .with_ansi(false) // Disable colors in file
        .init();

    info!("Loaded configuration");
    let identity = settings.device_identity().map_err(|e| {
        eprintln!("FATAL: Failed to resolve ETSU device identity: {}", e);
        e
    })?;
    info!(
        "Using device identity: {} ({})",
        identity.device_name, identity.device_id
    );

    if let Err(e) = platform::initialize_monitor_info() {
        error!("Failed to initialize monitor info using GLFW: {}. Distance calculation might be inaccurate or use defaults.", e);
    }

    let local_db_path = settings
        .get_local_sqlite_path()?
        .to_string_lossy()
        .to_string();
    let (sqlite_pool, pg_pool_option) =
        db::setup_database_pools(&local_db_path, &settings.database).await?;
    let supabase_option = db::setup_supabase_client(&settings.database).await;

    if let Err(e) = db::run_migrations(&sqlite_pool, &pg_pool_option).await {
        error!(
            "Database migration failed: {}. Application might not function correctly.",
            e
        );
        // Consider exiting if migrations are critical
        // return Err(e);
    }

    let (shutdown_tx, _) = broadcast::channel::<()>(1);
    let shutdown_tx_clone = shutdown_tx.clone();

    let signals = setup_signal_handlers(shutdown_tx_clone)?;

    let signal_task = tokio::spawn(handle_signals(signals, shutdown_tx.clone()));

    let metrics_state = Arc::new(MetricsState::default());
    let (input_tx, input_rx) = mpsc::channel::<input::InputEvent>(1024);

    info!("Spawning core tasks...");

    input::listen_for_input(input_tx).await?;

    let metrics_state_clone = Arc::clone(&metrics_state);
    let processing_interval = settings.processing_interval();

    let mut shutdown_rx1 = shutdown_tx.subscribe();
    let processing_handle = tokio::spawn(async move {
        tokio::select! {
            res = processing::aggregate_metrics(input_rx, metrics_state_clone, processing_interval) => res,
            _ = shutdown_rx1.recv() => {
                debug!("Processing task received shutdown signal");
                Ok(())
            }
        }
    });

    let metrics_state_clone = Arc::clone(&metrics_state);
    let capture_warning_after = Duration::from_secs(30);
    let executable_path = std::env::current_exe()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| "<unknown>".to_string());
    let mut shutdown_rx_capture = shutdown_tx.subscribe();
    let capture_health_handle = tokio::spawn(async move {
        tokio::select! {
            res = monitor_input_capture(metrics_state_clone, capture_warning_after, executable_path) => res,
            _ = shutdown_rx_capture.recv() => {
                debug!("Capture health task received shutdown signal");
                Ok(())
            }
        }
    });

    let metrics_state_clone = Arc::clone(&metrics_state);
    let saving_interval = settings.saving_interval();
    let sqlite_pool_clone = sqlite_pool.clone();
    let pg_pool_option_clone = pg_pool_option.clone();
    let supabase_option_clone = supabase_option.clone();
    let identity_clone = identity.clone();

    let mut shutdown_rx2 = shutdown_tx.subscribe();
    let persistence_handle = tokio::spawn(async move {
        tokio::select! {
            res = persistence::save_metrics_periodically(
                metrics_state_clone,
                sqlite_pool_clone,
                pg_pool_option_clone,
                supabase_option_clone,
                identity_clone,
                saving_interval,
            ) => res,
            _ = shutdown_rx2.recv() => {
                debug!("Persistence task received shutdown signal");
                Ok(())
            }
        }
    });

    info!("All tasks spawned. Etsu running in background.");
    info!("Press Ctrl+C to exit");

    let mut shutdown_rx_main = shutdown_tx.subscribe();
    let _ = shutdown_rx_main.recv().await;
    info!("Initiating shutdown...");

    signal_task.abort();

    info!("Shutting down tasks...");

    info!("Stopping input listener...");

    let timeout = tokio::time::Duration::from_secs(5);

    let processing_timeout = tokio::time::timeout(timeout, processing_handle);
    let persistence_timeout = tokio::time::timeout(timeout, persistence_handle);

    let (processing_result, persistence_result, capture_health_result) = tokio::join!(
        processing_timeout,
        persistence_timeout,
        tokio::time::timeout(timeout, capture_health_handle)
    );

    if processing_result.is_err() {
        warn!("Processing task did not complete within timeout, aborting");
    }

    if persistence_result.is_err() {
        warn!("Persistence task did not complete within timeout, aborting");
    }

    if capture_health_result.is_err() {
        warn!("Capture health task did not complete within timeout, aborting");
    }

    info!("Closing database pools...");
    let close_sqlite = tokio::spawn(async move { sqlite_pool.close().await });
    let close_pg = tokio::spawn(async move {
        if let Some(pg_pool) = pg_pool_option {
            pg_pool.close().await;
        }
    });
    let _ = tokio::try_join!(close_sqlite, close_pg);
    info!("Database pools closed.");

    info!("Etsu shutdown complete.");
    Ok(())
}

/// Sets up the signal handlers for SIGTERM, SIGINT, and SIGQUIT
fn setup_signal_handlers(_shutdown_tx: broadcast::Sender<()>) -> Result<Signals> {
    info!("Setting up signal handlers...");

    let signals = match Signals::new([SIGTERM, SIGINT, SIGQUIT]) {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to register signal handlers: {}", e);
            return Err(AppError::Initialization(format!(
                "Signal handler setup failed: {}",
                e
            )));
        }
    };

    info!("Signal handlers registered");
    Ok(signals)
}

/// Handles signals and triggers shutdown
async fn handle_signals(mut signals: Signals, shutdown_tx: broadcast::Sender<()>) {
    while let Some(signal) = signals.next().await {
        match signal {
            SIGTERM | SIGINT | SIGQUIT => {
                info!("Received signal {}, triggering shutdown...", signal);
                let _ = shutdown_tx.send(());
                break;
            }
            _ => warn!("Received unexpected signal: {}", signal),
        }
    }
}

async fn monitor_input_capture(
    state: Arc<MetricsState>,
    warning_after: Duration,
    executable_path: String,
) -> Result<()> {
    let start = std::time::Instant::now();
    let mut interval = time::interval(Duration::from_secs(15));
    let mut startup_warning_emitted = false;
    let mut stalled_warning_emitted = false;

    loop {
        interval.tick().await;

        let events_seen = state.input_events_seen.load(Ordering::Relaxed);
        if events_seen == 0 && start.elapsed() >= warning_after {
            if !startup_warning_emitted {
                warn!(
                    "ETSU is running but has not observed any keyboard or mouse events in the first {} seconds. \
This usually means macOS is blocking input capture for {}. Check Input Monitoring / Accessibility permissions for the ETSU binary path.",
                    warning_after.as_secs(),
                    executable_path
                );
                startup_warning_emitted = true;
            }
            continue;
        }

        if events_seen == 0 {
            continue;
        }

        if startup_warning_emitted {
            info!("Input capture resumed after startup warning.");
            startup_warning_emitted = false;
        }

        let last_input_event_unix_secs = state.last_input_event_unix_secs.load(Ordering::Relaxed);
        if last_input_event_unix_secs == 0 {
            continue;
        }

        let now_unix_secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let idle_secs = now_unix_secs.saturating_sub(last_input_event_unix_secs);
        if idle_secs >= 300 {
            if !stalled_warning_emitted {
                warn!(
                    "ETSU has been running but has not seen an input event for {} minutes. \
If this Mac is in active use, check Input Monitoring / Accessibility permissions for {}.",
                    idle_secs / 60,
                    executable_path
                );
                stalled_warning_emitted = true;
            }
        } else if stalled_warning_emitted {
            info!("Input capture resumed after an idle warning.");
            stalled_warning_emitted = false;
        }
    }
}
