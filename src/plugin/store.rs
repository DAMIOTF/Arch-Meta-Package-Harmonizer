//! Per-user plugin store with atomic install/remove and GitHub update checks.
//!
//! Plugins are directories under `$XDG_DATA_HOME/amph/plugins/<id>` (falling
//! back to `~/.local/share/amph/plugins`). A directory contains:
//!
//! - `plugin.json` — the strict manifest;
//! - the manifest's Wasm `entry` for functionality plugins;
//! - `install.json` — provenance recorded by AMPH;
//! - `disabled` — an empty marker file when the plugin is disabled.
//!
//! Installation downloads and verifies every asset before creating a hidden
//! staging directory. Publishing that directory is a single `rename`, so a
//! failed install or update never leaves a partially written plugin. Removal
//! first renames the directory out of the way and then deletes it.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use super::github::{GitHubClient, ReleaseAsset, ReleaseProvider, Repository, Sha256Digest};
use super::manifest::{MAX_MANIFEST_BYTES, PluginKind, PluginManifest, validate_id};
use super::wasm::{MAX_MODULE_BYTES, validate_module};

/// Manifest file name inside a plugin directory.
pub const MANIFEST_FILE: &str = "plugin.json";
/// Install provenance file name inside a plugin directory.
pub const RECORD_FILE: &str = "install.json";
/// Marker file name that disables a plugin.
pub const DISABLED_FILE: &str = "disabled";
/// Version of the install provenance record.
pub const RECORD_SCHEMA: u32 = 1;

const MAX_RECORD_BYTES: usize = 16 * 1024;

static STAGING_COUNTER: AtomicU64 = AtomicU64::new(0);

/// An installed plugin and its local state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledPlugin {
    pub manifest: PluginManifest,
    pub path: PathBuf,
    pub enabled: bool,
    pub install: Option<InstallRecord>,
}

/// Provenance for a plugin installed from a GitHub release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstallRecord {
    pub schema: u32,
    pub id: String,
    /// `owner/repo`.
    pub repository: String,
    /// GitHub release tag that was installed.
    pub release_tag: String,
    /// Unix timestamp of installation.
    pub installed_at: u64,
    /// Lowercase SHA-256 of `plugin.json`.
    pub manifest_sha256: String,
    /// Lowercase SHA-256 of the Wasm entry, when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wasm_sha256: Option<String>,
}

/// Result of checking one plugin against its latest GitHub release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginUpdate {
    pub id: String,
    pub current_version: String,
    pub current_tag: String,
    pub latest_tag: String,
    pub update_available: bool,
}

/// Result of checking every installed plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckOutcome {
    UpToDate(PluginUpdate),
    UpdateAvailable(PluginUpdate),
    NotTracked { id: String },
    Error { id: String, message: String },
}

/// Result of updating every installed plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateOutcome {
    Updated {
        id: String,
        from_tag: String,
        to_tag: String,
    },
    UpToDate {
        id: String,
    },
    Skipped {
        id: String,
        reason: String,
    },
    Failed {
        id: String,
        message: String,
    },
}

/// A per-user plugin store rooted at one directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginStore {
    root: PathBuf,
}

impl PluginStore {
    /// Create a store rooted at an explicit directory.
    ///
    /// This is useful for tests and for hosts that own their data directory.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Use the per-user XDG data directory.
    pub fn discover() -> Result<Self> {
        let root = default_root().ok_or_else(|| {
            anyhow::anyhow!("cannot locate the per-user data directory; set XDG_DATA_HOME or HOME")
        })?;
        Ok(Self::new(root))
    }

    /// Store root (`.../amph/plugins`).
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// List valid installed plugins, sorted by id.
    ///
    /// Directories without a `plugin.json` and hidden staging/backup
    /// directories are ignored. A directory that contains a manifest but fails
    /// strict validation is an error so corrupt state is not silently hidden.
    pub fn list(&self) -> Result<Vec<InstalledPlugin>> {
        let entries = match fs::read_dir(&self.root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("cannot read plugin store {}", self.root.display()));
            }
        };

        let mut plugins = Vec::new();
        for entry in entries {
            let entry = entry.with_context(|| {
                format!(
                    "cannot read an entry in plugin store {}",
                    self.root.display()
                )
            })?;
            let file_type = entry.file_type().with_context(|| {
                format!("cannot inspect plugin entry {}", entry.path().display())
            })?;
            if !file_type.is_dir() {
                continue;
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            if name.starts_with('.') || !entry.path().join(MANIFEST_FILE).is_file() {
                continue;
            }
            plugins.push(self.get(name)?);
        }
        plugins.sort_by(|left, right| left.manifest.id.cmp(&right.manifest.id));
        Ok(plugins)
    }

    /// Load one installed plugin.
    pub fn get(&self, id: &str) -> Result<InstalledPlugin> {
        validate_id(id)?;
        let path = self.root.join(id);
        let metadata = fs::symlink_metadata(&path)
            .with_context(|| format!("plugin `{id}` is not installed"))?;
        if metadata.file_type().is_symlink() {
            bail!("plugin `{id}` is a symbolic link; refusing to use it");
        }
        if !metadata.is_dir() {
            bail!("plugin `{id}` is not a directory");
        }

        let manifest_path = path.join(MANIFEST_FILE);
        if !optional_regular_file(&manifest_path, "manifest")? {
            bail!("plugin `{id}` is missing its manifest");
        }
        let manifest_bytes = read_bounded(&manifest_path, MAX_MANIFEST_BYTES)?;
        let manifest = PluginManifest::parse(&manifest_bytes)
            .with_context(|| format!("invalid plugin manifest {}", manifest_path.display()))?;
        if manifest.id != id {
            bail!(
                "plugin manifest id `{}` does not match its directory `{id}`",
                manifest.id
            );
        }

        if manifest.is_functionality() {
            let entry = manifest
                .entry
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("functionality plugin `{id}` has no Wasm entry"))?;
            let entry_path = path.join(entry);
            if !optional_regular_file(&entry_path, "Wasm entry")? {
                bail!("plugin `{id}` is missing its Wasm entry `{entry}`");
            }
        }

        let record_path = path.join(RECORD_FILE);
        let install = if optional_regular_file(&record_path, "install record")? {
            let bytes = read_bounded(&record_path, MAX_RECORD_BYTES)?;
            let record: InstallRecord = serde_json::from_slice(&bytes)
                .with_context(|| format!("cannot parse {}", record_path.display()))?;
            validate_record(&record, id)?;
            Some(record)
        } else {
            None
        };

        let disabled_path = path.join(DISABLED_FILE);
        let enabled = !optional_regular_file(&disabled_path, "disabled marker")?;
        Ok(InstalledPlugin {
            manifest,
            path,
            enabled,
            install,
        })
    }

    /// Run a functionality plugin's stateless JSON analyzer.
    ///
    /// The installed plugin is reloaded with [`PluginStore::get`], so disabled
    /// plugins, symlinked files, and malformed manifests are rejected by the
    /// normal store checks. Theme plugins have no executable analyzer and are
    /// rejected as well. The manifest entry is loaded with the bounded store
    /// reader, then [`super::wasm::analyze_json`] validates and executes it with
    /// the default runtime limits. Output is returned exactly as parsed; the
    /// Wasm layer rejects missing, undersized, oversized, out-of-bounds, or
    /// invalid-JSON responses.
    pub fn analyze_json(&self, id: &str, request: &serde_json::Value) -> Result<serde_json::Value> {
        let plugin = self.get(id)?;
        if !plugin.enabled {
            bail!("plugin `{id}` is disabled");
        }
        if plugin.manifest.is_theme() {
            bail!("plugin `{id}` is a theme plugin and has no JSON analyzer");
        }
        let entry = plugin
            .manifest
            .entry
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("functionality plugin `{id}` has no Wasm entry"))?;
        let module_bytes = read_bounded(&plugin.path.join(entry), MAX_MODULE_BYTES)?;
        super::wasm::analyze_json(&module_bytes, request)
    }

    /// Install or update a plugin from exactly `https://github.com/{owner}/{repo}`.
    ///
    /// Only the latest stable GitHub release is accepted. Every selected asset
    /// must have a GitHub-provided SHA-256 digest, which is verified against
    /// the downloaded bytes before anything is published. Functionality Wasm is
    /// validated in the sandbox before installation. An existing disabled state
    /// is preserved across updates.
    pub fn install(&self, url: &str) -> Result<InstalledPlugin> {
        self.install_with(&GitHubClient::new(), url)
    }

    /// Install using an explicit release transport.
    pub fn install_with(
        &self,
        provider: &dyn ReleaseProvider,
        url: &str,
    ) -> Result<InstalledPlugin> {
        let repository = Repository::parse_url(url)?;
        let release = provider.latest_release(&repository)?;

        let manifest_asset = release.asset(MANIFEST_FILE)?;
        if manifest_asset.size > MAX_MANIFEST_BYTES as u64 {
            bail!(
                "plugin manifest asset is too large ({} bytes, limit {MAX_MANIFEST_BYTES})",
                manifest_asset.size
            );
        }
        let manifest_digest = require_digest(manifest_asset)?;
        let manifest_bytes =
            provider.download_asset(&repository, manifest_asset, MAX_MANIFEST_BYTES)?;
        manifest_digest.verify(&manifest_bytes)?;
        let manifest = PluginManifest::parse(&manifest_bytes)?;

        let (wasm_bytes, wasm_digest) = match manifest.kind {
            PluginKind::Functionality => {
                let entry = manifest
                    .entry
                    .as_deref()
                    .expect("validated functionality manifest has an entry");
                let wasm_asset = release.asset(entry)?;
                if wasm_asset.size > MAX_MODULE_BYTES as u64 {
                    bail!(
                        "Wasm module is too large ({} bytes, limit {MAX_MODULE_BYTES})",
                        wasm_asset.size
                    );
                }
                let digest = require_digest(wasm_asset)?;
                let bytes = provider.download_asset(&repository, wasm_asset, MAX_MODULE_BYTES)?;
                digest.verify(&bytes)?;
                validate_module(&bytes)
                    .with_context(|| format!("plugin `{}` failed Wasm validation", manifest.id))?;
                (Some(bytes), Some(digest.as_str().to_string()))
            }
            PluginKind::Theme => (None, None),
        };

        let enabled = self
            .get(&manifest.id)
            .map(|plugin| plugin.enabled)
            .unwrap_or(true);
        let record = InstallRecord {
            schema: RECORD_SCHEMA,
            id: manifest.id.clone(),
            repository: repository.full_name(),
            release_tag: release.tag.clone(),
            installed_at: now_secs(),
            manifest_sha256: manifest_digest.as_str().to_string(),
            wasm_sha256: wasm_digest,
        };
        self.publish(
            &manifest,
            &manifest_bytes,
            wasm_bytes.as_deref(),
            &record,
            enabled,
        )?;
        self.get(&manifest.id)
    }

    /// Check one tracked plugin against the latest GitHub release.
    pub fn check(&self, id: &str) -> Result<PluginUpdate> {
        self.check_with(&GitHubClient::new(), id)
    }

    /// Check one plugin using an explicit release transport.
    pub fn check_with(&self, provider: &dyn ReleaseProvider, id: &str) -> Result<PluginUpdate> {
        let plugin = self.get(id)?;
        let record = plugin.install.as_ref().ok_or_else(|| {
            anyhow::anyhow!("plugin `{id}` has no GitHub install record and cannot be checked")
        })?;
        let repository = Repository::parse_full_name(&record.repository)?;
        let release = provider.latest_release(&repository)?;
        let asset = release.asset(MANIFEST_FILE)?;
        let digest = require_digest(asset)?;
        let wasm_changed = match plugin.manifest.kind {
            PluginKind::Functionality => {
                let entry = plugin.manifest.entry.as_deref().ok_or_else(|| {
                    anyhow::anyhow!("functionality plugin `{id}` has no Wasm entry")
                })?;
                let wasm_asset = release.asset(entry)?;
                let wasm_digest = require_digest(wasm_asset)?;
                record.wasm_sha256.as_deref() != Some(wasm_digest.as_str())
            }
            PluginKind::Theme => false,
        };
        let update_available = release.tag != record.release_tag
            || digest.as_str() != record.manifest_sha256
            || wasm_changed;
        Ok(PluginUpdate {
            id: id.to_string(),
            current_version: plugin.manifest.version,
            current_tag: record.release_tag.clone(),
            latest_tag: release.tag,
            update_available,
        })
    }

    /// Check every installed plugin, reporting per-plugin failures.
    pub fn check_all(&self) -> Result<Vec<CheckOutcome>> {
        self.check_all_with(&GitHubClient::new())
    }

    /// Check every installed plugin using an explicit release transport.
    pub fn check_all_with(&self, provider: &dyn ReleaseProvider) -> Result<Vec<CheckOutcome>> {
        let mut outcomes = Vec::new();
        for plugin in self.list()? {
            let id = plugin.manifest.id.clone();
            if plugin.install.is_none() {
                outcomes.push(CheckOutcome::NotTracked { id });
                continue;
            }
            outcomes.push(match self.check_with(provider, &id) {
                Ok(update) if update.update_available => CheckOutcome::UpdateAvailable(update),
                Ok(update) => CheckOutcome::UpToDate(update),
                Err(error) => CheckOutcome::Error {
                    id,
                    message: error.to_string(),
                },
            });
        }
        Ok(outcomes)
    }

    /// Update every tracked plugin, continuing after individual failures.
    pub fn update_all(&self) -> Result<Vec<UpdateOutcome>> {
        self.update_all_with(&GitHubClient::new())
    }

    /// Update every tracked plugin using an explicit release transport.
    pub fn update_all_with(&self, provider: &dyn ReleaseProvider) -> Result<Vec<UpdateOutcome>> {
        let mut outcomes = Vec::new();
        for plugin in self.list()? {
            let id = plugin.manifest.id.clone();
            let Some(record) = plugin.install else {
                outcomes.push(UpdateOutcome::Skipped {
                    id,
                    reason: "no GitHub install record".to_string(),
                });
                continue;
            };
            let update = match self.check_with(provider, &id) {
                Ok(update) => update,
                Err(error) => {
                    outcomes.push(UpdateOutcome::Failed {
                        id,
                        message: error.to_string(),
                    });
                    continue;
                }
            };
            if !update.update_available {
                outcomes.push(UpdateOutcome::UpToDate { id });
                continue;
            }
            let url = format!("https://github.com/{}", record.repository);
            match self.install_with(provider, &url) {
                Ok(installed) => {
                    let to_tag = installed
                        .install
                        .as_ref()
                        .map(|record| record.release_tag.clone())
                        .unwrap_or(update.latest_tag);
                    outcomes.push(UpdateOutcome::Updated {
                        id,
                        from_tag: record.release_tag,
                        to_tag,
                    });
                }
                Err(error) => outcomes.push(UpdateOutcome::Failed {
                    id,
                    message: error.to_string(),
                }),
            }
        }
        Ok(outcomes)
    }

    /// Enable or disable an installed plugin.
    pub fn set_enabled(&self, id: &str, enabled: bool) -> Result<InstalledPlugin> {
        let plugin = self.get(id)?;
        let marker = plugin.path.join(DISABLED_FILE);
        if enabled {
            match fs::remove_file(&marker) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(error).with_context(|| format!("cannot enable plugin `{id}`"));
                }
            }
        } else {
            write_atomic(&marker, b"")?;
        }
        self.get(id)
    }

    /// Enable an installed plugin.
    pub fn enable(&self, id: &str) -> Result<InstalledPlugin> {
        self.set_enabled(id, true)
    }

    /// Disable an installed plugin.
    pub fn disable(&self, id: &str) -> Result<InstalledPlugin> {
        self.set_enabled(id, false)
    }

    /// Remove an installed plugin.
    ///
    /// The directory is first renamed outside the visible id namespace, then
    /// deleted, so a partial removal cannot leave a usable-looking plugin.
    pub fn remove(&self, id: &str) -> Result<()> {
        self.get(id)?;
        let path = self.root.join(id);
        let trash = self.hidden_path(id, "trash");
        fs::rename(&path, &trash)
            .with_context(|| format!("cannot remove plugin `{id}` from {}", path.display()))?;
        let _ = fs::remove_dir_all(trash);
        Ok(())
    }

    fn publish(
        &self,
        manifest: &PluginManifest,
        manifest_bytes: &[u8],
        wasm_bytes: Option<&[u8]>,
        record: &InstallRecord,
        enabled: bool,
    ) -> Result<()> {
        fs::create_dir_all(&self.root)
            .with_context(|| format!("cannot create plugin store {}", self.root.display()))?;

        let staging = self.hidden_path(&manifest.id, "staging");
        let mut guard = StagingGuard::new(staging.clone());
        fs::create_dir(&staging)
            .with_context(|| format!("cannot create staging directory {}", staging.display()))?;

        fs::write(staging.join(MANIFEST_FILE), manifest_bytes)
            .with_context(|| format!("cannot write {}", staging.join(MANIFEST_FILE).display()))?;
        if let (Some(entry), Some(bytes)) = (manifest.entry.as_deref(), wasm_bytes) {
            fs::write(staging.join(entry), bytes)
                .with_context(|| format!("cannot write {}", staging.join(entry).display()))?;
        }
        let record_bytes =
            serde_json::to_vec_pretty(record).context("cannot serialize the install record")?;
        fs::write(staging.join(RECORD_FILE), record_bytes)
            .with_context(|| format!("cannot write {}", staging.join(RECORD_FILE).display()))?;
        if !enabled {
            fs::write(staging.join(DISABLED_FILE), b"").with_context(|| {
                format!("cannot write {}", staging.join(DISABLED_FILE).display())
            })?;
        }

        let destination = self.root.join(&manifest.id);
        if let Ok(metadata) = fs::symlink_metadata(&destination)
            && metadata.file_type().is_symlink()
        {
            bail!(
                "plugin destination {} is a symbolic link; refusing to replace it",
                destination.display()
            );
        }

        let had_existing = destination.exists();
        let backup = if had_existing {
            let backup = self.hidden_path(&manifest.id, "backup");
            fs::rename(&destination, &backup).with_context(|| {
                format!(
                    "cannot move the existing plugin {} out of the way",
                    destination.display()
                )
            })?;
            Some(backup)
        } else {
            None
        };

        match fs::rename(&staging, &destination) {
            Ok(()) => {
                guard.disarm();
                if let Some(backup) = backup {
                    let _ = fs::remove_dir_all(backup);
                }
                Ok(())
            }
            Err(error) => {
                if let Some(backup) = &backup {
                    let _ = fs::rename(backup, &destination);
                }
                Err(error).with_context(|| {
                    format!(
                        "cannot publish plugin {} to {}",
                        manifest.id,
                        destination.display()
                    )
                })
            }
        }
    }

    fn hidden_path(&self, id: &str, kind: &str) -> PathBuf {
        let sequence = STAGING_COUNTER.fetch_add(1, Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or(0);
        self.root.join(format!(
            ".{id}.{kind}-{}-{sequence}-{nanos}",
            std::process::id()
        ))
    }
}

/// Default per-user plugin directory:
/// `$XDG_DATA_HOME/amph/plugins` or `~/.local/share/amph/plugins`.
pub fn default_root() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_DATA_HOME") {
        let path = PathBuf::from(path);
        if !path.as_os_str().is_empty() {
            return Some(path.join("amph").join("plugins"));
        }
    }
    std::env::var_os("HOME").map(|home| {
        PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("amph")
            .join("plugins")
    })
}

fn require_digest(asset: &ReleaseAsset) -> Result<&Sha256Digest> {
    asset.digest.as_ref().ok_or_else(|| {
        anyhow::anyhow!(
            "GitHub release asset `{}` has no SHA-256 digest; refusing to install",
            asset.name
        )
    })
}
fn read_bounded(path: &Path, limit: usize) -> Result<Vec<u8>> {
    let metadata = fs::metadata(path).with_context(|| format!("cannot read {}", path.display()))?;
    if metadata.len() > limit as u64 {
        bail!("{} exceeds the {limit}-byte limit", path.display());
    }
    fs::read(path).with_context(|| format!("cannot read {}", path.display()))
}

/// Report whether an optional store file is present as a regular file.
///
/// Uses `symlink_metadata` so a symlink is rejected instead of followed. This
/// keeps a tampered plugin directory from redirecting host reads outside the
/// plugin directory.
fn optional_regular_file(path: &Path, label: &str) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!(
                "plugin {label} {} is a symbolic link; refusing to use it",
                path.display()
            );
        }
        Ok(metadata) if metadata.is_file() => Ok(true),
        Ok(_) => bail!("plugin {label} {} is not a regular file", path.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => {
            Err(error).with_context(|| format!("cannot inspect plugin {label} {}", path.display()))
        }
    }
}

fn validate_record(record: &InstallRecord, id: &str) -> Result<()> {
    if record.schema != RECORD_SCHEMA {
        bail!(
            "unsupported install record schema {} for plugin `{id}`",
            record.schema
        );
    }
    if record.id != id {
        bail!(
            "install record id `{}` does not match plugin `{id}`",
            record.id
        );
    }
    Repository::parse_full_name(&record.repository)
        .with_context(|| format!("invalid install record for plugin `{id}`"))?;
    if record.manifest_sha256.len() != 64
        || !record
            .manifest_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        bail!("invalid manifest SHA-256 in install record for plugin `{id}`");
    }
    if let Some(wasm) = &record.wasm_sha256
        && (wasm.len() != 64 || !wasm.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        bail!("invalid Wasm SHA-256 in install record for plugin `{id}`");
    }
    Ok(())
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("invalid path {}", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("cannot create {}", parent.display()))?;
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temporary, bytes)
        .with_context(|| format!("cannot write {}", temporary.display()))?;
    fs::rename(&temporary, path).with_context(|| format!("cannot replace {}", path.display()))?;
    Ok(())
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

struct StagingGuard {
    path: PathBuf,
    armed: bool,
}

impl StagingGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for StagingGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use crate::plugin::github::{Release, sha256_hex};

    const SOURCE: &str = "https://github.com/example/thermal";

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new(label: &str) -> Self {
            let sequence = STAGING_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "amph-plugin-{label}-{}-{sequence}",
                std::process::id()
            ));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn store(&self) -> PluginStore {
            PluginStore::new(self.0.join("plugins"))
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn digest(bytes: &[u8]) -> Option<Sha256Digest> {
        Sha256Digest::parse(&format!("sha256:{}", sha256_hex(bytes))).ok()
    }

    fn functionality_manifest(version: &str) -> Vec<u8> {
        serde_json::json!({
            "schema": 1,
            "id": "example.thermal",
            "name": "Thermal",
            "version": version,
            "kind": "functionality",
            "description": "Test plugin",
            "api": 1,
            "entry": "plugin.wasm"
        })
        .to_string()
        .into_bytes()
    }

    fn theme_manifest(version: &str) -> Vec<u8> {
        serde_json::json!({
            "schema": 1,
            "id": "example.thermal",
            "name": "Thermal",
            "version": version,
            "kind": "theme",
            "theme": {"palette": "nord", "accent": "cyan"}
        })
        .to_string()
        .into_bytes()
    }

    fn valid_wasm() -> Vec<u8> {
        wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (func (export "amph_abi_version") (result i32) i32.const 1)
                (func (export "amph_alloc") (param i32) (result i32) i32.const 1024)
                (func (export "amph_analyze") (param i32 i32) (result i64)
                    (i64.const 0)))"#,
        )
        .unwrap()
    }

    fn alternate_valid_wasm() -> Vec<u8> {
        wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (func (export "amph_abi_version") (result i32) i32.const 1)
                (func (export "amph_alloc") (param i32) (result i32) i32.const 1024)
                (func (export "amph_analyze") (param i32 i32) (result i64)
                    (i64.const 0))
                (func (export "amph_variant")))"#,
        )
        .unwrap()
    }

    fn echo_wasm() -> Vec<u8> {
        wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (func (export "amph_abi_version") (result i32) i32.const 1)
                (func (export "amph_alloc") (param i32) (result i32) i32.const 1024)
                (func (export "amph_analyze") (param $ptr i32) (param $len i32) (result i64)
                    (memory.copy (i32.const 2048) (local.get $ptr) (local.get $len))
                    (i64.or
                        (i64.shl (i64.extend_i32_u (i32.const 2048)) (i64.const 32))
                        (i64.extend_i32_u (local.get $len)))))"#,
        )
        .unwrap()
    }

    fn invalid_json_wasm() -> Vec<u8> {
        wat::parse_str(
            r#"(module
                (memory (export "memory") 1)
                (data (i32.const 2048) "not json")
                (func (export "amph_abi_version") (result i32) i32.const 1)
                (func (export "amph_alloc") (param i32) (result i32) i32.const 1024)
                (func (export "amph_analyze") (param i32 i32) (result i64)
                    (i64.or
                        (i64.shl (i64.extend_i32_u (i32.const 2048)) (i64.const 32))
                        (i64.const 8))))"#,
        )
        .unwrap()
    }

    struct FakeProvider {
        release: std::sync::Mutex<Release>,
        assets: std::sync::Mutex<HashMap<String, Vec<u8>>>,
    }

    impl FakeProvider {
        fn new(tag: &str, manifest: Vec<u8>, wasm: Option<Vec<u8>>) -> Self {
            let mut assets = HashMap::new();
            assets.insert(MANIFEST_FILE.to_string(), manifest.clone());
            let mut release_assets = vec![ReleaseAsset {
                name: MANIFEST_FILE.to_string(),
                url: asset_url(tag, MANIFEST_FILE),
                size: manifest.len() as u64,
                digest: digest(&manifest),
            }];
            if let Some(wasm) = wasm {
                assets.insert("plugin.wasm".to_string(), wasm.clone());
                release_assets.push(ReleaseAsset {
                    name: "plugin.wasm".to_string(),
                    url: asset_url(tag, "plugin.wasm"),
                    size: wasm.len() as u64,
                    digest: digest(&wasm),
                });
            }
            Self {
                release: std::sync::Mutex::new(Release {
                    tag: tag.to_string(),
                    assets: release_assets,
                }),
                assets: std::sync::Mutex::new(assets),
            }
        }

        fn replace(&self, tag: &str, manifest: Vec<u8>, wasm: Option<Vec<u8>>) {
            let replacement = FakeProvider::new(tag, manifest, wasm);
            *self.release.lock().unwrap() = replacement.release.into_inner().unwrap();
            *self.assets.lock().unwrap() = replacement.assets.into_inner().unwrap();
        }

        fn mutate_asset(&self, name: &str, change: impl FnOnce(&mut ReleaseAsset)) {
            let mut release = self.release.lock().unwrap();
            let asset = release
                .assets
                .iter_mut()
                .find(|asset| asset.name == name)
                .unwrap();
            change(asset);
        }
    }

    fn asset_url(tag: &str, name: &str) -> String {
        format!("https://github.com/example/thermal/releases/download/{tag}/{name}")
    }

    impl ReleaseProvider for FakeProvider {
        fn latest_release(&self, _repository: &Repository) -> Result<Release> {
            Ok(self.release.lock().unwrap().clone())
        }

        fn download_asset(
            &self,
            _repository: &Repository,
            asset: &ReleaseAsset,
            max_bytes: usize,
        ) -> Result<Vec<u8>> {
            let bytes = self
                .assets
                .lock()
                .unwrap()
                .get(&asset.name)
                .cloned()
                .ok_or_else(|| anyhow::anyhow!("missing fake asset {}", asset.name))?;
            if bytes.len() > max_bytes {
                bail!("fake asset too large");
            }
            Ok(bytes)
        }
    }

    #[test]
    fn installs_lists_and_tracks_functionality_plugins() {
        let root = TempRoot::new("install");
        let store = root.store();
        let provider = FakeProvider::new(
            "v1.0.0",
            functionality_manifest("1.0.0"),
            Some(valid_wasm()),
        );

        let installed = store.install_with(&provider, SOURCE).unwrap();
        assert!(installed.enabled);
        assert_eq!(installed.manifest.version, "1.0.0");
        assert_eq!(installed.install.as_ref().unwrap().release_tag, "v1.0.0");
        assert_eq!(
            installed.install.as_ref().unwrap().repository,
            "example/thermal"
        );
        assert!(installed.path.join(MANIFEST_FILE).is_file());
        assert!(installed.path.join("plugin.wasm").is_file());
        assert!(installed.path.join(RECORD_FILE).is_file());
        assert!(!installed.path.join(DISABLED_FILE).exists());

        let listed = store.list().unwrap();
        assert_eq!(listed, vec![installed]);
    }

    #[test]
    fn installs_theme_plugins_without_wasm() {
        let root = TempRoot::new("theme");
        let store = root.store();
        let provider = FakeProvider::new("v1.0.0", theme_manifest("1.0.0"), None);

        let installed = store.install_with(&provider, SOURCE).unwrap();
        assert!(installed.manifest.is_theme());
        assert!(!installed.path.join("plugin.wasm").exists());
        assert!(installed.install.unwrap().wasm_sha256.is_none());
    }

    #[test]
    fn analyzes_a_functionality_plugin_through_the_store() {
        let root = TempRoot::new("analyze");
        let store = root.store();
        let provider =
            FakeProvider::new("v1.0.0", functionality_manifest("1.0.0"), Some(echo_wasm()));
        store.install_with(&provider, SOURCE).unwrap();

        let request = serde_json::json!({
            "packages": ["bash", "nano"],
            "score": 0.5,
            "nested": {"ok": true}
        });
        let response = store.analyze_json("example.thermal", &request).unwrap();
        assert_eq!(response, request);
    }

    #[test]
    fn analyze_json_rejects_disabled_plugins() {
        let root = TempRoot::new("analyze-disabled");
        let store = root.store();
        let provider =
            FakeProvider::new("v1.0.0", functionality_manifest("1.0.0"), Some(echo_wasm()));
        store.install_with(&provider, SOURCE).unwrap();
        store.disable("example.thermal").unwrap();

        let error = store
            .analyze_json("example.thermal", &serde_json::json!({}))
            .unwrap_err()
            .to_string();
        assert!(error.contains("disabled"), "{error}");
    }

    #[test]
    fn analyze_json_rejects_theme_plugins() {
        let root = TempRoot::new("analyze-theme");
        let store = root.store();
        let provider = FakeProvider::new("v1.0.0", theme_manifest("1.0.0"), None);
        store.install_with(&provider, SOURCE).unwrap();

        let error = store
            .analyze_json("example.thermal", &serde_json::json!({}))
            .unwrap_err()
            .to_string();
        assert!(error.contains("theme plugin"), "{error}");
        assert!(error.contains("no JSON analyzer"), "{error}");
    }

    #[test]
    fn analyze_json_rejects_a_tampered_module() {
        let root = TempRoot::new("analyze-bad-module");
        let store = root.store();
        let provider =
            FakeProvider::new("v1.0.0", functionality_manifest("1.0.0"), Some(echo_wasm()));
        let installed = store.install_with(&provider, SOURCE).unwrap();
        fs::write(installed.path.join("plugin.wasm"), b"\0asmbroken").unwrap();

        let error = format!(
            "{:#}",
            store
                .analyze_json("example.thermal", &serde_json::json!({}))
                .unwrap_err()
        );
        assert!(error.contains("WebAssembly"), "{error}");
    }

    #[test]
    fn analyze_json_rejects_invalid_plugin_output() {
        let root = TempRoot::new("analyze-bad-output");
        let store = root.store();
        let provider = FakeProvider::new(
            "v1.0.0",
            functionality_manifest("1.0.0"),
            Some(invalid_json_wasm()),
        );
        store.install_with(&provider, SOURCE).unwrap();

        let error = store
            .analyze_json("example.thermal", &serde_json::json!({}))
            .unwrap_err()
            .to_string();
        assert!(error.contains("invalid JSON"), "{error}");
    }

    #[test]
    fn enable_disable_and_remove_are_persistent_and_atomic() {
        let root = TempRoot::new("state");
        let store = root.store();
        let provider = FakeProvider::new(
            "v1.0.0",
            functionality_manifest("1.0.0"),
            Some(valid_wasm()),
        );
        store.install_with(&provider, SOURCE).unwrap();

        assert!(!store.disable("example.thermal").unwrap().enabled);
        assert!(
            store
                .get("example.thermal")
                .unwrap()
                .path
                .join(DISABLED_FILE)
                .is_file()
        );
        assert!(store.enable("example.thermal").unwrap().enabled);
        assert!(
            !store
                .get("example.thermal")
                .unwrap()
                .path
                .join(DISABLED_FILE)
                .exists()
        );

        store.remove("example.thermal").unwrap();
        assert!(store.get("example.thermal").is_err());
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn updates_preserve_the_disabled_state() {
        let root = TempRoot::new("update");
        let store = root.store();
        let provider = FakeProvider::new(
            "v1.0.0",
            functionality_manifest("1.0.0"),
            Some(valid_wasm()),
        );
        store.install_with(&provider, SOURCE).unwrap();
        store.disable("example.thermal").unwrap();

        provider.replace(
            "v1.1.0",
            functionality_manifest("1.1.0"),
            Some(valid_wasm()),
        );
        let updated = store.install_with(&provider, SOURCE).unwrap();
        assert_eq!(updated.manifest.version, "1.1.0");
        assert!(!updated.enabled);
        assert_eq!(updated.install.unwrap().release_tag, "v1.1.0");
    }

    #[test]
    fn digest_mismatch_keeps_the_existing_installation() {
        let root = TempRoot::new("digest");
        let store = root.store();
        let provider = FakeProvider::new(
            "v1.0.0",
            functionality_manifest("1.0.0"),
            Some(valid_wasm()),
        );
        store.install_with(&provider, SOURCE).unwrap();

        provider.replace(
            "v1.1.0",
            functionality_manifest("1.1.0"),
            Some(valid_wasm()),
        );
        provider.mutate_asset(MANIFEST_FILE, |asset| {
            asset.digest = Sha256Digest::parse(&format!("sha256:{}", "0".repeat(64))).ok();
        });
        let error = store
            .install_with(&provider, SOURCE)
            .unwrap_err()
            .to_string();
        assert!(error.contains("SHA-256 mismatch"), "{error}");
        assert_eq!(
            store.get("example.thermal").unwrap().manifest.version,
            "1.0.0"
        );
    }

    #[test]
    fn missing_github_digest_refuses_installation() {
        let root = TempRoot::new("missing-digest");
        let store = root.store();
        let provider = FakeProvider::new(
            "v1.0.0",
            functionality_manifest("1.0.0"),
            Some(valid_wasm()),
        );
        provider.mutate_asset(MANIFEST_FILE, |asset| asset.digest = None);

        let error = store
            .install_with(&provider, SOURCE)
            .unwrap_err()
            .to_string();
        assert!(error.contains("no SHA-256 digest"), "{error}");
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn invalid_wasm_is_rejected_before_publishing() {
        let root = TempRoot::new("bad-wasm");
        let store = root.store();
        let provider = FakeProvider::new(
            "v1.0.0",
            functionality_manifest("1.0.0"),
            Some(b"\0asmbroken".to_vec()),
        );
        assert!(store.install_with(&provider, SOURCE).is_err());
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn manifest_id_cannot_escape_the_store_root() {
        let root = TempRoot::new("escape");
        let store = root.store();
        let manifest = serde_json::json!({
            "schema": 1,
            "id": "../escape",
            "name": "Escape",
            "version": "1.0.0",
            "kind": "theme",
            "theme": {"palette": "nord"}
        })
        .to_string()
        .into_bytes();
        let provider = FakeProvider::new("v1.0.0", manifest, None);
        assert!(store.install_with(&provider, SOURCE).is_err());
        assert!(root.0.join("escape").symlink_metadata().is_err());
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn check_and_update_all_follow_the_latest_release() {
        let root = TempRoot::new("check");
        let store = root.store();
        let provider = FakeProvider::new(
            "v1.0.0",
            functionality_manifest("1.0.0"),
            Some(valid_wasm()),
        );
        store.install_with(&provider, SOURCE).unwrap();

        let current = store.check_with(&provider, "example.thermal").unwrap();
        assert!(!current.update_available);
        assert_eq!(current.latest_tag, "v1.0.0");
        assert_eq!(
            store.check_all_with(&provider).unwrap(),
            vec![CheckOutcome::UpToDate(current)]
        );

        provider.replace(
            "v1.1.0",
            functionality_manifest("1.1.0"),
            Some(valid_wasm()),
        );
        let available = store.check_with(&provider, "example.thermal").unwrap();
        assert!(available.update_available);
        assert_eq!(
            store.check_all_with(&provider).unwrap(),
            vec![CheckOutcome::UpdateAvailable(available.clone())]
        );

        assert_eq!(
            store.update_all_with(&provider).unwrap(),
            vec![UpdateOutcome::Updated {
                id: "example.thermal".to_string(),
                from_tag: "v1.0.0".to_string(),
                to_tag: "v1.1.0".to_string(),
            }]
        );
        assert!(
            !store
                .check_with(&provider, "example.thermal")
                .unwrap()
                .update_available
        );
    }

    #[test]
    fn check_detects_a_wasm_digest_change_under_the_same_tag_and_manifest() {
        let root = TempRoot::new("check-wasm-digest");
        let store = root.store();
        let manifest = functionality_manifest("1.0.0");
        let provider = FakeProvider::new("v1.0.0", manifest.clone(), Some(valid_wasm()));
        store.install_with(&provider, SOURCE).unwrap();
        assert!(
            !store
                .check_with(&provider, "example.thermal")
                .unwrap()
                .update_available
        );

        // Same tag and byte-identical manifest, but new Wasm bytes.
        provider.replace("v1.0.0", manifest, Some(alternate_valid_wasm()));
        let changed = store.check_with(&provider, "example.thermal").unwrap();
        assert!(changed.update_available);
        assert_eq!(changed.current_tag, "v1.0.0");
        assert_eq!(changed.latest_tag, "v1.0.0");
        assert_eq!(
            store.check_all_with(&provider).unwrap(),
            vec![CheckOutcome::UpdateAvailable(changed)]
        );
    }

    #[test]
    fn check_fails_when_the_latest_wasm_digest_is_missing() {
        let root = TempRoot::new("check-missing-wasm-digest");
        let store = root.store();
        let provider = FakeProvider::new(
            "v1.0.0",
            functionality_manifest("1.0.0"),
            Some(valid_wasm()),
        );
        store.install_with(&provider, SOURCE).unwrap();
        provider.mutate_asset("plugin.wasm", |asset| asset.digest = None);

        let error = store
            .check_with(&provider, "example.thermal")
            .unwrap_err()
            .to_string();
        assert!(error.contains("no SHA-256 digest"), "{error}");
        assert!(error.contains("plugin.wasm"), "{error}");

        let outcomes = store.check_all_with(&provider).unwrap();
        let [CheckOutcome::Error { id, message }] = outcomes.as_slice() else {
            panic!("expected a per-plugin error, got {outcomes:?}");
        };
        assert_eq!(id, "example.thermal");
        assert!(message.contains("no SHA-256 digest"), "{message}");
    }

    #[cfg(unix)]
    fn assert_symlinked_store_file_is_rejected(file: &str, disable_first: bool) {
        let root = TempRoot::new("symlink-file");
        let store = root.store();
        let provider = FakeProvider::new(
            "v1.0.0",
            functionality_manifest("1.0.0"),
            Some(valid_wasm()),
        );
        store.install_with(&provider, SOURCE).unwrap();
        if disable_first {
            store.disable("example.thermal").unwrap();
        }

        let plugin_path = store.get("example.thermal").unwrap().path;
        let target = plugin_path.join(file);
        let outside = root.0.join(format!("outside-{file}"));
        fs::rename(&target, &outside).unwrap();
        if let Err(error) = std::os::unix::fs::symlink(&outside, &target) {
            // Some test filesystems do not support symlinks. Do not fail the
            // suite merely because the platform cannot exercise this case.
            eprintln!("skipping symlink test for {file}: {error}");
            return;
        }

        let error = format!("{:#}", store.get("example.thermal").unwrap_err());
        assert!(error.contains("symbolic link"), "{error}");
    }

    #[cfg(unix)]
    #[test]
    fn rejects_a_symlinked_manifest_file() {
        assert_symlinked_store_file_is_rejected(MANIFEST_FILE, false);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_a_symlinked_install_record_file() {
        assert_symlinked_store_file_is_rejected(RECORD_FILE, false);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_a_symlinked_wasm_entry_file() {
        assert_symlinked_store_file_is_rejected("plugin.wasm", false);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_a_symlinked_disabled_marker_file() {
        assert_symlinked_store_file_is_rejected(DISABLED_FILE, true);
    }

    #[test]
    fn check_reports_untracked_plugins() {
        let root = TempRoot::new("untracked");
        let store = root.store();
        let provider = FakeProvider::new(
            "v1.0.0",
            functionality_manifest("1.0.0"),
            Some(valid_wasm()),
        );
        store.install_with(&provider, SOURCE).unwrap();
        fs::remove_file(store.root().join("example.thermal").join(RECORD_FILE)).unwrap();

        assert_eq!(
            store.check_all_with(&provider).unwrap(),
            vec![CheckOutcome::NotTracked {
                id: "example.thermal".to_string()
            }]
        );
    }
}
