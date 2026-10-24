use std::collections::{BTreeSet, VecDeque};
use std::time::{Duration, Instant};

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use ratatui::widgets::TableState;
use tui_tree_widget::TreeState;

use crate::tui::cleanup::{CleanupError, CleanupPhase, CleanupRequest, CleanupResponse};

use crate::tui::event::{InputEvent, ScanMessage};
use crate::tui::metrics::{self, Analysis, Check, GraphData, Metrics};
use crate::tui::model::{Package, Snapshot, TimelineItem, format_size};
use crate::tui::theme::{Theme, ThemeKind};
use crate::tui::update::{
    DEFAULT_CHANNEL, UPDATE_CHANNELS, UpdateError, UpdateInfo, UpdateOutcome, UpdateRequest,
    normalize_channel,
};

pub const TAB_COUNT: usize = 7;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Tab {
    Dashboard,
    Packages,
    Graph,
    Cleanup,
    Harmony,
    Checks,
    Timeline,
}

impl Tab {
    pub const ALL: [Tab; TAB_COUNT] = [
        Tab::Dashboard,
        Tab::Packages,
        Tab::Graph,
        Tab::Cleanup,
        Tab::Harmony,
        Tab::Checks,
        Tab::Timeline,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Dashboard => "Dashboard",
            Tab::Packages => "Packages",
            Tab::Graph => "Graph",
            Tab::Cleanup => "Cleanup",
            Tab::Harmony => "Harmony",
            Tab::Checks => "Checks",
            Tab::Timeline => "Timeline",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|tab| *tab == self).unwrap_or(0)
    }

    pub fn from_index(index: usize) -> Option<Self> {
        Self::ALL.get(index).copied()
    }

    pub fn supports_filter(self) -> bool {
        matches!(
            self,
            Tab::Packages | Tab::Graph | Tab::Cleanup | Tab::Checks | Tab::Timeline
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageSort {
    Size,
    Name,
    Reason,
}

impl PackageSort {
    pub fn label(self) -> &'static str {
        match self {
            PackageSort::Size => "SIZE",
            PackageSort::Name => "NAME",
            PackageSort::Reason => "REASON",
        }
    }

    fn next(self) -> Self {
        match self {
            PackageSort::Size => PackageSort::Name,
            PackageSort::Name => PackageSort::Reason,
            PackageSort::Reason => PackageSort::Size,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScanPhase {
    Starting,
    Running,
    Complete,
    Failed,
}

#[derive(Debug, Clone)]
pub struct ScanState {
    pub phase: ScanPhase,
    pub stage: String,
    pub done: usize,
    pub total: usize,
    pub detail: String,
    pub error: Option<String>,
    pub started: Instant,
    pub finalized: bool,
    pub received_snapshot: bool,
    last_stage: String,
    last_milestone: u8,
}

impl ScanState {
    fn new() -> Self {
        Self {
            phase: ScanPhase::Starting,
            stage: "Waiting for scanner".to_string(),
            done: 0,
            total: 0,
            detail: String::new(),
            error: None,
            started: Instant::now(),
            finalized: false,
            received_snapshot: false,
            last_stage: String::new(),
            last_milestone: 0,
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self.phase, ScanPhase::Starting | ScanPhase::Running)
    }

    pub fn percent(&self) -> Option<u16> {
        if self.total == 0 {
            return None;
        }
        Some(((self.done.min(self.total) * 100) / self.total) as u16)
    }

    pub fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }
}

#[derive(Debug, Clone, Default)]
pub struct Filters {
    pub packages: String,
    pub graph: String,
    pub cleanup: String,
    pub checks: String,
    pub timeline: String,
}

impl Filters {
    pub fn get(&self, tab: Tab) -> &str {
        match tab {
            Tab::Packages => &self.packages,
            Tab::Graph => &self.graph,
            Tab::Cleanup => &self.cleanup,
            Tab::Checks => &self.checks,
            Tab::Timeline => &self.timeline,
            Tab::Dashboard | Tab::Harmony => "",
        }
    }

    pub fn set(&mut self, tab: Tab, value: String) {
        match tab {
            Tab::Packages => self.packages = value,
            Tab::Graph => self.graph = value,
            Tab::Cleanup => self.cleanup = value,
            Tab::Checks => self.checks = value,
            Tab::Timeline => self.timeline = value,
            Tab::Dashboard | Tab::Harmony => {}
        }
    }
}

#[derive(Debug, Clone)]
pub struct FilterEdit {
    pub tab: Tab,
    original: String,
}

#[derive(Debug, Clone)]
pub struct CleanupPreviewState {
    pub packages: Vec<String>,
    pub output: String,
    pub can_remove: bool,
    pub note: Option<String>,
    pub scroll: u16,
    pub typed: String,
    pub error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CleanupResultState {
    pub packages: Vec<String>,
    pub success: bool,
    pub output: String,
    pub note: Option<String>,
    pub scroll: u16,
}

#[derive(Debug, Clone)]
pub struct CleanupErrorState {
    pub title: String,
    pub message: String,
    pub removal: bool,
}

#[derive(Debug, Clone)]
pub enum Overlay {
    None,
    Help,
    Detail,
    CleanupBusy,
    CleanupPreview(CleanupPreviewState),
    CleanupResult(CleanupResultState),
    CleanupError(CleanupErrorState),
    UpdatePrompt,
    UpdateBusy,
    UpdateResult(UpdateResultState),
    /// Channel selection opened with `Shift+V`. The highlighted row lives in
    /// [`App::channel_cursor`] so a host channel update cannot move it.
    ChannelPicker,
}

impl Overlay {
    pub fn is_cleanup(&self) -> bool {
        matches!(
            self,
            Overlay::CleanupBusy
                | Overlay::CleanupPreview(_)
                | Overlay::CleanupResult(_)
                | Overlay::CleanupError(_)
        )
    }
}

/// Where an announced update stands in the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdatePhase {
    /// A newer version was reported; the install action is available.
    Available,
    /// The host installer reported success; a restart is required.
    Installed,
}

/// The update banner's persistent state. This outlives the modal prompt so the
/// install action (or the restart reminder) stays visible on every tab.
#[derive(Debug, Clone)]
pub struct UpdateNotice {
    info: UpdateInfo,
    phase: UpdatePhase,
}

impl UpdateNotice {
    fn available(info: UpdateInfo) -> Self {
        Self {
            info,
            phase: UpdatePhase::Available,
        }
    }

    pub fn info(&self) -> &UpdateInfo {
        &self.info
    }

    pub fn phase(&self) -> UpdatePhase {
        self.phase
    }

    pub fn installed(&self) -> bool {
        self.phase == UpdatePhase::Installed
    }
}

/// The install handler's result, as shown in the modal and banner afterwards.
#[derive(Debug, Clone)]
pub struct UpdateResultState {
    pub version: String,
    pub success: bool,
    pub output: String,
    pub note: Option<String>,
    pub scroll: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct RowHit {
    pub rect: Rect,
    pub index: usize,
}

#[derive(Debug, Clone, Default)]
pub struct UiHits {
    pub tabs: Vec<(Rect, Tab)>,
    pub rows: Vec<RowHit>,
    /// The update banner, when one is on screen.
    pub update: Option<Rect>,
    /// The Install control inside the update prompt.
    pub update_install: Option<Rect>,
}

impl UiHits {
    pub fn clear(&mut self) {
        self.tabs.clear();
        self.rows.clear();
        self.update = None;
        self.update_install = None;
    }
}

#[derive(Debug, Clone, Default)]
pub struct LayoutCache {
    pub viewport: Rect,
    pub tab_bar: Rect,
    pub body: Rect,
    pub detail: Rect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastTone {
    Info,
    Success,
    Warning,
    Danger,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub text: String,
    pub tone: ToastTone,
    pub until: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionEventKind {
    Boot,
    Scan,
    Snapshot,
    Interface,
    Safety,
    Warning,
    Error,
}

impl SessionEventKind {
    pub fn label(self) -> &'static str {
        match self {
            SessionEventKind::Boot => "BOOT",
            SessionEventKind::Scan => "SCAN",
            SessionEventKind::Snapshot => "SNAPSHOT",
            SessionEventKind::Interface => "UI",
            SessionEventKind::Safety => "SAFETY",
            SessionEventKind::Warning => "WARN",
            SessionEventKind::Error => "ERROR",
        }
    }

    pub fn marker(self) -> &'static str {
        match self {
            SessionEventKind::Boot => "◇",
            SessionEventKind::Scan => "◌",
            SessionEventKind::Snapshot => "◆",
            SessionEventKind::Interface => "·",
            SessionEventKind::Safety => "▸",
            SessionEventKind::Warning => "▲",
            SessionEventKind::Error => "■",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SessionEvent {
    pub at: f64,
    pub kind: SessionEventKind,
    pub title: String,
    pub detail: String,
}

/// A merged Timeline row: pacman history from the snapshot plus TUI session
/// events. Transaction rows are supplied by the host scanner.
#[derive(Debug, Clone)]
pub enum TimelineEntry {
    Transaction(TimelineItem),
    Session(SessionEvent),
}

impl TimelineEntry {
    pub fn title(&self) -> &str {
        match self {
            TimelineEntry::Transaction(item) => &item.title,
            TimelineEntry::Session(event) => &event.title,
        }
    }

    pub fn detail(&self) -> &str {
        match self {
            TimelineEntry::Transaction(item) => &item.detail,
            TimelineEntry::Session(event) => &event.detail,
        }
    }

    pub fn timestamp(&self) -> String {
        match self {
            TimelineEntry::Transaction(item) => item.timestamp.clone(),
            TimelineEntry::Session(event) => {
                format!("T+{}", crate::tui::widgets::format_elapsed(event.at))
            }
        }
    }

    pub fn kind_label(&self) -> &'static str {
        match self {
            TimelineEntry::Transaction(item) => item.kind.label(),
            TimelineEntry::Session(event) => event.kind.label(),
        }
    }

    pub fn marker(&self) -> &'static str {
        match self {
            TimelineEntry::Transaction(item) => item.kind.marker(),
            TimelineEntry::Session(event) => event.kind.marker(),
        }
    }

    pub fn searchable(&self) -> String {
        format!(
            "{} {} {} {}",
            self.timestamp(),
            self.title(),
            self.detail(),
            self.kind_label()
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct Inventory {
    pub packages: Vec<Package>,
    pub metrics: Metrics,
    pub graph: GraphData,
    pub checks: Vec<Check>,
    pub scanned_at: String,
    /// Stored health scores, oldest first, supplied by the host scanner.
    pub health_history: Vec<u8>,
    /// Pacman/package transactions supplied by the host scanner.
    pub timeline: Vec<TimelineItem>,
}

impl Inventory {
    fn from_parts(
        packages: Vec<Package>,
        scanned_at: String,
        health_history: Vec<u8>,
        timeline: Vec<TimelineItem>,
    ) -> Self {
        let Analysis {
            metrics,
            graph,
            checks,
        } = metrics::analyze(&packages);
        Self {
            packages,
            metrics,
            graph,
            checks,
            scanned_at,
            health_history,
            timeline,
        }
    }

    fn preview() -> Self {
        let snapshot = Snapshot::demo();
        Self::from_parts(
            snapshot.packages,
            snapshot.scanned_at,
            snapshot.health_history,
            snapshot.timeline,
        )
    }
}

pub struct App {
    pub(crate) theme: Theme,
    pub(crate) active_tab: Tab,
    pub(crate) inventory: Inventory,
    pub(crate) filters: Filters,
    pub(crate) package_sort: PackageSort,
    pub(crate) package_state: TableState,
    pub(crate) graph_tree_state: TreeState<String>,
    pub(crate) cleanup_state: TableState,
    pub(crate) checks_state: TableState,
    pub(crate) timeline_selected: usize,
    pub(crate) cleanup_selected: BTreeSet<String>,
    pub(crate) cleanup_queue: VecDeque<CleanupRequest>,
    pub(crate) cleanup_enabled: bool,
    pub(crate) update: Option<UpdateNotice>,
    pub(crate) update_queue: VecDeque<UpdateRequest>,
    pub(crate) update_enabled: bool,
    /// The channel the host has persisted, mirrored locally for display.
    pub(crate) update_channel: String,
    /// Highlighted row while the channel picker is open.
    pub(crate) channel_cursor: usize,
    pub(crate) filter_edit: Option<FilterEdit>,
    pub(crate) overlay: Overlay,
    pub(crate) scan: ScanState,
    pub(crate) events: Vec<SessionEvent>,
    booted: Instant,
    pub(crate) toast: Option<Toast>,
    pub(crate) detail_visible: bool,
    should_quit: bool,
    clear_request: bool,
    pub(crate) tick: u64,
    pub(crate) hits: UiHits,
    pub(crate) layout: LayoutCache,
    pub(crate) sample: bool,
    pub(crate) stale_snapshot: bool,
}

impl App {
    pub fn new() -> Self {
        Self::new_internal(Theme::from_environment())
    }

    /// Build the app with an explicit starting palette, typically loaded from
    /// the user configuration by the CLI. `NO_COLOR` is still honored.
    pub fn new_with_theme(theme: ThemeKind) -> Self {
        Self::new_internal(Theme::from_kind(theme))
    }

    fn new_internal(theme: Theme) -> Self {
        let inventory = Inventory::preview();
        let mut app = Self {
            theme,
            active_tab: Tab::Dashboard,
            inventory,
            filters: Filters::default(),
            package_sort: PackageSort::Size,
            package_state: TableState::default(),
            graph_tree_state: TreeState::default(),
            cleanup_state: TableState::default(),
            checks_state: TableState::default(),
            timeline_selected: 0,
            cleanup_selected: BTreeSet::new(),
            cleanup_queue: VecDeque::new(),
            cleanup_enabled: false,
            update: None,
            update_queue: VecDeque::new(),
            update_enabled: false,
            update_channel: DEFAULT_CHANNEL.to_string(),
            channel_cursor: 0,
            filter_edit: None,
            overlay: Overlay::None,
            scan: ScanState::new(),
            events: Vec::new(),
            booted: Instant::now(),
            toast: None,
            detail_visible: true,
            should_quit: false,
            clear_request: false,
            tick: 0,
            hits: UiHits::default(),
            layout: LayoutCache::default(),
            sample: true,
            stale_snapshot: false,
        };

        app.sync_selections();
        app.log_event(
            SessionEventKind::Boot,
            "Interface ready",
            "AMPH TUI initialized. No system command has been run.",
        );
        app.log_event(
            SessionEventKind::Boot,
            "Sample preview loaded",
            format!(
                "{} illustrative packages · awaiting scanner",
                app.inventory.packages.len()
            ),
        );
        app
    }

    pub(crate) fn set_cleanup_enabled(&mut self, enabled: bool) {
        self.cleanup_enabled = enabled;
    }

    pub(crate) fn set_update_enabled(&mut self, enabled: bool) {
        self.update_enabled = enabled;
    }

    pub fn should_quit(&self) -> bool {
        self.should_quit
    }

    pub fn take_clear_request(&mut self) -> bool {
        let requested = self.clear_request;
        self.clear_request = false;
        requested
    }

    // ---------------------------------------------------------------------
    // Scan lifecycle
    // ---------------------------------------------------------------------

    pub fn scan_started(&mut self) {
        self.scan.phase = ScanPhase::Starting;
        self.scan.stage = "Starting scanner".to_string();
        self.scan.started = Instant::now();
        self.scan.error = None;
        self.log_event(
            SessionEventKind::Scan,
            "Scanner dispatched",
            "The scan runs on a background thread; the interface stays responsive.",
        );
    }

    pub fn on_scan_message(&mut self, message: ScanMessage) {
        match message {
            ScanMessage::Progress {
                stage,
                done,
                total,
                detail,
            } => {
                if self.scan.phase == ScanPhase::Failed {
                    return;
                }
                self.scan.phase = ScanPhase::Running;
                self.scan.stage = stage.clone();
                self.scan.done = done;
                self.scan.total = total;
                self.scan.detail = detail;
                self.scan.error = None;

                let milestone = done
                    .min(total)
                    .saturating_mul(100)
                    .checked_div(total)
                    .unwrap_or(0) as u8;
                let threshold = [25u8, 50, 75, 100]
                    .into_iter()
                    .filter(|value| *value <= milestone)
                    .max()
                    .unwrap_or(0);

                if stage != self.scan.last_stage || threshold > self.scan.last_milestone {
                    self.scan.last_stage = stage.clone();
                    self.scan.last_milestone = threshold;
                    let detail_text = if self.scan.detail.is_empty() {
                        format!("{done} / {total} steps")
                    } else {
                        self.scan.detail.clone()
                    };
                    self.log_event(SessionEventKind::Scan, stage, detail_text);
                }
            }
            ScanMessage::Snapshot(snapshot) => {
                self.install_snapshot(snapshot);
            }
            ScanMessage::UpdateChannel(branch) => {
                self.on_update_channel(branch);
            }
            ScanMessage::UpdateAvailable(info) => {
                self.on_update_available(info);
            }
            ScanMessage::Warning(message) => {
                self.toast(&message, ToastTone::Warning);
                self.log_event(SessionEventKind::Warning, "Scanner warning", message);
            }
            ScanMessage::Failed(message) => {
                self.scan.phase = ScanPhase::Failed;
                self.scan.stage = "Scan failed".to_string();
                self.scan.error = Some(message.clone());
                self.toast(
                    "Scan failed — preview data remains visible",
                    ToastTone::Danger,
                );
                self.log_event(SessionEventKind::Error, "Scan failed", message);
            }
            ScanMessage::Finished => {
                self.scan.finalized = true;
                if self.scan.is_active() {
                    if self.scan.received_snapshot {
                        self.scan.phase = ScanPhase::Complete;
                        self.scan.stage = "Scan complete".to_string();
                    } else {
                        self.scan.phase = ScanPhase::Failed;
                        self.scan.stage = "Scanner finished without a snapshot".to_string();
                        self.scan.error = Some(
                            "The scanner returned but did not send a Snapshot message.".to_string(),
                        );
                        self.toast(
                            "Scanner finished without a snapshot — showing preview",
                            ToastTone::Warning,
                        );
                        self.log_event(
                            SessionEventKind::Warning,
                            "No snapshot received",
                            "The scanner returned without sending Snapshot.",
                        );
                    }
                }
            }
        }
    }

    pub fn scan_thread_finished(&mut self) {
        if self.scan.finalized {
            return;
        }
        self.scan.finalized = true;
        if self.scan.is_active() {
            self.scan.phase = ScanPhase::Failed;
            self.scan.stage = "Scanner exited unexpectedly".to_string();
            self.scan.error =
                Some("The scanner thread exited without sending Finished.".to_string());
            self.toast(
                "Scanner exited early — preview data remains visible",
                ToastTone::Warning,
            );
            self.log_event(
                SessionEventKind::Warning,
                "Scanner exited early",
                "No Finished message was received. The preview snapshot is still shown.",
            );
        }
    }

    fn install_snapshot(&mut self, snapshot: Snapshot) {
        let total_size = snapshot.total_size();
        let Snapshot {
            packages,
            scanned_at,
            health_history,
            timeline,
        } = snapshot;
        let count = packages.len();
        let scanned_at = if scanned_at.trim().is_empty() {
            "snapshot received".to_string()
        } else {
            scanned_at
        };
        let transaction_count = timeline.len();

        let cleanup_was_open = self.overlay.is_cleanup();
        self.inventory =
            Inventory::from_parts(packages, scanned_at.clone(), health_history, timeline);
        self.sample = false;
        self.cleanup_selected.clear();
        self.cleanup_queue.clear();
        self.package_state = TableState::default();
        self.graph_tree_state = TreeState::default();
        self.cleanup_state = TableState::default();
        self.stale_snapshot = false;
        self.checks_state = TableState::default();
        self.timeline_selected = 0;
        if cleanup_was_open {
            self.overlay = Overlay::None;
        }

        self.scan.phase = ScanPhase::Complete;
        self.scan.stage = "Snapshot received".to_string();
        self.scan.done = count;
        self.scan.total = count;
        self.scan.detail = format!("{count} packages · {}", format_size(total_size));
        self.scan.received_snapshot = true;
        self.scan.error = None;
        self.sync_selections();

        self.log_event(
            SessionEventKind::Snapshot,
            "Snapshot installed",
            format!(
                "{count} packages · {} · scanned at {scanned_at} · {transaction_count} transaction(s)",
                format_size(total_size)
            ),
        );
        if cleanup_was_open {
            self.log_event(
                SessionEventKind::Safety,
                "Cleanup preview closed",
                "A new snapshot replaced the package set, so the preview was discarded.",
            );
        }
        self.toast(
            &format!("Snapshot ready · {count} packages"),
            ToastTone::Success,
        );
    }

    // ---------------------------------------------------------------------
    // Update flow
    // ---------------------------------------------------------------------

    fn on_update_channel(&mut self, branch: String) {
        let branch = normalize_channel(branch);
        if branch == self.update_channel {
            return;
        }
        self.update_channel = branch.clone();
        // The picker cursor follows the host only while the picker is closed.
        // Moving it during navigation would make the highlighted row jump.
        if matches!(self.overlay, Overlay::None) {
            self.channel_cursor = channel_index(&branch);
        }
        self.log_event(
            SessionEventKind::Interface,
            "Update channel",
            format!("{branch} is the selected update channel."),
        );
    }

    fn on_update_available(&mut self, info: UpdateInfo) {
        if !info.is_actionable() {
            self.log_event(
                SessionEventKind::Warning,
                "Update check returned no version",
                "The background check did not provide a target version, so no update action is offered.",
            );
            return;
        }

        // An installer is in flight; its result owns the screen and the banner.
        if matches!(self.overlay, Overlay::UpdateBusy) {
            return;
        }

        // A check may finish after the user switched channels. Never show an
        // announcement for a channel that is no longer selected: choosing a
        // branch always installs, and a stale result would be misleading.
        if info.branch() != self.update_channel {
            self.log_event(
                SessionEventKind::Interface,
                "Update notice ignored",
                format!(
                    "{} was reported on the {} channel, but {} is selected.",
                    info.latest(),
                    info.branch(),
                    self.update_channel
                ),
            );
            return;
        }

        if let Some(existing) = &self.update {
            if existing.installed() {
                return;
            }
            if existing.info().latest() == info.latest()
                && existing.info().branch() == info.branch()
            {
                return;
            }
        }

        let current = info.current().to_string();
        let latest = info.latest().to_string();
        let branch = info.branch().to_string();
        self.update = Some(UpdateNotice::available(info));
        self.toast(
            &format!("Update available · {latest} · {branch} · press u"),
            ToastTone::Warning,
        );
        self.log_event(
            SessionEventKind::Interface,
            "Update available",
            format!(
                "The GitHub {branch} channel reports {latest}; this session runs {current}. Press u to review."
            ),
        );
    }

    fn open_update_prompt(&mut self) {
        match self.update.as_ref() {
            None => self.toast("No update available yet", ToastTone::Info),
            Some(notice) if notice.installed() => {
                self.toast(
                    &format!(
                        "Update installed · restart AMPH to load {}",
                        notice.info().latest()
                    ),
                    ToastTone::Success,
                );
            }
            Some(_) => {
                self.overlay = Overlay::UpdatePrompt;
                self.log_event(
                    SessionEventKind::Interface,
                    "Update prompt opened",
                    "Install only runs through the injected handler.",
                );
            }
        }
    }

    /// Request the announced install from the injected handler.
    ///
    /// Shared by the Enter key and a click on the prompt's Install control. It
    /// never runs an installer; it only queues the request for the main loop.
    /// Returns `true` when a request was queued.
    fn confirm_update_install(&mut self) -> bool {
        if !self.update_enabled {
            self.log_event(
                SessionEventKind::Warning,
                "Update install unavailable",
                "No update handler is attached to this session, so no install was requested.",
            );
            return false;
        }

        let Some(notice) = self.update.as_ref().filter(|notice| !notice.installed()) else {
            return false;
        };
        let version = notice.info().latest().to_string();
        let branch = notice.info().branch().to_string();
        self.update_queue
            .push_back(UpdateRequest::install(version.clone(), branch.clone()));
        self.overlay = Overlay::UpdateBusy;
        self.log_event(
            SessionEventKind::Safety,
            "Update install requested",
            format!(
                "{version} from the {branch} channel sent to the injected handler. The terminal is suspended while it runs; the TUI executes nothing itself."
            ),
        );
        true
    }

    fn open_channel_picker(&mut self) {
        self.channel_cursor = channel_index(&self.update_channel);
        self.overlay = Overlay::ChannelPicker;
        self.log_event(
            SessionEventKind::Interface,
            "Channel picker opened",
            "Choosing a branch installs its latest build through the injected handler.",
        );
    }

    /// Select `branch`, remembering it locally and asking the host to install.
    ///
    /// Returns `true` when the picker should close: either the channel is
    /// already selected (a safe no-op) or the install request was queued.
    /// Returns `false` when no installer is attached and the picker should stay
    /// open so the user can see why nothing happened.
    fn choose_channel(&mut self, branch: &str) -> bool {
        if branch == self.update_channel {
            self.overlay = Overlay::None;
            self.toast(
                &format!("{branch} is already the current channel"),
                ToastTone::Info,
            );
            self.log_event(
                SessionEventKind::Interface,
                "Channel unchanged",
                format!("{branch} is already selected. No install was requested."),
            );
            return true;
        }

        if !self.update_enabled {
            self.toast(
                "No update installer is attached to this session",
                ToastTone::Warning,
            );
            self.log_event(
                SessionEventKind::Warning,
                "Channel switch unavailable",
                "No update handler is attached, so the channel was not changed.",
            );
            return false;
        }

        let version = format!("latest {branch} build");
        self.update_channel = branch.to_string();
        // The old notice belongs to the previous channel and must not remain
        // on screen as if it still described the selected branch.
        self.update = None;
        self.update_queue
            .push_back(UpdateRequest::install(version.clone(), branch));
        self.overlay = Overlay::UpdateBusy;
        self.log_event(
            SessionEventKind::Safety,
            "Channel install requested",
            format!(
                "{branch} selected. {version} sent to the injected handler, which saves the channel and installs it. The TUI executes nothing itself."
            ),
        );
        true
    }

    pub(crate) fn take_update_request(&mut self) -> Option<UpdateRequest> {
        self.update_queue.pop_front()
    }

    pub(crate) fn on_update_handler_result(
        &mut self,
        request: &UpdateRequest,
        result: Result<UpdateOutcome, UpdateError>,
    ) {
        let version = request.version().to_string();
        match result {
            Ok(outcome) => {
                let success = outcome.success;
                if success && let Some(notice) = self.update.as_mut() {
                    notice.phase = UpdatePhase::Installed;
                }
                self.overlay = Overlay::UpdateResult(UpdateResultState {
                    version: version.clone(),
                    success,
                    output: outcome.output,
                    note: outcome.note,
                    scroll: 0,
                });
                if success {
                    self.toast(
                        &format!("Update installed · restart AMPH to load {version}"),
                        ToastTone::Success,
                    );
                    self.log_event(
                        SessionEventKind::Interface,
                        "Update installed",
                        format!(
                            "{version} is installed. Restart AMPH to load it; this session keeps the version it started with."
                        ),
                    );
                } else {
                    self.toast(
                        "Update installer reported failure · current version unchanged",
                        ToastTone::Warning,
                    );
                    self.log_event(
                        SessionEventKind::Warning,
                        "Update install failed",
                        format!("The handler reported failure for {version}."),
                    );
                }
            }
            Err(error) => {
                let message = error.message().to_string();
                self.overlay = Overlay::UpdateResult(UpdateResultState {
                    version,
                    success: false,
                    output: message.clone(),
                    note: None,
                    scroll: 0,
                });
                self.toast(
                    "Update installer failed · current version unchanged",
                    ToastTone::Warning,
                );
                self.log_event(SessionEventKind::Error, "Update install failed", message);
            }
        }
    }

    // ---------------------------------------------------------------------
    // Input
    // ---------------------------------------------------------------------

    pub fn on_input(&mut self, input: InputEvent) {
        match input {
            InputEvent::Key(key) => self.on_key(key),
            InputEvent::Mouse(mouse) => self.on_mouse(mouse),
            InputEvent::Resize => {}
        }
    }

    fn on_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }

        let control = key.modifiers.contains(KeyModifiers::CONTROL);
        if control && matches!(key.code, KeyCode::Char('c')) {
            self.should_quit = true;
            return;
        }

        if self.filter_edit.is_some() {
            self.on_filter_key(key);
            return;
        }

        if !matches!(self.overlay, Overlay::None) {
            self.on_overlay_key(key);
            return;
        }

        match key.code {
            KeyCode::Char('?') => self.toggle_help(),
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Tab => self.cycle_tab(1),
            KeyCode::BackTab => self.cycle_tab(-1),
            KeyCode::Char('h') | KeyCode::Left => self.horizontal_move(false),
            KeyCode::Char('l') | KeyCode::Right => self.horizontal_move(true),
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            KeyCode::PageDown => self.move_selection(10),
            KeyCode::PageUp => self.move_selection(-10),
            KeyCode::Char('g') | KeyCode::Home => self.jump_selection(false),
            KeyCode::Char('G') | KeyCode::End => self.jump_selection(true),
            KeyCode::Char('/') => self.start_filter(),
            KeyCode::Char('t') => self.cycle_theme(true),
            KeyCode::Char('T') => self.cycle_theme(false),
            KeyCode::Char('i') => self.toggle_detail(),
            KeyCode::Char('V')
                if key.modifiers.contains(KeyModifiers::SHIFT)
                    && !control
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                self.open_channel_picker();
            }
            KeyCode::Char('u') => self.open_update_prompt(),
            KeyCode::Char('s') if self.active_tab == Tab::Packages => self.cycle_sort(),
            KeyCode::Char(' ') if self.active_tab == Tab::Graph => {
                self.graph_tree_state.toggle_selected();
                self.graph_tree_state.scroll_selected_into_view();
            }
            KeyCode::Char(' ') if self.active_tab == Tab::Cleanup => self.toggle_cleanup_cursor(),
            KeyCode::Char('x') if self.active_tab == Tab::Cleanup => self.toggle_cleanup_cursor(),
            KeyCode::Char('a') if self.active_tab == Tab::Cleanup => self.select_all_cleanup(),
            KeyCode::Char('c') if self.active_tab == Tab::Cleanup => self.clear_cleanup(),
            KeyCode::Char('p') | KeyCode::Char('P') if self.active_tab == Tab::Cleanup => {
                self.open_cleanup_preview()
            }
            KeyCode::Enter => self.enter_active(),
            KeyCode::Esc => self.clear_context(),
            KeyCode::Char(character @ '1'..='7') => {
                let index = character as usize - '1' as usize;
                if let Some(tab) = Tab::from_index(index) {
                    self.set_tab(tab);
                }
            }
            _ => {}
        }
    }

    fn on_overlay_key(&mut self, key: KeyEvent) {
        if key.kind == KeyEventKind::Release {
            return;
        }
        let overlay = std::mem::replace(&mut self.overlay, Overlay::None);
        match overlay {
            Overlay::None => {}
            Overlay::Help => match key.code {
                KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q') | KeyCode::Enter => {}
                _ => {
                    self.overlay = Overlay::Help;
                }
            },
            Overlay::Detail => match key.code {
                KeyCode::Esc | KeyCode::Char('i') | KeyCode::Enter | KeyCode::Char('q') => {}
                _ => {
                    self.overlay = Overlay::Detail;
                }
            },
            Overlay::CleanupBusy => {
                self.overlay = Overlay::CleanupBusy;
            }
            Overlay::CleanupPreview(mut state) => {
                let max_scroll = state.output.lines().count().saturating_sub(1) as u16;
                match key.code {
                    KeyCode::Esc => {
                        self.log_event(
                            SessionEventKind::Safety,
                            "Cleanup preview closed",
                            "No removal request was sent.",
                        );
                        return;
                    }
                    KeyCode::Up => {
                        state.scroll = state.scroll.saturating_sub(1);
                    }
                    KeyCode::Down => {
                        state.scroll = state.scroll.saturating_add(1).min(max_scroll);
                    }
                    KeyCode::PageUp => {
                        state.scroll = state.scroll.saturating_sub(8);
                    }
                    KeyCode::PageDown => {
                        state.scroll = state.scroll.saturating_add(8).min(max_scroll);
                    }
                    KeyCode::Home => {
                        state.scroll = 0;
                    }
                    KeyCode::End => {
                        state.scroll = max_scroll;
                    }
                    KeyCode::Backspace => {
                        state.typed.pop();
                        state.error = None;
                    }
                    KeyCode::Enter => {
                        if !state.can_remove {
                            state.error = Some(
                                "This preview is read-only; the handler did not enable removal."
                                    .to_string(),
                            );
                        } else if state.typed != "remove" {
                            state.error =
                                Some("Type remove exactly to request removal.".to_string());
                        } else {
                            self.cleanup_queue.push_back(CleanupRequest::Remove {
                                packages: state.packages.clone(),
                            });
                            self.overlay = Overlay::CleanupBusy;
                            self.log_event(
                                SessionEventKind::Safety,
                                "Removal requested",
                                format!(
                                    "{} package(s) sent to the injected handler after typed confirmation.",
                                    state.packages.len()
                                ),
                            );
                            return;
                        }
                    }
                    KeyCode::Char(character)
                        if state.can_remove
                            && !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT) =>
                    {
                        if state.typed.chars().count() < 16 {
                            state.typed.push(character);
                        }
                        state.error = None;
                    }
                    _ => {}
                }
                self.overlay = Overlay::CleanupPreview(state);
            }
            Overlay::CleanupResult(mut state) => {
                let max_scroll = state.output.lines().count().saturating_sub(1) as u16;
                match key.code {
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                        return;
                    }
                    KeyCode::Up => state.scroll = state.scroll.saturating_sub(1),
                    KeyCode::Down => {
                        state.scroll = state.scroll.saturating_add(1).min(max_scroll);
                    }
                    KeyCode::PageUp => state.scroll = state.scroll.saturating_sub(8),
                    KeyCode::PageDown => {
                        state.scroll = state.scroll.saturating_add(8).min(max_scroll);
                    }
                    KeyCode::Home => state.scroll = 0,
                    KeyCode::End => state.scroll = max_scroll,
                    _ => {}
                }
                self.overlay = Overlay::CleanupResult(state);
            }
            Overlay::CleanupError(state) => match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {}
                _ => {
                    self.overlay = Overlay::CleanupError(state);
                }
            },
            Overlay::ChannelPicker => {
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('V') => {
                        self.log_event(
                            SessionEventKind::Interface,
                            "Channel picker closed",
                            "No channel change was requested.",
                        );
                        return;
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.channel_cursor = self.channel_cursor.saturating_sub(1);
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.channel_cursor =
                            (self.channel_cursor + 1).min(UPDATE_CHANNELS.len() - 1);
                    }
                    KeyCode::Home | KeyCode::Char('g') => self.channel_cursor = 0,
                    KeyCode::End | KeyCode::Char('G') => {
                        self.channel_cursor = UPDATE_CHANNELS.len() - 1;
                    }
                    KeyCode::Enter | KeyCode::Char(' ') => {
                        let branch = UPDATE_CHANNELS[self.channel_cursor];
                        if self.choose_channel(branch) {
                            return;
                        }
                    }
                    _ => {}
                }
                self.overlay = Overlay::ChannelPicker;
            }
            Overlay::UpdatePrompt => {
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('u') => {
                        self.log_event(
                            SessionEventKind::Interface,
                            "Update prompt closed",
                            "No install request was sent.",
                        );
                        return;
                    }
                    KeyCode::Enter if self.confirm_update_install() => return,
                    _ => {}
                }
                self.overlay = Overlay::UpdatePrompt;
            }
            Overlay::UpdateBusy => {
                self.overlay = Overlay::UpdateBusy;
            }
            Overlay::UpdateResult(mut state) => {
                let max_scroll = state.output.lines().count().saturating_sub(1) as u16;
                match key.code {
                    KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                        return;
                    }
                    KeyCode::Char('u') if !state.success => {
                        self.overlay = Overlay::UpdatePrompt;
                        return;
                    }
                    KeyCode::Up => state.scroll = state.scroll.saturating_sub(1),
                    KeyCode::Down => {
                        state.scroll = state.scroll.saturating_add(1).min(max_scroll);
                    }
                    KeyCode::PageUp => state.scroll = state.scroll.saturating_sub(8),
                    KeyCode::PageDown => {
                        state.scroll = state.scroll.saturating_add(8).min(max_scroll);
                    }
                    KeyCode::Home => state.scroll = 0,
                    KeyCode::End => state.scroll = max_scroll,
                    _ => {}
                }
                self.overlay = Overlay::UpdateResult(state);
            }
        }
    }

    fn on_filter_key(&mut self, key: KeyEvent) {
        let Some(edit) = self.filter_edit.clone() else {
            return;
        };

        match key.code {
            KeyCode::Esc => {
                self.filters.set(edit.tab, edit.original);
                self.filter_edit = None;
                self.sync_selections();
            }
            KeyCode::Enter => {
                self.filter_edit = None;
                self.sync_selections();
            }
            KeyCode::Backspace => {
                let mut value = self.filters.get(edit.tab).to_string();
                value.pop();
                self.filters.set(edit.tab, value);
                self.sync_selections();
            }
            KeyCode::Delete => {
                self.filters.set(edit.tab, String::new());
                self.sync_selections();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.filters.set(edit.tab, String::new());
                self.sync_selections();
            }
            KeyCode::Char(character)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                let mut value = self.filters.get(edit.tab).to_string();
                if value.chars().count() < 120 {
                    value.push(character);
                }
                self.filters.set(edit.tab, value);
                self.sync_selections();
            }
            _ => {
                self.filter_edit = Some(edit);
            }
        }
    }

    fn on_mouse(&mut self, mouse: MouseEvent) {
        if matches!(self.overlay, Overlay::UpdatePrompt) {
            // The prompt is modal: only its own Install control is clickable.
            // Every other click is swallowed so it cannot reach the tabs, rows,
            // banner, or graph underneath.
            if self.update_enabled
                && matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left))
                && let Some(rect) = self.hits.update_install
                && contains(rect, mouse.column, mouse.row)
            {
                self.confirm_update_install();
            }
            return;
        }

        if !matches!(self.overlay, Overlay::None) {
            return;
        }

        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(rect) = self.hits.update
                    && contains(rect, mouse.column, mouse.row)
                {
                    self.open_update_prompt();
                    return;
                }

                if let Some((_, tab)) = self
                    .hits
                    .tabs
                    .iter()
                    .find(|(rect, _)| contains(*rect, mouse.column, mouse.row))
                {
                    let tab = *tab;
                    self.set_tab(tab);
                    return;
                }

                if self.active_tab == Tab::Graph
                    && self
                        .graph_tree_state
                        .click_at(Position::new(mouse.column, mouse.row))
                {
                    return;
                }

                if let Some(hit) = self
                    .hits
                    .rows
                    .iter()
                    .find(|hit| contains(hit.rect, mouse.column, mouse.row))
                    .copied()
                {
                    if self.active_tab == Tab::Cleanup
                        && mouse.column <= hit.rect.x.saturating_add(6)
                    {
                        self.toggle_cleanup_index(hit.index);
                    } else {
                        self.select_index(hit.index);
                    }
                }
            }
            MouseEventKind::ScrollUp => self.move_selection(-3),
            MouseEventKind::ScrollDown => self.move_selection(3),
            _ => {}
        }
    }

    // ---------------------------------------------------------------------
    // Navigation and actions
    // ---------------------------------------------------------------------

    fn set_tab(&mut self, tab: Tab) {
        self.filter_edit = None;
        self.active_tab = tab;
        self.sync_selections();
    }

    fn horizontal_move(&mut self, forward: bool) {
        if self.active_tab == Tab::Graph {
            let changed = if forward {
                self.graph_tree_state.key_right()
            } else {
                self.graph_tree_state.key_left()
            };
            if changed {
                self.graph_tree_state.scroll_selected_into_view();
            }
            return;
        }
        self.cycle_tab(if forward { 1 } else { -1 });
    }

    fn cycle_tab(&mut self, delta: isize) {
        let count = Tab::ALL.len() as isize;
        let index = (self.active_tab.index() as isize + delta).rem_euclid(count);
        if let Some(tab) = Tab::from_index(index as usize) {
            self.set_tab(tab);
        }
    }

    fn cycle_theme(&mut self, forward: bool) {
        self.theme = if forward {
            self.theme.next()
        } else {
            self.theme.previous()
        };
        self.clear_request = true;
        self.toast(
            &format!("Theme · {}", self.theme.kind.label()),
            ToastTone::Info,
        );
        self.log_event(
            SessionEventKind::Interface,
            "Theme changed",
            format!(
                "{} palette{}",
                self.theme.kind.label(),
                if self.theme.no_color {
                    " · NO_COLOR"
                } else {
                    ""
                }
            ),
        );
    }

    fn toggle_help(&mut self) {
        self.overlay = if matches!(self.overlay, Overlay::Help) {
            Overlay::None
        } else {
            Overlay::Help
        };
    }

    fn toggle_detail(&mut self) {
        if self.layout.viewport.width >= 76 {
            self.detail_visible = !self.detail_visible;
        } else {
            self.overlay = if matches!(self.overlay, Overlay::Detail) {
                Overlay::None
            } else {
                Overlay::Detail
            };
        }
    }

    fn enter_active(&mut self) {
        match self.active_tab {
            Tab::Cleanup => self.open_cleanup_preview(),
            Tab::Dashboard
            | Tab::Packages
            | Tab::Graph
            | Tab::Harmony
            | Tab::Checks
            | Tab::Timeline => {
                self.overlay = Overlay::Detail;
            }
        }
    }

    fn start_filter(&mut self) {
        let target = match self.active_tab {
            Tab::Dashboard => Tab::Packages,
            tab if tab.supports_filter() => tab,
            _ => {
                self.toast("No list to filter on this tab", ToastTone::Info);
                return;
            }
        };

        if target != self.active_tab {
            self.set_tab(target);
        }

        let original = self.filters.get(target).to_string();
        self.filter_edit = Some(FilterEdit {
            tab: target,
            original,
        });
    }

    fn clear_context(&mut self) {
        if self.filter_edit.is_some() {
            return;
        }

        let active_filter = self.filters.get(self.active_tab).to_string();
        if !active_filter.is_empty() {
            self.filters.set(self.active_tab, String::new());
            self.sync_selections();
            self.toast("Filter cleared", ToastTone::Info);
            return;
        }

        if self.active_tab == Tab::Cleanup && !self.cleanup_selected.is_empty() {
            self.clear_cleanup();
        }
    }

    fn cycle_sort(&mut self) {
        let selected_name = self.selected_package().map(|package| package.name.clone());
        self.package_sort = self.package_sort.next();
        self.sync_selections();
        if let Some(name) = selected_name
            && let Some(index) = self
                .package_indices()
                .iter()
                .position(|package_index| self.inventory.packages[*package_index].name == name)
        {
            self.package_state.select(Some(index));
        }
        self.toast(
            &format!(
                "Sorted by {}",
                self.package_sort.label().to_ascii_lowercase()
            ),
            ToastTone::Info,
        );
    }

    fn active_len(&self) -> usize {
        match self.active_tab {
            Tab::Packages => self.package_indices().len(),
            Tab::Graph => self.graph_indices().len(),
            Tab::Cleanup => self.cleanup_indices().len(),
            Tab::Checks => self.check_indices().len(),
            Tab::Timeline => self.filtered_timeline().len(),
            Tab::Dashboard | Tab::Harmony => 0,
        }
    }

    fn move_selection(&mut self, delta: isize) {
        if self.active_tab == Tab::Graph {
            let steps = delta.unsigned_abs().min(64);
            for _ in 0..steps {
                let changed = if delta > 0 {
                    self.graph_tree_state.key_down()
                } else {
                    self.graph_tree_state.key_up()
                };
                if !changed {
                    break;
                }
            }
            self.graph_tree_state.scroll_selected_into_view();
            return;
        }

        let len = self.active_len();
        if len == 0 {
            return;
        }

        let current = self.selection_position().unwrap_or(0) as isize;
        let next = (current + delta).clamp(0, len as isize - 1) as usize;
        self.set_selection_position(next);
    }

    fn jump_selection(&mut self, end: bool) {
        if self.active_tab == Tab::Graph {
            if end {
                self.graph_tree_state.select_last();
            } else {
                self.graph_tree_state.select_first();
            }
            self.graph_tree_state.scroll_selected_into_view();
            return;
        }

        let len = self.active_len();
        if len == 0 {
            return;
        }
        self.set_selection_position(if end { len - 1 } else { 0 });
    }

    fn selection_position(&self) -> Option<usize> {
        match self.active_tab {
            Tab::Packages => self.package_state.selected(),
            Tab::Cleanup => self.cleanup_state.selected(),
            Tab::Checks => self.checks_state.selected(),
            Tab::Timeline => Some(self.timeline_selected),
            Tab::Graph | Tab::Dashboard | Tab::Harmony => None,
        }
    }

    fn set_selection_position(&mut self, index: usize) {
        match self.active_tab {
            Tab::Packages => self.package_state.select(Some(index)),
            Tab::Cleanup => self.cleanup_state.select(Some(index)),
            Tab::Checks => self.checks_state.select(Some(index)),
            Tab::Timeline => self.timeline_selected = index,
            Tab::Graph | Tab::Dashboard | Tab::Harmony => {}
        }
    }

    fn select_index(&mut self, index: usize) {
        if index < self.active_len() {
            self.set_selection_position(index);
        }
    }

    // ---------------------------------------------------------------------
    // Cleanup safety flow
    // ---------------------------------------------------------------------

    fn toggle_cleanup_cursor(&mut self) {
        if let Some(index) = self.cleanup_state.selected() {
            self.toggle_cleanup_index(index);
        }
    }

    fn toggle_cleanup_index(&mut self, index: usize) {
        let indices = self.cleanup_indices();
        let Some(package_index) = indices.get(index) else {
            return;
        };
        let name = self.inventory.packages[*package_index].name.clone();
        if !self.cleanup_selected.insert(name.clone()) {
            self.cleanup_selected.remove(&name);
        }
    }

    fn select_all_cleanup(&mut self) {
        let names = self
            .cleanup_indices()
            .into_iter()
            .map(|index| self.inventory.packages[index].name.clone())
            .collect::<Vec<_>>();
        for name in names {
            self.cleanup_selected.insert(name);
        }
        self.toast("All visible candidates selected", ToastTone::Info);
    }

    fn clear_cleanup(&mut self) {
        self.cleanup_selected.clear();
        self.toast("Cleanup selection cleared", ToastTone::Info);
    }

    fn open_cleanup_preview(&mut self) {
        if self.cleanup_selected.is_empty() {
            self.toast(
                "Select candidates with Space, then request a preview",
                ToastTone::Warning,
            );
            return;
        }

        let mut names = self
            .inventory
            .packages
            .iter()
            .filter(|package| package.is_cleanup_candidate())
            .filter(|package| self.cleanup_selected.contains(&package.name))
            .map(|package| package.name.clone())
            .collect::<Vec<_>>();
        names.sort();
        names.dedup();

        if names.is_empty() {
            self.toast(
                "No valid cleanup candidates remain in the selection",
                ToastTone::Warning,
            );
            return;
        }

        let count = names.len();
        self.cleanup_queue
            .push_back(CleanupRequest::Preview { packages: names });
        self.overlay = Overlay::CleanupBusy;
        self.log_event(
            SessionEventKind::Safety,
            "Preview requested",
            format!("{count} package(s) sent to the injected handler. The TUI built no command."),
        );
    }

    pub(crate) fn take_cleanup_request(&mut self) -> Option<CleanupRequest> {
        self.cleanup_queue.pop_front()
    }

    pub(crate) fn on_cleanup_handler_result(
        &mut self,
        request: &CleanupRequest,
        result: Result<CleanupResponse, CleanupError>,
    ) {
        let phase = request.phase();
        let packages = request.packages().to_vec();

        match result {
            Ok(CleanupResponse::Preview(preview)) if phase == CleanupPhase::Preview => {
                let non_empty = !preview.output.trim().is_empty();
                let can_remove = preview.can_remove && non_empty;
                let mut error = None;
                if preview.can_remove && !non_empty {
                    error = Some(
                        "The handler enabled removal but returned no preview output; removal is blocked."
                            .to_string(),
                    );
                }
                self.overlay = Overlay::CleanupPreview(CleanupPreviewState {
                    packages,
                    output: preview.output,
                    can_remove,
                    note: preview.note,
                    scroll: 0,
                    typed: String::new(),
                    error,
                });
                self.log_event(
                    SessionEventKind::Safety,
                    "Preview received",
                    format!(
                        "{} package(s) · {}",
                        request.packages().len(),
                        if can_remove {
                            "typed remove is available"
                        } else {
                            "removal unavailable"
                        }
                    ),
                );
            }
            Ok(CleanupResponse::Removed(outcome)) if phase == CleanupPhase::Remove => {
                self.cleanup_selected.clear();
                self.stale_snapshot = true;
                let success = outcome.success;
                self.overlay = Overlay::CleanupResult(CleanupResultState {
                    packages,
                    success,
                    output: outcome.output,
                    note: outcome.note,
                    scroll: 0,
                });
                self.log_event(
                    if success {
                        SessionEventKind::Safety
                    } else {
                        SessionEventKind::Warning
                    },
                    if success {
                        "Removal completed"
                    } else {
                        "Removal reported failure"
                    },
                    format!(
                        "{} package(s) · handler success={success} · snapshot marked stale",
                        request.packages().len()
                    ),
                );
                self.toast(
                    if success {
                        "Handler reported removal complete · rescan to refresh"
                    } else {
                        "Handler reported removal failure · check system state"
                    },
                    if success {
                        ToastTone::Success
                    } else {
                        ToastTone::Warning
                    },
                );
            }
            Ok(response) => {
                let removal = phase == CleanupPhase::Remove;
                if removal {
                    self.cleanup_selected.clear();
                    self.stale_snapshot = true;
                }
                let message = format!(
                    "The handler returned a {} result for a {} request. The result was not applied.",
                    response.phase().label(),
                    phase.label()
                );
                self.overlay = Overlay::CleanupError(CleanupErrorState {
                    title: format!("Cleanup {} failed", phase.label()),
                    message: message.clone(),
                    removal,
                });
                self.log_event(SessionEventKind::Error, "Cleanup result mismatch", message);
            }
            Err(error) => {
                let removal = phase == CleanupPhase::Remove;
                if removal {
                    self.cleanup_selected.clear();
                    self.stale_snapshot = true;
                }
                let mut message = error.message().to_string();
                if removal {
                    message.push_str(
                        "\n\nThe snapshot may be stale. Check the system state before continuing.",
                    );
                }
                self.overlay = Overlay::CleanupError(CleanupErrorState {
                    title: format!("Cleanup {} failed", phase.label()),
                    message: message.clone(),
                    removal,
                });
                self.log_event(
                    SessionEventKind::Error,
                    format!("Cleanup {} failed", phase.label()),
                    message,
                );
            }
        }
    }

    // ---------------------------------------------------------------------
    // Data views
    // ---------------------------------------------------------------------

    pub fn package_indices(&self) -> Vec<usize> {
        metrics::package_indices(
            &self.inventory.packages,
            &self.filters.packages,
            self.package_sort,
        )
    }

    pub fn graph_indices(&self) -> Vec<usize> {
        metrics::graph_indices(
            &self.inventory.packages,
            &self.inventory.graph,
            &self.filters.graph,
        )
    }

    pub fn cleanup_indices(&self) -> Vec<usize> {
        metrics::cleanup_indices(&self.inventory.packages, &self.filters.cleanup)
    }

    pub fn check_indices(&self) -> Vec<usize> {
        metrics::check_indices(&self.inventory.checks, &self.filters.checks)
    }

    /// Merged timeline rows: snapshot transactions first, then TUI session
    /// events newest-first.
    pub fn timeline_entries(&self) -> Vec<TimelineEntry> {
        let mut entries = Vec::with_capacity(self.inventory.timeline.len() + self.events.len());
        entries.extend(
            self.inventory
                .timeline
                .iter()
                .cloned()
                .map(TimelineEntry::Transaction),
        );
        entries.extend(
            self.events
                .iter()
                .rev()
                .cloned()
                .map(TimelineEntry::Session),
        );
        entries
    }

    pub fn filtered_timeline(&self) -> Vec<TimelineEntry> {
        let entries = self.timeline_entries();
        metrics::timeline_indices(&entries, &self.filters.timeline)
            .into_iter()
            .map(|index| entries[index].clone())
            .collect()
    }

    pub fn selected_package(&self) -> Option<&Package> {
        match self.active_tab {
            Tab::Packages => self
                .package_state
                .selected()
                .and_then(|index| self.package_indices().get(index).copied())
                .map(|index| &self.inventory.packages[index]),
            Tab::Graph => {
                let name = self.graph_tree_state.selected().last()?;
                self.inventory
                    .packages
                    .iter()
                    .find(|package| &package.name == name)
            }
            Tab::Cleanup => self
                .cleanup_state
                .selected()
                .and_then(|index| self.cleanup_indices().get(index).copied())
                .map(|index| &self.inventory.packages[index]),
            _ => None,
        }
    }

    pub fn selected_check(&self) -> Option<&Check> {
        let index = self.checks_state.selected()?;
        let check_index = *self.check_indices().get(index)?;
        self.inventory.checks.get(check_index)
    }

    pub fn selected_timeline(&self) -> Option<TimelineEntry> {
        self.filtered_timeline()
            .get(self.timeline_selected)
            .cloned()
    }

    pub fn selected_package_index(&self) -> Option<usize> {
        match self.active_tab {
            Tab::Packages => self
                .package_state
                .selected()
                .and_then(|index| self.package_indices().get(index).copied()),
            Tab::Graph => {
                let name = self.graph_tree_state.selected().last()?;
                self.inventory
                    .packages
                    .iter()
                    .position(|package| &package.name == name)
            }
            Tab::Cleanup => self
                .cleanup_state
                .selected()
                .and_then(|index| self.cleanup_indices().get(index).copied()),
            _ => None,
        }
    }

    pub fn cleanup_selected_count(&self) -> usize {
        self.cleanup_selected.len()
    }

    pub fn cleanup_selected_bytes(&self) -> u64 {
        self.inventory
            .packages
            .iter()
            .filter(|package| self.cleanup_selected.contains(&package.name))
            .map(|package| package.size)
            .sum()
    }

    pub fn cleanup_all_visible_selected(&self) -> bool {
        let indices = self.cleanup_indices();
        !indices.is_empty()
            && indices.iter().all(|index| {
                self.cleanup_selected
                    .contains(&self.inventory.packages[*index].name)
            })
    }

    pub fn active_filter(&self) -> &str {
        self.filters.get(self.active_tab)
    }

    pub fn filter_edit(&self) -> Option<&FilterEdit> {
        self.filter_edit.as_ref()
    }

    pub fn theme(&self) -> &Theme {
        &self.theme
    }

    pub fn active_tab(&self) -> Tab {
        self.active_tab
    }

    pub fn overlay(&self) -> &Overlay {
        &self.overlay
    }

    pub fn scan(&self) -> &ScanState {
        &self.scan
    }

    pub fn sample(&self) -> bool {
        self.sample
    }

    pub fn inventory(&self) -> &Inventory {
        &self.inventory
    }

    pub fn is_detail_visible(&self) -> bool {
        self.detail_visible
    }

    pub fn graph_tree_state(&self) -> &TreeState<String> {
        &self.graph_tree_state
    }

    pub fn graph_tree_state_mut(&mut self) -> &mut TreeState<String> {
        &mut self.graph_tree_state
    }

    pub fn cleanup_enabled(&self) -> bool {
        self.cleanup_enabled
    }

    pub fn update_notice(&self) -> Option<&UpdateNotice> {
        self.update.as_ref()
    }

    pub fn update_enabled(&self) -> bool {
        self.update_enabled
    }

    /// The channel the host currently has selected.
    pub fn update_channel(&self) -> &str {
        &self.update_channel
    }

    /// The branch highlighted in the open channel picker.
    pub fn highlighted_channel(&self) -> &'static str {
        UPDATE_CHANNELS
            .get(self.channel_cursor)
            .copied()
            .unwrap_or(DEFAULT_CHANNEL)
    }

    pub fn stale_snapshot(&self) -> bool {
        self.stale_snapshot
    }

    pub fn active_toast(&self) -> Option<&Toast> {
        self.toast
            .as_ref()
            .filter(|toast| Instant::now() < toast.until)
    }

    // ---------------------------------------------------------------------
    // Internals
    // ---------------------------------------------------------------------

    fn sync_selections(&mut self) {
        let package_len = self.package_indices().len();
        clamp_table(&mut self.package_state, package_len);

        let cleanup_len = self.cleanup_indices().len();
        clamp_table(&mut self.cleanup_state, cleanup_len);

        let checks_len = self.check_indices().len();
        clamp_table(&mut self.checks_state, checks_len);

        let timeline_len = self.filtered_timeline().len();
        if timeline_len == 0 {
            self.timeline_selected = 0;
        } else {
            self.timeline_selected = self.timeline_selected.min(timeline_len - 1);
        }

        // Drop selections that are no longer present or no longer candidates.
        // Filtering must not silently deselect hidden rows.
        let valid = self
            .inventory
            .packages
            .iter()
            .filter(|package| package.is_cleanup_candidate())
            .map(|package| package.name.clone())
            .collect::<BTreeSet<_>>();
        self.cleanup_selected.retain(|name| valid.contains(name));
    }

    pub fn toast(&mut self, text: &str, tone: ToastTone) {
        self.toast = Some(Toast {
            text: text.to_string(),
            tone,
            until: Instant::now() + Duration::from_millis(3200),
        });
    }

    pub fn log_event(
        &mut self,
        kind: SessionEventKind,
        title: impl Into<String>,
        detail: impl Into<String>,
    ) {
        let at = self.booted.elapsed().as_secs_f64();
        self.events.push(SessionEvent {
            at,
            kind,
            title: title.into(),
            detail: detail.into(),
        });
        if self.events.len() > 240 {
            let overflow = self.events.len() - 240;
            self.events.drain(0..overflow);
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

fn channel_index(branch: &str) -> usize {
    UPDATE_CHANNELS
        .iter()
        .position(|candidate| *candidate == branch)
        .unwrap_or(0)
}

fn clamp_table(state: &mut TableState, len: usize) {
    if len == 0 {
        state.select(None);
        *state.offset_mut() = 0;
        return;
    }

    let selected = state.selected().unwrap_or(0).min(len - 1);
    state.select(Some(selected));
    let max_offset = len.saturating_sub(1);
    *state.offset_mut() = state.offset().min(max_offset);
}

fn contains(rect: Rect, column: u16, row: u16) -> bool {
    column >= rect.x
        && column < rect.x.saturating_add(rect.width)
        && row >= rect.y
        && row < rect.y.saturating_add(rect.height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::cleanup::{CleanupOutcome, CleanupPreview};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn shift_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::SHIFT)
    }

    fn ready_cleanup_app(enabled: bool) -> App {
        let mut app = App::new();
        app.on_scan_message(ScanMessage::snapshot(Snapshot::demo()));
        app.on_scan_message(ScanMessage::finished());
        app.set_cleanup_enabled(enabled);
        app.active_tab = Tab::Cleanup;
        app.cleanup_state.select(Some(0));
        app.toggle_cleanup_cursor();
        app
    }

    fn preview_response(app: &mut App, output: &str, can_remove: bool) -> CleanupRequest {
        app.open_cleanup_preview();
        let request = app.take_cleanup_request().expect("preview request");
        assert_eq!(request.phase(), CleanupPhase::Preview);
        app.on_cleanup_handler_result(
            &request,
            Ok(CleanupResponse::Preview(CleanupPreview::new(
                output, can_remove,
            ))),
        );
        request
    }

    fn type_remove(app: &mut App) {
        for character in "remove".chars() {
            app.on_overlay_key(key(KeyCode::Char(character)));
        }
        app.on_overlay_key(key(KeyCode::Enter));
    }

    #[test]
    fn cleanup_preview_then_remove_uses_exact_contract() {
        let mut app = ready_cleanup_app(true);
        let preview_request = preview_response(&mut app, "pacman -Rns --print output", true);
        assert_eq!(preview_request.packages().len(), 1);
        match &app.overlay {
            Overlay::CleanupPreview(state) => {
                assert!(state.can_remove);
                assert_eq!(state.output, "pacman -Rns --print output");
            }
            _ => panic!("expected cleanup preview overlay"),
        }

        type_remove(&mut app);
        let remove_request = app.take_cleanup_request().expect("remove request");
        assert_eq!(remove_request.phase(), CleanupPhase::Remove);
        assert_eq!(remove_request.packages(), preview_request.packages());

        app.on_cleanup_handler_result(
            &remove_request,
            Ok(CleanupResponse::Removed(CleanupOutcome::new(
                true, "removed",
            ))),
        );
        assert!(matches!(app.overlay, Overlay::CleanupResult(_)));
        assert!(app.stale_snapshot());
        assert!(app.cleanup_selected.is_empty());
    }

    #[test]
    fn read_only_preview_never_queues_removal() {
        let mut app = ready_cleanup_app(false);
        preview_response(&mut app, "Read-only preview", false);
        type_remove(&mut app);
        assert!(app.take_cleanup_request().is_none());
        match &app.overlay {
            Overlay::CleanupPreview(state) => {
                assert!(!state.can_remove);
                assert!(state.error.is_some());
            }
            _ => panic!("expected preview overlay"),
        }
    }

    #[test]
    fn wrong_phrase_never_queues_removal() {
        let mut app = ready_cleanup_app(true);
        preview_response(&mut app, "preview output", true);
        for character in "remov".chars() {
            app.on_overlay_key(key(KeyCode::Char(character)));
        }
        app.on_overlay_key(key(KeyCode::Enter));
        assert!(app.take_cleanup_request().is_none());
        match &app.overlay {
            Overlay::CleanupPreview(state) => assert!(state.error.is_some()),
            _ => panic!("expected preview overlay"),
        }
    }

    #[test]
    fn handler_error_is_shown_and_marks_removal_state_stale() {
        let mut app = ready_cleanup_app(true);
        let preview_request = preview_response(&mut app, "preview output", true);
        type_remove(&mut app);
        let remove_request = app.take_cleanup_request().expect("remove request");
        app.on_cleanup_handler_result(
            &remove_request,
            Err(CleanupError::new("simulated removal failure")),
        );
        assert!(matches!(app.overlay, Overlay::CleanupError(_)));
        assert!(app.stale_snapshot());
        let _ = preview_request;
    }

    #[test]
    fn snapshot_history_and_transactions_are_installed_and_merged() {
        let mut app = App::new();
        let snapshot = Snapshot::new(Vec::new(), "now")
            .with_health_history(vec![70, 60, 50])
            .with_timeline(vec![crate::tui::model::TimelineItem::new(
                "2026-10-22 21:14",
                "linux updated",
                "6.11 → 6.12",
                crate::tui::model::TimelineKind::Upgrade,
            )]);
        app.on_scan_message(ScanMessage::snapshot(snapshot));
        app.on_scan_message(ScanMessage::finished());

        assert_eq!(app.inventory.health_history, vec![70, 60, 50]);
        assert_eq!(app.inventory.timeline.len(), 1);
        let entries = app.timeline_entries();
        assert!(matches!(entries[0], TimelineEntry::Transaction(_)));
        assert!(
            entries
                .iter()
                .any(|entry| matches!(entry, TimelineEntry::Session(_)))
        );
    }

    #[test]
    fn explicit_theme_is_used_at_startup() {
        let app = App::new_with_theme(ThemeKind::Nord);
        assert_eq!(app.theme.kind, ThemeKind::Nord);
    }

    fn click(app: &mut App, column: u16, row: u16) {
        app.on_mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column,
            row,
            modifiers: KeyModifiers::NONE,
        });
    }

    fn update_info() -> UpdateInfo {
        UpdateInfo::new("0.2.0-beta", "0.3.0")
            .with_branch("main")
            .with_url("https://example.invalid/amph/releases")
            .with_notes("Faster package scans.")
    }

    fn screen_text(terminal: &Terminal<TestBackend>) -> String {
        let buffer = terminal.backend().buffer();
        let area = buffer.area;
        let mut text = String::new();
        for y in area.y..area.y.saturating_add(area.height) {
            for x in area.x..area.x.saturating_add(area.width) {
                if let Some(cell) = buffer.cell((x, y)) {
                    text.push_str(cell.symbol());
                }
            }
            text.push('\n');
        }
        text
    }

    #[test]
    fn update_notice_appears_without_stealing_focus() {
        let mut app = App::new();
        app.on_scan_message(ScanMessage::update_available(update_info()));

        let notice = app.update_notice().expect("update notice");
        assert_eq!(notice.info().latest(), "0.3.0");
        assert!(!notice.installed());
        assert_eq!(app.active_tab, Tab::Dashboard);
        assert!(matches!(app.overlay, Overlay::None));
    }

    #[test]
    fn update_prompt_without_a_handler_never_queues_an_install() {
        let mut app = App::new();
        app.on_scan_message(ScanMessage::update_available(update_info()));

        app.on_key(key(KeyCode::Char('u')));
        assert!(matches!(app.overlay, Overlay::UpdatePrompt));

        app.on_key(key(KeyCode::Enter));
        assert!(app.take_update_request().is_none());
        assert!(matches!(app.overlay, Overlay::UpdatePrompt));

        app.on_key(key(KeyCode::Esc));
        assert!(matches!(app.overlay, Overlay::None));
    }

    #[test]
    fn update_prompt_queues_the_exact_announced_version() {
        let mut app = App::new();
        app.set_update_enabled(true);
        app.on_scan_message(ScanMessage::update_available(update_info()));

        app.on_key(key(KeyCode::Char('u')));
        app.on_key(key(KeyCode::Enter));

        let request = app.take_update_request().expect("install request");
        assert_eq!(request.version(), "0.3.0");
        assert_eq!(request.branch(), "main");
        assert!(matches!(app.overlay, Overlay::UpdateBusy));

        // Modal keys cannot dismiss or duplicate work while the handler runs.
        app.on_key(key(KeyCode::Enter));
        app.on_key(key(KeyCode::Esc));
        assert!(matches!(app.overlay, Overlay::UpdateBusy));
        assert!(app.take_update_request().is_none());
    }

    #[test]
    fn clicking_the_install_action_queues_the_update_once() {
        let mut app = App::new();
        app.set_update_enabled(true);
        app.on_scan_message(ScanMessage::update_available(update_info()));
        app.on_key(key(KeyCode::Char('u')));

        let backend = TestBackend::new(110, 30);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| crate::tui::views::draw(frame, &mut app))
            .expect("prompt frame");
        let target = app.hits.update_install.expect("install click target");

        click(&mut app, target.x + 1, target.y);
        let request = app.take_update_request().expect("install request");
        assert_eq!(request.version(), "0.3.0");
        assert!(matches!(app.overlay, Overlay::UpdateBusy));

        // Once the terminal is suspending, further clicks cannot queue work.
        click(&mut app, target.x + 1, target.y);
        assert!(app.take_update_request().is_none());
    }

    #[test]
    fn update_prompt_swallows_clicks_and_only_install_is_clickable() {
        let mut app = App::new();
        app.on_scan_message(ScanMessage::update_available(update_info()));
        app.on_key(key(KeyCode::Char('u')));

        let backend = TestBackend::new(110, 30);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| crate::tui::views::draw(frame, &mut app))
            .expect("prompt frame");

        // With no handler the action is unavailable, so there is no target.
        assert!(app.hits.update_install.is_none());
        click(&mut app, 55, 15);
        assert!(app.take_update_request().is_none());
        assert!(matches!(app.overlay, Overlay::UpdatePrompt));

        // A click on the Packages tab must not fall through the modal.
        click(&mut app, 15, 2);
        assert_eq!(app.active_tab, Tab::Dashboard);
        assert!(matches!(app.overlay, Overlay::UpdatePrompt));

        // With a handler the target exists, but clicks outside it still do
        // nothing: no install, no tab change, prompt stays open.
        app.set_update_enabled(true);
        terminal
            .draw(|frame| crate::tui::views::draw(frame, &mut app))
            .expect("prompt frame with handler");
        let target = app.hits.update_install.expect("install click target");
        click(&mut app, target.x.saturating_sub(1), target.y);
        assert!(app.take_update_request().is_none());
        click(&mut app, 15, 2);
        assert_eq!(app.active_tab, Tab::Dashboard);
        assert!(matches!(app.overlay, Overlay::UpdatePrompt));
    }

    #[test]
    fn successful_install_requires_a_restart_and_keeps_the_banner() {
        let mut app = App::new();
        app.set_update_enabled(true);
        app.on_scan_message(ScanMessage::update_available(update_info()));
        app.on_key(key(KeyCode::Char('u')));
        app.on_key(key(KeyCode::Enter));
        let request = app.take_update_request().expect("install request");

        app.on_update_handler_result(
            &request,
            Ok(UpdateOutcome::new(true, "installed to ~/.cargo/bin")),
        );
        match &app.overlay {
            Overlay::UpdateResult(state) => {
                assert!(state.success);
                assert_eq!(state.version, "0.3.0");
            }
            _ => panic!("expected update result overlay"),
        }
        assert!(app.update_notice().expect("notice").installed());

        app.on_key(key(KeyCode::Enter));
        assert!(matches!(app.overlay, Overlay::None));
        assert!(app.update_notice().expect("notice").installed());
    }

    #[test]
    fn failed_install_keeps_the_update_action_available() {
        let mut app = App::new();
        app.set_update_enabled(true);
        app.on_scan_message(ScanMessage::update_available(update_info()));
        app.on_key(key(KeyCode::Char('u')));
        app.on_key(key(KeyCode::Enter));
        let request = app.take_update_request().expect("install request");

        app.on_update_handler_result(
            &request,
            Err(UpdateError::new("simulated installer failure")),
        );
        match &app.overlay {
            Overlay::UpdateResult(state) => {
                assert!(!state.success);
                assert!(state.output.contains("simulated installer failure"));
            }
            _ => panic!("expected update result overlay"),
        }
        assert!(!app.update_notice().expect("notice").installed());

        // A failed result offers a direct retry path.
        app.on_key(key(KeyCode::Char('u')));
        assert!(matches!(app.overlay, Overlay::UpdatePrompt));
    }

    #[test]
    fn update_key_does_not_hijack_filter_typing() {
        let mut app = App::new();
        app.set_update_enabled(true);
        app.on_scan_message(ScanMessage::update_available(update_info()));
        app.on_key(key(KeyCode::Char('/')));

        app.on_key(key(KeyCode::Char('u')));

        assert!(app.filter_edit().is_some());
        assert!(matches!(app.overlay, Overlay::None));
        assert_eq!(app.active_filter(), "u");
    }

    #[test]
    fn update_banner_is_visible_without_replacing_the_dashboard() {
        let mut app = App::new();
        app.on_scan_message(ScanMessage::snapshot(Snapshot::demo()));
        app.on_scan_message(ScanMessage::finished());
        app.set_update_enabled(true);
        app.on_scan_message(ScanMessage::update_available(update_info()));

        let backend = TestBackend::new(110, 30);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| crate::tui::views::draw(frame, &mut app))
            .expect("update frame");

        let screen = screen_text(&terminal);
        assert!(screen.contains("UPDATE AVAILABLE"));
        assert!(screen.contains("0.3.0"));
        assert!(screen.contains("Dashboard"));
        assert!(screen.contains("HEALTH"));

        app.on_key(key(KeyCode::Char('u')));
        terminal
            .draw(|frame| crate::tui::views::draw(frame, &mut app))
            .expect("prompt frame");
        let prompt = screen_text(&terminal);
        assert!(prompt.contains("UPDATE AVAILABLE"));
        assert!(prompt.contains("Install update"));
        assert!(prompt.contains("Click Install or press Enter"));
    }

    #[test]
    fn update_banner_and_prompt_render_on_compact_terminals() {
        let mut app = App::new();
        app.set_update_enabled(true);
        app.on_scan_message(ScanMessage::update_available(update_info()));

        for (width, height) in [(40, 12), (64, 20), (80, 24)] {
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).expect("terminal");
            terminal
                .draw(|frame| crate::tui::views::draw(frame, &mut app))
                .expect("compact banner frame");

            app.on_key(key(KeyCode::Char('u')));
            terminal
                .draw(|frame| crate::tui::views::draw(frame, &mut app))
                .expect("compact prompt frame");
            app.on_key(key(KeyCode::Esc));
        }
    }

    #[test]
    fn successful_install_result_renders_the_restart_instruction() {
        let mut app = App::new();
        app.set_update_enabled(true);
        app.on_scan_message(ScanMessage::update_available(update_info()));
        app.on_key(key(KeyCode::Char('u')));
        app.on_key(key(KeyCode::Enter));
        let request = app.take_update_request().expect("install request");
        app.on_update_handler_result(
            &request,
            Ok(UpdateOutcome::new(true, "installed to ~/.cargo/bin")),
        );

        let backend = TestBackend::new(110, 30);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| crate::tui::views::draw(frame, &mut app))
            .expect("result frame");

        let screen = screen_text(&terminal);
        assert!(screen.contains("RESTART AMPH"));
        assert!(screen.contains("installed to ~/.cargo/bin"));
    }

    #[test]
    fn shift_v_opens_the_channel_picker_and_marks_the_current_channel() {
        let mut app = App::new();
        app.set_update_enabled(true);
        app.on_scan_message(ScanMessage::update_channel(" main "));
        assert_eq!(app.update_channel(), "main");

        app.on_key(shift_key(KeyCode::Char('V')));

        assert!(matches!(app.overlay, Overlay::ChannelPicker));
        assert_eq!(app.highlighted_channel(), "main");

        let backend = TestBackend::new(110, 30);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| crate::tui::views::draw(frame, &mut app))
            .expect("picker frame");

        let screen = screen_text(&terminal);
        assert!(screen.contains("UPDATE CHANNEL"));
        assert!(screen.contains("SELECT A CHANNEL"));
        assert!(screen.contains("CURRENT  main"));
        assert!(screen.contains("development"));
        assert!(screen.contains("choose"));

        app.on_key(key(KeyCode::Esc));
        terminal
            .draw(|frame| crate::tui::views::draw(frame, &mut app))
            .expect("closed picker frame");
        assert!(screen_text(&terminal).contains("shift+v channel"));
    }

    #[test]
    fn channel_picker_renders_on_compact_terminals() {
        let mut app = App::new();
        app.set_update_enabled(true);
        app.on_scan_message(ScanMessage::update_channel("main"));

        for (width, height) in [(40, 12), (64, 20), (80, 24)] {
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).expect("terminal");
            app.on_key(shift_key(KeyCode::Char('V')));
            terminal
                .draw(|frame| crate::tui::views::draw(frame, &mut app))
                .expect("compact picker frame");
            app.on_key(key(KeyCode::Esc));
            assert!(matches!(app.overlay, Overlay::None));
        }
    }

    #[test]
    fn choosing_development_requests_an_install_from_that_branch() {
        let mut app = App::new();
        app.set_update_enabled(true);
        app.on_scan_message(ScanMessage::update_channel("main"));
        app.on_key(shift_key(KeyCode::Char('V')));
        app.on_key(key(KeyCode::Char('j')));

        assert_eq!(app.highlighted_channel(), "development");
        app.on_key(key(KeyCode::Enter));

        assert!(matches!(app.overlay, Overlay::UpdateBusy));
        assert_eq!(app.update_channel(), "development");
        let request = app.take_update_request().expect("branch install request");
        assert_eq!(request.version(), "latest development build");
        assert_eq!(request.branch(), "development");

        app.on_update_handler_result(
            &request,
            Ok(UpdateOutcome::new(true, "installed from development")),
        );
        assert!(matches!(app.overlay, Overlay::UpdateResult(_)));

        let backend = TestBackend::new(110, 30);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| crate::tui::views::draw(frame, &mut app))
            .expect("branch result frame");
        let screen = screen_text(&terminal);
        assert!(screen.contains("RESTART AMPH"));
        assert!(screen.contains("latest development build"));
    }

    #[test]
    fn choosing_the_current_channel_is_a_safe_no_op() {
        let mut app = App::new();
        app.set_update_enabled(true);
        app.on_scan_message(ScanMessage::update_channel("main"));
        app.on_key(shift_key(KeyCode::Char('V')));

        app.on_key(key(KeyCode::Enter));

        assert!(matches!(app.overlay, Overlay::None));
        assert_eq!(app.update_channel(), "main");
        assert!(app.take_update_request().is_none());
        assert!(
            app.active_toast()
                .is_some_and(|toast| toast.text.contains("already the current channel"))
        );
    }

    #[test]
    fn shift_v_does_not_hijack_filter_typing() {
        let mut app = App::new();
        app.on_key(key(KeyCode::Char('/')));

        app.on_key(shift_key(KeyCode::Char('V')));

        assert!(app.filter_edit().is_some());
        assert!(matches!(app.overlay, Overlay::None));
        assert_eq!(app.active_filter(), "V");
    }

    #[test]
    fn channel_picker_navigates_without_selecting() {
        let mut app = App::new();
        app.set_update_enabled(true);
        app.on_scan_message(ScanMessage::update_channel("development"));
        app.on_key(shift_key(KeyCode::Char('V')));
        assert_eq!(app.highlighted_channel(), "development");

        app.on_key(key(KeyCode::Char('k')));
        assert_eq!(app.highlighted_channel(), "main");
        assert!(matches!(app.overlay, Overlay::ChannelPicker));
        assert!(app.take_update_request().is_none());

        app.on_key(key(KeyCode::Enter));
        // main is now a different channel, so Enter requests its build.
        let request = app.take_update_request().expect("main install request");
        assert_eq!(request.version(), "latest main build");
        assert_eq!(request.branch(), "main");
    }

    #[test]
    fn update_notice_is_bound_to_the_selected_channel() {
        let mut app = App::new();
        app.set_update_enabled(true);
        app.on_scan_message(ScanMessage::update_channel("development"));
        app.on_scan_message(ScanMessage::update_available(
            UpdateInfo::new("0.3.0-beta", "0.4.0-beta").with_branch("development"),
        ));

        assert_eq!(
            app.update_notice().expect("notice").info().branch(),
            "development"
        );

        // A late background check from another channel must not replace the
        // notice for the selected branch.
        app.on_scan_message(ScanMessage::update_available(update_info()));
        assert_eq!(
            app.update_notice().expect("notice").info().branch(),
            "development"
        );

        let backend = TestBackend::new(110, 30);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| crate::tui::views::draw(frame, &mut app))
            .expect("development notice frame");
        let screen = screen_text(&terminal);
        assert!(screen.contains("CHANNEL DEVELOPMENT"));
        assert!(screen.contains("development"));

        app.on_key(key(KeyCode::Char('u')));
        terminal
            .draw(|frame| crate::tui::views::draw(frame, &mut app))
            .expect("development prompt frame");
        let prompt = screen_text(&terminal);
        assert!(prompt.contains("DEVELOPMENT CHANNEL"));
        assert!(prompt.contains("development branch"));

        app.on_key(key(KeyCode::Enter));
        let request = app.take_update_request().expect("development install");
        assert_eq!(request.version(), "0.4.0-beta");
        assert_eq!(request.branch(), "development");
    }

    #[test]
    fn graph_tree_receives_keyboard_events() {
        let mut app = App::new();
        app.on_scan_message(ScanMessage::snapshot(Snapshot::demo()));
        app.on_scan_message(ScanMessage::finished());
        app.active_tab = Tab::Graph;

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|frame| crate::tui::views::draw(frame, &mut app))
            .expect("first graph frame");
        let first = app
            .selected_package()
            .map(|package| package.name.clone())
            .expect("first graph selection");

        app.on_key(key(KeyCode::Char('j')));
        let second = app
            .selected_package()
            .map(|package| package.name.clone())
            .expect("second graph selection");
        assert_ne!(first, second);
        assert!(!app.graph_tree_state().selected().is_empty());
    }
}
