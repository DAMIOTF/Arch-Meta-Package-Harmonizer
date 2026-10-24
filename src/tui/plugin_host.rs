//! Mapping and host adapter between the plugin backend and the manager UI.
//!
//! The backend lives in [`amph::plugin`]. This module only translates
//! [`InstalledPlugin`] and [`PluginUpdate`] into the bounded [`PluginRow`] DTO
//! and maps [`PluginAction`] values onto the public [`PluginStore`] methods. No
//! policy is reimplemented here: digests, sandbox validation, store layout, and
//! manifest rules stay in the backend.
//!
//! A host can attach [`StorePluginHandler`] to
//! [`crate::tui::run_with_theme_and_update_and_plugins`]. Initial rows can also
//! be sent through [`crate::tui::ScanMessage::Plugins`].

use amph::plugin::{
    CheckOutcome, InstalledPlugin, PluginKind, PluginStore, PluginUpdate, UpdateOutcome,
};
use anyhow::Result;
use serde_json::Value;

use crate::tui::plugins::{
    PLUGIN_DETAIL_MAX, PluginAction, PluginError, PluginHandler, PluginHandlerResult,
    PluginOutcome, PluginRow, PluginRowKind, sanitize_text,
};

/// Map one backend row into the manager's bounded display DTO.
///
/// Manifest text is sanitized again by [`PluginRow`] constructors, so a corrupt
/// or hostile manifest cannot inject terminal control characters into the UI.
impl From<&InstalledPlugin> for PluginRow {
    fn from(plugin: &InstalledPlugin) -> Self {
        let manifest = &plugin.manifest;
        let mut row = PluginRow::new(&manifest.id, &manifest.name)
            .with_version(&manifest.version)
            .with_kind(match manifest.kind {
                PluginKind::Theme => PluginRowKind::Theme,
                PluginKind::Functionality => PluginRowKind::Functionality,
            })
            .with_enabled(plugin.enabled);

        if let Some(description) = &manifest.description {
            row = row.with_description(description);
        }
        if let Some(record) = &plugin.install {
            row = row
                .with_repository(&record.repository)
                .with_tag(&record.release_tag);
        }
        match manifest.kind {
            PluginKind::Theme => {
                if let Some(theme) = &manifest.theme {
                    row = row.with_theme_spec(theme.clone());
                }
            }
            PluginKind::Functionality => {
                if let Some(api) = manifest.api {
                    row = row.with_api(api);
                }
                if let Some(entry) = &manifest.entry {
                    row = row.with_entry(entry);
                }
            }
        }
        row
    }
}

/// Map installed plugins without update information.
pub fn rows_from_installed(installed: &[InstalledPlugin]) -> Vec<PluginRow> {
    installed.iter().map(PluginRow::from).collect()
}

/// Map installed plugins and apply the matching [`PluginUpdate`] rows.
///
/// A plugin with no announced update keeps its installed version and does not
/// show an update marker. The release tag is shown separately from the manifest
/// version.
pub fn rows_from_updates(
    installed: &[InstalledPlugin],
    updates: &[PluginUpdate],
) -> Vec<PluginRow> {
    installed
        .iter()
        .map(|plugin| {
            let row = PluginRow::from(plugin);
            match updates
                .iter()
                .find(|update| update.id == plugin.manifest.id)
            {
                Some(update) => apply_update(row, update),
                None => row.clear_update(),
            }
        })
        .collect()
}

/// Map installed plugins plus `check_all` outcomes into manager rows.
pub fn rows_from_check(installed: &[InstalledPlugin], outcomes: &[CheckOutcome]) -> Vec<PluginRow> {
    let updates = outcomes
        .iter()
        .filter_map(|outcome| match outcome {
            CheckOutcome::UpdateAvailable(update) | CheckOutcome::UpToDate(update) => {
                Some(update.clone())
            }
            CheckOutcome::NotTracked { .. } | CheckOutcome::Error { .. } => None,
        })
        .collect::<Vec<_>>();
    rows_from_updates(installed, &updates)
}

fn apply_update(row: PluginRow, update: &PluginUpdate) -> PluginRow {
    let row = row.with_tag(&update.current_tag);
    if update.update_available {
        row.with_latest_version(&update.latest_tag)
            .with_update_available(true)
    } else {
        row.clear_update()
    }
}

/// Turn an arbitrary analyze response into a bounded one-line summary and a
/// bounded detail. Text is sanitized; structured values are compacted to JSON.
fn summarize_analysis_response(response: &Value) -> (String, Option<String>) {
    let summary = response
        .get("summary")
        .and_then(Value::as_str)
        .or_else(|| response.get("message").and_then(Value::as_str))
        .or_else(|| response.as_str())
        .map(str::trim)
        .filter(|summary| !summary.is_empty())
        .map(|summary| sanitize_text(summary, PLUGIN_DETAIL_MAX))
        .unwrap_or_else(|| "Plugin analysis finished".to_string());

    let detail_source = response.get("detail").or_else(|| response.get("findings"));
    let detail = match detail_source {
        Some(Value::String(detail)) => Some(detail.clone()),
        Some(value) => serde_json::to_string(value).ok(),
        None if response.is_null() || response.is_string() => None,
        None if response.get("summary").and_then(Value::as_str).is_some()
            || response.get("message").and_then(Value::as_str).is_some() =>
        {
            None
        }
        None => serde_json::to_string(response).ok(),
    }
    .map(|detail| sanitize_text(&detail, PLUGIN_DETAIL_MAX))
    .filter(|detail| !detail.is_empty());

    (summary, detail)
}

/// A ready-to-use [`PluginHandler`] backed by the real plugin store.
///
/// Construction is explicit so the host decides when to touch the filesystem:
/// [`Self::discover`] uses the per-user XDG data directory, while [`Self::new`]
/// accepts a store rooted elsewhere. Handler results include the authoritative
/// refreshed rows, and a failed action is returned as [`PluginError`].
#[derive(Debug, Clone)]
pub struct StorePluginHandler {
    store: PluginStore,
}

impl StorePluginHandler {
    pub fn new(store: PluginStore) -> Self {
        Self { store }
    }

    /// Discover the per-user plugin store.
    pub fn discover() -> Result<Self> {
        Ok(Self::new(PluginStore::discover()?))
    }

    pub fn store(&self) -> &PluginStore {
        &self.store
    }

    fn list_rows(&self) -> Result<Vec<PluginRow>> {
        Ok(rows_from_installed(&self.store.list()?))
    }

    fn execute(&self, action: PluginAction) -> Result<PluginOutcome> {
        match action {
            PluginAction::Refresh => {
                let rows = self.list_rows()?;
                Ok(
                    PluginOutcome::new(true, format!("{} installed plugin(s)", rows.len()))
                        .with_plugins(rows),
                )
            }
            PluginAction::Install { url } => {
                let plugin = self.store.install(&url)?;
                let rows = self.list_rows()?;
                Ok(PluginOutcome::new(
                    true,
                    format!(
                        "Installed {} {}",
                        plugin.manifest.name, plugin.manifest.version
                    ),
                )
                .with_detail(format!(
                    "Source: {url} · GitHub release assets and SHA-256 digests were verified by the host."
                ))
                .with_plugins(rows))
            }
            PluginAction::CheckUpdates => {
                let installed = self.store.list()?;
                let outcomes = self.store.check_all()?;
                let rows = rows_from_check(&installed, &outcomes);
                let summary = check_summary(&outcomes);
                let mut outcome =
                    PluginOutcome::new(summary.success, summary.message).with_plugins(rows);
                if let Some(detail) = summary.detail {
                    outcome = outcome.with_detail(detail);
                }
                Ok(outcome)
            }
            PluginAction::UpdateAll => {
                let outcomes = self.store.update_all()?;
                let rows = self.list_rows()?;
                let summary = update_summary(&outcomes);
                let mut outcome =
                    PluginOutcome::new(summary.success, summary.message).with_plugins(rows);
                if let Some(detail) = summary.detail {
                    outcome = outcome.with_detail(detail);
                }
                Ok(outcome)
            }
            PluginAction::Analyze { id, request } => {
                // The backend method owns Wasm loading, sandbox limits, and
                // execution. The TUI only turns its JSON response into bounded
                // display text.
                let response = self.store.analyze_json(&id, &request)?;
                let (message, detail) = summarize_analysis_response(&response);
                let mut outcome = PluginOutcome::new(true, message);
                if let Some(detail) = detail {
                    outcome = outcome.with_detail(detail);
                }
                Ok(outcome)
            }
            PluginAction::SetEnabled { id, enabled } => {
                let plugin = self.store.set_enabled(&id, enabled)?;
                let rows = self.list_rows()?;
                Ok(PluginOutcome::new(
                    true,
                    format!(
                        "{} {}",
                        if enabled { "Enabled" } else { "Disabled" },
                        plugin.manifest.name
                    ),
                )
                .with_plugins(rows))
            }
            PluginAction::Remove { id } => {
                self.store.remove(&id)?;
                let rows = self.list_rows()?;
                Ok(PluginOutcome::new(true, format!("Removed plugin {id}")).with_plugins(rows))
            }
        }
    }
}

impl PluginHandler for StorePluginHandler {
    fn handle(&mut self, action: PluginAction) -> PluginHandlerResult {
        self.execute(action)
            .map_err(|error| PluginError::new(format!("{error:#}")))
    }
}

struct OperationSummary {
    success: bool,
    message: String,
    detail: Option<String>,
}

fn check_summary(outcomes: &[CheckOutcome]) -> OperationSummary {
    let updates = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, CheckOutcome::UpdateAvailable(_)))
        .count();
    let not_tracked = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, CheckOutcome::NotTracked { .. }))
        .count();
    let errors = outcomes
        .iter()
        .filter_map(|outcome| match outcome {
            CheckOutcome::Error { id, message } => Some(format!("{id}: {message}")),
            _ => None,
        })
        .collect::<Vec<_>>();

    let message = format!(
        "Checked {} plugin(s) · {} update(s)",
        outcomes.len(),
        updates
    );
    let detail = if !errors.is_empty() {
        Some(format!(
            "{} check(s) failed · {}",
            errors.len(),
            errors.join(" · ")
        ))
    } else if not_tracked > 0 {
        Some(format!(
            "{not_tracked} installed plugin(s) have no GitHub install record and were not checked."
        ))
    } else {
        None
    };

    OperationSummary {
        success: errors.is_empty(),
        message,
        detail,
    }
}

fn update_summary(outcomes: &[UpdateOutcome]) -> OperationSummary {
    let updated = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, UpdateOutcome::Updated { .. }))
        .count();
    let skipped = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, UpdateOutcome::Skipped { .. }))
        .count();
    let failures = outcomes
        .iter()
        .filter_map(|outcome| match outcome {
            UpdateOutcome::Failed { id, message } => Some(format!("{id}: {message}")),
            _ => None,
        })
        .collect::<Vec<_>>();

    let message = format!(
        "Updated {updated} plugin(s) · {} current · {} skipped",
        outcomes
            .len()
            .saturating_sub(updated + skipped + failures.len()),
        skipped
    );
    let detail = if !failures.is_empty() {
        Some(format!(
            "{} update(s) failed · {}",
            failures.len(),
            failures.join(" · ")
        ))
    } else if skipped > 0 {
        Some(format!(
            "{skipped} plugin(s) were skipped because they have no GitHub install record."
        ))
    } else {
        None
    };

    OperationSummary {
        success: failures.is_empty(),
        message,
        detail,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use amph::plugin::{InstallRecord, PluginManifest, RECORD_SCHEMA};
    use serde_json::json;

    use super::*;

    fn install_record(repository: &str, release_tag: &str) -> InstallRecord {
        InstallRecord {
            schema: RECORD_SCHEMA,
            id: "demo".to_string(),
            repository: repository.to_string(),
            release_tag: release_tag.to_string(),
            installed_at: 1,
            manifest_sha256: "0".repeat(64),
            wasm_sha256: None,
        }
    }

    fn functionality_plugin(enabled: bool) -> InstalledPlugin {
        let manifest = PluginManifest::from_json(
            r#"{
                "schema": 1,
                "id": "demo",
                "name": "Demo Plugin",
                "version": "1.2.3",
                "kind": "functionality",
                "description": "Adds analysis notes",
                "api": 1,
                "entry": "plugin.wasm"
            }"#,
        )
        .expect("manifest");
        InstalledPlugin {
            manifest,
            path: PathBuf::from("/tmp/demo"),
            enabled,
            install: Some(install_record("owner/demo", "v1.2.3")),
        }
    }

    fn theme_plugin() -> InstalledPlugin {
        let manifest = PluginManifest::from_json(
            r#"{
                "schema": 1,
                "id": "nord-theme",
                "name": "Nord Theme",
                "version": "0.9.0",
                "kind": "theme",
                "theme": { "palette": "nord" }
            }"#,
        )
        .expect("manifest");
        InstalledPlugin {
            manifest,
            path: PathBuf::from("/tmp/nord"),
            enabled: true,
            install: None,
        }
    }

    fn temp_root(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "amph-plugin-ui-{label}-{}-{nanos}",
            std::process::id()
        ))
    }

    #[test]
    fn functionality_manifest_fields_map_to_a_bounded_row() {
        let row = PluginRow::from(&functionality_plugin(false));
        assert_eq!(row.id(), "demo");
        assert_eq!(row.display_name(), "Demo Plugin");
        assert_eq!(row.version_label(), "1.2.3");
        assert_eq!(row.kind(), PluginRowKind::Functionality);
        assert!(!row.enabled());
        assert_eq!(row.description(), Some("Adds analysis notes"));
        assert_eq!(row.api(), Some(1));
        assert_eq!(row.entry(), Some("plugin.wasm"));
        assert_eq!(row.repository_label(), "owner/demo");
        assert_eq!(row.tag(), Some("v1.2.3"));
    }

    #[test]
    fn declarative_theme_fields_map_without_an_entry() {
        let row = PluginRow::from(&theme_plugin());
        assert_eq!(row.kind(), PluginRowKind::Theme);
        assert_eq!(row.theme_palette(), Some("nord"));
        let spec = row.theme_spec().expect("full theme spec");
        assert_eq!(spec.accent, "cyan");
        assert_eq!(spec.good, "green");
        assert_eq!(spec.warn, "yellow");
        assert_eq!(spec.bad, "red");
        assert_eq!(spec.muted, "bright black");
        assert_eq!(spec.info, "blue");
        assert!(row.entry().is_none());
        assert!(row.api().is_none());
        assert_eq!(row.repository_label(), "source not reported");
    }

    #[test]
    fn updates_are_applied_and_cleared_per_plugin() {
        let installed = vec![functionality_plugin(true), theme_plugin()];
        let updates = vec![PluginUpdate {
            id: "demo".to_string(),
            current_version: "1.2.3".to_string(),
            current_tag: "v1.2.3".to_string(),
            latest_tag: "v1.3.0".to_string(),
            update_available: true,
        }];
        let rows = rows_from_updates(&installed, &updates);
        assert_eq!(rows[0].latest_version(), Some("v1.3.0"));
        assert!(rows[0].has_update());
        assert!(!rows[1].has_update());

        let rows = rows_from_updates(
            &installed,
            &[PluginUpdate {
                id: "demo".to_string(),
                current_version: "1.2.3".to_string(),
                current_tag: "v1.2.3".to_string(),
                latest_tag: "v1.2.3".to_string(),
                update_available: false,
            }],
        );
        assert!(!rows[0].has_update());

        // A republished asset with the same tag is still an update because the
        // backend compares GitHub digests as well as tags.
        let rows = rows_from_updates(
            &installed,
            &[PluginUpdate {
                id: "demo".to_string(),
                current_version: "1.2.3".to_string(),
                current_tag: "v1.2.3".to_string(),
                latest_tag: "v1.2.3".to_string(),
                update_available: true,
            }],
        );
        assert!(rows[0].has_update());
    }

    #[test]
    fn check_outcomes_map_updates_and_summarize_failures() {
        let installed = vec![functionality_plugin(true), theme_plugin()];
        let outcomes = vec![
            CheckOutcome::UpdateAvailable(PluginUpdate {
                id: "demo".to_string(),
                current_version: "1.2.3".to_string(),
                current_tag: "v1.2.3".to_string(),
                latest_tag: "v1.3.0".to_string(),
                update_available: true,
            }),
            CheckOutcome::NotTracked {
                id: "nord-theme".to_string(),
            },
        ];
        let rows = rows_from_check(&installed, &outcomes);
        assert_eq!(rows.len(), 2);
        assert!(rows[0].has_update());
        assert!(!rows[1].has_update());

        let summary = check_summary(&[
            outcomes[0].clone(),
            CheckOutcome::Error {
                id: "demo".to_string(),
                message: "network unavailable".to_string(),
            },
        ]);
        assert!(!summary.success);
        assert!(summary.message.contains("1 update"));
        assert!(
            summary
                .detail
                .as_deref()
                .is_some_and(|detail| detail.contains("demo: network unavailable"))
        );
    }

    #[test]
    fn analyze_response_is_summarized_and_bounded() {
        let (summary, detail) = summarize_analysis_response(&json!({
            "summary": "  found\u{0007} an issue  ",
            "detail": "line one\nline two"
        }));
        assert_eq!(summary, "found an issue");
        assert_eq!(detail.as_deref(), Some("line one line two"));

        let (summary, detail) = summarize_analysis_response(&json!({
            "findings": [{"package": "linux", "note": "review"}]
        }));
        assert_eq!(summary, "Plugin analysis finished");
        assert!(
            detail
                .as_deref()
                .is_some_and(|detail| detail.contains("\"package\":\"linux\""))
        );
        assert!(detail.expect("detail").chars().count() <= 240);

        let (summary, _) = summarize_analysis_response(&json!({
            "summary": "x".repeat(500)
        }));
        assert_eq!(summary.chars().count(), 240);
    }

    #[test]
    fn store_handler_calls_the_backend_analyzer_and_formats_the_result() {
        let root = temp_root("analyze");
        let plugin_dir = root.join("echo");
        fs::create_dir_all(&plugin_dir).expect("plugin dir");
        fs::write(
            plugin_dir.join("plugin.json"),
            r#"{
                "schema": 1,
                "id": "echo",
                "name": "Echo Analyzer",
                "version": "1.0.0",
                "kind": "functionality",
                "api": 1,
                "entry": "plugin.wasm"
            }"#,
        )
        .expect("manifest");
        let wasm = wat::parse_str(
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
        .expect("wat");
        fs::write(plugin_dir.join("plugin.wasm"), wasm).expect("wasm");

        let request = json!({
            "api": 1,
            "op": "analyze",
            "snapshot": {"scanned_at": "now", "packages": []}
        });
        let mut handler = StorePluginHandler::new(PluginStore::new(&root));
        let outcome = handler
            .handle(PluginAction::Analyze {
                id: "echo".to_string(),
                request: request.clone(),
            })
            .expect("analyze");

        assert!(outcome.success);
        assert_eq!(outcome.message, "Plugin analysis finished");
        let detail = outcome.detail.expect("echoed detail");
        assert!(detail.contains("\"op\":\"analyze\""));
        assert!(detail.contains("scanned_at"));

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn store_handler_refreshes_and_removes_without_network() {
        let root = temp_root("remove");
        let plugin_dir = root.join("demo");
        fs::create_dir_all(&plugin_dir).expect("plugin dir");
        fs::write(
            plugin_dir.join("plugin.json"),
            r#"{
                "schema": 1,
                "id": "demo",
                "name": "Demo Plugin",
                "version": "1.0.0",
                "kind": "theme",
                "theme": { "palette": "nord" }
            }"#,
        )
        .expect("manifest");

        let mut handler = StorePluginHandler::new(PluginStore::new(&root));
        let outcome = handler.handle(PluginAction::Refresh).expect("refresh");
        assert_eq!(outcome.plugins.as_ref().map(Vec::len), Some(1));

        let outcome = handler
            .handle(PluginAction::Remove {
                id: "demo".to_string(),
            })
            .expect("remove");
        assert_eq!(outcome.plugins.as_ref().map(Vec::len), Some(0));

        fs::remove_dir_all(root).ok();
    }

    #[test]
    fn store_handler_toggles_enabled_state() {
        let root = temp_root("enable");
        let plugin_dir = root.join("demo");
        fs::create_dir_all(&plugin_dir).expect("plugin dir");
        fs::write(
            plugin_dir.join("plugin.json"),
            r#"{
                "schema": 1,
                "id": "demo",
                "name": "Demo Plugin",
                "version": "1.0.0",
                "kind": "theme",
                "theme": { "palette": "nord" }
            }"#,
        )
        .expect("manifest");

        let mut handler = StorePluginHandler::new(PluginStore::new(&root));
        let outcome = handler
            .handle(PluginAction::SetEnabled {
                id: "demo".to_string(),
                enabled: false,
            })
            .expect("disable");
        assert!(outcome.success);
        assert!(!outcome.plugins.expect("rows")[0].enabled());

        fs::remove_dir_all(root).ok();
    }
}
