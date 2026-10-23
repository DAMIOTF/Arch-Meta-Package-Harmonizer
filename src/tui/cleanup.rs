//! Cleanup integration contract.
//!
//! The TUI **never** constructs or executes a package-manager command. It only
//! sends a request to an injected [`CleanupHandler`], and the host application
//! runs the real command with its own process and privilege handling.
//!
//! A removal is a two-phase callback:
//!
//! 1. The user asks for a preview. The TUI sends
//!    [`CleanupRequest::Preview`] with the selected names. The handler runs the
//!    dry-run command (for example `pacman -Rns --print -- <names>` with
//!    `LC_ALL=C`) and returns the captured output in [`CleanupPreview`].
//! 2. The TUI displays that output verbatim. Only after the user types the exact
//!    word `remove` does the TUI send [`CleanupRequest::Remove`]. The handler may
//!    now run the actual removal (for example `sudo pacman -Rns -- <names>` with
//!    `LC_ALL=C`) and return [`CleanupOutcome`].
//!
//! Between those calls the TUI leaves raw mode, disables mouse capture, leaves
//! the alternate screen, and restores the cursor. The handler therefore runs on
//! a normal terminal and may prompt for a sudo password. The TUI re-enters its
//! screen afterwards and presents the returned preview or outcome.
//!
//! A handler can return [`CleanupError`] instead of a response. The UI shows the
//! error without exiting and without pretending the operation succeeded.
//!
//! # Example
//!
//! ```ignore
//! use amph::tui::{run_with_cleanup, CleanupError, CleanupOutcome, CleanupPreview,
//!                   CleanupRequest, CleanupResponse};
//!
//! run_with_cleanup(scan, |request| match request {
//!     CleanupRequest::Preview { packages } => {
//!         // Host code runs `pacman -Rns --print -- <packages>` here.
//!         Ok(CleanupResponse::Preview(CleanupPreview {
//!             output: captured_stdout_and_stderr,
//!             can_remove: status.success(),
//!             note: None,
//!         }))
//!     }
//!     CleanupRequest::Remove { packages } => {
//!         // Host code runs `sudo pacman -Rns -- <packages>` here, with LC_ALL=C.
//!         Ok(CleanupResponse::Removed(CleanupOutcome {
//!             success: status.success(),
//!             output: captured_output,
//!             note: None,
//!         }))
//!     }
//! })
//! ```

/// Which half of the cleanup conversation a request belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CleanupPhase {
    Preview,
    Remove,
}

impl CleanupPhase {
    pub fn label(self) -> &'static str {
        match self {
            CleanupPhase::Preview => "preview",
            CleanupPhase::Remove => "removal",
        }
    }
}

/// A request sent from the TUI to the injected handler.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupRequest {
    Preview { packages: Vec<String> },
    Remove { packages: Vec<String> },
}

impl CleanupRequest {
    pub fn phase(&self) -> CleanupPhase {
        match self {
            CleanupRequest::Preview { .. } => CleanupPhase::Preview,
            CleanupRequest::Remove { .. } => CleanupPhase::Remove,
        }
    }

    pub fn packages(&self) -> &[String] {
        match self {
            CleanupRequest::Preview { packages } | CleanupRequest::Remove { packages } => packages,
        }
    }
}

/// The handler's dry-run output shown to the user before any removal request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupPreview {
    /// Plain text captured from the handler's preview command.
    pub output: String,
    /// The handler should set this to `true` only when the preview completed
    /// successfully and the user may proceed to a real removal.
    pub can_remove: bool,
    /// Optional short note, for example "dry run only".
    pub note: Option<String>,
}

impl CleanupPreview {
    pub fn new(output: impl Into<String>, can_remove: bool) -> Self {
        Self {
            output: output.into(),
            can_remove,
            note: None,
        }
    }

    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }
}

/// The handler's result after an explicit removal request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CleanupOutcome {
    /// Whether the host command reported success.
    pub success: bool,
    /// Combined stdout/stderr from the host command.
    pub output: String,
    /// Optional short note.
    pub note: Option<String>,
}

impl CleanupOutcome {
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

/// A successful handler response. The variant must match the request phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupResponse {
    Preview(CleanupPreview),
    Removed(CleanupOutcome),
}

impl CleanupResponse {
    pub fn phase(&self) -> CleanupPhase {
        match self {
            CleanupResponse::Preview(_) => CleanupPhase::Preview,
            CleanupResponse::Removed(_) => CleanupPhase::Remove,
        }
    }
}

/// A handler failure that can be displayed in the UI.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct CleanupError {
    message: String,
}

impl CleanupError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

pub type CleanupHandlerResult = Result<CleanupResponse, CleanupError>;

/// Host-side cleanup implementation.
///
/// The handler is called by the TUI main loop with the terminal suspended. It
/// runs no process inside the TUI and is responsible for its own command
/// execution, privilege prompt, and `LC_ALL=C` environment.
pub trait CleanupHandler {
    fn handle(&mut self, request: CleanupRequest) -> CleanupHandlerResult;
}

impl<F> CleanupHandler for F
where
    F: FnMut(CleanupRequest) -> CleanupHandlerResult,
{
    fn handle(&mut self, request: CleanupRequest) -> CleanupHandlerResult {
        self(request)
    }
}

/// The handler attached by the non-destructive [`crate::tui::run`] wrapper.
///
/// It never removes anything. The preview explains how to opt into a real
/// cleanup handler.
#[derive(Debug, Default, Clone, Copy)]
pub struct ReadOnlyCleanupHandler;

impl CleanupHandler for ReadOnlyCleanupHandler {
    fn handle(&mut self, request: CleanupRequest) -> CleanupHandlerResult {
        match request {
            CleanupRequest::Preview { packages } => {
                let count = packages.len();
                Ok(CleanupResponse::Preview(
                    CleanupPreview::new(
                        format!(
                            "Read-only AMPH session.\n\n{count} package(s) were selected, but no cleanup handler is attached.\n\nLaunch AMPH with run_with_cleanup(...) or run_with_theme(...) to preview and optionally remove packages through host code."
                        ),
                        false,
                    )
                    .with_note("No handler attached · removal disabled"),
                ))
            }
            CleanupRequest::Remove { .. } => Err(CleanupError::new(
                "Read-only AMPH session: removal is disabled because no cleanup handler is attached.",
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_preview_cannot_remove() {
        let mut handler = ReadOnlyCleanupHandler;
        let response = handler
            .handle(CleanupRequest::Preview {
                packages: vec!["one".to_string(), "two".to_string()],
            })
            .expect("read-only preview");
        match response {
            CleanupResponse::Preview(preview) => {
                assert!(!preview.can_remove);
                assert!(preview.output.contains("Read-only"));
            }
            CleanupResponse::Removed(_) => panic!("unexpected removal response"),
        }
    }

    #[test]
    fn closure_is_a_cleanup_handler() {
        let mut handler = |request: CleanupRequest| {
            assert_eq!(request.phase(), CleanupPhase::Preview);
            assert_eq!(request.packages(), &["pkg".to_string()]);
            Ok(CleanupResponse::Preview(CleanupPreview::new(
                "dry run", true,
            )))
        };
        let response = handler
            .handle(CleanupRequest::Preview {
                packages: vec!["pkg".to_string()],
            })
            .unwrap();
        assert_eq!(response.phase(), CleanupPhase::Preview);
    }
}
