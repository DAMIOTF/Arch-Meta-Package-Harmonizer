use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::{
    Block, BorderType, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Widget,
};

use crate::tui::app::{App, Tab};
use crate::tui::model::format_size;
use crate::tui::theme::Theme;

pub fn panel(title: &str, theme: &Theme) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border))
        .style(theme.panel_style())
        .title(Line::from(vec![
            Span::raw(" "),
            Span::styled(
                title.to_ascii_uppercase(),
                Style::default()
                    .fg(theme.muted)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" "),
        ]))
}

pub fn accent_panel(title: &str, theme: &Theme, accent: Color) -> Block<'static> {
    panel(title, theme).border_style(Style::default().fg(accent))
}

pub fn tab_bar(frame: &mut Frame, area: Rect, app: &App) -> Vec<(Rect, Tab)> {
    let theme = &app.theme;
    frame.render_widget(Block::default().style(theme.mantle_style()), area);

    let numbered = Tab::ALL
        .iter()
        .map(|tab| format!("{} {}", tab.index() + 1, tab.label()))
        .collect::<Vec<_>>();
    let labels = Tab::ALL
        .iter()
        .map(|tab| tab.label().to_string())
        .collect::<Vec<_>>();
    let numbers = (1..=Tab::ALL.len())
        .map(|index| index.to_string())
        .collect::<Vec<_>>();

    let numbered_width = total_width(&numbered);
    let label_width = total_width(&labels);
    let number_width = total_width(&numbers);

    let (texts, compact_leading) = if numbered_width + 2 <= area.width as usize {
        (numbered, 1u16)
    } else if label_width + 2 <= area.width as usize {
        (labels, 1u16)
    } else if number_width + 2 <= area.width as usize {
        (numbers, 1u16)
    } else {
        (numbers, 0u16)
    };

    let mut x = area.x.saturating_add(compact_leading);
    let mut hits = Vec::new();

    for (index, text) in texts.iter().enumerate() {
        if index > 0 {
            x = x.saturating_add(1);
        }

        let width = text.chars().count() as u16;
        if x.saturating_add(width) > area.x.saturating_add(area.width) {
            break;
        }

        let tab = Tab::ALL[index];
        let style = if tab == app.active_tab {
            theme.tab_active_style()
        } else {
            theme.tab_inactive_style()
        };

        frame
            .buffer_mut()
            .set_stringn(x, area.y, text.as_str(), width as usize, style);
        hits.push((
            Rect {
                x,
                y: area.y,
                width,
                height: 1,
            },
            tab,
        ));
        x = x.saturating_add(width);
    }

    let remaining = area.x.saturating_add(area.width).saturating_sub(x);
    if remaining >= 24 {
        let summary = format!(
            "{} PKG · {} · {} ORPHAN ",
            app.inventory.metrics.total_packages,
            crate::tui::model::format_size(app.inventory.metrics.total_size),
            app.inventory.metrics.orphans
        );
        let summary_width = summary.chars().count() as u16;
        if summary_width <= remaining {
            frame.buffer_mut().set_stringn(
                area.x
                    .saturating_add(area.width)
                    .saturating_sub(summary_width),
                area.y,
                &summary,
                summary_width as usize,
                theme.muted_style(),
            );
        }
    }

    hits
}

fn total_width(items: &[String]) -> usize {
    items.iter().map(|item| item.chars().count()).sum::<usize>() + items.len().saturating_sub(1)
}

/// Large block numerals for the Harmony hero. Three digits fit comfortably.
pub fn big_number_lines(value: u32, color: Color) -> Vec<Line<'static>> {
    let digits = value.to_string();
    let mut rows = vec![String::new(); 5];

    for (index, digit) in digits.chars().enumerate() {
        if index > 0 {
            for row in &mut rows {
                row.push(' ');
            }
        }
        let pattern = digit_pattern(digit);
        for (row, line) in rows.iter_mut().zip(pattern.iter()) {
            row.push_str(line);
        }
    }

    rows.into_iter()
        .map(|row| {
            Line::from(Span::styled(
                row,
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            ))
        })
        .collect()
}

pub fn big_number_width(value: u32) -> u16 {
    (value.to_string().chars().count() as u16) * 4 - 1
}

fn digit_pattern(digit: char) -> [&'static str; 5] {
    match digit {
        '0' => ["███", "█ █", "█ █", "█ █", "███"],
        '1' => [" ██", "  █", "  █", "  █", "███"],
        '2' => ["███", "  █", "███", "█  ", "███"],
        '3' => ["███", "  █", "███", "  █", "███"],
        '4' => ["█ █", "█ █", "███", "  █", "  █"],
        '5' => ["███", "█  ", "███", "  █", "███"],
        '6' => ["███", "█  ", "███", "█ █", "███"],
        '7' => ["███", "  █", "  █", "  █", "  █"],
        '8' => ["███", "█ █", "███", "█ █", "███"],
        '9' => ["███", "█ █", "███", "  █", "███"],
        _ => ["   ", "   ", "   ", "   ", "   "],
    }
}

pub fn meter_line(label: &str, value: u8, width: u16, theme: &Theme) -> Line<'static> {
    let width = width.max(4);
    let filled = ((width as u32 * value.min(100) as u32) / 100) as usize;
    let empty = width as usize - filled;
    let color = theme.score_color(value);

    Line::from(vec![
        Span::styled(format!("{label:<14}"), Style::default().fg(theme.text)),
        Span::styled("█".repeat(filled), Style::default().fg(color)),
        Span::styled("░".repeat(empty), Style::default().fg(theme.surface_alt)),
        Span::styled(
            format!(" {value:>3}"),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
    ])
}

pub fn hint_spans(hints: &[(&str, &str)], theme: &Theme) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (index, (key, action)) in hints.iter().enumerate() {
        if index > 0 {
            spans.push(Span::styled(" · ", Style::default().fg(theme.border)));
        }
        spans.push(Span::styled(
            (*key).to_string(),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            (*action).to_string(),
            Style::default().fg(theme.muted),
        ));
    }
    spans
}

pub fn hint_line(hints: &[(&str, &str)], theme: &Theme) -> Line<'static> {
    Line::from(hint_spans(hints, theme))
}

pub fn truncate(text: &str, width: u16) -> String {
    let width = width as usize;
    if width == 0 {
        return String::new();
    }
    let count = text.chars().count();
    if count <= width {
        return text.to_string();
    }
    if width == 1 {
        return "…".to_string();
    }
    let mut value = text.chars().take(width - 1).collect::<String>();
    value.push('…');
    value
}

pub fn centered_rect(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let width = ((area.width as u32 * percent_x as u32) / 100)
        .max(36)
        .min(area.width as u32) as u16;
    let height = ((area.height as u32 * percent_y as u32) / 100)
        .max(12)
        .min(area.height as u32) as u16;
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

pub fn centered_rect_height(area: Rect, percent_x: u16, height: u16) -> Rect {
    let width = ((area.width as u32 * percent_x as u32) / 100)
        .max(36)
        .min(area.width as u32) as u16;
    let height = height.min(area.height);
    Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    }
}

pub fn scrollbar(theme: &Theme) -> Scrollbar<'static> {
    Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .thumb_symbol("▐")
        .track_symbol(Some("│"))
        .begin_symbol(None)
        .end_symbol(None)
        .style(Style::default().fg(theme.border))
        .thumb_style(Style::default().fg(theme.accent))
        .track_style(Style::default().fg(theme.border))
}

pub fn render_scrollbar(
    frame: &mut Frame,
    area: Rect,
    len: usize,
    position: usize,
    visible: usize,
    theme: &Theme,
) {
    if len == 0 || len <= visible || area.height < 4 || area.width < 3 {
        return;
    }

    let bar_area = Rect {
        x: area.x.saturating_add(area.width.saturating_sub(1)),
        y: area.y.saturating_add(1),
        width: 1,
        height: area.height.saturating_sub(2),
    };
    let mut state = ScrollbarState::new(len)
        .viewport_content_length(visible.max(1))
        .position(position.min(len.saturating_sub(1)));
    frame.render_stateful_widget(scrollbar(theme), bar_area, &mut state);
}

pub fn empty_state(frame: &mut Frame, area: Rect, title: &str, hint: &str, theme: &Theme) {
    if area.height < 4 {
        return;
    }

    let lines = vec![
        Line::from(Span::styled(
            "╌╌╌╌╌╌╌╌╌╌",
            Style::default().fg(theme.border),
        )),
        Line::from(Span::styled(
            title.to_string(),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            hint.to_string(),
            Style::default().fg(theme.muted),
        )),
    ];

    let paragraph = Paragraph::new(Text::from(lines)).alignment(Alignment::Center);
    let height = 3u16.min(area.height);
    let content = Rect {
        x: area.x,
        y: area.y + area.height.saturating_sub(height) / 2,
        width: area.width,
        height,
    };
    frame.render_widget(paragraph, content);
}

pub fn spinner(tick: u64) -> &'static str {
    const FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
    FRAMES[(tick as usize / 3) % FRAMES.len()]
}

pub fn score_word(score: u8) -> &'static str {
    if score >= 85 {
        "resonant"
    } else if score >= 70 {
        "steady"
    } else if score >= 50 {
        "uneven"
    } else {
        "frayed"
    }
}

pub fn format_elapsed(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    let minutes = seconds / 60;
    let seconds = seconds % 60;
    format!("{minutes:02}:{seconds:02}")
}

/// One proportional block in the dashboard disk treemap.
#[derive(Debug, Clone)]
pub struct TreemapItem {
    pub label: String,
    pub value: u64,
    pub color: Color,
}

impl TreemapItem {
    pub fn new(label: impl Into<String>, value: u64, color: Color) -> Self {
        Self {
            label: label.into(),
            value: value.max(1),
            color,
        }
    }
}

/// A small proportional treemap used on the Dashboard.
///
/// The layout is a deterministic binary slice: items are split into two groups
/// with approximately equal total value, then each group receives a
/// proportional slice of the longer screen axis. All leaf rectangles are
/// proportional to their values and contained in the original area.
pub struct DiskTreemap<'a> {
    items: &'a [TreemapItem],
    text_color: Color,
}

impl<'a> DiskTreemap<'a> {
    pub fn new(items: &'a [TreemapItem], text_color: Color) -> Self {
        Self { items, text_color }
    }
}

impl Widget for DiskTreemap<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 || self.items.is_empty() {
            return;
        }

        let mut placements = Vec::new();
        let indices = (0..self.items.len()).collect::<Vec<_>>();
        layout_treemap(self.items, &indices, area, &mut placements);

        for (rect, index) in placements {
            let item = &self.items[index];
            let fill = Style::default().bg(item.color).fg(self.text_color);
            for y in rect.y..rect.y.saturating_add(rect.height) {
                buf.set_stringn(
                    rect.x,
                    y,
                    " ".repeat(rect.width as usize),
                    rect.width as usize,
                    fill,
                );
            }

            if rect.width < 4 {
                continue;
            }
            let text_width = rect.width.saturating_sub(2);
            let label = truncate(&item.label, text_width);
            let label_y = rect.y + rect.height.saturating_sub(1) / 2;
            buf.set_stringn(
                rect.x.saturating_add(1),
                label_y,
                &label,
                text_width as usize,
                fill,
            );

            if rect.height >= 3 {
                let size = format_size(item.value);
                let size_y = label_y.saturating_add(1);
                if size.chars().count() as u16 <= text_width
                    && size_y < rect.y.saturating_add(rect.height)
                {
                    buf.set_stringn(
                        rect.x.saturating_add(1),
                        size_y,
                        &size,
                        text_width as usize,
                        fill,
                    );
                }
            }
        }
    }
}

fn layout_treemap(
    items: &[TreemapItem],
    indices: &[usize],
    area: Rect,
    out: &mut Vec<(Rect, usize)>,
) {
    if items.is_empty() || indices.is_empty() || area.width == 0 || area.height == 0 {
        return;
    }
    if items.len() == 1 {
        out.push((area, indices[0]));
        return;
    }

    let total = items
        .iter()
        .map(|item| item.value.max(1))
        .sum::<u64>()
        .max(1);
    let mut accumulated = 0u64;
    let mut split = 1usize;
    for (index, item) in items.iter().enumerate() {
        accumulated = accumulated.saturating_add(item.value.max(1));
        if accumulated.saturating_mul(2) >= total {
            split = index + 1;
            break;
        }
    }
    let split = split.clamp(1, items.len() - 1);
    let left_total = items[..split]
        .iter()
        .map(|item| item.value.max(1))
        .sum::<u64>();
    let ratio = left_total as f64 / total as f64;

    if area.width >= area.height && area.width >= 2 {
        let left_width =
            ((area.width as f64 * ratio).round() as u16).clamp(1, area.width.saturating_sub(1));
        let left_area = Rect {
            x: area.x,
            y: area.y,
            width: left_width,
            height: area.height,
        };
        let right_area = Rect {
            x: area.x.saturating_add(left_width),
            y: area.y,
            width: area.width.saturating_sub(left_width),
            height: area.height,
        };
        layout_treemap(&items[..split], &indices[..split], left_area, out);
        layout_treemap(&items[split..], &indices[split..], right_area, out);
    } else if area.height >= 2 {
        let top_height =
            ((area.height as f64 * ratio).round() as u16).clamp(1, area.height.saturating_sub(1));
        let top_area = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: top_height,
        };
        let bottom_area = Rect {
            x: area.x,
            y: area.y.saturating_add(top_height),
            width: area.width,
            height: area.height.saturating_sub(top_height),
        };
        layout_treemap(&items[..split], &indices[..split], top_area, out);
        layout_treemap(&items[split..], &indices[split..], bottom_area, out);
    } else {
        out.push((area, indices[0]));
    }
}

#[cfg(test)]
mod treemap_tests {
    use super::*;

    fn items() -> Vec<TreemapItem> {
        vec![
            TreemapItem::new("alpha", 50, Color::Red),
            TreemapItem::new("beta", 30, Color::Green),
            TreemapItem::new("gamma", 20, Color::Blue),
        ]
    }

    #[test]
    fn layout_covers_each_item_without_overlap() {
        let area = Rect::new(0, 0, 20, 10);
        let items = items();
        let mut placements = Vec::new();
        layout_treemap(&items, &[0, 1, 2], area, &mut placements);

        assert_eq!(placements.len(), 3);
        for (rect, _) in &placements {
            assert!(rect.x >= area.x);
            assert!(rect.y >= area.y);
            assert!(rect.right() <= area.right());
            assert!(rect.bottom() <= area.bottom());
        }
        for left in 0..placements.len() {
            for right in (left + 1)..placements.len() {
                let a = placements[left].0;
                let b = placements[right].0;
                let overlap = a.intersection(b);
                assert_eq!(overlap.area(), 0);
            }
        }
    }

    #[test]
    fn larger_values_get_larger_areas() {
        let area = Rect::new(0, 0, 30, 12);
        let items = items();
        let mut placements = Vec::new();
        layout_treemap(&items, &[0, 1, 2], area, &mut placements);
        let area_for = |index: usize| {
            placements
                .iter()
                .find(|(_, item)| *item == index)
                .map(|(rect, _)| rect.area())
                .unwrap()
        };
        assert!(area_for(0) > area_for(2));
    }
}
