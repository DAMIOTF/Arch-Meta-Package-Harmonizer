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
//! The host selects a persisted update channel (`main` or `development`) and
//! sends it first with [`ScanMessage::UpdateChannel`]. It then checks that
//! branch in the background; when a newer version exists it sends
//! [`ScanMessage::UpdateAvailable`] with an [`UpdateInfo`] carrying the same
//! branch. The interface shows a one-line banner on every tab and a `u` action;
//! it never opens a modal on its own and never interrupts a scan, filter, or
//! cleanup flow. A notice for another channel is ignored so the selected branch
//! stays authoritative.
//!
//! Pressing `shift+v` opens the channel picker. It lists both branches, marks
//! the current one, and installs the highlighted branch when `Enter` is chosen.
//! Selecting the already-current channel is a safe no-op with a clear message.
//!
//! Installing, including switching channels, uses an injected [`UpdateHandler`]
//! via [`run_with_theme_and_update`]. The flow is:
//!
//! 1. The user presses `u` to review an announced version, or `shift+v` to pick
//!    a channel. Both paths use the branch carried by the update or the picker.
//! 2. `Enter` queues [`UpdateRequest::Install`] with the version and branch; the
//!    TUI suspends the terminal and calls the host handler on the main thread.
//! 3. The handler persists the branch, installs the build by its own method, and
//!    returns [`UpdateOutcome`].
//! 4. The TUI shows the output and states clearly that AMPH must be restarted
//!    to load the new version. A persistent `RESTART AMPH` banner remains until
//!    the session ends.
//!
//! The TUI performs no network request and runs no installer itself. Hosts that
//! do not attach an update handler may still send [`ScanMessage::UpdateAvailable`];
//! the banner appears but the install action and channel switch stay disabled.
//!
//! ```ignore
//! // Report the persisted channel before any update check result.
//! let branch = update::current_branch();
//! let _ = tx.send(tui::ScanMessage::update_channel(branch.as_str()));
//!
//! // Give the update check its own thread so the package scan never waits.
//! let update_tx = tx.clone();
//! std::thread::spawn(move || {
//!     // Only send when the remote is actually newer (`check_for_update` filters).
//!     if let Ok(Some(found)) = update::check_for_update_on(branch) {
//!         let info = tui::UpdateInfo::new(found.current, found.latest)
//!             .with_branch(found.branch)
//!             .with_url(update::REPOSITORY);
//!         let _ = update_tx.send(tui::ScanMessage::update_available(info));
//!     }
//! });
//!
//! // Host installer, called with the terminal suspended.
//! let installer = |request: tui::UpdateRequest| {
//!     match update::UpdateBranch::parse(request.branch()) {
//!         Ok(branch) => match update::install_update_from_branch(branch) {
//!             Ok(()) => Ok(tui::UpdateOutcome::new(
//!                 true,
//!                 format!("amph {} installed", request.version()),
//!             )),
//!             Err(error) => Err(tui::UpdateError::new(format!("{error:#}"))),
//!         },
//!         Err(error) => Err(tui::UpdateError::new(format!("{error:#}"))),
//!     }
//! };
//!
//! tui::run_with_theme_and_update(tui_scan, theme, cleanup, installer)
//! ```
//!
//! # Plugin manager
//!
//! `Shift+P` opens a workspace modal without adding an eighth tab. It has three
//! sections: Installed, Add from GitHub, and Updates. A pasted GitHub URL is
//! normalized and shown for review before any install request is queued.
//! Removal always needs an explicit confirmation. Bracketed paste is enabled by
//! the terminal session, so a URL arrives as one [`InputEvent::Paste`] value.
//!
//! The host supplies rows with [`ScanMessage::Plugins`] and can refresh them
//! from operation results. [`plugin_host`] maps `InstalledPlugin` and
//! `PluginUpdate` rows from `amph::plugin` into the bounded [`PluginRow`] DTO,
//! and [`StorePluginHandler`] maps each action to the matching
//! `PluginStore::{list, install, check_all, update_all, analyze_json,
//! set_enabled, remove}` method. Theme plugins are declarative; functionality
//! plugins use Wasm ABI v1 and run in the backend sandbox. The backend verifies
//! GitHub release asset digests before publishing an install.
//!
//! Pressing `a` on an enabled functionality row asks for confirmation and
//! queues [`PluginAction::Analyze`] with the current package snapshot. Pressing
//! `T` on an enabled theme row applies that plugin's validated palette and
//! semantic colors to this session only; it does not save configuration.
//!
//! The TUI queues [`PluginAction`] values; the full wrapper
//! [`run_with_theme_and_update_and_plugins`] drains them into an injected
//! [`PluginHandler`] with the terminal suspended. A handler returns
//! [`PluginOutcome`] (including authoritative refreshed rows) or
//! [`PluginError`].
//!
//! ```ignore
//! let plugins = |action: tui::PluginAction| {
//!     // Host code fetches, installs into its WASM sandbox, checks updates, or
//!     // removes a plugin. It must not ask the TUI to perform that work.
//!     Ok(tui::PluginOutcome::new(true, "Plugin action finished").with_plugins(rows))
//! };
//!
//! tui::run_with_theme_and_update_and_plugins(tui_scan, theme, cleanup, installer, plugins)
//! ```

mod app;
pub mod cleanup;
mod demo;
pub mod event;
mod fuzzy;
mod metrics;
pub mod model;
pub mod plugin_host;
pub mod plugins;
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
use crossterm::event::{
    DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

pub use app::{App, PluginSection, Tab, TimelineEntry};
pub use cleanup::{
    CleanupError, CleanupHandler, CleanupHandlerResult, CleanupOutcome, CleanupPhase,
    CleanupPreview, CleanupRequest, CleanupResponse, ReadOnlyCleanupHandler,
};
pub use event::{InputEvent, ScanMessage};
pub use model::{Package, Snapshot, TimelineItem, TimelineKind};
pub use plugin_host::{
    StorePluginHandler, rows_from_check, rows_from_installed, rows_from_updates,
};
pub use plugins::{
    PLUGIN_ANALYZE_API, PLUGIN_URL_MAX, PluginAction, PluginError, PluginHandler,
    PluginHandlerResult, PluginOutcome, PluginRow, PluginRowKind, PluginUrlError,
    ReadOnlyPluginHandler, normalize_github_url,
};
pub use theme::{Theme, ThemeApplyError, ThemeColorField, ThemeKind, parse_ansi_color};
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
        ReadOnlyPluginHandler,
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
    run_internal(
        scan,
        handler,
        true,
        ReadOnlyUpdateHandler,
        false,
        ReadOnlyPluginHandler,
        false,
        None,
    )
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
        ReadOnlyPluginHandler,
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
        ReadOnlyPluginHandler,
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
    run_internal(
        scan,
        ReadOnlyCleanupHandler,
        false,
        update,
        true,
        ReadOnlyPluginHandler,
        false,
        None,
    )
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
        ReadOnlyPluginHandler,
        false,
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
    run_internal(
        scan,
        cleanup,
        true,
        update,
        true,
        ReadOnlyPluginHandler,
        false,
        Some(theme),
    )
}

/// Launch with a palette, cleanup handler, update handler, and plugin handler.
///
/// This is the full plugin-manager integration point. The TUI queues a
/// [`PluginAction`] only after review or explicit confirmation; the injected
/// [`PluginHandler`] runs with the terminal suspended and returns the
/// authoritative refreshed [`PluginRow`] list through [`PluginOutcome`]. The
/// TUI never installs, updates, or removes a plugin itself.
///
/// Initial plugin rows can be sent from the scanner with
/// [`ScanMessage::Plugins`].
pub fn run_with_theme_and_update_and_plugins<C, U, P>(
    scan: impl FnOnce(Sender<ScanMessage>) + Send + 'static,
    theme: ThemeKind,
    cleanup: C,
    update: U,
    plugins: P,
) -> Result<()>
where
    C: CleanupHandler,
    U: UpdateHandler,
    P: PluginHandler,
{
    run_internal(
        scan,
        cleanup,
        true,
        update,
        true,
        plugins,
        true,
        Some(theme),
    )
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

#[allow(clippy::too_many_arguments)]
fn run_internal<C, U, P>(
    scan: impl FnOnce(Sender<ScanMessage>) + Send + 'static,
    mut cleanup_handler: C,
    cleanup_enabled: bool,
    mut update_handler: U,
    update_enabled: bool,
    mut plugin_handler: P,
    plugins_enabled: bool,
    theme: Option<ThemeKind>,
) -> Result<()>
where
    C: CleanupHandler,
    U: UpdateHandler,
    P: PluginHandler,
{
    install_panic_hook();

    let mut session = TerminalSession::start()?;
    let mut app = match theme {
        Some(theme) => App::new_with_theme(theme),
        None => App::new(),
    };
    app.set_cleanup_enabled(cleanup_enabled);
    app.set_update_enabled(update_enabled);
    app.set_plugins_enabled(plugins_enabled);

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

        // A host handler runs with the terminal suspended. Draw its busy modal
        // first so the user sees what is about to happen instead of a bare
        // shell while a network install or update check runs.
        if app.has_pending_handler_work() {
            session
                .terminal
                .draw(|frame| views::draw(frame, &mut app))?;
        }

        while let Some(request) = app.take_cleanup_request() {
            let result = run_cleanup_handler(&mut session, &mut cleanup_handler, request.clone())?;
            app.on_cleanup_handler_result(&request, result);
        }

        while let Some(request) = app.take_update_request() {
            let result = run_update_handler(&mut session, &mut update_handler, request.clone())?;
            app.on_update_handler_result(&request, result);
        }

        while let Some(action) = app.take_plugin_action() {
            let result = run_plugin_handler(&mut session, &mut plugin_handler, action.clone())?;
            app.on_plugin_handler_result(&action, result);
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

fn run_plugin_handler<P>(
    session: &mut TerminalSession,
    handler: &mut P,
    action: PluginAction,
) -> Result<PluginHandlerResult>
where
    P: PluginHandler,
{
    session.suspend()?;
    let result = panic::catch_unwind(AssertUnwindSafe(|| handler.handle(action)));
    let resumed = session.resume();

    match resumed {
        Ok(()) => Ok(match result {
            Ok(result) => result,
            Err(_) => Err(PluginError::new(
                "Plugin handler panicked; no result was applied. The plugin list may be unchanged.",
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
    if let Err(error) = execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    ) {
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
        DisableBracketedPaste,
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
        if let Err(error) = execute!(
            stdout,
            EnterAlternateScreen,
            EnableMouseCapture,
            EnableBracketedPaste
        ) {
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
