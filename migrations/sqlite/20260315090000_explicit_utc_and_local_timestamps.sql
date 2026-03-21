ALTER TABLE metrics ADD COLUMN timestamp_local DATETIME;
ALTER TABLE metrics ADD COLUMN local_utc_offset_minutes INTEGER;
