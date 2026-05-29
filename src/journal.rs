use crate::db::MetricsData;
use anyhow::Context;
use chrono::{Local, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    pub journal_id: String,
    pub timestamp_utc: String,
    pub timestamp_local: String,
    pub local_utc_offset_minutes: i32,
    pub keypresses: usize,
    pub mouse_clicks: usize,
    pub scroll_steps: usize,
    pub mouse_distance_in: f64,
}

impl JournalEntry {
    pub fn new(data: &MetricsData) -> Self {
        let utc = Utc::now();
        let local = utc.with_timezone(&Local);

        Self {
            journal_id: Uuid::new_v4().to_string(),
            timestamp_utc: utc.naive_utc().format("%Y-%m-%d %H:%M:%S%.f").to_string(),
            timestamp_local: local
                .naive_local()
                .format("%Y-%m-%d %H:%M:%S%.f")
                .to_string(),
            local_utc_offset_minutes: local.offset().local_minus_utc() / 60,
            keypresses: data.keypresses,
            mouse_clicks: data.mouse_clicks,
            scroll_steps: data.scroll_steps,
            mouse_distance_in: data.mouse_distance_in,
        }
    }

    pub fn metrics_data(&self) -> MetricsData {
        MetricsData {
            keypresses: self.keypresses,
            mouse_clicks: self.mouse_clicks,
            scroll_steps: self.scroll_steps,
            mouse_distance_in: self.mouse_distance_in,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MetricsJournal {
    path: PathBuf,
}

impl MetricsJournal {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn append(&self, entry: &JournalEntry) -> anyhow::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create journal directory {}", parent.display())
            })?;
        }

        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .with_context(|| format!("Failed to open metrics journal {}", self.path.display()))?;

        serde_json::to_writer(&mut file, entry)
            .with_context(|| format!("Failed to serialize journal entry {}", entry.journal_id))?;
        file.write_all(b"\n")
            .context("Failed to write journal newline")?;
        file.sync_data()
            .with_context(|| format!("Failed to fsync metrics journal {}", self.path.display()))?;

        Ok(())
    }

    pub fn load_entries(&self) -> anyhow::Result<Vec<JournalEntry>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }

        let file = File::open(&self.path)
            .with_context(|| format!("Failed to open metrics journal {}", self.path.display()))?;
        let reader = BufReader::new(file);
        let mut entries = Vec::new();

        for (index, line) in reader.lines().enumerate() {
            let line = line.with_context(|| {
                format!(
                    "Failed to read line {} from metrics journal {}",
                    index + 1,
                    self.path.display()
                )
            })?;
            if line.trim().is_empty() {
                continue;
            }
            let entry = serde_json::from_str::<JournalEntry>(&line).with_context(|| {
                format!(
                    "Failed to parse line {} from metrics journal {}",
                    index + 1,
                    self.path.display()
                )
            })?;
            entries.push(entry);
        }

        Ok(entries)
    }

    pub fn checkpoint_empty(&self) -> anyhow::Result<()> {
        if self.path.exists() {
            fs::remove_file(&self.path).with_context(|| {
                format!(
                    "Failed to remove checkpointed journal {}",
                    self.path.display()
                )
            })?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journal_round_trips_entries_and_checkpoints() {
        let path = std::env::temp_dir().join(format!("etsu-journal-test-{}.jsonl", Uuid::new_v4()));
        let journal = MetricsJournal::new(path.clone());
        let data = MetricsData {
            keypresses: 12,
            mouse_clicks: 3,
            scroll_steps: 45,
            mouse_distance_in: 67.5,
        };
        let entry = JournalEntry::new(&data);

        journal.append(&entry).expect("append journal entry");
        let loaded = journal.load_entries().expect("load journal entries");

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].journal_id, entry.journal_id);
        assert_eq!(loaded[0].keypresses, 12);
        assert_eq!(loaded[0].mouse_clicks, 3);
        assert_eq!(loaded[0].scroll_steps, 45);
        assert_eq!(loaded[0].mouse_distance_in, 67.5);

        journal.checkpoint_empty().expect("checkpoint journal");
        assert!(!path.exists());
    }
}
