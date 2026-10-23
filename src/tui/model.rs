use serde::{Deserialize, Serialize};

/// One installed package as the interface understands it.
///
/// This is the integration boundary. The backend can map its own records to
/// this shape without exposing any backend types to the TUI.
///
/// `reason` is also used as the cleanup classification. Put one of `orphan`,
/// `unused dependency`, `dependency`, or `explicit` in it (case-insensitive).
/// The cleanup flow only offers packages whose reason mentions `orphan`,
/// `unused`, or `removable`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Package {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    /// Installed size in bytes.
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub explicit: bool,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub depends: Vec<String>,
    #[serde(default)]
    pub provides: Vec<String>,
    #[serde(default)]
    pub optdepends: Vec<String>,
    #[serde(default)]
    pub foreign: bool,
}

impl Package {
    pub fn size_label(&self) -> String {
        format_size(self.size)
    }

    pub fn reason_label(&self) -> &str {
        if !self.reason.is_empty() {
            &self.reason
        } else if self.explicit {
            "explicit"
        } else {
            "dependency"
        }
    }

    pub fn is_orphan(&self) -> bool {
        let reason = self.reason.to_ascii_lowercase();
        reason.contains("orphan")
    }

    pub fn is_cleanup_candidate(&self) -> bool {
        let reason = self.reason.to_ascii_lowercase();
        reason.contains("orphan") || reason.contains("unused") || reason.contains("removable")
    }

    pub fn searchable(&self) -> String {
        let mut haystack = format!(
            "{} {} {} {} {}",
            self.name,
            self.version,
            self.description,
            self.reason_label(),
            self.depends.join(" ")
        );
        haystack.push(' ');
        haystack.push_str(&self.provides.join(" "));
        haystack.push(' ');
        haystack.push_str(&self.optdepends.join(" "));
        haystack
    }
}

/// One compact pacman transaction supplied by the host scanner.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimelineItem {
    #[serde(default)]
    pub timestamp: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub detail: String,
    #[serde(default)]
    pub kind: TimelineKind,
}

impl TimelineItem {
    pub fn new(
        timestamp: impl Into<String>,
        title: impl Into<String>,
        detail: impl Into<String>,
        kind: TimelineKind,
    ) -> Self {
        Self {
            timestamp: timestamp.into(),
            title: title.into(),
            detail: detail.into(),
            kind,
        }
    }
}

/// Coarse pacman transaction classification for the Timeline tab.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimelineKind {
    Install,
    Upgrade,
    Remove,
    Downgrade,
    Reinstall,
    #[default]
    Other,
}

impl TimelineKind {
    pub fn label(self) -> &'static str {
        match self {
            TimelineKind::Install => "INSTALL",
            TimelineKind::Upgrade => "UPGRADE",
            TimelineKind::Remove => "REMOVE",
            TimelineKind::Downgrade => "DOWNGRADE",
            TimelineKind::Reinstall => "REINSTALL",
            TimelineKind::Other => "PACMAN",
        }
    }

    pub fn marker(self) -> &'static str {
        match self {
            TimelineKind::Install => "+",
            TimelineKind::Upgrade => "↑",
            TimelineKind::Remove => "−",
            TimelineKind::Downgrade => "↓",
            TimelineKind::Reinstall => "↻",
            TimelineKind::Other => "◆",
        }
    }
}

/// A complete view of the installed package set.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Snapshot {
    pub packages: Vec<Package>,
    #[serde(default)]
    pub scanned_at: String,
    /// Optional stored health scores, oldest first. Callers may omit this.
    #[serde(default)]
    pub health_history: Vec<u8>,
    /// Optional pacman/package transaction history. Callers may omit this.
    #[serde(default)]
    pub timeline: Vec<TimelineItem>,
}

impl Snapshot {
    pub fn new(packages: Vec<Package>, scanned_at: impl Into<String>) -> Self {
        Self {
            packages,
            scanned_at: scanned_at.into(),
            health_history: Vec::new(),
            timeline: Vec::new(),
        }
    }

    pub fn with_health_history(mut self, history: impl Into<Vec<u8>>) -> Self {
        self.health_history = history.into();
        self
    }

    pub fn with_timeline(mut self, timeline: Vec<TimelineItem>) -> Self {
        self.timeline = timeline;
        self
    }

    /// An illustrative dataset used only for the preview before a real scan
    /// lands. Every package is local in-memory data; no system access happens.
    pub fn demo() -> Self {
        crate::tui::demo::snapshot()
    }

    pub fn total_size(&self) -> u64 {
        self.packages.iter().map(|package| package.size).sum()
    }
}

/// Strip version constraints and architecture suffixes from a dependency token.
pub fn normalize_dependency(token: &str) -> String {
    token
        .split_whitespace()
        .next()
        .unwrap_or(token)
        .split(['<', '>', '='])
        .next()
        .unwrap_or(token)
        .trim_matches(|character: char| {
            character == ',' || character == ':' || character.is_whitespace()
        })
        .to_string()
}

/// Human-readable binary size, matching the rest of AMPH's reporting.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0usize;

    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }

    if unit == 0 {
        format!("{} {}", bytes, UNITS[unit])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

pub fn format_count(value: usize) -> String {
    match value {
        0..=999 => value.to_string(),
        1_000..=999_999 => format!("{:.1}K", value as f64 / 1_000.0),
        _ => format!("{:.1}M", value as f64 / 1_000_000.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sizes_use_binary_units() {
        assert_eq!(format_size(0), "0 B");
        assert_eq!(format_size(1024), "1.0 KiB");
        assert_eq!(format_size(1024 * 1024), "1.0 MiB");
    }

    #[test]
    fn dependency_tokens_are_normalized() {
        assert_eq!(normalize_dependency("glibc>=2.38"), "glibc");
        assert_eq!(normalize_dependency("libfoo: extra"), "libfoo");
    }

    #[test]
    fn optional_history_fields_default_to_empty() {
        let snapshot: Snapshot = serde_json::from_str(r#"{"packages":[],"scanned_at":"test"}"#)
            .expect("snapshot without optional history");
        assert!(snapshot.health_history.is_empty());
        assert!(snapshot.timeline.is_empty());
    }

    #[test]
    fn timeline_items_round_trip() {
        let snapshot = Snapshot::new(Vec::new(), "now")
            .with_health_history(vec![80, 72, 64])
            .with_timeline(vec![TimelineItem::new(
                "2026-10-22 21:14",
                "linux 6.12",
                "upgraded",
                TimelineKind::Upgrade,
            )]);
        let encoded = serde_json::to_string(&snapshot).expect("serialize");
        let decoded: Snapshot = serde_json::from_str(&encoded).expect("deserialize");
        assert_eq!(decoded.health_history, vec![80, 72, 64]);
        assert_eq!(decoded.timeline.len(), 1);
        assert_eq!(decoded.timeline[0].kind, TimelineKind::Upgrade);
        assert_eq!(decoded.timeline[0].timestamp, "2026-10-22 21:14");
    }

    #[test]
    fn cleanup_classification_is_reason_based() {
        let package = Package {
            name: "example".to_string(),
            reason: "orphan".to_string(),
            ..Package::default()
        };
        assert!(package.is_cleanup_candidate());
        assert!(!Package::default().is_cleanup_candidate());
    }
}
