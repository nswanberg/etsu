use crate::config::DeviceIdentity;
use crate::db::{self, MetricsData, SupabaseClient};
use crate::error::Result;
use crate::journal::{JournalEntry, MetricsJournal};
use crate::state::MetricsState;
use sqlx::{Pool, Postgres, Sqlite};
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::time;
use tracing::{debug, error, info, instrument, warn};

const SUPABASE_SYNC_TIMEOUT: Duration = Duration::from_secs(30);
const SUPABASE_STALENESS_THRESHOLD: Duration = Duration::from_secs(60 * 60);
const MAX_CONSECUTIVE_LOCAL_PERSISTENCE_FAILURES: u32 = 3;

#[instrument(skip(state, sqlite_pool, pg_pool_option, saving_interval))]
pub async fn save_metrics_periodically(
    state: Arc<MetricsState>,
    sqlite_pool: Pool<Sqlite>,
    pg_pool_option: Option<Pool<Postgres>>,
    identity: DeviceIdentity,
    saving_interval: Duration,
    journal_path: PathBuf,
) -> Result<()> {
    debug!(
        "Starting metrics persistence task with interval: {:?}",
        saving_interval
    );
    let mut interval_timer = time::interval(saving_interval);
    let journal = MetricsJournal::new(journal_path);
    let mut pending_memory = MetricsData::default();
    let mut consecutive_local_failures = 0;

    if let Err(e) = db::backfill_sqlite_identity(&sqlite_pool, &identity).await {
        error!("Failed to backfill local SQLite device identity: {}", e);
    }

    if let Err(e) = replay_journal(&journal, &sqlite_pool, &identity).await {
        fatal_persistence_exit(&format!(
            "Failed to replay metrics journal {} at startup: {}",
            journal.path().display(),
            e
        ));
    }

    match db::load_initial_totals(&sqlite_pool, &identity.device_id).await {
        Ok((keys, clicks, scrolls, distance)) => {
            state.total.keypresses.store(keys, Ordering::Relaxed);
            state.total.mouse_clicks.store(clicks, Ordering::Relaxed);
            state.total.scroll_steps.store(scrolls, Ordering::Relaxed);
            *state.total.mouse_distance_in.lock().await = distance;
            debug!("Successfully loaded initial totals into state from local DB.");
        }
        Err(e) => {
            error!(
                "Failed to load initial totals from local DB: {}. Starting from zero.",
                e
            );
        }
    }

    loop {
        interval_timer.tick().await;

        let (keys, clicks, scrolls, distance) = state.interval.reset().await;

        state
            .total
            .add_interval(keys, clicks, scrolls, distance)
            .await;

        let interval_data = MetricsData {
            keypresses: keys,
            mouse_clicks: clicks,
            scroll_steps: scrolls,
            mouse_distance_in: distance,
        };
        pending_memory.add_assign(&interval_data);

        if !pending_memory.is_empty() {
            debug!(
                "Attempting to journal metrics: K={}, C={}, S={}, D={:.2}",
                pending_memory.keypresses,
                pending_memory.mouse_clicks,
                pending_memory.scroll_steps,
                pending_memory.mouse_distance_in
            );

            let entry = JournalEntry::new(&pending_memory);
            match journal.append(&entry) {
                Ok(()) => {
                    pending_memory = MetricsData::default();
                }
                Err(e) => {
                    consecutive_local_failures += 1;
                    error!(
                        "Failed to append metrics journal {}: {}",
                        journal.path().display(),
                        e
                    );
                    maybe_exit_after_local_failures(consecutive_local_failures);
                    continue;
                }
            }
        }

        match replay_journal(&journal, &sqlite_pool, &identity).await {
            Ok(replayed) => {
                if replayed > 0 {
                    info!(
                        "Persisted {} journaled metrics entries to local SQLite",
                        replayed
                    );
                }
                consecutive_local_failures = 0;
            }
            Err(e) => {
                consecutive_local_failures += 1;
                error!(
                    "Failed to persist metrics journal {} to local SQLite: {}",
                    journal.path().display(),
                    e
                );
                maybe_exit_after_local_failures(consecutive_local_failures);
                continue;
            }
        }

        if !interval_data.is_empty() {
            if let Some(ref pg_pool) = pg_pool_option {
                if let Err(e) =
                    db::persist_metrics_postgres(pg_pool, &interval_data, &identity).await
                {
                    error!("Failed to persist metrics to remote Postgres: {}", e);
                }
            }
        }
    }
}

async fn replay_journal(
    journal: &MetricsJournal,
    sqlite_pool: &Pool<Sqlite>,
    identity: &DeviceIdentity,
) -> Result<usize> {
    let entries = journal.load_entries()?;
    if entries.is_empty() {
        return Ok(0);
    }

    for entry in &entries {
        db::persist_metrics_journal_entry_sqlite(sqlite_pool, entry, identity).await?;
    }

    journal.checkpoint_empty()?;
    Ok(entries.len())
}

fn maybe_exit_after_local_failures(consecutive_failures: u32) {
    if consecutive_failures >= MAX_CONSECUTIVE_LOCAL_PERSISTENCE_FAILURES {
        fatal_persistence_exit(&format!(
            "Local metrics persistence failed {} consecutive times",
            consecutive_failures
        ));
    }
}

fn fatal_persistence_exit(message: &str) -> ! {
    error!("{}. Exiting so launchd can restart ETSU.", message);
    std::process::exit(1);
}

#[instrument(skip(supabase, sqlite_pool, sync_interval))]
pub async fn sync_to_remote_periodically(
    supabase: SupabaseClient,
    sqlite_pool: Pool<Sqlite>,
    sync_interval: Duration,
) -> Result<()> {
    debug!(
        "Starting remote sync task with interval: {:?}",
        sync_interval
    );
    let mut interval_timer = time::interval(sync_interval);
    let task_started = Instant::now();
    let mut last_success: Option<Instant> = None;
    let mut staleness_warned = false;
    let mut last_error: Option<String> = None;

    loop {
        interval_timer.tick().await;

        db::probe_supabase_local_time_columns(&supabase).await;

        match time::timeout(
            SUPABASE_SYNC_TIMEOUT,
            db::sync_to_supabase(&supabase, &sqlite_pool),
        )
        .await
        {
            Ok(Ok(count)) => {
                if last_error.is_some() || staleness_warned {
                    info!("Supabase sync recovered after prior failures.");
                }
                last_error = None;
                last_success = Some(Instant::now());
                staleness_warned = false;
                if count > 0 {
                    debug!("Remote sync completed: {} rows synced", count);
                }
            }
            Ok(Err(e)) => {
                let msg = e.to_string();
                if last_error.as_deref() != Some(msg.as_str()) {
                    warn!("Failed to sync metrics to Supabase: {}. Will retry.", msg);
                    last_error = Some(msg);
                }
            }
            Err(_) => {
                let msg = format!("timed out after {}s", SUPABASE_SYNC_TIMEOUT.as_secs());
                if last_error.as_deref() != Some(msg.as_str()) {
                    warn!("Supabase sync {}. Will retry.", msg);
                    last_error = Some(msg);
                }
            }
        }

        let since_success = last_success
            .map(|t| t.elapsed())
            .unwrap_or_else(|| task_started.elapsed());
        if since_success >= SUPABASE_STALENESS_THRESHOLD && !staleness_warned {
            match db::count_unsynced_metrics(&sqlite_pool).await {
                Ok(unsynced) if unsynced > 0 => {
                    warn!(
                        "Supabase sync has not succeeded in {} minute(s); {} row(s) queued locally.",
                        since_success.as_secs() / 60,
                        unsynced
                    );
                    staleness_warned = true;
                }
                Ok(_) => {}
                Err(e) => {
                    debug!("Failed to query local unsynced row count: {}", e);
                }
            }
        }
    }
}
