use std::io;
use std::time::Duration;

use crossterm::event::{self, Event, KeyEvent, MouseEvent};

use super::model::Snapshot;
use super::update::UpdateInfo;

/// Messages sent from the scanner thread to the interface.
///
/// A scanner does not have to send every variant. The useful minimum is a
/// sequence of [`ScanMessage::Progress`] updates followed by one
/// [`ScanMessage::Snapshot`] and [`ScanMessage::Finished`].
///
/// [`ScanMessage::UpdateAvailable`] is independent of the scan lifecycle: a
/// host may send it before, during, or after the snapshot, and may never send
/// it when no background update check is configured.
#[derive(Debug, Clone)]
pub enum ScanMessage {
    Progress {
        stage: String,
        done: usize,
        total: usize,
        detail: String,
    },
    Snapshot(Snapshot),
    /// A background check found a newer version. The host has already compared
    /// versions; the TUI only presents the result.
    UpdateAvailable(UpdateInfo),
    Warning(String),
    Failed(String),
    Finished,
}

impl ScanMessage {
    pub fn progress(stage: impl Into<String>, done: usize, total: usize) -> Self {
        Self::progress_detail(stage, done, total, "")
    }

    pub fn progress_detail(
        stage: impl Into<String>,
        done: usize,
        total: usize,
        detail: impl Into<String>,
    ) -> Self {
        Self::Progress {
            stage: stage.into(),
            done,
            total,
            detail: detail.into(),
        }
    }

    pub fn snapshot(snapshot: Snapshot) -> Self {
        Self::Snapshot(snapshot)
    }

    pub fn update_available(info: UpdateInfo) -> Self {
        Self::UpdateAvailable(info)
    }

    pub fn warning(message: impl Into<String>) -> Self {
        Self::Warning(message.into())
    }

    pub fn failed(message: impl Into<String>) -> Self {
        Self::Failed(message.into())
    }

    pub fn finished() -> Self {
        Self::Finished
    }
}

/// A thin, app-owned view of terminal input.
#[derive(Debug)]
pub enum InputEvent {
    Key(KeyEvent),
    Mouse(MouseEvent),
    Resize,
}

/// Wait for a terminal event and translate it into the interface's input type.
///
/// Non-input events (focus changes, paste notifications) are consumed but return
/// `Ok(None)`. Resize is surfaced so the next draw can reflow even if the
/// terminal backend did not already reflect it.
pub fn poll_input(timeout: Duration) -> io::Result<Option<InputEvent>> {
    if !event::poll(timeout)? {
        return Ok(None);
    }

    let input = match event::read()? {
        Event::Key(key) => Some(InputEvent::Key(key)),
        Event::Mouse(mouse) => Some(InputEvent::Mouse(mouse)),
        Event::Resize(_, _) => Some(InputEvent::Resize),
        _ => None,
    };

    Ok(input)
}
