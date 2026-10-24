//! Update notification and install contract.
//!
//! AMPH checks for a newer build outside the TUI. The scanner reports the
//! result with [`crate::tui::ScanMessage::UpdateAvailable`], and the interface
//! shows it without interrupting the screen in use.
//!
//! An update belongs to a branch: `main` is the stable channel and
//! `development` carries newer builds. The host reports the persisted channel
//! with [`crate::tui::ScanMessage::UpdateChannel`] at startup, and every
//! [`UpdateInfo`] and [`UpdateRequest`] carries the branch it refers to so a
//! notification can never be mistaken for another channel.
//!
//! Installing is a one-shot callback. The TUI **never** downloads or executes an
//! installer itself: it sends [`UpdateRequest::Install`] to an injected
//! [`UpdateHandler`] and the host decides how to install. The handler runs on
//! the main thread with the terminal suspended (raw mode, mouse capture, and the
//! alternate screen restored), so a host installer may print progress or ask for
//! privilege escalation. Switching channels uses the same callback: the host
//! persists the branch and installs its latest build.
//!
//! The interface only reports that the handler finished. A running process
//! cannot replace its own executable in place, so the UI then asks the user to
//! restart AMPH; it never claims the new version is already active.
//!
//! # Example
//!
//! ```ignore
//! use amph::tui::{run_with_theme_and_update, UpdateHandler, UpdateOutcome,
//!                   UpdateRequest};
//!
//! let update = |request: UpdateRequest| {
//!     let version = request.version();
//!     // Host code installs that version here, with LC_ALL=C and its own
//!     // privilege handling. The terminal is already suspended.
//!     Ok(UpdateOutcome::new(true, captured_output))
//! };
//!
//! run_with_theme_and_update(scan, theme, cleanup, update)?;
//! ```

/// Branch used when a host does not name an update channel.
pub const DEFAULT_CHANNEL: &str = "main";

/// Selectable update channels, in presentation order.
pub const UPDATE_CHANNELS: [&str; 2] = [DEFAULT_CHANNEL, "development"];

/// Normalize a channel name received from the host or chosen in the interface.
///
/// Channel names are Git branch names. Surrounding whitespace is discarded and
/// an empty value falls back to [`DEFAULT_CHANNEL`]; unknown non-empty names
/// are preserved so a host can add a channel without changing the TUI.
pub fn normalize_channel(branch: impl AsRef<str>) -> String {
    let branch = branch.as_ref().trim();
    if branch.is_empty() {
        DEFAULT_CHANNEL.to_string()
    } else {
        branch.to_string()
    }
}

/// A newer version announced by the host's background check.
///
/// The host has already compared versions; the TUI displays the two values and
/// does not try to parse them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    current: String,
    latest: String,
    branch: String,
    url: Option<String>,
    notes: Option<String>,
}

impl UpdateInfo {
    pub fn new(current: impl Into<String>, latest: impl Into<String>) -> Self {
        Self {
            current: current.into().trim().to_string(),
            latest: latest.into().trim().to_string(),
            branch: DEFAULT_CHANNEL.to_string(),
            url: None,
            notes: None,
        }
    }

    /// Attach the channel the announced build is published on.
    ///
    /// The host should set this from the same persisted channel it used for the
    /// check. A missing or blank value falls back to `main`.
    pub fn with_branch(mut self, branch: impl Into<String>) -> Self {
        self.branch = normalize_channel(branch.into());
        self
    }

    /// Attach a release page or changelog URL shown in the prompt.
    pub fn with_url(mut self, url: impl Into<String>) -> Self {
        let url = url.into().trim().to_string();
        if !url.is_empty() {
            self.url = Some(url);
        }
        self
    }

    /// Attach a short release note. Keep it to a sentence or two; the prompt
    /// wraps it into the available width.
    pub fn with_notes(mut self, notes: impl Into<String>) -> Self {
        let notes = notes.into().trim().to_string();
        if !notes.is_empty() {
            self.notes = Some(notes);
        }
        self
    }

    pub fn current(&self) -> &str {
        &self.current
    }

    pub fn latest(&self) -> &str {
        &self.latest
    }

    /// The channel this build was published on.
    pub fn branch(&self) -> &str {
        &self.branch
    }

    pub fn url(&self) -> Option<&str> {
        self.url.as_deref()
    }

    pub fn notes(&self) -> Option<&str> {
        self.notes.as_deref()
    }

    /// A version string is required before the interface offers an install.
    pub fn is_actionable(&self) -> bool {
        !self.latest.is_empty()
    }
}

/// A request sent from the TUI to the injected update handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateRequest {
    Install { version: String, branch: String },
}

impl UpdateRequest {
    /// Ask the host to persist `branch` and install its latest build.
    ///
    /// Channel switches use a descriptive version such as
    /// `latest development build`. The host owns persistence and installation.
    pub fn install(version: impl Into<String>, branch: impl Into<String>) -> Self {
        Self::Install {
            version: version.into(),
            branch: normalize_channel(branch.into()),
        }
    }

    pub fn version(&self) -> &str {
        match self {
            UpdateRequest::Install { version, .. } => version,
        }
    }

    /// The branch the host must persist and install from.
    pub fn branch(&self) -> &str {
        match self {
            UpdateRequest::Install { branch, .. } => branch,
        }
    }
}

/// The handler's result after an install attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateOutcome {
    /// Whether the installer reported success.
    pub success: bool,
    /// Combined stdout/stderr from the host installer.
    pub output: String,
    /// Optional short note, for example "installed to ~/.cargo/bin".
    pub note: Option<String>,
}

impl UpdateOutcome {
    pub fn new(success: bool, output: impl Into<String>) -> Self {
        Self {
            success,
            output: output.into(),
            note: None,
        }
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }
}

/// A handler failure that can be displayed in the UI.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct UpdateError {
    message: String,
}

impl UpdateError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

pub type UpdateHandlerResult = Result<UpdateOutcome, UpdateError>;

/// Host-side update implementation.
///
/// The handler is called by the TUI main loop with the terminal suspended. It
/// runs no process inside the TUI and is responsible for its own installation
/// method, output capture, and privilege prompt.
pub trait UpdateHandler {
    fn handle(&mut self, request: UpdateRequest) -> UpdateHandlerResult;
}

impl<F> UpdateHandler for F
where
    F: FnMut(UpdateRequest) -> UpdateHandlerResult,
{
    fn handle(&mut self, request: UpdateRequest) -> UpdateHandlerResult {
        self(request)
    }
}

/// The handler attached by wrappers that do not accept a real updater.
///
/// It refuses every install instead of pretending the update succeeded. The
/// interface still displays an announced update, but the installer action stays
/// unavailable.
#[derive(Debug, Default, Clone, Copy)]
pub struct ReadOnlyUpdateHandler;

impl UpdateHandler for ReadOnlyUpdateHandler {
    fn handle(&mut self, _request: UpdateRequest) -> UpdateHandlerResult {
        Err(UpdateError::new(
            "No update installer is attached to this session. Relaunch AMPH with an update handler to install.",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn info_builders_trim_and_keep_optional_context() {
        let info = UpdateInfo::new(" 0.2.0-beta ", " 0.3.0 ")
            .with_url("  https://example.invalid/release  ")
            .with_notes("  Faster scans.  ");
        assert_eq!(info.current(), "0.2.0-beta");
        assert_eq!(info.latest(), "0.3.0");
        assert_eq!(info.branch(), DEFAULT_CHANNEL);
        assert_eq!(info.url(), Some("https://example.invalid/release"));
        assert_eq!(info.notes(), Some("Faster scans."));
        assert!(info.is_actionable());
        assert!(!UpdateInfo::new("1", " ").is_actionable());
    }

    #[test]
    fn update_info_carries_and_normalizes_its_channel() {
        let info = UpdateInfo::new("0.3.0", "0.4.0").with_branch("  development ");
        assert_eq!(info.branch(), "development");

        // A host that cannot name a channel must not produce a blank one.
        assert_eq!(
            UpdateInfo::new("0.3.0", "0.4.0").with_branch(" ").branch(),
            DEFAULT_CHANNEL
        );
    }

    #[test]
    fn install_request_carries_version_and_branch() {
        let request = UpdateRequest::install("latest development build", " development ");
        assert_eq!(request.version(), "latest development build");
        assert_eq!(request.branch(), "development");

        assert_eq!(
            UpdateRequest::install("0.3.0", "").branch(),
            DEFAULT_CHANNEL
        );
    }

    #[test]
    fn closure_is_an_update_handler() {
        let mut handler = |request: UpdateRequest| {
            assert_eq!(request.version(), "0.3.0");
            assert_eq!(request.branch(), "main");
            Ok(UpdateOutcome::new(true, "installed"))
        };
        let outcome = handler
            .handle(UpdateRequest::install("0.3.0", "main"))
            .unwrap();
        assert!(outcome.success);
    }

    #[test]
    fn read_only_handler_refuses_installs() {
        let mut handler = ReadOnlyUpdateHandler;
        let error = handler
            .handle(UpdateRequest::install("0.3.0", "main"))
            .expect_err("read-only updates");
        assert!(error.message().contains("No update installer"));
    }
}
