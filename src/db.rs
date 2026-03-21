use crate::config::{DeviceIdentity, RemoteDatabaseSettings};
use crate::error::Result;
use chrono::{DateTime, Local, NaiveDateTime, Utc};
use postgrest::Postgrest;
use sea_query::{Alias, Expr, Iden, Query, SqliteQueryBuilder};
use sea_query_binder::SqlxBinder;
use sqlx::{migrate::Migrator, PgPool, Pool, Postgres, Sqlite, SqlitePool, Transaction};
use std::path::Path;
use tracing::{debug, info, instrument, warn};

static SQLITE_MIGRATOR: Migrator = sqlx::migrate!("./migrations/sqlite");
static POSTGRES_MIGRATOR: Migrator = sqlx::migrate!("./migrations/postgres");

#[derive(Iden)]
#[iden = "metrics"]
enum MetricsIden {
    Table,
    #[allow(dead_code)]
    Id,
    Keypresses,
    MouseClicks,
    MouseDistanceIn,
    #[allow(dead_code)]
    MouseDistanceMi,
    ScrollSteps,
    DeviceId,
    #[allow(dead_code)]
    DeviceName,
    #[allow(dead_code)]
    Timestamp,
    #[allow(dead_code)]
    TimestampLocal,
    #[allow(dead_code)]
    LocalUtcOffsetMinutes,
}

#[derive(Iden)]
#[iden = "metrics_summary"]
enum MetricsSummaryIden {
    Table,
    DeviceId,
    #[allow(dead_code)]
    LastUpdated,
    TotalKeypresses,
    TotalMouseClicks,
    TotalMouseTravelIn,
    #[allow(dead_code)]
    TotalMouseTravelMi,
    TotalScrollSteps,
}

#[derive(Debug, Clone)]
pub struct MetricsData {
    pub keypresses: usize,
    pub mouse_clicks: usize,
    pub scroll_steps: usize,
    pub mouse_distance_in: f64,
}

#[derive(Debug, Clone)]
struct CaptureTimestamps {
    utc: DateTime<Utc>,
    local: DateTime<Local>,
}

impl CaptureTimestamps {
    fn now() -> Self {
        let utc = Utc::now();
        let local = utc.with_timezone(&Local);
        Self { utc, local }
    }

    fn utc_naive(&self) -> NaiveDateTime {
        self.utc.naive_utc()
    }

    fn local_naive(&self) -> NaiveDateTime {
        self.local.naive_local()
    }

    fn local_utc_offset_minutes(&self) -> i32 {
        self.local.offset().local_minus_utc() / 60
    }
}

fn parse_sqlite_timestamp(value: &str) -> Option<NaiveDateTime> {
    const SQLITE_TIMESTAMP_FORMATS: [&str; 4] = [
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
    ];

    SQLITE_TIMESTAMP_FORMATS
        .iter()
        .find_map(|format| NaiveDateTime::parse_from_str(value, format).ok())
}

fn sqlite_utc_timestamp_for_remote(value: &str) -> String {
    parse_sqlite_timestamp(value)
        .map(|timestamp| DateTime::<Utc>::from_naive_utc_and_offset(timestamp, Utc).to_rfc3339())
        .unwrap_or_else(|| value.to_string())
}

fn sqlite_local_timestamp_for_remote(value: &str) -> String {
    parse_sqlite_timestamp(value)
        .map(|timestamp| timestamp.format("%Y-%m-%dT%H:%M:%S%.f").to_string())
        .unwrap_or_else(|| value.to_string())
}

#[instrument(skip(remote_settings))]
pub async fn setup_database_pools(
    local_db_path: &str,
    remote_settings: &RemoteDatabaseSettings,
) -> Result<(Pool<Sqlite>, Option<Pool<Postgres>>)> {
    info!("Setting up database pools...");

    info!("Setting up local SQLite pool at: {}", local_db_path);
    if let Some(parent_dir) = Path::new(local_db_path).parent() {
        tokio::fs::create_dir_all(parent_dir).await?;
    }
    let sqlite_pool = SqlitePool::connect_with(
        sqlx::sqlite::SqliteConnectOptions::new()
            .filename(local_db_path)
            .create_if_missing(true),
    )
    .await?;
    info!("Local SQLite pool created.");

    let pg_pool_option: Option<Pool<Postgres>> = match &remote_settings.postgres_url {
        Some(url) if !url.is_empty() => {
            info!("Setting up remote Postgres pool for URL...");
            match PgPool::connect(url).await {
                Ok(pool) => {
                    info!("Remote Postgres pool created.");
                    Some(pool)
                }
                Err(e) => {
                    warn!("Failed to connect to remote Postgres DB: {}. Remote sync will be disabled.", e);
                    None
                }
            }
        }
        _ => {
            info!("No remote Postgres URL configured.");
            None
        }
    };

    Ok((sqlite_pool, pg_pool_option))
}

#[instrument(skip(sqlite_pool, pg_pool_option))]
pub async fn run_migrations(
    sqlite_pool: &Pool<Sqlite>,
    pg_pool_option: &Option<Pool<Postgres>>,
) -> Result<()> {
    info!("Running database migrations...");

    info!("Running migrations on local SQLite DB...");
    SQLITE_MIGRATOR.run(sqlite_pool).await?;
    info!("Local SQLite migrations completed.");

    if let Some(pg_pool) = pg_pool_option {
        info!("Running migrations on remote Postgres DB...");
        match POSTGRES_MIGRATOR.run(pg_pool).await {
            Ok(_) => info!("Remote Postgres migrations completed."),
            Err(e) => {
                warn!(
                    "Failed to run migrations on remote Postgres DB: {}. Remote sync might fail.",
                    e
                );
            }
        }
    }
    Ok(())
}

#[instrument(skip(pool))]
pub async fn load_initial_totals(pool: &Pool<Sqlite>, device_id: &str) -> Result<(usize, usize, usize, f64)> {
    // First try loading from the summary table
    match load_initial_totals_from_summary(pool, device_id).await {
        Ok(totals) => Ok(totals),
        Err(e) => {
            warn!("Failed to load totals from summary table: {}. Falling back to aggregating metrics table.", e);
            load_initial_totals_from_metrics(pool, device_id).await
        }
    }
}

#[instrument(skip(pool))]
async fn load_initial_totals_from_metrics(
    pool: &Pool<Sqlite>,
    device_id: &str,
) -> Result<(usize, usize, usize, f64)> {
    info!("Loading initial totals by aggregating metrics table...");
    let query = Query::select()
        .expr_as(
            Expr::col(MetricsIden::Keypresses).sum(),
            Alias::new("total_keys"),
        )
        .expr_as(
            Expr::col(MetricsIden::MouseClicks).sum(),
            Alias::new("total_clicks"),
        )
        .expr_as(
            Expr::col(MetricsIden::ScrollSteps).sum(),
            Alias::new("total_scrolls"),
        )
        .expr_as(
            Expr::col(MetricsIden::MouseDistanceIn).sum(),
            Alias::new("total_distance"),
        )
        .from(MetricsIden::Table)
        .and_where(Expr::col(MetricsIden::DeviceId).eq(device_id))
        .to_owned();

    let (sql, values) = query.build_sqlx(SqliteQueryBuilder);

    let row_opt = sqlx::query_with(&sql, values).fetch_optional(pool).await?;

    match row_opt {
        Some(r) => {
            use sqlx::Row;
            let keys: i64 = r.try_get("total_keys").unwrap_or(0);
            let clicks: i64 = r.try_get("total_clicks").unwrap_or(0);
            let scrolls: i64 = r.try_get("total_scrolls").unwrap_or(0);
            let distance: f64 = r.try_get("total_distance").unwrap_or(0.0);
            info!(
                "Loaded totals from metrics table: K={}, C={}, S={}, D={:.2}",
                keys, clicks, scrolls, distance
            );
            Ok((keys as usize, clicks as usize, scrolls as usize, distance))
        }
        None => {
            info!("No previous data found in metrics table, starting totals from zero.");
            Ok((0, 0, 0, 0.0))
        }
    }
}

#[instrument(skip(pool))]
async fn load_initial_totals_from_summary(
    pool: &Pool<Sqlite>,
    device_id: &str,
) -> Result<(usize, usize, usize, f64)> {
    info!("Loading initial totals from metrics_summary table...");
    let query = Query::select()
        .columns([
            MetricsSummaryIden::TotalKeypresses,
            MetricsSummaryIden::TotalMouseClicks,
            MetricsSummaryIden::TotalScrollSteps,
            MetricsSummaryIden::TotalMouseTravelIn,
        ])
        .from(MetricsSummaryIden::Table)
        .and_where(Expr::col(MetricsSummaryIden::DeviceId).eq(device_id))
        .limit(1)
        .to_owned();

    let (sql, values) = query.build_sqlx(SqliteQueryBuilder);

    let row_opt = sqlx::query_with(&sql, values).fetch_optional(pool).await?;

    match row_opt {
        Some(r) => {
            use sqlx::Row;
            let keys: i64 = r.try_get(MetricsSummaryIden::TotalKeypresses.to_string().as_str())?;
            let clicks: i64 =
                r.try_get(MetricsSummaryIden::TotalMouseClicks.to_string().as_str())?;
            let scrolls: i64 =
                r.try_get(MetricsSummaryIden::TotalScrollSteps.to_string().as_str())?;
            let distance: f64 =
                r.try_get(MetricsSummaryIden::TotalMouseTravelIn.to_string().as_str())?;
            info!(
                "Loaded totals from summary: K={}, C={}, S={}, D={:.2}",
                keys, clicks, scrolls, distance
            );
            Ok((keys as usize, clicks as usize, scrolls as usize, distance))
        }
        None => {
            warn!("Metrics summary row for device {} not found! Initializing totals to zero. Please check migrations.", device_id);
            Ok((0, 0, 0, 0.0))
        }
    }
}

#[instrument(skip(pool, identity))]
pub async fn backfill_sqlite_identity(pool: &Pool<Sqlite>, identity: &DeviceIdentity) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE metrics
        SET device_id = ?, device_name = ?
        WHERE device_id IS NULL OR device_id = ''
        "#,
    )
    .bind(&identity.device_id)
    .bind(&identity.device_name)
    .execute(pool)
    .await?;

    sqlx::query(
        r#"
        INSERT INTO metrics_summary (
            device_id,
            device_name,
            last_updated,
            total_keypresses,
            total_mouse_clicks,
            total_mouse_travel_in,
            total_mouse_travel_mi,
            total_scroll_steps
        )
        SELECT
            ?,
            ?,
            COALESCE(MAX(timestamp), CURRENT_TIMESTAMP),
            COALESCE(SUM(keypresses), 0),
            COALESCE(SUM(mouse_clicks), 0),
            COALESCE(SUM(mouse_distance_in), 0),
            COALESCE(SUM(mouse_distance_mi), 0),
            COALESCE(SUM(scroll_steps), 0)
        FROM metrics
        WHERE device_id = ?
        ON CONFLICT(device_id) DO UPDATE SET
            device_name = excluded.device_name,
            last_updated = excluded.last_updated,
            total_keypresses = excluded.total_keypresses,
            total_mouse_clicks = excluded.total_mouse_clicks,
            total_mouse_travel_in = excluded.total_mouse_travel_in,
            total_mouse_travel_mi = excluded.total_mouse_travel_mi,
            total_scroll_steps = excluded.total_scroll_steps
        "#,
    )
    .bind(&identity.device_id)
    .bind(&identity.device_name)
    .bind(&identity.device_id)
    .execute(pool)
    .await?;

    Ok(())
}

async fn persist_metrics_sqlite_in_tx(
    tx: &mut Transaction<'_, Sqlite>,
    data: &MetricsData,
    identity: &DeviceIdentity,
) -> Result<()> {
    let distance_mi = data.mouse_distance_in / 63360.0;
    let capture_timestamps = CaptureTimestamps::now();

    sqlx::query(
        r#"
        INSERT INTO metrics (
            keypresses,
            mouse_clicks,
            scroll_steps,
            mouse_distance_in,
            mouse_distance_mi,
            device_id,
            device_name,
            timestamp,
            timestamp_local,
            local_utc_offset_minutes
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
        .bind(data.keypresses as i64)
        .bind(data.mouse_clicks as i64)
        .bind(data.scroll_steps as i64)
        .bind(data.mouse_distance_in)
        .bind(distance_mi)
        .bind(&identity.device_id)
        .bind(&identity.device_name)
        .bind(capture_timestamps.utc_naive())
        .bind(capture_timestamps.local_naive())
        .bind(capture_timestamps.local_utc_offset_minutes())
        .execute(&mut **tx)
        .await?;

    Ok(())
}

async fn persist_metrics_postgres_in_tx(
    tx: &mut Transaction<'_, Postgres>,
    data: &MetricsData,
    identity: &DeviceIdentity,
) -> Result<()> {
    let distance_mi = data.mouse_distance_in / 63360.0;
    let capture_timestamps = CaptureTimestamps::now();

    sqlx::query(
        r#"
        INSERT INTO metrics (
            keypresses,
            mouse_clicks,
            scroll_steps,
            mouse_distance_in,
            mouse_distance_mi,
            device_id,
            device_name,
            timestamp,
            timestamp_local,
            local_utc_offset_minutes
        ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
        "#,
    )
        .bind(data.keypresses as i64)
        .bind(data.mouse_clicks as i64)
        .bind(data.scroll_steps as i64)
        .bind(data.mouse_distance_in)
        .bind(distance_mi)
        .bind(&identity.device_id)
        .bind(&identity.device_name)
        .bind(capture_timestamps.utc)
        .bind(capture_timestamps.local_naive())
        .bind(capture_timestamps.local_utc_offset_minutes())
        .execute(&mut **tx)
        .await?;

    Ok(())
}

#[instrument(skip(pool, data), fields(db_type = "sqlite"))]
pub async fn persist_metrics_sqlite(
    pool: &Pool<Sqlite>,
    data: &MetricsData,
    identity: &DeviceIdentity,
) -> Result<()> {
    if data.keypresses == 0
        && data.mouse_clicks == 0
        && data.scroll_steps == 0
        && data.mouse_distance_in == 0.0
    {
        return Ok(());
    }

    persist_metrics_transactional_sqlite(pool, data, identity).await
}

#[instrument(skip(pool, data), fields(db_type = "postgres"))]
pub async fn persist_metrics_postgres(
    pool: &Pool<Postgres>,
    data: &MetricsData,
    identity: &DeviceIdentity,
) -> Result<()> {
    if data.keypresses == 0
        && data.mouse_clicks == 0
        && data.scroll_steps == 0
        && data.mouse_distance_in == 0.0
    {
        return Ok(());
    }

    persist_metrics_transactional_postgres(pool, data, identity).await
}

#[instrument(skip(pool, data), fields(db_type = "sqlite"))]
pub async fn persist_metrics_transactional_sqlite(
    pool: &Pool<Sqlite>,
    data: &MetricsData,
    identity: &DeviceIdentity,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    let result = persist_metrics_sqlite_in_tx(&mut tx, data, identity).await;

    match result {
        Ok(_) => {
            tx.commit().await?;
            debug!(
                "SQLite transaction committed for metrics interval: {:?}",
                data
            );
            Ok(())
        }
        Err(e) => {
            warn!("SQLite transaction failed, rolling back: {}", e);
            let _ = tx.rollback().await;
            Err(e)
        }
    }
}

#[instrument(skip(pool, data), fields(db_type = "postgres"))]
pub async fn persist_metrics_transactional_postgres(
    pool: &Pool<Postgres>,
    data: &MetricsData,
    identity: &DeviceIdentity,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    let result = persist_metrics_postgres_in_tx(&mut tx, data, identity).await;

    match result {
        Ok(_) => {
            tx.commit().await?;
            debug!(
                "Postgres transaction committed for metrics interval: {:?}",
                data
            );
            Ok(())
        }
        Err(e) => {
            warn!("Postgres transaction failed, rolling back: {}", e);
            let _ = tx.rollback().await;
            Err(e)
        }
    }
}

// --- Supabase REST API sync ---

const SUPABASE_BATCH_SIZE: u32 = 100;

#[derive(Clone)]
pub struct SupabaseClient {
    client: Postgrest,
    supports_local_time_columns: bool,
}

pub async fn setup_supabase_client(
    settings: &RemoteDatabaseSettings,
) -> Option<SupabaseClient> {
    let url = settings.supabase_url.as_deref().filter(|s| !s.is_empty())?;
    let api_key = settings.supabase_api_key.as_deref().filter(|s| !s.is_empty())?;

    let rest_url = format!("{}/rest/v1", url.trim_end_matches('/'));
    let client = Postgrest::new(&rest_url)
        .insert_header("apikey", api_key);

    // Verify connectivity
    let resp = client
        .from("metrics")
        .select("id")
        .limit(0)
        .execute()
        .await;

    match resp {
        Ok(r) if r.status().is_success() => {
            info!("Supabase REST API connected at {}", url);
        }
        Ok(r) => {
            warn!(
                "Supabase REST API returned HTTP {}: check API key and table permissions. Remote sync will be disabled.",
                r.status()
            );
            return None;
        }
        Err(e) => {
            warn!(
                "Failed to reach Supabase REST API at {}: {}. Remote sync will be disabled.",
                url, e
            );
            return None;
        }
    }

    let supports_local_time_columns = match client
        .from("metrics")
        .select("timestamp_local,local_utc_offset_minutes")
        .limit(0)
        .execute()
        .await
    {
        Ok(r) if r.status().is_success() => true,
        Ok(r) => {
            warn!(
                "Supabase metrics table does not yet expose timestamp_local/local_utc_offset_minutes (HTTP {}). Continuing without local timestamp sync.",
                r.status()
            );
            false
        }
        Err(e) => {
            warn!(
                "Failed to probe Supabase local timestamp columns: {}. Continuing without local timestamp sync.",
                e
            );
            false
        }
    };

    Some(SupabaseClient {
        client,
        supports_local_time_columns,
    })
}

/// Sync all unsynced local SQLite rows to Supabase in batches. Returns the number of rows synced.
pub async fn sync_to_supabase(
    supabase: &SupabaseClient,
    sqlite_pool: &Pool<Sqlite>,
) -> Result<u64> {
    let mut total_synced: u64 = 0;

    loop {
        let rows = sqlx::query_as::<_, (i64, i64, i64, i64, f64, f64, Option<String>, Option<String>, String, Option<String>, Option<i32>)>(
            r#"SELECT id, keypresses, mouse_clicks, scroll_steps, mouse_distance_in, mouse_distance_mi, device_id, device_name, timestamp, timestamp_local, local_utc_offset_minutes
               FROM metrics
               WHERE supabase_synced_at IS NULL
               ORDER BY id ASC
               LIMIT ?"#,
        )
        .bind(SUPABASE_BATCH_SIZE)
        .fetch_all(sqlite_pool)
        .await?;

        if rows.is_empty() {
            break;
        }

        let batch_len = rows.len() as u64;
        let first_id = rows.first().map(|r| r.0).unwrap_or(0);
        let last_id = rows.last().map(|r| r.0).unwrap_or(0);

        let json_rows: Vec<serde_json::Value> = rows
            .iter()
            .map(|r| {
                let mut row = serde_json::Map::from_iter([
                    (
                        "keypresses".to_string(),
                        serde_json::json!(r.1),
                    ),
                    (
                        "mouse_clicks".to_string(),
                        serde_json::json!(r.2),
                    ),
                    (
                        "scroll_steps".to_string(),
                        serde_json::json!(r.3),
                    ),
                    (
                        "mouse_distance_in".to_string(),
                        serde_json::json!(r.4),
                    ),
                    (
                        "mouse_distance_mi".to_string(),
                        serde_json::json!(r.5),
                    ),
                    (
                        "device_id".to_string(),
                        serde_json::json!(r.6),
                    ),
                    (
                        "device_name".to_string(),
                        serde_json::json!(r.7),
                    ),
                    (
                        "timestamp".to_string(),
                        serde_json::json!(sqlite_utc_timestamp_for_remote(&r.8)),
                    ),
                ]);

                if supabase.supports_local_time_columns {
                    row.insert(
                        "timestamp_local".to_string(),
                        serde_json::json!(
                            r.9.as_deref().map(sqlite_local_timestamp_for_remote)
                        ),
                    );
                    row.insert(
                        "local_utc_offset_minutes".to_string(),
                        serde_json::json!(r.10),
                    );
                }

                serde_json::Value::Object(row)
            })
            .collect();

        let body = serde_json::to_string(&json_rows).map_err(|e| {
            crate::error::AppError::Initialization(format!("JSON serialization failed: {}", e))
        })?;

        let resp = supabase
            .client
            .from("metrics")
            .insert(body)
            .execute()
            .await
            .map_err(|e| {
                crate::error::AppError::Initialization(format!("Supabase insert failed: {}", e))
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(crate::error::AppError::Initialization(format!(
                "Supabase insert failed ({}): {}",
                status, text
            )));
        }

        // Mark batch as synced
        sqlx::query(
            "UPDATE metrics SET supabase_synced_at = CURRENT_TIMESTAMP WHERE id >= ? AND id <= ? AND supabase_synced_at IS NULL",
        )
        .bind(first_id)
        .bind(last_id)
        .execute(sqlite_pool)
        .await?;

        total_synced += batch_len;
        info!(
            "Supabase sync: {} rows (ids {}..{}), {} total",
            batch_len, first_id, last_id, total_synced
        );
    }

    Ok(total_synced)
}
