//! Filesystem readers for pacman's local and sync databases.
//!
//! The local database is parsed in parallel with Rayon; parallel iterator
//! collection preserves the deterministic order established by the path sort.
//!
//! No pacman subprocesses are spawned here and `pacman -Qi` is never used.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use anyhow::{Context, Result};
use flate2::read::GzDecoder;
use rayon::prelude::*;

use crate::model::{Package, Snapshot, parse_sections, unix_now};

/// Phase reported by [`Progress`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressPhase {
    LocalPackages,
    SyncDatabase,
    Foreign,
}

/// One progress event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProgressEvent {
    pub phase: ProgressPhase,
    pub done: usize,
    pub total: usize,
}

impl ProgressEvent {
    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            1.0
        } else {
            (self.done as f32 / self.total as f32).clamp(0.0, 1.0)
        }
    }
}

type ProgressCallback = Arc<dyn Fn(ProgressEvent) + Send + Sync>;

/// Cheap, cloneable progress sink.  Construct with [`Progress::new`].
#[derive(Clone, Default)]
pub struct Progress {
    callback: Option<ProgressCallback>,
}

impl Progress {
    pub fn new<F>(callback: F) -> Self
    where
        F: Fn(ProgressEvent) + Send + Sync + 'static,
    {
        Self {
            callback: Some(Arc::new(callback)),
        }
    }

    pub fn none() -> Self {
        Self::default()
    }

    pub fn emit(&self, phase: ProgressPhase, done: usize, total: usize) {
        if let Some(callback) = &self.callback {
            callback(ProgressEvent { phase, done, total });
        }
    }
}

/// Detailed result from a scan, including non-fatal warnings.
#[derive(Debug, Clone)]
pub struct LoadResult {
    pub snapshot: Snapshot,
    pub warnings: Vec<String>,
}

/// Default pacman database root.  `AMPH_PACMAN_ROOT` is useful for tests and
/// cross-root inspections.
pub fn default_pacman_root() -> PathBuf {
    std::env::var_os("AMPH_PACMAN_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/var/lib/pacman"))
}

/// Load a snapshot from the default pacman root.
pub fn load_local_snapshot(progress: Option<&Progress>) -> Result<Snapshot> {
    load_snapshot(&default_pacman_root(), progress)
}

/// Backwards-friendly alias for [`load_local_snapshot`].
pub fn collect_snapshot(progress: Option<&Progress>) -> Result<Snapshot> {
    load_local_snapshot(progress)
}

/// Load and index the installed packages from `root/local/*/desc`.
pub fn load_snapshot(root: &Path, progress: Option<&Progress>) -> Result<Snapshot> {
    Ok(load_snapshot_detailed(root, progress)?.snapshot)
}

/// Load a snapshot while retaining parse warnings.
pub fn load_snapshot_detailed(root: &Path, progress: Option<&Progress>) -> Result<LoadResult> {
    let local_dir = root.join("local");
    let sync_dir = root.join("sync");

    let mut warnings = Vec::new();
    let local_entries = collect_desc_entries(&local_dir, &mut warnings)?;
    let total = local_entries.len();

    if let Some(progress) = progress {
        progress.emit(ProgressPhase::LocalPackages, 0, total);
    }

    let parsed = parallel_map(
        local_entries,
        |(path, fallback_name)| {
            let text = match fs::read_to_string(&path) {
                Ok(text) => text,
                Err(error) => {
                    return Err(format!("{}: {error}", path.display()));
                }
            };
            Package::from_desc(Some(&fallback_name), &text)
                .map_err(|error| format!("{}: {error}", path.display()))
        },
        progress,
        ProgressPhase::LocalPackages,
    );

    let mut packages: Vec<Package> = Vec::with_capacity(total);
    for result in parsed {
        match result {
            Ok(package) => packages.push(package),
            Err(warning) => warnings.push(warning),
        }
    }

    let sync_names = match load_sync_package_names(&sync_dir, progress) {
        Ok(names) => names,
        Err(error) => {
            warnings.push(format!("could not read sync databases: {error}"));
            BTreeSet::new()
        }
    };

    apply_foreign(&mut packages, &sync_names, progress);

    let mut snapshot = Snapshot {
        packages,
        scanned_at: unix_now(),
    };
    snapshot.normalize();

    Ok(LoadResult { snapshot, warnings })
}

fn collect_desc_entries(
    local_dir: &Path,
    warnings: &mut Vec<String>,
) -> Result<Vec<(PathBuf, String)>> {
    let entries = fs::read_dir(local_dir)
        .with_context(|| format!("cannot read local package database {}", local_dir.display()))?;

    let mut paths: Vec<(PathBuf, String)> = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                warnings.push(format!("local database entry: {error}"));
                continue;
            }
        };
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let fallback_name = entry.file_name().to_string_lossy().into_owned();
        let desc = path.join("desc");
        if desc.is_file() {
            paths.push((desc, fallback_name));
        }
    }
    paths.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(paths)
}

/// Read all package names from sync databases.
///
/// Both extracted sync directories (`sync/core/desc`, `sync/core.db/desc`) and
/// regular pacman `.db` tar archives are supported.  The tar reader is a small
/// purpose-built implementation: it only reads regular file entries whose name
/// ends in `desc`, which is exactly what the sync database contains.
pub fn load_sync_package_names(
    sync_dir: &Path,
    progress: Option<&Progress>,
) -> Result<BTreeSet<String>> {
    if !sync_dir.exists() {
        return Ok(BTreeSet::new());
    }

    let entries = fs::read_dir(sync_dir)
        .with_context(|| format!("cannot read sync database directory {}", sync_dir.display()))?;
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = with_context_entry(entry, "sync database entry")?;
        paths.push(entry.path());
    }
    paths.sort();

    let total = paths.len();
    if let Some(progress) = progress {
        progress.emit(ProgressPhase::SyncDatabase, 0, total);
    }

    let mut names = BTreeSet::new();
    let mut done = 0usize;
    for path in paths {
        let is_files_database = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.ends_with(".files"))
            .unwrap_or(false);
        if !is_files_database {
            collect_sync_names_from_path(&path, &mut names);
        }
        done += 1;
        if let Some(progress) = progress {
            progress.emit(ProgressPhase::SyncDatabase, done, total.max(1));
        }
    }

    Ok(names)
}

fn with_context_entry(entry: std::io::Result<fs::DirEntry>, context: &str) -> Result<fs::DirEntry> {
    entry.with_context(|| context.to_string())
}

fn collect_sync_names_from_path(path: &Path, names: &mut BTreeSet<String>) {
    if path.is_dir() {
        for desc in find_desc_files(path, 0) {
            if let Ok(text) = fs::read_to_string(&desc)
                && let Some(name) = name_from_desc(&text)
            {
                names.insert(name);
            }
        }
        return;
    }

    if path.extension().and_then(|ext| ext.to_str()) != Some("db") {
        return;
    }

    if let Ok(data) = fs::read(path)
        && let Ok(uncompressed) = decode_sync_db(&data)
    {
        for desc in tar_desc_entries(&uncompressed) {
            if let Some(name) = name_from_desc(&desc) {
                names.insert(name);
            }
        }
    }
}

/// Decode the archive formats supported for pacman sync databases. Pacman
/// commonly stores `.db` files as gzip-compressed tar archives; zstd is also
/// supported when selected in `pacman.conf`. Uncompressed tar data is accepted
/// as well (useful for fixtures and extracted database snapshots).
fn decode_sync_db(data: &[u8]) -> io::Result<Vec<u8>> {
    if data.starts_with(&[0x1f, 0x8b]) {
        let mut decoded = Vec::new();
        GzDecoder::new(data).read_to_end(&mut decoded)?;
        return Ok(decoded);
    }
    if data.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        return zstd::stream::decode_all(data);
    }
    Ok(data.to_vec())
}

#[cfg(test)]
fn gzip(data: &[u8]) -> Vec<u8> {
    use flate2::Compression;
    use flate2::write::GzEncoder;
    use std::io::Write;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(data).expect("compress test tar");
    encoder.finish().expect("finish compressed test tar")
}

#[cfg(test)]
fn zstd_compress(data: &[u8]) -> Vec<u8> {
    zstd::stream::encode_all(data, 1).expect("compress test tar")
}

fn find_desc_files(dir: &Path, depth: usize) -> Vec<PathBuf> {
    if depth > 2 {
        return Vec::new();
    }
    let mut out = Vec::new();
    let direct = dir.join("desc");
    if direct.is_file() {
        out.push(direct);
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(find_desc_files(&path, depth + 1));
        }
    }
    out
}

fn name_from_desc(text: &str) -> Option<String> {
    parse_sections(text)
        .get("NAME")
        .and_then(|values| values.first())
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
}

fn tar_desc_entries(data: &[u8]) -> Vec<String> {
    let mut descriptions = Vec::new();
    let mut offset = 0usize;

    while offset.saturating_add(512) <= data.len() {
        let header = &data[offset..offset + 512];
        if header.iter().all(|byte| *byte == 0) {
            break;
        }

        let name = tar_string(&header[0..100]);
        let prefix = tar_string(&header[345..500]);
        let full_name = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        let typeflag = header[156];
        let size = tar_octal(&header[124..136]).unwrap_or(0);
        let data_start = offset.saturating_add(512);
        let Some(data_end) = data_start.checked_add(size) else {
            break;
        };
        if data_end > data.len() {
            break;
        }

        let is_regular = typeflag == 0 || typeflag == b'0';
        if is_regular
            && full_name.ends_with("desc")
            && let Ok(text) = std::str::from_utf8(&data[data_start..data_end])
        {
            descriptions.push(text.to_string());
        }

        let padded = size.saturating_add(511) / 512 * 512;
        let Some(next) = data_start.checked_add(padded) else {
            break;
        };
        if next <= offset {
            break;
        }
        offset = next;
    }

    descriptions
}

fn tar_string(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_string()
}

fn tar_octal(bytes: &[u8]) -> Option<usize> {
    if bytes.first().is_some_and(|byte| byte & 0x80 != 0) {
        // GNU base-256 encoding.  Sync database entries are small, but handling
        // it keeps the reader from silently accepting a bogus size.
        let mut value: u128 = 0;
        for byte in bytes {
            value = (value << 8) | u128::from(*byte & 0x7f);
        }
        return usize::try_from(value).ok();
    }
    let text = tar_string(bytes);
    if text.is_empty() {
        return Some(0);
    }
    usize::from_str_radix(&text, 8).ok()
}

/// Mark packages absent from all sync databases as foreign.  An empty sync set
/// is treated as "unknown" rather than marking the entire system foreign.
pub fn apply_foreign(
    packages: &mut [Package],
    sync_names: &BTreeSet<String>,
    progress: Option<&Progress>,
) {
    let known = !sync_names.is_empty();
    let total = packages.len();
    for (index, package) in packages.iter_mut().enumerate() {
        package.foreign = known && !sync_names.contains(&package.name);
        if let Some(progress) = progress {
            progress.emit(ProgressPhase::Foreign, index + 1, total);
        }
    }
}

/// Explicitly determine foreign packages without a pacman subprocess.
pub fn determine_foreign(root: &Path) -> Result<BTreeSet<String>> {
    load_sync_package_names(&root.join("sync"), None)
}

/// Inventory entries `(path, fallback_name)` from the local database.
pub fn local_desc_paths(root: &Path) -> Result<Vec<PathBuf>> {
    let mut warnings = Vec::new();
    let entries = collect_desc_entries(&root.join("local"), &mut warnings)?;
    Ok(entries.into_iter().map(|(path, _)| path).collect())
}

fn parallel_map<T, R, F>(
    items: Vec<T>,
    work: F,
    progress: Option<&Progress>,
    phase: ProgressPhase,
) -> Vec<R>
where
    T: Send,
    R: Send,
    F: Fn(T) -> R + Sync + Send,
{
    let total = items.len();
    if total == 0 {
        return Vec::new();
    }

    let completed = AtomicUsize::new(0);
    items
        .into_par_iter()
        .map(|item| {
            let value = work(item);
            let done = completed.fetch_add(1, Ordering::Relaxed) + 1;
            if let Some(progress) = progress {
                progress.emit(phase, done, total);
            }
            value
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::time::{SystemTime, UNIX_EPOCH};

    const APP_DESC: &str = include_str!("../tests/fixtures/desc/app/desc");
    const BAR_DESC: &str = include_str!("../tests/fixtures/desc/bar/desc");

    struct TempRoot {
        path: PathBuf,
    }

    impl TempRoot {
        fn new(label: &str) -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|value| value.as_nanos())
                .unwrap_or(0);
            let path = std::env::temp_dir().join(format!(
                "amph-source-test-{label}-{}-{unique}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("create temp root");
            Self { path }
        }

        fn write(&self, relative: &str, contents: &str) {
            let path = self.path.join(relative);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create parent");
            }
            fs::write(path, contents).expect("write fixture");
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn loads_local_desc_and_marks_foreign_against_sync() {
        let root = TempRoot::new("load");
        root.write("local/app-1.0-1/desc", APP_DESC);
        root.write("local/bar-2.0-1/desc", BAR_DESC);
        root.write("local/broken-1.0-1/desc", "%VERSION%\n1.0\n");
        fs::create_dir_all(root.path.join("local/invalid-1.0-1")).expect("invalid dir");
        fs::write(
            root.path.join("local/invalid-1.0-1/desc"),
            [0xff, 0xfe, 0x00],
        )
        .expect("invalid utf8");
        root.write("local/ignored/notes.txt", "not a desc");
        root.write("sync/core/desc", "%NAME%\nbar\n\n%VERSION%\n2.0-1\n");

        let ticks = Arc::new(AtomicUsize::new(0));
        let tick_counter = Arc::clone(&ticks);
        let progress = Progress::new(move |_| {
            tick_counter.fetch_add(1, Ordering::Relaxed);
        });

        let result =
            load_snapshot_detailed(&root.path, Some(&progress)).expect("snapshot should load");

        assert_eq!(result.snapshot.packages.len(), 3);
        assert!(
            result
                .warnings
                .iter()
                .any(|warning| warning.contains("invalid-1.0-1"))
        );
        assert!(
            result.snapshot.find("broken-1.0-1").is_some(),
            "directory name fallback"
        );
        let app = result.snapshot.find("app").expect("app");
        assert!(app.foreign);
        let bar = result.snapshot.find("bar").expect("bar");
        assert!(!bar.foreign);
        assert!(ticks.load(Ordering::Relaxed) > 0);
    }

    #[test]
    fn reads_sync_names_from_tar_database() {
        let root = TempRoot::new("tar");
        root.write("local/app-1.0-1/desc", APP_DESC);

        let desc = "%NAME%\napp\n\n%VERSION%\n1.0-1\n";
        let archive = build_tar(&[("app-1.0-1/desc", desc)]);
        let sync = root.path.join("sync");
        fs::create_dir_all(&sync).expect("sync dir");
        fs::write(sync.join("core.db"), gzip(&archive)).expect("write gzip database");
        fs::write(sync.join("extra.db"), zstd_compress(&archive)).expect("write zstd database");
        fs::write(sync.join("multilib.db"), archive).expect("write uncompressed database");

        let names = load_sync_package_names(&sync, None).expect("sync names");
        assert!(names.contains("app"));
    }

    #[test]
    fn empty_sync_database_does_not_mark_everything_foreign() {
        let root = TempRoot::new("empty-sync");
        root.write("local/app-1.0-1/desc", APP_DESC);
        let snapshot = load_snapshot(&root.path, None).expect("snapshot");
        assert!(!snapshot.find("app").expect("app").foreign);
    }

    fn build_tar(entries: &[(&str, &str)]) -> Vec<u8> {
        let mut out = Vec::new();
        for (name, contents) in entries {
            let mut header = [0u8; 512];
            header[..name.len()].copy_from_slice(name.as_bytes());
            let size = format!("{:011o}", contents.len());
            header[124..124 + size.len()].copy_from_slice(size.as_bytes());
            header[156] = b'0';
            out.extend_from_slice(&header);
            out.extend_from_slice(contents.as_bytes());
            let padding = (512 - (contents.len() % 512)) % 512;
            out.extend(std::iter::repeat_n(0u8, padding));
        }
        out.extend_from_slice(&[0u8; 1024]);
        out
    }
}
