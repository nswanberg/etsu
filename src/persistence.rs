use crate::config::DeviceIdentity;
use crate::db::{self, MetricsData, SupabaseClient};
use crate::error::Result;
use crate::state::MetricsState;
use sqlx::{Pool, Postgres, Sqlite};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;
use tokio::time;
use tracing::{debug, error, instrument};

#[instrument(skip(state, sqlite_pool, pg_pool_option, supabase_option, saving_interval))]
pub async fn save_metrics_periodically(
    state: Arc<MetricsState>,
    sqlite_pool: Pool<Sqlite>,
    pg_pool_option: Option<Pool<Postgres>>,
    supabase_option: Option<SupabaseClient>,
    identity: DeviceIdentity,
    saving_interval: Duration,
) -> Result<()> {
    // Note: saving_interval is used for both metric persistence and Supabase sync.
    // Supabase sync happens after each SQLite write, so new rows plus any previously
    // unsynced rows are all picked up every interval.
    debug!(
        "Starting metrics persistence task with interval: {:?}",
        saving_interval
    );
    let mut interval_timer = time::interval(saving_interval);

    if let Err(e) = db::backfill_sqlite_identity(&sqlite_pool, &identity).await {
        error!("Failed to backfill local SQLite device identity: {}", e);
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

        if keys > 0 || clicks > 0 || scrolls > 0 || distance > 0.0 {
            let data_to_save = MetricsData {
                keypresses: keys,
                mouse_clicks: clicks,
                scroll_steps: scrolls,
                mouse_distance_in: distance,
            };
            debug!(
                "Attempting to persist metrics: K={}, C={}, S={}, D={:.2}",
                keys, clicks, scrolls, distance
            );

            if let Err(e) = db::persist_metrics_sqlite(&sqlite_pool, &data_to_save, &identity).await {
                error!("Failed to persist metrics to local SQLite: {}", e);
            }

            if let Some(ref pg_pool) = pg_pool_option {
                if let Err(e) = db::persist_metrics_postgres(pg_pool, &data_to_save, &identity).await {
                    error!("Failed to persist metrics to remote Postgres: {}", e);
                }
            }

        }

        if let Some(ref supabase) = supabase_option {
            if let Err(e) = db::sync_to_supabase(supabase, &sqlite_pool).await {
                error!("Failed to sync metrics to Supabase: {}", e);
            }
        }
    }
}
