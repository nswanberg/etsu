ALTER TABLE metrics ADD COLUMN IF NOT EXISTS device_id TEXT;
ALTER TABLE metrics ADD COLUMN IF NOT EXISTS device_name TEXT;

DROP TRIGGER IF EXISTS update_metrics_summary ON metrics;
DROP FUNCTION IF EXISTS update_metrics_summary_fn();

ALTER TABLE metrics_summary RENAME TO metrics_summary_legacy;

CREATE TABLE IF NOT EXISTS metrics_summary (
    device_id TEXT PRIMARY KEY NOT NULL,
    device_name TEXT NOT NULL,
    last_updated TIMESTAMP WITH TIME ZONE,
    total_keypresses BIGINT NOT NULL DEFAULT 0,
    total_mouse_clicks BIGINT NOT NULL DEFAULT 0,
    total_mouse_travel_in DOUBLE PRECISION NOT NULL DEFAULT 0,
    total_mouse_travel_mi DOUBLE PRECISION NOT NULL DEFAULT 0,
    total_scroll_steps BIGINT NOT NULL DEFAULT 0
);

DROP TABLE metrics_summary_legacy;

CREATE OR REPLACE FUNCTION update_metrics_summary_fn()
RETURNS TRIGGER AS $$
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
    ON CONFLICT (device_id) DO UPDATE
    SET
        device_name = EXCLUDED.device_name,
        total_keypresses = metrics_summary.total_keypresses + NEW.keypresses,
        total_mouse_clicks = metrics_summary.total_mouse_clicks + NEW.mouse_clicks,
        total_mouse_travel_in = metrics_summary.total_mouse_travel_in + NEW.mouse_distance_in,
        total_mouse_travel_mi = metrics_summary.total_mouse_travel_mi + NEW.mouse_distance_mi,
        total_scroll_steps = metrics_summary.total_scroll_steps + NEW.scroll_steps,
        last_updated = CURRENT_TIMESTAMP;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER update_metrics_summary
    AFTER INSERT ON metrics
    FOR EACH ROW
    EXECUTE FUNCTION update_metrics_summary_fn();
