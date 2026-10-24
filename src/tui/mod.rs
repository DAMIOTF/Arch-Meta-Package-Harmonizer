//! AMPH's interactive terminal interface.
//!
//! The TUI is deliberately self-contained. It does not reach into the scan,
//! analyzer, cleanup, or storage modules directly; the host application feeds it
//! through [`run`] and the small serializable display model in [`model`].
//!
//! # Wiring it up
//!
//! The crate needs these dependencies (the manifest is intentionally not part of
//! this directory):
//!
//! ```toml
//! ratatui = "0.30"          # required by tui-tree-widget 0.24.1
//! crossterm = "0.28"        # or newer
//! nucleo-matcher = "0.3"    # fuzzy `/` filtering
//! tui-tree-widget = "0.24.1" # Graph dependency tree
//! serde = { version = "1", features = ["derive"] }
//! anyhow = "1"
//! ```
//!
//! Declare the module and launch it with a scanner that owns its own work:
//!
//! ```ignore
//! pub mod tui;
//!
//! fn main() -> anyhow::Result<()> {
//!     tui::run(|tx| {
//!         // The closure runs on a background thread. Send progress and a final
//!         // snapshot whenever the backend is ready.
//!         let _ = tx.send(tui::ScanMessage::progress("Reading packages", 12, 64));
//!         let packages = Vec::new(); // map backend records into tui::Package values
//!         let snapshot = tui::Snapshot::new(packages, "2026-10-23 20:45");
//!         let _ = tx.send(tui::ScanMessage::snapshot(snapshot));
//!         let _ = tx.send(tui::ScanMessage::finished());
//!     })
//! }
//! ```
//!
//! # Safety boundary
//!
//! The TUI never constructs or spawns a cleanup command. [`run`] is read-only:
//! its attached handler cannot remove packages. A host that wants cleanup uses
//! [`run_with_cleanup`] or [`run_with_theme`] and supplies a [`CleanupHandler`].
//! That handler runs on the main thread while the terminal is suspended:
//!
//! 1. The TUI sends [`CleanupRequest::Preview`] with selected names.
//! 2. The host runs the dry-run command and returns [`CleanupPreview`] output.
//! 3. The TUI displays that output verbatim. Only after the user types exact
//!    `remove` does it send [`CleanupRequest::Remove`].
//! 4. The host runs the real removal and returns [`CleanupOutcome`].
//!
//! The TUI does not build either command. The handler should run
//! `pacman -Rns --print -- <names>` for preview and
//! `sudo pacman -Rns -- <names>` for removal, with `LC_ALL=C`, and return the
//! captured text plus status.
//!
//! # Update notifications
//!
//! The host checks GitHub `main` in the background on startup. When a newer
//! version exists it sends [`ScanMessage::UpdateAvailable`] with an
//! [`UpdateInfo`]. The interface then shows a one-line banner on every tab and
//! a `u` action; it never opens a modal on its own and never interrupts a scan,
//! filter, or cleanup flow.
//!
//! Installing uses an injected [`UpdateHandler`] via
//! [`run_with_theme_and_update`]. The flow is:
//!
//! 1. The user presses `u` and reviews current/latest versions.
//! 2. `Enter` queues [`UpdateRequest::Install`]; the TUI suspends the terminal
//!    and calls the host handler on the main thread.
//! 3. The handler installs the version by its own method and returns
//!    [`UpdateOutcome`].
//! 4. The TUI shows the output and states clearly that AMPH must be restarted
//!    to load the new version. A persistent `RESTART AMPH` banner remains until
//!    the session ends.
//!
//! The TUI performs no network request and runs no installer itself. Hosts that
//! do not attach an update handler may still send [`ScanMessage::UpdateAvailable`];
//! the banner appears but the install action stays disabled.
//!
//! ```ignore
//! // Give the update check its own thread so the package scan never waits.
//! let update_tx = tx.clone();
//! std::thread::spawn(move || {
//!     // Only send when the remote is actually newer (`check_for_update` filters).
//!     if let Ok(Some(found)) = update::check_for_update() {
//!         let info = tui::UpdateInfo::new(found.current, found.latest)
//!             .with_url(update::REPOSITORY);
//!         let _ = update_tx.send(tui::ScanMessage::update_available(info));
//!     }
//! });
//!
//! // Host installer, called with the terminal suspended.
//! let installer = |request: tui::UpdateRequest| match update::install_update() {
//!     Ok(()) => Ok(tui::UpdateOutcome::new(
//!         true,
//!         format!("amph {} installed", request.version()),
//!     )),
//!     Err(error) => Err(tui::UpdateError::new(format!("{error:#}"))),
//! };
//!
//! tui::run_with_theme_and_update(tui_scan, theme, cleanup, installer)
//! ```

mod app;
pub mod cleanup;
mod demo;
pub mod event;
mod fuzzy;
mod metrics;
pub mod model;
pub mod theme;
pub mod update;
pub mod views;
pub mod widgets;

use std::io::{self, Stdout};
use std::panic::{self, AssertUnwindSafe};
use std::sync::mpsc::{self, Sender};
use std::thread;
use std::time::Duration;

use anyhow::Result;
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

pub use app::{App, Tab, TimelineEntry};
pub use cleanup::{
    CleanupError, CleanupHandler, CleanupHandlerResult, CleanupOutcome, CleanupPhase,
    CleanupPreview, CleanupRequest, CleanupResponse, ReadOnlyCleanupHandler,
};
pub use event::{InputEvent, ScanMessage};
pub use model::{Package, Snapshot, TimelineItem, TimelineKind};
pub use theme::{Theme, ThemeKind};
pub use update::{
    ReadOnlyUpdateHandler, UpdateError, UpdateHandler, UpdateHandlerResult, UpdateInfo,
    UpdateOutcome, UpdateRequest,
};

/// How long the interface waits for a terminal event before redrawing.
const POLL_INTERVAL: Duration = Duration::from_millis(33);

/// Launch the read-only dashboard and run `scan` on a background thread.
///
/// This wrapper attaches a handler that can never remove packages. Use
/// [`run_with_cleanup`] when the host has a real cleanup implementation.
pub fn run(scan: impl FnOnce(Sender<ScanMessage>) + Send + 'static) -> Result<()> {
    run_internal(
        scan,
        ReadOnlyCleanupHandler,
        false,
        ReadOnlyUpdateHandler,
        false,
        None,
    )
}

/// Launch the dashboard with a real, explicit cleanup handler.
///
/// `scan` runs on a background thread. `handler` is called on the main thread
/// with the terminal suspended (raw mode/mouse/alternate-screen restored).
/// Read [`cleanup`] for the two-phase request/result contract.
pub fn run_with_cleanup<H>(
    scan: impl FnOnce(Sender<ScanMessage>) + Send + 'static,
    handler: H,
) -> Result<()>
where
    H: CleanupHandler,
{
    run_internal(scan, handler, true, ReadOnlyUpdateHandler, false, None)
}

/// Launch with an explicit initial palette and cleanup handler.
///
/// This is the CLI/main integration point for `Config::theme`: map the user's
/// configured palette to a [`ThemeKind`] and pass it here. `NO_COLOR` is still
/// honored. Use [`run_read_only_with_theme`] when no cleanup handler exists.
pub fn run_with_theme<H>(
    scan: impl FnOnce(Sender<ScanMessage>) + Send + 'static,
    theme: ThemeKind,
    handler: H,
) -> Result<()>
where
    H: CleanupHandler,
{
    run_internal(
        scan,
        handler,
        true,
        ReadOnlyUpdateHandler,
        false,
        Some(theme),
    )
}

/// Read-only launch with an explicit initial palette.
pub fn run_read_only_with_theme(
    scan: impl FnOnce(Sender<ScanMessage>) + Send + 'static,
    theme: ThemeKind,
) -> Result<()> {
    run_internal(
        scan,
        ReadOnlyCleanupHandler,
        false,
        ReadOnlyUpdateHandler,
        false,
        Some(theme),
    )
}

/// Read-only launch with an update handler and the default palette.
///
/// `update` is called on the main thread with the terminal suspended when the
/// user installs an announced update. Read [`update`] for the callback
/// contract.
pub fn run_with_update<U>(
    scan: impl FnOnce(Sender<ScanMessage>) + Send + 'static,
    update: U,
) -> Result<()>
where
    U: UpdateHandler,
{
    run_internal(scan, ReadOnlyCleanupHandler, false, update, true, None)
}

/// Read-only launch with an explicit palette and an update handler.
pub fn run_read_only_with_theme_and_update<U>(
    scan: impl FnOnce(Sender<ScanMessage>) + Send + 'static,
    theme: ThemeKind,
    update: U,
) -> Result<()>
where
    U: UpdateHandler,
{
    run_internal(
        scan,
        ReadOnlyCleanupHandler,
        false,
        update,
        true,
        Some(theme),
    )
}

/// Launch with a palette, a cleanup handler, and an update handler.
///
/// This is the full CLI/main integration point: `theme` comes from
/// `Config::theme`, `cleanup` performs the preview-first removal flow, and
/// `update` installs a version announced by [`ScanMessage::UpdateAvailable`].
/// Both handlers run with the terminal suspended. Neither runs in the TUI.
pub fn run_with_theme_and_update<C, U>(
    scan: impl FnOnce(Sender<ScanMessage>) + Send + 'static,
    theme: ThemeKind,
    cleanup: C,
    update: U,
) -> Result<()>
where
    C: CleanupHandler,
    U: UpdateHandler,
{
    run_internal(scan, cleanup, true, update, true, Some(theme))
}

/// Run a self-contained preview with a scripted sample scan.
///
/// This is useful for checking the interface without a pacman backend. The data
/// is illustrative and never touches the machine.
pub fn run_demo() -> Result<()> {
    run(|tx| {
        let _ = tx.send(ScanMessage::progress_detail(
            "Reading local package database",
            0,
            64,
            "Opening the illustrative sample",
        ));
        thread::sleep(Duration::from_millis(500));
        let _ = tx.send(ScanMessage::progress_detail(
            "Resolving dependency edges",
            21,
            64,
            "Sample data only",
        ));
        thread::sleep(Duration::from_millis(500));
        let _ = tx.send(ScanMessage::progress_detail(
            "Measuring package footprints",
            47,
            64,
            "Sample data only",
        ));
        thread::sleep(Duration::from_millis(450));
        let _ = tx.send(ScanMessage::snapshot(Snapshot::demo()));
        let _ = tx.send(ScanMessage::finished());
    })
}

fn run_internal<C, U>(
    scan: impl FnOnce(Sender<ScanMessage>) + Send + 'static,
    mut cleanup_handler: C,
    cleanup_enabled: bool,
    mut update_handler: U,
    update_enabled: bool,
    theme: Option<ThemeKind>,
) -> Result<()>
where
    C: CleanupHandler,
    U: UpdateHandler,
{
    install_panic_hook();

    let mut session = TerminalSession::start()?;
    let mut app = match theme {
        Some(theme) => App::new_with_theme(theme),
        None => App::new(),
    };
    app.set_cleanup_enabled(cleanup_enabled);
    app.set_update_enabled(update_enabled);

    let (scan_tx, scan_rx) = mpsc::channel::<ScanMessage>();
    let worker = thread::Builder::new()
        .name("amph-scan".to_string())
        .spawn(move || scan(scan_tx))?;

    app.scan_started();

    loop {
        drain_scan(&scan_rx, &mut app);

        if worker.is_finished() {
            app.scan_thread_finished();
        }

        if let Some(input) = event::poll_input(POLL_INTERVAL)? {
            app.on_input(input);
        }

        // Pick up anything the scanner sent while the terminal was blocking.
        drain_scan(&scan_rx, &mut app);

        while let Some(request) = app.take_cleanup_request() {
            let result = run_cleanup_handler(&mut session, &mut cleanup_handler, request.clone())?;
            app.on_cleanup_handler_result(&request, result);
        }

        while let Some(request) = app.take_update_request() {
            let result = run_update_handler(&mut session, &mut update_handler, request.clone())?;
            app.on_update_handler_result(&request, result);
        }

        if app.take_clear_request() {
            session.terminal.clear()?;
        }

        session
            .terminal
            .draw(|frame| views::draw(frame, &mut app))?;

        if app.should_quit() {
            break;
        }
    }

    if worker.is_finished() {
        let _ = worker.join();
    }

    Ok(())
}

fn run_cleanup_handler<H>(
    session: &mut TerminalSession,
    handler: &mut H,
    request: CleanupRequest,
) -> Result<CleanupHandlerResult>
where
    H: CleanupHandler,
{
    session.suspend()?;
    let result = panic::catch_unwind(AssertUnwindSafe(|| handler.handle(request)));
    let resumed = session.resume();

    match resumed {
        Ok(()) => Ok(match result {
            Ok(result) => result,
            Err(_) => Err(CleanupError::new(
                "Cleanup handler panicked; no result was applied.",
            )),
        }),
        Err(error) => {
            restore_terminal();
            Err(error)
        }
    }
}

fn run_update_handler<U>(
    session: &mut TerminalSession,
    handler: &mut U,
    request: UpdateRequest,
) -> Result<UpdateHandlerResult>
where
    U: UpdateHandler,
{
    session.suspend()?;
    let result = panic::catch_unwind(AssertUnwindSafe(|| handler.handle(request)));
    let resumed = session.resume();

    match resumed {
        Ok(()) => Ok(match result {
            Ok(result) => result,
            Err(_) => Err(UpdateError::new(
                "Update handler panicked; no result was applied. The installed version is unchanged.",
            )),
        }),
        Err(error) => {
            restore_terminal();
            Err(error)
        }
    }
}

fn drain_scan(rx: &mpsc::Receiver<ScanMessage>, app: &mut App) {
    while let Ok(message) = rx.try_recv() {
        app.on_scan_message(message);
    }
}

fn init_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;

    let mut stdout = io::stdout();
    if let Err(error) = execute!(stdout, EnterAlternateScreen, EnableMouseCapture) {
        let _ = disable_raw_mode();
        return Err(error.into());
    }

    let backend = CrosstermBackend::new(stdout);
    let mut terminal = match Terminal::new(backend) {
        Ok(terminal) => terminal,
        Err(error) => {
            restore_terminal();
            return Err(error.into());
        }
    };

    if let Err(error) = terminal.clear().and_then(|()| terminal.hide_cursor()) {
        restore_terminal();
        return Err(error.into());
    }

    Ok(terminal)
}

fn install_panic_hook() {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        // Release builds use `panic = "abort"`, so even a scanner panic must
        // restore the terminal before the process dies. In unwind builds a
        // scanner panic is reported as a scan failure and the UI keeps its
        // alternate screen intact.
        let on_scan_thread = std::thread::current().name() == Some("amph-scan");
        if !on_scan_thread || cfg!(panic = "abort") {
            restore_terminal();
        }
        previous(info);
    }));
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let mut stdout = io::stdout();
    let _ = execute!(
        stdout,
        DisableMouseCapture,
        crossterm::cursor::Show,
        LeaveAlternateScreen
    );
}

struct TerminalSession {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    suspended: bool,
}

impl TerminalSession {
    fn start() -> Result<Self> {
        Ok(Self {
            terminal: init_terminal()?,
            suspended: false,
        })
    }

    fn suspend(&mut self) -> Result<()> {
        if self.suspended {
            return Ok(());
        }
        let _ = self.terminal.show_cursor();
        restore_terminal();
        self.suspended = true;
        Ok(())
    }

    fn resume(&mut self) -> Result<()> {
        if !self.suspended {
            return Ok(());
        }

        enable_raw_mode()?;
        let mut stdout = io::stdout();
        if let Err(error) = execute!(stdout, EnterAlternateScreen, EnableMouseCapture) {
            restore_terminal();
            return Err(error.into());
        }

        if let Err(error) = self
            .terminal
            .clear()
            .and_then(|()| self.terminal.hide_cursor())
        {
            restore_terminal();
            return Err(error.into());
        }

        self.suspended = false;
        Ok(())
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        restore_terminal();
    }
}
