ALTER TABLE metrics ADD COLUMN device_id TEXT;
ALTER TABLE metrics ADD COLUMN device_name TEXT;

DROP TRIGGER IF EXISTS update_metrics_summary;

ALTER TABLE metrics_summary RENAME TO metrics_summary_legacy;

CREATE TABLE IF NOT EXISTS metrics_summary (
    device_id TEXT PRIMARY KEY NOT NULL,
    device_name TEXT NOT NULL,
    last_updated DATETIME,
    total_keypresses BIGINT NOT NULL DEFAULT 0,
    total_mouse_clicks BIGINT NOT NULL DEFAULT 0,
    total_mouse_travel_in REAL NOT NULL DEFAULT 0,
    total_mouse_travel_mi REAL NOT NULL DEFAULT 0,
    total_scroll_steps BIGINT NOT NULL DEFAULT 0
);

DROP TABLE metrics_summary_legacy;

CREATE TRIGGER IF NOT EXISTS update_metrics_summary
    AFTER INSERT ON metrics
    FOR EACH ROW
BEGIN
    INSERT INTO metrics_summary (
        device_id,
        device_name,
        last_updated,
        total_keypresses,
        total_mouse_clicks,
        total_mouse_travel_in,
        total_mouse_travel_mi,
        total_scroll_steps
    ) VALUES (
        COALESCE(NEW.device_id, 'unassigned'),
        COALESCE(NEW.device_name, 'Unknown Device'),
        CURRENT_TIMESTAMP,
        NEW.keypresses,
        NEW.mouse_clicks,
        NEW.mouse_distance_in,
        NEW.mouse_distance_mi,
        NEW.scroll_steps
    )
    ON CONFLICT(device_id) DO UPDATE SET
        device_name = excluded.device_name,
        total_keypresses = total_keypresses + NEW.keypresses,
        total_mouse_clicks = total_mouse_clicks + NEW.mouse_clicks,
        total_mouse_travel_in = total_mouse_travel_in + NEW.mouse_distance_in,
        total_mouse_travel_mi = total_mouse_travel_mi + NEW.mouse_distance_mi,
        total_scroll_steps = total_scroll_steps + NEW.scroll_steps,
        last_updated = CURRENT_TIMESTAMP;
END;
