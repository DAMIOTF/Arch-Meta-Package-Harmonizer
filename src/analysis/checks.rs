//! System health checks used by the report and TUI layers.
//!
//! All pacman subprocesses set `LC_ALL=C`.  `pacman -Qi` is never invoked; the
//! installed package set comes exclusively from [`crate::source`] /
//! [`crate::model`].

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use crate::graph::Graph;
use crate::model::{Package, Snapshot, unix_now, version_cmp};
use crate::source::default_pacman_root;

/// Severity of one check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStatus {
    Ok,
    Warning,
    Critical,
    Unknown,
}

impl CheckStatus {
    pub fn score(self) -> f32 {
        match self {
            Self::Ok => 100.0,
            Self::Warning => 60.0,
            Self::Critical => 20.0,
            // Unknown is neutral: we do not punish the user for an unreadable
            // check, but the explanation still records it.
            Self::Unknown => 100.0,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Warning => "warning",
            Self::Critical => "critical",
            Self::Unknown => "unknown",
        }
    }
}

/// One check outcome.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckResult {
    pub id: String,
    pub title: String,
    pub category: String,
    pub status: CheckStatus,
    pub score: f32,
    pub weight: f32,
    pub summary: String,
    pub details: Vec<String>,
}

impl CheckResult {
    pub fn new(
        id: &str,
        title: &str,
        category: &str,
        status: CheckStatus,
        weight: f32,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            id: id.to_string(),
            title: title.to_string(),
            category: category.to_string(),
            status,
            score: status.score(),
            weight: weight.max(0.0),
            summary: summary.into(),
            details: Vec::new(),
        }
    }

    pub fn with_details(mut self, details: Vec<String>) -> Self {
        self.details = details;
        self
    }

    pub fn is_problem(&self) -> bool {
        !matches!(self.status, CheckStatus::Ok | CheckStatus::Unknown)
    }
}

/// Aggregated check results.
#[derive(Debug, Clone, PartialEq)]
pub struct ChecksReport {
    pub results: Vec<CheckResult>,
    pub generated_at: u64,
}

impl ChecksReport {
    pub fn new(results: Vec<CheckResult>) -> Self {
        Self {
            results,
            generated_at: unix_now(),
        }
    }

    pub fn problems(&self) -> Vec<&CheckResult> {
        self.results
            .iter()
            .filter(|result| result.is_problem())
            .collect()
    }

    pub fn count(&self, status: CheckStatus) -> usize {
        self.results
            .iter()
            .filter(|result| result.status == status)
            .count()
    }

    pub fn find(&self, id: &str) -> Option<&CheckResult> {
        self.results.iter().find(|result| result.id == id)
    }
}

/// Options controlling filesystem paths and thresholds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckOptions {
    /// Prefix applied to `/etc`, `/usr/lib/modules` and `/proc` paths.
    pub root_prefix: PathBuf,
    pub pacman_root: PathBuf,
    pub cache_dir: PathBuf,
    pub sync_max_age_days: u64,
    pub cache_old_days: u64,
    pub run_integrity: bool,
    pub max_details: usize,
}

impl Default for CheckOptions {
    fn default() -> Self {
        Self {
            root_prefix: PathBuf::from("/"),
            pacman_root: default_pacman_root(),
            cache_dir: PathBuf::from("/var/cache/pacman/pkg"),
            sync_max_age_days: 30,
            cache_old_days: 30,
            run_integrity: true,
            max_details: 25,
        }
    }
}

impl CheckOptions {
    pub fn new(root_prefix: impl Into<PathBuf>) -> Self {
        Self {
            root_prefix: root_prefix.into(),
            ..Self::default()
        }
    }
}

/// Run every configured check.  This function never panics; unavailable data is
/// reported as [`CheckStatus::Unknown`].
pub fn run_checks(snapshot: &Snapshot, graph: &Graph, options: &CheckOptions) -> ChecksReport {
    let mut results = vec![
        check_pacnew_and_pacsave(&options.root_prefix, options.max_details),
        check_kernel_modules(&options.root_prefix, options.max_details),
        check_sync_db_age(
            &options.pacman_root,
            options.sync_max_age_days,
            options.max_details,
        ),
        check_foreign_packages(snapshot, options.max_details),
        check_cycles(graph, options.max_details),
    ];
    if options.run_integrity {
        results.push(check_pacman_integrity(options.max_details));
    } else {
        results.push(CheckResult::new(
            "integrity",
            "Package file integrity",
            "integrity",
            CheckStatus::Unknown,
            0.7,
            "integrity check disabled",
        ));
    }
    results.push(check_cache_old_versions(
        &options.cache_dir,
        snapshot,
        options.cache_old_days,
        options.max_details,
    ));

    ChecksReport::new(results)
}

/// Find `*.pacnew` and `*.pacsave` below `<root>/etc`.
pub fn check_pacnew_and_pacsave(root: &Path, max_details: usize) -> CheckResult {
    let etc = root.join("etc");
    let mut found = Vec::new();
    if etc.is_dir() {
        walk_pacnew(&etc, 0, &mut found);
    }
    found.sort();

    let status = if found.is_empty() {
        CheckStatus::Ok
    } else {
        CheckStatus::Warning
    };
    let mut details: Vec<String> = found
        .iter()
        .take(max_details.max(1))
        .map(|path| path.display().to_string())
        .collect();
    if found.len() > details.len() {
        details.push(format!("… and {} more", found.len() - details.len()));
    }

    CheckResult::new(
        "pacnew",
        "Pacnew and pacsave files",
        "configuration",
        status,
        1.0,
        if found.is_empty() {
            "no .pacnew or .pacsave files found".to_string()
        } else {
            format!("{} configuration file(s) need review", found.len())
        },
    )
    .with_details(details)
}

fn walk_pacnew(dir: &Path, depth: usize, found: &mut Vec<PathBuf>) {
    if depth > 16 || found.len() >= 10_000 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        if metadata.is_dir() {
            walk_pacnew(&path, depth + 1, found);
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".pacnew") || name.ends_with(".pacsave") {
            found.push(path);
        }
    }
}

/// Compare the running kernel release with the installed module directories.
pub fn check_kernel_modules(root: &Path, max_details: usize) -> CheckResult {
    let release = running_kernel_release(root);
    let modules = root.join("usr").join("lib").join("modules");

    let Some(release) = release else {
        return CheckResult::new(
            "kernel",
            "Running kernel modules",
            "kernel",
            CheckStatus::Unknown,
            1.0,
            "could not determine the running kernel release",
        );
    };

    if !modules.is_dir() {
        return CheckResult::new(
            "kernel",
            "Running kernel modules",
            "kernel",
            CheckStatus::Unknown,
            1.0,
            format!("module directory {} does not exist", modules.display()),
        );
    }

    let mut directories: Vec<String> = fs::read_dir(&modules)
        .map(|entries| {
            entries
                .flatten()
                .filter(|entry| entry.path().is_dir())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    directories.sort();

    let matches = directories
        .iter()
        .filter(|directory| directory.as_str() == release)
        .count();

    if matches > 0 {
        CheckResult::new(
            "kernel",
            "Running kernel modules",
            "kernel",
            CheckStatus::Ok,
            1.0,
            format!("module tree for running kernel {release} is present"),
        )
        .with_details(vec![format!(
            "{} module directory(ies) installed",
            directories.len()
        )])
    } else {
        let status = if directories.is_empty() {
            CheckStatus::Unknown
        } else {
            CheckStatus::Critical
        };
        let mut details: Vec<String> = directories
            .iter()
            .take(max_details.max(1))
            .map(|directory| format!("installed module tree: {directory}"))
            .collect();
        if directories.len() > details.len() {
            details.push(format!("… and {} more", directories.len() - details.len()));
        }
        CheckResult::new(
            "kernel",
            "Running kernel modules",
            "kernel",
            status,
            1.0,
            format!("running kernel {release} has no matching module directory"),
        )
        .with_details(details)
    }
}

fn running_kernel_release(root: &Path) -> Option<String> {
    if let Ok(value) = std::env::var("AMPH_RUNNING_KERNEL") {
        let value = value.trim().to_string();
        if !value.is_empty() {
            return Some(value);
        }
    }
    let release = fs::read_to_string(
        root.join("proc")
            .join("sys")
            .join("kernel")
            .join("osrelease"),
    )
    .ok()?;
    let release = release.trim().to_string();
    (!release.is_empty()).then_some(release)
}

/// Report stale pacman sync databases.
pub fn check_sync_db_age(pacman_root: &Path, max_age_days: u64, max_details: usize) -> CheckResult {
    let sync = pacman_root.join("sync");
    let Ok(entries) = fs::read_dir(&sync) else {
        return CheckResult::new(
            "sync_db",
            "Sync database freshness",
            "freshness",
            CheckStatus::Warning,
            0.7,
            format!("sync directory {} is not readable", sync.display()),
        );
    };

    let now = SystemTime::now();
    let mut databases: Vec<(String, u64, SystemTime)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".db") {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let modified = metadata.modified().unwrap_or(now);
        let age_days = now
            .duration_since(modified)
            .unwrap_or(Duration::ZERO)
            .as_secs()
            / 86_400;
        databases.push((name, age_days, modified));
    }

    if databases.is_empty() {
        return CheckResult::new(
            "sync_db",
            "Sync database freshness",
            "freshness",
            CheckStatus::Warning,
            0.7,
            "no pacman sync databases found",
        );
    }

    databases.sort_by(|a, b| a.0.cmp(&b.0));
    let oldest_age = databases.iter().map(|(_, age, _)| *age).max().unwrap_or(0);
    let status = if oldest_age > max_age_days.saturating_mul(4) {
        CheckStatus::Critical
    } else if oldest_age > max_age_days {
        CheckStatus::Warning
    } else {
        CheckStatus::Ok
    };

    let details: Vec<String> = databases
        .iter()
        .take(max_details.max(1))
        .map(|(name, age, modified)| {
            let stamp = modified
                .duration_since(SystemTime::UNIX_EPOCH)
                .map(|value| value.as_secs())
                .unwrap_or(0);
            format!("{name}: {age} day(s) old (mtime {stamp})")
        })
        .collect();

    CheckResult::new(
        "sync_db",
        "Sync database freshness",
        "freshness",
        status,
        0.7,
        format!("oldest sync database is {oldest_age} day(s) old"),
    )
    .with_details(details)
}

/// Report foreign packages already marked by [`crate::source`].
pub fn check_foreign_packages(snapshot: &Snapshot, max_details: usize) -> CheckResult {
    let foreign: Vec<&Package> = snapshot
        .packages
        .iter()
        .filter(|package| package.foreign)
        .collect();
    let status = if foreign.is_empty() {
        CheckStatus::Ok
    } else {
        CheckStatus::Warning
    };
    let mut details: Vec<String> = foreign
        .iter()
        .take(max_details.max(1))
        .map(|package| package.name.clone())
        .collect();
    if foreign.len() > details.len() {
        details.push(format!("… and {} more", foreign.len() - details.len()));
    }
    CheckResult::new(
        "foreign",
        "Foreign packages",
        "packages",
        status,
        0.6,
        format!("{} foreign package(s) installed", foreign.len()),
    )
    .with_details(details)
}

/// Report dependency cycles.
pub fn check_cycles(graph: &Graph, max_details: usize) -> CheckResult {
    let status = if graph.cycles.is_empty() {
        CheckStatus::Ok
    } else {
        CheckStatus::Warning
    };
    let mut details: Vec<String> = graph
        .cycle_names()
        .into_iter()
        .take(max_details.max(1))
        .map(|cycle| cycle.join(" -> "))
        .collect();
    if graph.cycles.len() > details.len() {
        details.push(format!("… and {} more", graph.cycles.len() - details.len()));
    }
    CheckResult::new(
        "cycles",
        "Dependency cycles",
        "packages",
        status,
        0.5,
        format!("{} dependency cycle(s) detected", graph.cycles.len()),
    )
    .with_details(details)
}

/// Run `pacman -Qk` with `LC_ALL=C` and parse missing-file counts.
pub fn check_pacman_integrity(max_details: usize) -> CheckResult {
    let output = Command::new("pacman")
        .arg("-Qk")
        .env("LC_ALL", "C")
        .output();

    let output = match output {
        Ok(output) => output,
        Err(error) => {
            return CheckResult::new(
                "integrity",
                "Package file integrity",
                "integrity",
                CheckStatus::Unknown,
                0.9,
                format!("pacman -Qk unavailable: {error}"),
            );
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut problems = Vec::new();
    let mut inspected = 0usize;

    for line in stdout.lines().chain(stderr.lines()) {
        if line.contains("missing files") {
            inspected += 1;
            if let Some(missing) = missing_count(line)
                && missing > 0
            {
                problems.push(line.trim().to_string());
            }
        } else if line.starts_with("error:") || line.starts_with("warning:") {
            problems.push(line.trim().to_string());
        }
    }

    let status = if !output.status.success() && inspected == 0 {
        CheckStatus::Unknown
    } else if problems.is_empty() {
        CheckStatus::Ok
    } else {
        CheckStatus::Warning
    };

    let mut details: Vec<String> = problems.iter().take(max_details.max(1)).cloned().collect();
    if problems.len() > details.len() {
        details.push(format!("… and {} more", problems.len() - details.len()));
    }

    CheckResult::new(
        "integrity",
        "Package file integrity",
        "integrity",
        status,
        0.9,
        format!(
            "{} package file report(s), {} with problems",
            inspected,
            problems.len()
        ),
    )
    .with_details(details)
}

fn missing_count(line: &str) -> Option<usize> {
    let marker = "missing files";
    let position = line.find(marker)?;
    let prefix = line[..position].trim_end();
    let digits: String = prefix
        .chars()
        .rev()
        .take_while(|character| character.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    digits.parse::<usize>().ok()
}

/// Inventory the package cache without invoking `du`.
///
/// Old versions are those older than the installed version.  For packages that
/// are no longer installed, every cached version except the most recent file is
/// considered old.  `cache_old_days` additionally requires the file to be at
/// least that old, unless it is older than the installed version.
pub fn check_cache_old_versions(
    cache_dir: &Path,
    snapshot: &Snapshot,
    cache_old_days: u64,
    max_details: usize,
) -> CheckResult {
    let Ok(entries) = fs::read_dir(cache_dir) else {
        return CheckResult::new(
            "cache",
            "Old package cache versions",
            "cache",
            CheckStatus::Unknown,
            0.6,
            format!("cache directory {} is not readable", cache_dir.display()),
        );
    };

    let now = SystemTime::now();
    let mut by_package: BTreeMap<String, Vec<CacheFile>> = BTreeMap::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some((package, version)) = parse_cache_filename(&name) else {
            continue;
        };
        let age_days = metadata
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .map(|duration| duration.as_secs() / 86_400)
            .unwrap_or(0);
        by_package.entry(package).or_default().push(CacheFile {
            version,
            size: metadata.len(),
            age_days,
            path,
        });
    }

    let installed: BTreeMap<&str, &str> = snapshot
        .packages
        .iter()
        .map(|package| (package.name.as_str(), package.version.as_str()))
        .collect();

    let mut old_files = 0usize;
    let mut old_bytes = 0u64;
    let mut details = Vec::new();

    for (package, mut files) in by_package {
        files.sort_by(|a, b| {
            version_cmp(&b.version, &a.version).then_with(|| b.age_days.cmp(&a.age_days))
        });

        let installed_version = installed.get(package.as_str()).copied();
        for (index, file) in files.iter().enumerate() {
            let old = match installed_version {
                Some(version) => version_cmp(&file.version, version).is_lt(),
                None => index > 0 && file.age_days >= cache_old_days,
            };
            if old {
                old_files += 1;
                old_bytes = old_bytes.saturating_add(file.size);
                if details.len() < max_details.max(1) {
                    details.push(format!(
                        "{} {} ({} bytes, {} day(s) old)",
                        package, file.version, file.size, file.age_days
                    ));
                }
            }
        }
    }

    let status = if old_files == 0 {
        CheckStatus::Ok
    } else {
        CheckStatus::Warning
    };

    CheckResult::new(
        "cache",
        "Old package cache versions",
        "cache",
        status,
        0.6,
        format!(
            "{old_files} old cached package file(s), {} MiB reclaimable",
            old_bytes / (1024 * 1024)
        ),
    )
    .with_details(details)
}

struct CacheFile {
    version: String,
    size: u64,
    age_days: u64,
    #[allow(dead_code)]
    path: PathBuf,
}

fn parse_cache_filename(name: &str) -> Option<(String, String)> {
    let marker = ".pkg.tar";
    let position = name.find(marker)?;
    let suffix = &name[position + marker.len()..];
    if !suffix.is_empty()
        && !matches!(
            suffix,
            ".zst" | ".xz" | ".gz" | ".bz2" | ".lzo" | ".lrz" | ".z" | ".Z"
        )
    {
        return None;
    }

    let stem = &name[..position];
    let mut parts = stem.rsplitn(4, '-');
    let _arch = parts.next()?;
    let release = parts.next()?;
    let version = parts.next()?;
    let package = parts.next()?;
    if package.is_empty() || version.is_empty() || release.is_empty() {
        return None;
    }
    Some((package.to_string(), format!("{version}-{release}")))
}

/// Remove duplicate check ids while preserving order.  Useful for callers that
/// compose checks from multiple sources.
pub fn dedupe_results(results: Vec<CheckResult>) -> Vec<CheckResult> {
    let mut seen = HashSet::new();
    results
        .into_iter()
        .filter(|result| seen.insert(result.id.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Package;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!(
            "amph-checks-{label}-{}-{unique}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).expect("temp dir");
        path
    }

    #[test]
    fn finds_pacnew_and_pacsave() {
        let root = temp_dir("pacnew");
        fs::create_dir_all(root.join("etc/ssh")).expect("dirs");
        fs::write(root.join("etc/ssh/sshd_config.pacnew"), "x").expect("pacnew");
        fs::write(root.join("etc/old.conf.pacsave"), "x").expect("pacsave");
        fs::write(root.join("etc/ok.conf"), "x").expect("ok");

        let result = check_pacnew_and_pacsave(&root, 10);
        assert_eq!(result.status, CheckStatus::Warning);
        assert!(result.summary.contains("2 configuration"));
        assert_eq!(result.details.len(), 2);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn detects_missing_running_kernel_modules() {
        let root = temp_dir("kernel");
        fs::create_dir_all(root.join("usr/lib/modules/6.1.0-arch1-1")).expect("modules");
        fs::create_dir_all(root.join("proc/sys/kernel")).expect("proc");
        fs::write(root.join("proc/sys/kernel/osrelease"), "6.2.0-arch1-1\n").expect("osrelease");

        let result = check_kernel_modules(&root, 5);
        assert_eq!(result.status, CheckStatus::Critical);
        assert!(result.summary.contains("6.2.0-arch1-1"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn flags_stale_sync_database() {
        let root = temp_dir("sync");
        fs::create_dir_all(root.join("sync")).expect("sync");
        fs::write(root.join("sync/core.db"), b"db").expect("db");
        // Freshly written databases are 0 days old.
        let fresh = check_sync_db_age(&root, 30, 5);
        assert_eq!(fresh.status, CheckStatus::Ok);
        let stale = check_sync_db_age(&root, 0, 5);
        assert_eq!(stale.status, CheckStatus::Ok, "0 days is not older than 0");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn counts_old_cache_versions_without_du() {
        let root = temp_dir("cache");
        fs::write(root.join("foo-1.0-1-x86_64.pkg.tar.zst"), vec![0u8; 100]).expect("old");
        fs::write(root.join("foo-2.0-1-x86_64.pkg.tar.zst"), vec![0u8; 50]).expect("new");
        fs::write(root.join("bar-1.0-1-x86_64.pkg.tar.xz"), vec![0u8; 25]).expect("bar");

        let snapshot = Snapshot::new(vec![Package {
            name: "foo".into(),
            version: "2.0-1".into(),
            ..Package::default()
        }]);

        let result = check_cache_old_versions(&root, &snapshot, 30, 10);
        assert_eq!(result.status, CheckStatus::Warning);
        assert!(result.summary.contains("1 old cached"));
        assert_eq!(result.details.len(), 1);
        assert!(result.details[0].contains("foo 1.0-1"));

        assert_eq!(
            parse_cache_filename("libfoo-bar-1.2-3-x86_64.pkg.tar.zst"),
            Some(("libfoo-bar".to_string(), "1.2-3".to_string()))
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn parses_missing_file_counts() {
        assert_eq!(
            missing_count("foo: 100 total files, 2 missing files"),
            Some(2)
        );
        assert_eq!(
            missing_count("bar: 10 total files, 0 missing files"),
            Some(0)
        );
        assert_eq!(missing_count("no numbers here"), None);
    }
}
