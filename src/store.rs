//! Compact, bounded scan history.
//!
//! History is stored as JSON and always trimmed to the configured limit.  Saves
//! are atomic (write a sibling temporary file, then rename) so an interrupted
//! write cannot corrupt the previous history.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// One compact history record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub timestamp: u64,
    pub packages: usize,
    pub explicit: usize,
    pub foreign: usize,
    pub health: f32,
    pub freed_bytes: u64,
    pub removed: usize,
    #[serde(default)]
    pub note: Option<String>,
}

impl HistoryEntry {
    pub fn new(timestamp: u64) -> Self {
        Self {
            timestamp,
            packages: 0,
            explicit: 0,
            foreign: 0,
            health: 100.0,
            freed_bytes: 0,
            removed: 0,
            note: None,
        }
    }
}

/// Bounded history file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct History {
    #[serde(default)]
    pub entries: Vec<HistoryEntry>,
    #[serde(default = "default_limit")]
    pub limit: usize,
}

fn default_limit() -> usize {
    100
}

impl Default for History {
    fn default() -> Self {
        Self::new(100)
    }
}

impl History {
    pub fn new(limit: usize) -> Self {
        Self {
            entries: Vec::new(),
            limit: limit.max(1),
        }
    }

    /// Load history, tolerating a missing file.  Malformed JSON is reported as
    /// an error rather than silently discarding the user's data.
    pub fn load(path: &Path, limit: usize) -> Result<Self> {
        let limit = limit.max(1);
        if !path.exists() {
            return Ok(Self::new(limit));
        }
        let bytes =
            fs::read(path).with_context(|| format!("cannot read history {}", path.display()))?;
        let mut history: History = serde_json::from_slice(&bytes)
            .with_context(|| format!("cannot parse history {}", path.display()))?;
        history.limit = limit;
        history.compact();
        Ok(history)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        let bytes = serde_json::to_vec_pretty(self).context("cannot serialize history")?;
        let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
        fs::write(&temporary, bytes)
            .with_context(|| format!("cannot write {}", temporary.display()))?;
        fs::rename(&temporary, path)
            .with_context(|| format!("cannot replace {}", path.display()))?;
        Ok(())
    }

    /// Append an entry, compact and persist.
    pub fn record(&mut self, entry: HistoryEntry, path: &Path) -> Result<()> {
        self.entries.push(entry);
        self.compact();
        self.save(path)
    }

    /// Sort by timestamp and keep the newest entry per timestamp, then enforce
    /// the configured bound.
    pub fn compact(&mut self) {
        self.limit = self.limit.max(1);
        self.entries.sort_by_key(|entry| entry.timestamp);

        let mut by_timestamp: BTreeMap<u64, HistoryEntry> = BTreeMap::new();
        for entry in self.entries.drain(..) {
            by_timestamp.insert(entry.timestamp, entry);
        }
        self.entries = by_timestamp.into_values().collect();

        let excess = self.entries.len().saturating_sub(self.limit);
        if excess > 0 {
            self.entries.drain(0..excess);
        }
    }

    pub fn recent(&self, count: usize) -> &[HistoryEntry] {
        let start = self.entries.len().saturating_sub(count);
        &self.entries[start..]
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// `$XDG_STATE_HOME/amph/history.json` or `~/.local/state/amph/history.json`.
pub fn default_history_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_STATE_HOME") {
        let path = PathBuf::from(path);
        if !path.as_os_str().is_empty() {
            return Some(path.join("amph").join("history.json"));
        }
    }
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join(".local")
            .join("state")
            .join("amph")
            .join("history.json")
    })
}

pub fn load_default(limit: usize) -> Result<History> {
    match default_history_path() {
        Some(path) => History::load(&path, limit),
        None => Ok(History::new(limit)),
    }
}

pub fn record_default(entry: HistoryEntry, limit: usize) -> Result<History> {
    match default_history_path() {
        Some(path) => {
            let mut history = History::load(&path, limit)?;
            history.record(entry, &path)?;
            Ok(history)
        }
        None => {
            let mut history = History::new(limit);
            history.entries.push(entry);
            history.compact();
            Ok(history)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempFile {
        path: PathBuf,
    }

    impl TempFile {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or(0);
            let path = std::env::temp_dir().join(format!(
                "amph-store-{label}-{}-{unique}.json",
                std::process::id()
            ));
            let _ = fs::remove_file(&path);
            Self { path }
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.path);
        }
    }

    #[test]
    fn records_compacts_and_bounds_history() {
        let file = TempFile::new("history");
        let mut history = History::new(3);

        for timestamp in 1..=5 {
            let mut entry = HistoryEntry::new(timestamp);
            entry.packages = timestamp as usize;
            history.record(entry, &file.path).expect("record");
        }

        assert_eq!(history.len(), 3);
        assert_eq!(
            history
                .entries
                .iter()
                .map(|e| e.timestamp)
                .collect::<Vec<_>>(),
            vec![3, 4, 5]
        );

        let loaded = History::load(&file.path, 10).expect("load");
        assert_eq!(loaded.len(), 3);
        assert_eq!(loaded.entries.last().map(|e| e.timestamp), Some(5));
    }

    #[test]
    fn missing_history_is_empty() {
        let file = TempFile::new("missing");
        let history = History::load(&file.path, 5).expect("load missing");
        assert!(history.is_empty());
    }

    #[test]
    fn duplicate_timestamps_keep_latest_record() {
        let mut history = History::new(10);
        let mut first = HistoryEntry::new(10);
        first.health = 50.0;
        let mut second = HistoryEntry::new(10);
        second.health = 99.0;
        history.entries.push(first);
        history.entries.push(second);

        history.compact();
        assert_eq!(history.len(), 1);
        assert!((history.entries[0].health - 99.0).abs() < f32::EPSILON);
    }
}
