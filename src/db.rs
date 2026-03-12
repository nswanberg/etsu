use crate::config::{DeviceIdentity, RemoteDatabaseSettings};
use crate::error::Result;
use sea_query::{Alias, Expr, Iden, PostgresQueryBuilder, Query, SqliteQueryBuilder};
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
    MouseDistanceMi,
    ScrollSteps,
    DeviceId,
    DeviceName,
    #[allow(dead_code)]
    Timestamp,
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

    let mut query_metrics = Query::insert();
    query_metrics
        .into_table(MetricsIden::Table)
        .columns([
            MetricsIden::Keypresses,
            MetricsIden::MouseClicks,
            MetricsIden::ScrollSteps,
            MetricsIden::MouseDistanceIn,
            MetricsIden::MouseDistanceMi,
            MetricsIden::DeviceId,
            MetricsIden::DeviceName,
        ])
        .values_panic([
            (data.keypresses as i64).into(),
            (data.mouse_clicks as i64).into(),
            (data.scroll_steps as i64).into(),
            data.mouse_distance_in.into(),
            distance_mi.into(),
            identity.device_id.clone().into(),
            identity.device_name.clone().into(),
        ]);
    let (sql_metrics, values_metrics) = query_metrics.build_sqlx(SqliteQueryBuilder);
    sqlx::query_with(&sql_metrics, values_metrics)
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

    let mut query_metrics = Query::insert();
    query_metrics
        .into_table(MetricsIden::Table)
        .columns([
            MetricsIden::Keypresses,
            MetricsIden::MouseClicks,
            MetricsIden::ScrollSteps,
            MetricsIden::MouseDistanceIn,
            MetricsIden::MouseDistanceMi,
            MetricsIden::DeviceId,
            MetricsIden::DeviceName,
        ])
        .values_panic([
            (data.keypresses as i64).into(),
            (data.mouse_clicks as i64).into(),
            (data.scroll_steps as i64).into(),
            data.mouse_distance_in.into(),
            distance_mi.into(),
            identity.device_id.clone().into(),
            identity.device_name.clone().into(),
        ]);
    let (sql_metrics, values_metrics) = query_metrics.build_sqlx(PostgresQueryBuilder);
    sqlx::query_with(&sql_metrics, values_metrics)
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
