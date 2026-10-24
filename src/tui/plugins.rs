//! Plugin manager UI contract.
//!
//! This module is deliberately display-only. It carries the rows the host wants
//! the interface to show, the actions the interface asks the host to perform,
//! and the result the host sends back. The TUI never installs, updates, or
//! removes a plugin itself; a [`PluginHandler`] does that on the main thread
//! with the terminal suspended.
//!
//! The host is authoritative for plugin state. A handler should return the
//! refreshed [`PluginRow`] list in [`PluginOutcome::with_plugins`] after an
//! install, update, or removal so the manager reflects disk state instead of
//! guessing. [`crate::tui::plugin_host`] maps backend `InstalledPlugin` and
//! `PluginUpdate` rows into this DTO and provides a `PluginStore`-backed
//! handler; this module stays free of backend policy.
//!
//! Functionality analysis uses [`PluginAction::Analyze`] with a stable,
//! documented JSON request built from the current package inventory. Theme
//! plugins carry their full validated [`ThemeSpec`]; applying one changes only
//! the current session palette and never writes configuration.
//!
//! # Example
//!
//! ```ignore
//! use amph::tui::{PluginAction, PluginError, PluginHandler, PluginOutcome};
//!
//! let plugins = |action: PluginAction| match action {
//!     PluginAction::Install { url } => {
//!         // Host code validates the URL again, fetches the repository, and
//!         // installs it into the WASM sandbox.
//!         Ok(PluginOutcome::new(true, "Plugin installed").with_plugins(rows))
//!     }
//!     PluginAction::CheckUpdates => Ok(
//!         PluginOutcome::new(true, "Update check finished").with_plugins(rows),
//!     ),
//!     PluginAction::UpdateAll => Ok(
//!         PluginOutcome::new(true, "All plugins updated").with_plugins(rows),
//!     ),
//!     PluginAction::Analyze { id, request } => {
//!         // Host calls PluginStore::analyze_json and formats its response.
//!         Ok(PluginOutcome::new(true, "Analysis finished"))
//!     }
//!     PluginAction::SetEnabled { id, enabled } => {
//!         // Host toggles one plugin by its stable id.
//!         Ok(PluginOutcome::new(true, "Plugin state changed").with_plugins(rows))
//!     }
//!     PluginAction::Remove { id } => {
//!         // Host removes one plugin by its stable id.
//!         Ok(PluginOutcome::new(true, "Plugin removed").with_plugins(rows))
//!     }
//!     PluginAction::Refresh => Ok(PluginOutcome::new(true, "Plugin list refreshed").with_plugins(rows)),
//! };
//! ```

use std::fmt;

use amph::plugin::ThemeSpec;
use serde_json::Value;

/// Maximum width of a plugin id shown or accepted by the interface.
pub const PLUGIN_ID_MAX: usize = 64;
/// Maximum width of a plugin name or display name.
pub const PLUGIN_NAME_MAX: usize = 64;
/// Maximum width of a reported version string.
pub const PLUGIN_VERSION_MAX: usize = 32;
/// Maximum width of a repository label.
pub const PLUGIN_REPOSITORY_MAX: usize = 120;
/// Maximum length of a GitHub URL entered or pasted into the manager.
pub const PLUGIN_URL_MAX: usize = 200;
/// Maximum length of a manifest description shown in the manager.
pub const PLUGIN_DESCRIPTION_MAX: usize = 200;
/// Maximum length of a Wasm entry filename or release tag.
pub const PLUGIN_ENTRY_MAX: usize = 120;
pub const PLUGIN_TAG_MAX: usize = 64;
/// Maximum length of a result message or detail line.
pub const PLUGIN_DETAIL_MAX: usize = 240;
/// Maximum number of rows accepted from the host at once.
pub const PLUGIN_ROW_LIMIT: usize = 256;
/// Stable API/operation version for the analyze request envelope.
pub const PLUGIN_ANALYZE_API: u32 = 1;

/// Which kind of plugin the manifest declares.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PluginRowKind {
    /// A declarative theme with no executable code.
    Theme,
    /// A Wasm analyze hook using ABI v1.
    Functionality,
    /// A row supplied without kind information.
    #[default]
    Unknown,
}

impl PluginRowKind {
    pub fn label(self) -> &'static str {
        match self {
            PluginRowKind::Theme => "theme",
            PluginRowKind::Functionality => "functionality",
            PluginRowKind::Unknown => "plugin",
        }
    }
}

/// Return `true` for invisible formatting characters that can misrepresent
/// display text (bidi overrides, zero-width joiners, BOMs, and similar).
fn is_format_control(character: char) -> bool {
    matches!(
        character,
        '\u{200B}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{2064}'
            | '\u{2066}'..='\u{206F}'
            | '\u{FEFF}'
    )
}

/// Sanitize plugin-supplied text before it is stored or rendered.
///
/// Control and formatting characters become spaces; runs of whitespace collapse
/// to one space; leading and trailing whitespace is removed; at most
/// `max_chars` characters are kept. Everything the manager renders passes
/// through this function or [`PluginRow`]'s constructors.
pub fn sanitize_text(value: &str, max_chars: usize) -> String {
    value
        .chars()
        .take(max_chars)
        .map(|character| {
            if character.is_control() || is_format_control(character) {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Why a URL could not be accepted as a GitHub repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PluginUrlError {
    Empty,
    TooLong,
    ControlCharacters,
    NotGitHub,
    MissingRepository,
    InvalidRepository,
}

impl PluginUrlError {
    /// A short, user-facing explanation. It contains no input text.
    pub fn message(self) -> &'static str {
        match self {
            PluginUrlError::Empty => "Paste a GitHub repository URL.",
            PluginUrlError::TooLong => {
                "The URL is too long. Use a repository URL under 200 characters."
            }
            PluginUrlError::ControlCharacters => {
                "The URL contains control characters. Remove them and try again."
            }
            PluginUrlError::NotGitHub => {
                "Only github.com repository URLs are accepted, for example https://github.com/owner/repo."
            }
            PluginUrlError::MissingRepository => "Include both the owner and the repository name.",
            PluginUrlError::InvalidRepository => {
                "Use a plain repository URL such as https://github.com/owner/repo."
            }
        }
    }
}

impl fmt::Display for PluginUrlError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for PluginUrlError {}

/// Normalize a pasted GitHub repository URL to a canonical HTTPS form.
///
/// Accepted forms include `https://github.com/owner/repo`,
/// `http://github.com/owner/repo/`, `www.github.com/owner/repo`, a missing
/// scheme, and a trailing `.git`. Query strings, fragments, extra path
/// segments, credentials, other hosts, and control characters are rejected.
/// The returned string never contains trailing whitespace, a query, or a
/// fragment.
pub fn normalize_github_url(input: &str) -> Result<String, PluginUrlError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(PluginUrlError::Empty);
    }
    if trimmed.chars().count() > PLUGIN_URL_MAX {
        return Err(PluginUrlError::TooLong);
    }
    if trimmed
        .chars()
        .any(|character| character.is_control() || is_format_control(character))
    {
        return Err(PluginUrlError::ControlCharacters);
    }

    let without_scheme = strip_prefix_ascii_case(trimmed, "https://")
        .or_else(|| strip_prefix_ascii_case(trimmed, "http://"))
        .unwrap_or(trimmed);
    if without_scheme.contains("://") || without_scheme.contains('@') {
        return Err(PluginUrlError::NotGitHub);
    }

    let without_www = strip_prefix_ascii_case(without_scheme, "www.").unwrap_or(without_scheme);
    let (host, path) = without_www
        .split_once('/')
        .ok_or(PluginUrlError::NotGitHub)?;
    if !host.eq_ignore_ascii_case("github.com") {
        return Err(PluginUrlError::NotGitHub);
    }

    let path = path.trim_end_matches('/');
    if path.is_empty() {
        return Err(PluginUrlError::MissingRepository);
    }

    let mut segments = path.split('/');
    let owner = segments.next().unwrap_or_default();
    let repository = segments.next().unwrap_or_default();
    if segments.next().is_some() {
        return Err(PluginUrlError::InvalidRepository);
    }
    let repository = repository.strip_suffix(".git").unwrap_or(repository);
    if owner.is_empty() || repository.is_empty() {
        return Err(PluginUrlError::MissingRepository);
    }

    // Match the backend's GitHub owner rules so review does not accept a URL
    // that `PluginStore::install` will reject.
    let owner_valid = (1..=39).contains(&owner.len())
        && owner
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
        && owner
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_alphanumeric())
        && owner
            .chars()
            .last()
            .is_some_and(|character| character.is_ascii_alphanumeric());

    // Match the backend's repository-name rules.
    let repository_valid = (1..=100).contains(&repository.len())
        && repository != "."
        && repository != ".."
        && repository.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        });

    if !owner_valid || !repository_valid {
        return Err(PluginUrlError::InvalidRepository);
    }

    Ok(format!("https://github.com/{owner}/{repository}"))
}

fn strip_prefix_ascii_case<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    let head = value.get(..prefix.len())?;
    if head.eq_ignore_ascii_case(prefix) {
        Some(&value[prefix.len()..])
    } else {
        None
    }
}

fn sanitize_theme_spec(theme: ThemeSpec) -> ThemeSpec {
    ThemeSpec {
        palette: sanitize_text(&theme.palette, PLUGIN_VERSION_MAX),
        accent: sanitize_text(&theme.accent, PLUGIN_VERSION_MAX),
        good: sanitize_text(&theme.good, PLUGIN_VERSION_MAX),
        warn: sanitize_text(&theme.warn, PLUGIN_VERSION_MAX),
        bad: sanitize_text(&theme.bad, PLUGIN_VERSION_MAX),
        muted: sanitize_text(&theme.muted, PLUGIN_VERSION_MAX),
        info: sanitize_text(&theme.info, PLUGIN_VERSION_MAX),
    }
}

/// One installed plugin, as the manager should display it.
///
/// All text fields, including theme color strings, are sanitized and bounded in
/// the constructors, so a host cannot inject control characters, unbounded
/// text, or a multi-line name into the manager. `id` is the stable key used by
/// update, analyze, enable, and remove actions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRow {
    id: String,
    name: String,
    display_name: String,
    version: String,
    repository: String,
    kind: PluginRowKind,
    enabled: bool,
    description: Option<String>,
    api: Option<u32>,
    entry: Option<String>,
    theme: Option<ThemeSpec>,
    tag: Option<String>,
    latest_version: Option<String>,
    update_available: bool,
}

impl PluginRow {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        let id = sanitize_text(&id.into(), PLUGIN_ID_MAX);
        let name = sanitize_text(&name.into(), PLUGIN_NAME_MAX);
        let name = if name.is_empty() {
            "plugin".to_string()
        } else {
            name
        };
        Self {
            display_name: name.clone(),
            id,
            name,
            version: String::new(),
            repository: String::new(),
            kind: PluginRowKind::Unknown,
            enabled: true,
            description: None,
            api: None,
            entry: None,
            theme: None,
            tag: None,
            latest_version: None,
            update_available: false,
        }
    }

    pub fn with_kind(mut self, kind: PluginRowKind) -> Self {
        self.kind = kind;
        self
    }

    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        let description = sanitize_text(&description.into(), PLUGIN_DESCRIPTION_MAX);
        self.description = if description.is_empty() {
            None
        } else {
            Some(description)
        };
        self
    }

    /// Attach the functionality ABI version from the manifest.
    pub fn with_api(mut self, api: u32) -> Self {
        self.api = Some(api);
        self
    }

    /// Attach the functionality Wasm entry filename from the manifest.
    pub fn with_entry(mut self, entry: impl Into<String>) -> Self {
        let entry = sanitize_text(&entry.into(), PLUGIN_ENTRY_MAX);
        self.entry = if entry.is_empty() { None } else { Some(entry) };
        self
    }

    /// Attach the full validated declarative theme from the manifest.
    ///
    /// Every string is sanitized again and bounded, so a row is safe to render
    /// even if a host builds it outside the backend mapper. Color validity is
    /// checked when the theme is applied.
    pub fn with_theme_spec(mut self, theme: ThemeSpec) -> Self {
        self.theme = Some(sanitize_theme_spec(theme));
        self
    }

    /// Attach the installed GitHub release tag.
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        let tag = sanitize_text(&tag.into(), PLUGIN_TAG_MAX);
        self.tag = if tag.is_empty() { None } else { Some(tag) };
        self
    }

    pub fn with_display_name(mut self, display_name: impl Into<String>) -> Self {
        let display_name = sanitize_text(&display_name.into(), PLUGIN_NAME_MAX);
        if !display_name.is_empty() {
            self.display_name = display_name;
        }
        self
    }

    pub fn with_version(mut self, version: impl Into<String>) -> Self {
        self.version = sanitize_text(&version.into(), PLUGIN_VERSION_MAX);
        self
    }

    pub fn with_repository(mut self, repository: impl Into<String>) -> Self {
        self.repository = sanitize_text(&repository.into(), PLUGIN_REPOSITORY_MAX);
        self
    }

    /// Attach the newer version announced by the host. A blank value clears the
    /// update marker.
    pub fn with_latest_version(mut self, latest_version: impl Into<String>) -> Self {
        let latest_version = sanitize_text(&latest_version.into(), PLUGIN_TAG_MAX);
        if latest_version.is_empty() {
            self.latest_version = None;
            self.update_available = false;
        } else {
            self.update_available = latest_version != self.version;
            self.latest_version = Some(latest_version);
        }
        self
    }

    /// Mark an update explicitly. This matters when the release tag did not
    /// change but its GitHub asset digest did.
    pub fn with_update_available(mut self, available: bool) -> Self {
        self.update_available = available && self.latest_version.is_some();
        self
    }

    /// Clear any announced update without touching the installed version.
    pub fn clear_update(mut self) -> Self {
        self.latest_version = None;
        self.update_available = false;
        self
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// The safe label the manager renders. Falls back to [`Self::name`].
    pub fn display_name(&self) -> &str {
        &self.display_name
    }

    pub fn kind(&self) -> PluginRowKind {
        self.kind
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub fn api(&self) -> Option<u32> {
        self.api
    }

    pub fn entry(&self) -> Option<&str> {
        self.entry.as_deref()
    }

    /// The full validated theme declaration carried by the manifest.
    pub fn theme_spec(&self) -> Option<&ThemeSpec> {
        self.theme.as_ref()
    }

    /// The declarative palette name, when this row carries a theme.
    pub fn theme_palette(&self) -> Option<&str> {
        self.theme
            .as_ref()
            .map(|theme| theme.palette.as_str())
            .filter(|palette| !palette.is_empty())
    }

    pub fn tag(&self) -> Option<&str> {
        self.tag.as_deref()
    }

    /// The version, or `not reported` when the host did not supply one.
    pub fn version_label(&self) -> &str {
        if self.version.is_empty() {
            "not reported"
        } else {
            &self.version
        }
    }

    /// The repository label, or `source not reported` when it is unavailable.
    pub fn repository_label(&self) -> &str {
        if self.repository.is_empty() {
            "source not reported"
        } else {
            &self.repository
        }
    }

    pub fn latest_version(&self) -> Option<&str> {
        self.latest_version.as_deref()
    }

    /// Whether the host has announced a different version for this plugin.
    pub fn has_update(&self) -> bool {
        self.update_available && self.latest_version.is_some()
    }
}

/// An action the manager asks the host to perform.
///
/// The UI queues this after review or confirmation. It never installs, updates,
/// analyzes, or removes anything directly. `Install` always carries a normalized
/// `https://github.com/owner/repo` URL; the host should validate it again before
/// any network or filesystem work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PluginAction {
    /// Re-read the installed plugin list without changing anything.
    Refresh,
    /// Ask the host to check every installed plugin for updates.
    CheckUpdates,
    /// Install the repository at a normalized GitHub URL.
    Install { url: String },
    /// Update every plugin the host listed with a newer version.
    UpdateAll,
    /// Run a functionality plugin over the current package snapshot.
    ///
    /// `request` has the stable envelope
    /// `{"api": 1, "op": "analyze", "snapshot": {"scanned_at": String,
    /// "packages": [{"name", "version", "size", "explicit", "foreign",
    /// "depends", "provides"}]}}`. It contains no host handles, paths,
    /// capabilities, environment, or filesystem access.
    Analyze { id: String, request: Value },
    /// Enable or disable one plugin.
    SetEnabled { id: String, enabled: bool },
    /// Remove one plugin by its stable id.
    Remove { id: String },
}

impl PluginAction {
    /// Build an `Install` action and normalize its URL, rejecting unsafe input.
    pub fn install(url: impl AsRef<str>) -> Result<Self, PluginUrlError> {
        Ok(Self::Install {
            url: normalize_github_url(url.as_ref())?,
        })
    }

    /// A short label used in the busy line and session log.
    pub fn label(&self) -> &'static str {
        match self {
            PluginAction::Refresh => "refreshing plugins",
            PluginAction::CheckUpdates => "checking for plugin updates",
            PluginAction::Install { .. } => "installing plugin",
            PluginAction::UpdateAll => "updating all plugins",
            PluginAction::Analyze { .. } => "analyzing packages",
            PluginAction::SetEnabled { enabled: true, .. } => "enabling plugin",
            PluginAction::SetEnabled { enabled: false, .. } => "disabling plugin",
            PluginAction::Remove { .. } => "removing plugin",
        }
    }

    /// The GitHub URL for an install action.
    pub fn url(&self) -> Option<&str> {
        match self {
            PluginAction::Install { url } => Some(url),
            _ => None,
        }
    }
}

/// The host's result after a plugin action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginOutcome {
    /// Whether the host operation reported success.
    pub success: bool,
    /// A short, safe summary shown by the manager.
    pub message: String,
    /// Optional detail, for example a captured failure reason.
    pub detail: Option<String>,
    /// The authoritative installed rows after the action. When present, the
    /// manager replaces its list with these sanitized rows.
    pub plugins: Option<Vec<PluginRow>>,
}

impl PluginOutcome {
    pub fn new(success: bool, message: impl Into<String>) -> Self {
        Self {
            success,
            message: sanitize_text(&message.into(), PLUGIN_DETAIL_MAX),
            detail: None,
            plugins: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        let detail = sanitize_text(&detail.into(), PLUGIN_DETAIL_MAX);
        if !detail.is_empty() {
            self.detail = Some(detail);
        }
        self
    }

    /// Replace the manager's installed list with this authoritative state.
    pub fn with_plugins(mut self, plugins: Vec<PluginRow>) -> Self {
        self.plugins = Some(plugins.into_iter().take(PLUGIN_ROW_LIMIT).collect());
        self
    }
}

/// A handler failure that can be displayed without exiting the manager.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct PluginError {
    message: String,
}

impl PluginError {
    pub fn new(message: impl Into<String>) -> Self {
        let message = sanitize_text(&message.into(), PLUGIN_DETAIL_MAX);
        Self {
            message: if message.is_empty() {
                "The plugin operation failed without a message.".to_string()
            } else {
                message
            },
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

pub type PluginHandlerResult = Result<PluginOutcome, PluginError>;

/// Host-side plugin implementation.
///
/// The handler is called by the TUI main loop with the terminal suspended. It
/// owns fetching, sandbox installation, update checks, removal, and the
/// authoritative refreshed list. It must not be implemented in the TUI.
pub trait PluginHandler {
    fn handle(&mut self, action: PluginAction) -> PluginHandlerResult;
}

impl<F> PluginHandler for F
where
    F: FnMut(PluginAction) -> PluginHandlerResult,
{
    fn handle(&mut self, action: PluginAction) -> PluginHandlerResult {
        self(action)
    }
}

/// Handler attached by launches that do not include a plugin backend.
///
/// It refuses every action so the manager never pretends an install, update, or
/// removal happened. Browsing already-supplied rows still works.
#[derive(Debug, Default, Clone, Copy)]
pub struct ReadOnlyPluginHandler;

impl PluginHandler for ReadOnlyPluginHandler {
    fn handle(&mut self, _action: PluginAction) -> PluginHandlerResult {
        Err(PluginError::new(
            "No plugin backend is attached to this session. Relaunch AMPH with a plugin handler to install, update, or remove plugins.",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_urls_are_normalized_to_https() {
        assert_eq!(
            normalize_github_url(" https://github.com/DAMIOTF/Arch-Meta-Package-Harmonizer/ "),
            Ok("https://github.com/DAMIOTF/Arch-Meta-Package-Harmonizer".to_string())
        );
        assert_eq!(
            normalize_github_url("github.com/owner/repo.git"),
            Ok("https://github.com/owner/repo".to_string())
        );
        assert_eq!(
            normalize_github_url("http://www.github.com/owner/repo\n"),
            Ok("https://github.com/owner/repo".to_string())
        );
    }

    #[test]
    fn non_github_or_malformed_urls_are_rejected() {
        assert_eq!(normalize_github_url(" "), Err(PluginUrlError::Empty));
        assert_eq!(
            normalize_github_url("https://gitlab.com/owner/repo"),
            Err(PluginUrlError::NotGitHub)
        );
        assert_eq!(
            normalize_github_url("https://github.com.evil.test/owner/repo"),
            Err(PluginUrlError::NotGitHub)
        );
        assert_eq!(
            normalize_github_url("https://github.com/owner"),
            Err(PluginUrlError::MissingRepository)
        );
        assert_eq!(
            normalize_github_url("https://github.com/owner/repo/tree/main"),
            Err(PluginUrlError::InvalidRepository)
        );
        assert_eq!(
            normalize_github_url("https://github.com/owner/repo?tab=readme"),
            Err(PluginUrlError::InvalidRepository)
        );
        assert_eq!(
            normalize_github_url("https://github.com/owner_name/repo"),
            Err(PluginUrlError::InvalidRepository)
        );
        assert_eq!(
            normalize_github_url(&format!("https://github.com/{}/repo", "a".repeat(40))),
            Err(PluginUrlError::InvalidRepository)
        );
        assert_eq!(
            normalize_github_url("https://github.com/owner/re\u{0007}po"),
            Err(PluginUrlError::ControlCharacters)
        );
        assert_eq!(
            normalize_github_url(&format!(
                "https://github.com/owner/{}",
                "a".repeat(PLUGIN_URL_MAX)
            )),
            Err(PluginUrlError::TooLong)
        );
    }

    #[test]
    fn display_text_is_bounded_and_stripped() {
        let sanitized = sanitize_text("  bad\u{0007}\nname  with   gaps  ", 40);
        assert_eq!(sanitized, "bad name with gaps");

        let long = sanitize_text(&"x".repeat(200), 12);
        assert_eq!(long.chars().count(), 12);

        // Bidi overrides become spaces rather than reversing rendered order.
        let bidi = sanitize_text("safe\u{202E}txet", 64);
        assert_eq!(bidi, "safe txet");
    }

    #[test]
    fn rows_bound_host_text_and_expose_updates() {
        let row = PluginRow::new(" id\u{0007} ", "  My\nPlugin  ")
            .with_version(" 1.2.3 ")
            .with_repository(" https://github.com/owner/repo ")
            .with_latest_version(" 1.3.0 ");
        assert_eq!(row.id(), "id");
        assert_eq!(row.name(), "My Plugin");
        assert_eq!(row.display_name(), "My Plugin");
        assert_eq!(row.version_label(), "1.2.3");
        assert_eq!(row.repository_label(), "https://github.com/owner/repo");
        assert_eq!(row.latest_version(), Some("1.3.0"));
        assert!(row.has_update());
        assert!(!row.clone().with_latest_version("1.2.3").has_update());
        assert!(!row.with_latest_version("").has_update());
    }

    #[test]
    fn actions_only_accept_normalized_install_urls() {
        let action = PluginAction::install(" github.com/owner/repo.git ").expect("install action");
        assert_eq!(action.url(), Some("https://github.com/owner/repo"));
        assert_eq!(action.label(), "installing plugin");

        let error = PluginAction::install("https://example.test/owner/repo").expect_err("rejected");
        assert_eq!(error, PluginUrlError::NotGitHub);
    }

    #[test]
    fn outcomes_sanitize_messages_and_are_authoritative_for_rows() {
        let rows = vec![PluginRow::new("one", "One")];
        let outcome = PluginOutcome::new(true, "  Installed\u{0007} safely ")
            .with_detail(" line one\nline two ")
            .with_plugins(rows.clone());
        assert_eq!(outcome.message, "Installed safely");
        assert_eq!(outcome.detail.as_deref(), Some("line one line two"));
        assert_eq!(outcome.plugins, Some(rows));

        let error = PluginError::new("  failed\u{0007} now ");
        assert_eq!(error.message(), "failed now");
    }

    #[test]
    fn read_only_handler_refuses_every_action() {
        let mut handler = ReadOnlyPluginHandler;
        let error = handler
            .handle(PluginAction::UpdateAll)
            .expect_err("no plugin backend");
        assert!(error.message().contains("No plugin backend"));
    }
}
