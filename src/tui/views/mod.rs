pub mod checks;
pub mod cleanup;
pub mod common;
pub mod dashboard;
pub mod detail;
pub mod footer;
pub mod graph;
pub mod harmony;
pub mod header;
pub mod overlays;
pub mod packages;
pub mod plugins;
pub mod timeline;
pub mod update;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Margin};
use ratatui::widgets::Block;

use crate::tui::app::{App, Tab};
use crate::tui::widgets;

pub fn draw(frame: &mut Frame, app: &mut App) {
    app.tick = app.tick.wrapping_add(1);
    app.hits.clear();

    let screen = frame.area();
    app.layout.viewport = screen;

    frame.render_widget(Block::default().style(app.theme.base_style()), screen);

    let margin = if screen.width > 40 { 1 } else { 0 };
    let inner = screen.inner(Margin {
        horizontal: margin,
        vertical: 0,
    });

    let header_height = if inner.height >= 22 { 2 } else { 1 };
    let scan_height = if app.scan.is_active() { 2 } else { 0 };
    // The update notice is one quiet line. On very short terminals it is
    // skipped rather than squeezing the working area; `u` still opens it.
    let update_height = if app.update_notice().is_some() && inner.height >= 10 {
        1
    } else {
        0
    };

    let chunks = Layout::vertical([
        Constraint::Length(header_height),
        Constraint::Length(1),
        Constraint::Length(update_height),
        Constraint::Length(scan_height),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .split(inner);

    header::render(frame, chunks[0], app);
    app.layout.tab_bar = chunks[1];
    app.hits.tabs = widgets::tab_bar(frame, chunks[1], app);

    if update_height > 0 {
        update::banner(frame, chunks[2], app);
    }

    if scan_height > 0 {
        common::scan_strip(frame, chunks[3], app);
    }

    let body = chunks[4];
    app.layout.body = body;

    let show_detail = app.detail_visible && body.width >= 74 && body.height >= 14;
    if show_detail {
        let panel_width = detail_width(body.width);
        let columns = Layout::horizontal([
            Constraint::Min(40),
            Constraint::Length(1),
            Constraint::Length(panel_width),
        ])
        .split(body);
        app.layout.detail = columns[2];
        active_view(frame, app, columns[0]);
        detail::render_side(frame, app, columns[2]);
    } else {
        app.layout.detail = ratatui::layout::Rect::default();
        active_view(frame, app, body);
    }

    footer::render(frame, chunks[5], app);
    overlays::render(frame, app, screen);
}

fn active_view(frame: &mut Frame, app: &mut App, area: ratatui::layout::Rect) {
    match app.active_tab {
        Tab::Dashboard => dashboard::render(frame, app, area),
        Tab::Packages => packages::render(frame, app, area),
        Tab::Graph => graph::render(frame, app, area),
        Tab::Cleanup => cleanup::render(frame, app, area),
        Tab::Harmony => harmony::render(frame, app, area),
        Tab::Checks => checks::render(frame, app, area),
        Tab::Timeline => timeline::render(frame, app, area),
    }
}

fn detail_width(width: u16) -> u16 {
    if width >= 130 {
        38
    } else if width >= 102 {
        34
    } else {
        28
    }
}
