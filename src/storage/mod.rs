use crate::analyzer::SystemAnalysis;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub collected_at: String,
    pub analysis: SystemAnalysis,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct HistoryFile {
    pub entries: Vec<HistoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryDelta {
    pub current_packages: usize,
    pub previous_packages: usize,
    pub added_packages: Vec<String>,
    pub removed_packages: Vec<String>,
    pub health_delta: i16,
    pub orphan_delta: i16,
    pub size_delta_bytes: i128,
}

pub fn record_scan(analysis: &SystemAnalysis) -> Result<()> {
    let mut history = load_history()?;
    history.entries.push(HistoryEntry {
        collected_at: analysis.snapshot.generated_at.clone(),
        analysis: analysis.clone(),
    });

    write_history(&history)
}

pub fn load_history() -> Result<HistoryFile> {
    let path = history_file_path()?;
    if !path.exists() {
        return Ok(HistoryFile::default());
    }

    let raw = fs::read_to_string(&path)
        .with_context(|| format!("Failed to read history file {}", path.display()))?;
    let history = serde_json::from_str(&raw)
        .with_context(|| format!("Failed to parse history file {}", path.display()))?;
    Ok(history)
}

pub fn compare_recent(history: &HistoryFile) -> Option<HistoryDelta> {
    let current = history.entries.last()?;
    let previous = history.entries.iter().rev().nth(1)?;

    let current_names = package_names(&current.analysis);
    let previous_names = package_names(&previous.analysis);

    let added_packages = current_names
        .difference(&previous_names)
        .cloned()
        .collect::<Vec<_>>();
    let removed_packages = previous_names
        .difference(&current_names)
        .cloned()
        .collect::<Vec<_>>();

    let current_size = current.analysis.stats.total_size_bytes as i128;
    let previous_size = previous.analysis.stats.total_size_bytes as i128;

    Some(HistoryDelta {
        current_packages: current.analysis.stats.total_packages,
        previous_packages: previous.analysis.stats.total_packages,
        added_packages,
        removed_packages,
        health_delta: current.analysis.health_score as i16 - previous.analysis.health_score as i16,
        orphan_delta: current.analysis.cleanup.orphans.len() as i16 - previous.analysis.cleanup.orphans.len() as i16,
        size_delta_bytes: current_size - previous_size,
    })
}

fn package_names(analysis: &SystemAnalysis) -> BTreeSet<String> {
    analysis
        .snapshot
        .packages
        .iter()
        .map(|package| package.name.clone())
        .collect()
}

fn write_history(history: &HistoryFile) -> Result<()> {
    let path = history_file_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create history directory {}", parent.display()))?;
    }

    let json = serde_json::to_string_pretty(history).context("Failed to serialize history file")?;
    fs::write(&path, json).with_context(|| format!("Failed to write history file {}", path.display()))?;
    Ok(())
}

pub fn history_file_path() -> Result<PathBuf> {
    let base = if let Ok(value) = env::var("XDG_STATE_HOME") {
        PathBuf::from(value)
    } else if let Ok(home) = env::var("HOME") {
        Path::new(&home).join(".local/state")
    } else {
        PathBuf::from(".")
    };

    Ok(base.join("amph").join("history.json"))
}
