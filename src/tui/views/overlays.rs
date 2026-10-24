use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};

use crate::tui::app::{
    App, CleanupErrorState, CleanupPreviewState, CleanupResultState, Overlay, ToastTone,
};
use crate::tui::theme::Theme;
use crate::tui::widgets;

pub fn render(frame: &mut Frame, app: &mut App, screen: Rect) {
    if matches!(app.overlay(), Overlay::None)
        && let Some(toast) = app.active_toast()
    {
        render_toast(frame, screen, toast.text.as_str(), toast.tone, &app.theme);
    }

    if !matches!(app.overlay(), Overlay::None | Overlay::Detail) && app.layout.body.height > 0 {
        // Give modal states a quiet stage instead of leaving the dashboard
        // visible around the edges. The header, tabs, scan strip, and footer
        // remain in place for orientation.
        frame.render_widget(Clear, app.layout.body);
        frame.render_widget(
            Block::default().style(app.theme.base_style()),
            app.layout.body,
        );
    }

    // The prompt records its Install click target while drawing, so it needs a
    // mutable app; the other overlays stay read-only.
    if matches!(app.overlay(), Overlay::UpdatePrompt) {
        crate::tui::views::update::render_prompt(frame, screen, app);
        return;
    }

    match app.overlay() {
        Overlay::None => {}
        Overlay::Help => render_help(frame, screen, app),
        Overlay::Detail => crate::tui::views::detail::render_overlay(frame, app, screen),
        Overlay::CleanupBusy => render_cleanup_busy(frame, screen, app),
        Overlay::CleanupPreview(state) => render_cleanup_preview(frame, screen, app, state),
        Overlay::CleanupResult(state) => render_cleanup_result(frame, screen, app, state),
        Overlay::CleanupError(state) => render_cleanup_error(frame, screen, app, state),
        Overlay::UpdatePrompt => {}
        Overlay::UpdateBusy => crate::tui::views::update::render_busy(frame, screen, app),
        Overlay::UpdateResult(state) => {
            crate::tui::views::update::render_result(frame, screen, app, state);
        }
        Overlay::ChannelPicker => {
            crate::tui::views::update::render_channel_picker(frame, screen, app);
        }
        Overlay::Plugins => {
            crate::tui::views::plugins::render_manager(frame, screen, app);
        }
    }
}

fn render_toast(frame: &mut Frame, screen: Rect, text: &str, tone: ToastTone, theme: &Theme) {
    let color = match tone {
        ToastTone::Info => theme.info,
        ToastTone::Success => theme.positive,
        ToastTone::Warning => theme.warning,
        ToastTone::Danger => theme.danger,
    };
    let text = widgets::truncate(text, 46);
    let width = (text.chars().count() as u16 + 3).min(screen.width);
    if width < 4 || screen.height < 2 {
        return;
    }
    let area = Rect {
        x: screen.x + screen.width.saturating_sub(width + 1),
        y: screen.y + screen.height.saturating_sub(3),
        width,
        height: 1,
    };

    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" {text} "),
            Style::default()
                .fg(color)
                .bg(theme.surface_alt)
                .add_modifier(Modifier::BOLD),
        ))
        .alignment(Alignment::Left),
        area,
    );
}

fn render_help(frame: &mut Frame, screen: Rect, app: &App) {
    let theme = &app.theme;
    let stage = if app.layout.body.height >= 16 {
        app.layout.body
    } else {
        screen
    };
    let area = widgets::centered_rect(stage, 94, 96);
    frame.render_widget(Clear, area);
    let block = widgets::accent_panel("Keys · ? close", theme, theme.accent);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    if inner.width < 66 {
        let lines = vec![
            help_heading("ESSENTIALS", theme),
            key_line("j / k", "move through the active list", theme),
            key_line("h / l", "previous / next tab", theme),
            key_line("1 – 7", "jump to a tab", theme),
            key_line("/", "filter the active list", theme),
            key_line("space", "pick a cleanup candidate", theme),
            key_line("p", "preview cleanup", theme),
            key_line("i", "inspect panel", theme),
            key_line("u", "update AMPH", theme),
            key_line("shift+v", "update channel", theme),
            key_line("shift+p", "plugin manager", theme),
            key_line("t", "cycle theme", theme),
            key_line("q", "quit", theme),
            Line::from(""),
            Line::from(Span::styled(
                "The TUI never runs pacman; the handler does.",
                Style::default().fg(theme.warning),
            )),
        ];
        frame.render_widget(
            Paragraph::new(Text::from(lines)).wrap(Wrap { trim: true }),
            inner,
        );
        return;
    }

    let columns = Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
        .spacing(2)
        .split(inner);

    let left = vec![
        help_heading("MOVE", theme),
        key_line("j / k", "down / up", theme),
        key_line("g / G", "first / last", theme),
        key_line("PgUp / PgDn", "jump by ten", theme),
        key_line("h / l", "previous / next tab", theme),
        key_line("1 – 7", "jump to a tab", theme),
        key_line("← / →", "Graph branch", theme),
        help_heading("SEARCH", theme),
        key_line("/", "filter the active list", theme),
        key_line("enter", "apply filter", theme),
        key_line("esc", "clear filter / close", theme),
        help_heading("INTERFACE", theme),
        key_line("i", "toggle inspect panel", theme),
        key_line("u", "review / install update", theme),
        key_line("shift+v", "switch update channel", theme),
        key_line("shift+p", "plugin manager", theme),
        key_line("t / T", "next / previous theme", theme),
        key_line("q", "quit", theme),
    ];

    let right = vec![
        help_heading("CLEANUP", theme),
        key_line("space / x", "toggle candidate", theme),
        key_line("a / c", "select all / clear", theme),
        key_line("p / enter", "open preview", theme),
        help_heading("SAFETY", theme),
        Line::from(Span::styled(
            "TUI never runs pacman.",
            Style::default().fg(theme.warning),
        )),
        Line::from(Span::styled(
            "Preview, then type remove once.",
            Style::default().fg(theme.warning),
        )),
        Line::from(Span::styled(
            "Removal may ask for sudo.",
            Style::default().fg(theme.warning),
        )),
        help_heading("UPDATE", theme),
        key_line("u", "review / install", theme),
        key_line("shift+v", "channel picker", theme),
        Line::from(Span::styled(
            "main · stable",
            Style::default().fg(theme.text),
        )),
        Line::from(Span::styled(
            "development · newer builds",
            Style::default().fg(theme.text),
        )),
        Line::from(Span::styled(
            "Restart AMPH after installing.",
            Style::default().fg(theme.warning),
        )),
        help_heading("PLUGINS", theme),
        key_line("shift+p", "open manager", theme),
        key_line("tab / 1-3", "switch section", theme),
        key_line("enter", "review / install", theme),
        key_line("space", "enable / disable", theme),
        key_line("a", "analyze functionality", theme),
        key_line("T", "apply theme (session)", theme),
        key_line("c / U", "check / update all", theme),
        key_line("R", "refresh plugin list", theme),
        Line::from(Span::styled(
            "Paste a GitHub repository URL.",
            Style::default().fg(theme.text),
        )),
        Line::from(Span::styled(
            "Themes are declarative; functionality uses ABI v1.",
            Style::default().fg(theme.muted),
        )),
        Line::from(Span::styled(
            "Release assets require GitHub SHA-256 digests.",
            Style::default().fg(theme.muted),
        )),
        help_heading("THEMES", theme),
        Line::from(Span::styled(
            "Catppuccin · Tokyo Night",
            Style::default().fg(theme.text),
        )),
        Line::from(Span::styled(
            "Gruvbox · Nord",
            Style::default().fg(theme.text),
        )),
        Line::from(Span::styled(
            "NO_COLOR=1 · monochrome",
            Style::default().fg(theme.muted),
        )),
        help_heading("MOUSE", theme),
        key_line("click", "tabs, rows, tree", theme),
        key_line("wheel", "scroll active list", theme),
    ];

    frame.render_widget(
        Paragraph::new(Text::from(left)).wrap(Wrap { trim: true }),
        columns[0],
    );
    frame.render_widget(
        Paragraph::new(Text::from(right)).wrap(Wrap { trim: true }),
        columns[1],
    );
}

fn help_heading(text: &str, theme: &Theme) -> Line<'static> {
    Line::from(Span::styled(
        text.to_string(),
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
    ))
}

fn key_line(key: &str, action: &str, theme: &Theme) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!("{key:<12}"),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(action.to_string(), Style::default().fg(theme.text)),
    ])
}

fn render_cleanup_busy(frame: &mut Frame, screen: Rect, app: &App) {
    let theme = &app.theme;
    let stage = if app.layout.body.height >= 16 {
        app.layout.body
    } else {
        screen
    };
    let area = widgets::centered_rect_height(stage, 56, 7);
    frame.render_widget(Clear, area);
    let block = widgets::accent_panel("Cleanup handler", theme, theme.warning);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height > 0 {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "Terminal suspended · waiting for handler",
                Style::default().fg(theme.muted),
            )))
            .wrap(Wrap { trim: true }),
            inner,
        );
    }
}

fn render_cleanup_preview(frame: &mut Frame, screen: Rect, app: &App, state: &CleanupPreviewState) {
    let theme = &app.theme;
    let stage = if app.layout.body.height >= 16 {
        app.layout.body
    } else {
        screen
    };
    let height = stage.height.saturating_sub(2).clamp(12, 22);
    let area = widgets::centered_rect_height(stage, 92, height);
    frame.render_widget(Clear, area);
    let block = widgets::accent_panel("Cleanup preview · handler output", theme, theme.warning);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .split(inner);

    let status = if state.can_remove {
        "REMOVAL AVAILABLE"
    } else {
        "READ-ONLY PREVIEW"
    };
    let status_color = if state.can_remove {
        theme.warning
    } else {
        theme.muted
    };
    let mut header = vec![
        Span::styled(
            format!("{} package(s)", state.packages.len()),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ·  ", Style::default().fg(theme.border)),
        Span::styled(
            status,
            Style::default()
                .fg(status_color)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(note) = &state.note {
        header.push(Span::styled("  ·  ", Style::default().fg(theme.border)));
        header.push(Span::styled(
            widgets::truncate(note, inner.width.saturating_sub(30)),
            Style::default().fg(theme.muted),
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(header)),
        Rect {
            x: rows[0].x,
            y: rows[0].y,
            width: rows[0].width,
            height: 1,
        },
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Output from the injected handler. The TUI never executes the command.",
            Style::default().fg(theme.muted),
        ))),
        Rect {
            x: rows[0].x,
            y: rows[0].y + 1,
            width: rows[0].width,
            height: 1,
        },
    );

    render_output_block(
        frame,
        rows[1],
        &state.output,
        state.scroll,
        theme,
        "Handler output",
    );

    if state.can_remove {
        let typed_color = if state.typed == "remove" {
            theme.positive
        } else {
            theme.text
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("TYPE ", Style::default().fg(theme.muted)),
                Span::styled(
                    "remove",
                    Style::default()
                        .fg(theme.danger)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("  > ", Style::default().fg(theme.border)),
                Span::styled(
                    state.typed.clone(),
                    Style::default()
                        .fg(typed_color)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled("▌", Style::default().fg(theme.accent)),
            ])),
            rows[2],
        );
    } else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "Removal is disabled by the handler for this preview.",
                Style::default().fg(theme.warning),
            ))),
            rows[2],
        );
    }

    if let Some(error) = &state.error {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                error.clone(),
                Style::default().fg(theme.danger),
            ))),
            rows[3],
        );
    }

    let hint = if state.can_remove {
        "↑/↓ scroll · type remove then Enter · Esc cancel"
    } else {
        "↑/↓ scroll · Esc close"
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            hint,
            Style::default().fg(theme.muted),
        ))),
        rows[4],
    );
}

fn render_cleanup_result(frame: &mut Frame, screen: Rect, app: &App, state: &CleanupResultState) {
    let theme = &app.theme;
    let stage = if app.layout.body.height >= 16 {
        app.layout.body
    } else {
        screen
    };
    let height = stage.height.saturating_sub(2).clamp(10, 22);
    let area = widgets::centered_rect_height(stage, 90, height);
    frame.render_widget(Clear, area);
    let accent = if state.success {
        theme.positive
    } else {
        theme.danger
    };
    let title = if state.success {
        "Cleanup result · success"
    } else {
        "Cleanup result · failed"
    };
    let block = widgets::accent_panel(title, theme, accent);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let rows = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(2),
        Constraint::Length(1),
    ])
    .split(inner);

    let mut header = vec![
        Span::styled(
            format!("{} package(s)", state.packages.len()),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ·  ", Style::default().fg(theme.border)),
        Span::styled(
            if state.success {
                "handler reported success"
            } else {
                "handler reported failure"
            },
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(note) = &state.note {
        header.push(Span::styled("  ·  ", Style::default().fg(theme.border)));
        header.push(Span::styled(
            widgets::truncate(note, inner.width.saturating_sub(30)),
            Style::default().fg(theme.muted),
        ));
    }
    frame.render_widget(
        Paragraph::new(Line::from(header)),
        Rect {
            x: rows[0].x,
            y: rows[0].y,
            width: rows[0].width,
            height: 1,
        },
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            if app.stale_snapshot() {
                "Package data may be stale. Restart the TUI to rescan."
            } else {
                "Review the handler output below."
            },
            Style::default().fg(if app.stale_snapshot() {
                theme.warning
            } else {
                theme.muted
            }),
        ))),
        Rect {
            x: rows[0].x,
            y: rows[0].y + 1,
            width: rows[0].width,
            height: 1,
        },
    );

    render_output_block(
        frame,
        rows[1],
        &state.output,
        state.scroll,
        theme,
        "Handler output",
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "↑/↓ scroll · Esc close",
            Style::default().fg(theme.muted),
        ))),
        rows[2],
    );
}

fn render_cleanup_error(frame: &mut Frame, screen: Rect, app: &App, state: &CleanupErrorState) {
    let theme = &app.theme;
    let stage = if app.layout.body.height >= 16 {
        app.layout.body
    } else {
        screen
    };
    let area = widgets::centered_rect_height(stage, 82, 12);
    frame.render_widget(Clear, area);
    let block = widgets::accent_panel(&state.title, theme, theme.danger);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let mut lines = vec![Line::from(Span::styled(
        state.message.clone(),
        Style::default().fg(theme.text),
    ))];
    if state.removal {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "The package set may be stale. Restart the TUI to rescan before another attempt.",
            Style::default().fg(theme.warning),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Esc close",
        Style::default().fg(theme.muted),
    )));
    frame.render_widget(
        Paragraph::new(Text::from(lines)).wrap(Wrap { trim: true }),
        inner,
    );
}

pub(super) fn render_output_block(
    frame: &mut Frame,
    area: Rect,
    output: &str,
    scroll: u16,
    theme: &Theme,
    title: &str,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let block = widgets::panel(title, theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let text = if output.trim().is_empty() {
        "No output was returned by the handler.".to_string()
    } else {
        output.to_string()
    };
    let lines = text.lines().count().max(1);
    frame.render_widget(
        Paragraph::new(text)
            .style(Style::default().fg(theme.text))
            .wrap(Wrap { trim: false })
            .scroll((scroll, 0)),
        inner,
    );
    widgets::render_scrollbar(
        frame,
        area,
        lines,
        scroll as usize,
        inner.height as usize,
        theme,
    );
}
