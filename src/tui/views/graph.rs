use std::collections::BTreeSet;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span, Text};
use ratatui::widgets::Paragraph;
use tui_tree_widget::{Tree, TreeItem};

use crate::tui::app::App;
use crate::tui::model::Package;
use crate::tui::theme::Theme;
use crate::tui::widgets;

pub fn render(frame: &mut Frame, app: &mut App, area: Rect) {
    if area.height == 0 {
        return;
    }

    let theme = app.theme;
    let indices = app.graph_indices();
    let filter = app.active_filter().to_string();
    let metrics = &app.inventory.metrics;
    let graph = &app.inventory.graph;

    let chunks = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(if area.height >= 16 { 6 } else { 4 }),
        Constraint::Min(0),
    ])
    .split(area);

    let mut header_spans = vec![
        Span::styled(
            "GRAPH",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                "  {} nodes · {} edges · {} unresolved",
                metrics.total_packages, graph.edges, graph.missing_edges
            ),
            Style::default().fg(theme.muted),
        ),
    ];
    if !filter.is_empty() {
        header_spans.push(Span::styled(
            format!("  ·  /{filter} ({} matched)", indices.len()),
            Style::default().fg(theme.warning),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(header_spans)), chunks[0]);

    if indices.is_empty() {
        app.graph_tree_state.select(Vec::new());
        let block = widgets::panel("Dependency tree", &theme);
        let inner = block.inner(chunks[2]);
        frame.render_widget(block, chunks[2]);
        widgets::empty_state(
            frame,
            inner,
            "No package matches this filter",
            "Press Esc to clear the filter.",
            &theme,
        );
        return;
    }

    let items = build_items(app, &indices, &theme);
    ensure_selection(&mut app.graph_tree_state, &items);
    let selected_name = app.graph_tree_state.selected().last().cloned();
    let selected_index = selected_name
        .as_ref()
        .and_then(|name| app.inventory.packages.iter().position(|p| &p.name == name));

    if let Some(index) = selected_index {
        render_lens(frame, chunks[1], app, index, &theme);
    }

    if chunks[2].height < 3 {
        return;
    }

    let tree_area = chunks[2];
    let block = widgets::panel("Dependency tree", &theme).title_bottom(Line::from(vec![
        Span::styled(" space ", Style::default().fg(theme.accent)),
        Span::styled("toggle ", Style::default().fg(theme.muted)),
        Span::styled("←/→ ", Style::default().fg(theme.accent)),
        Span::styled("branch ", Style::default().fg(theme.muted)),
        Span::styled("enter ", Style::default().fg(theme.accent)),
        Span::styled("inspect ", Style::default().fg(theme.muted)),
    ]));

    let Ok(tree) = Tree::new(&items) else {
        let inner = block.inner(tree_area);
        frame.render_widget(block, tree_area);
        widgets::empty_state(
            frame,
            inner,
            "Dependency tree could not be built",
            "The underlying package list had duplicate identifiers.",
            &theme,
        );
        return;
    };

    let tree = tree
        .block(block)
        .style(theme.text_style())
        .highlight_style(theme.selection_style())
        .highlight_symbol("▍ ")
        .node_closed_symbol("▸ ")
        .node_open_symbol("▾ ")
        .node_no_children_symbol("· ")
        .experimental_scrollbar(Some(widgets::scrollbar(&theme)));

    frame.render_stateful_widget(tree, tree_area, &mut app.graph_tree_state);
}

fn build_items(app: &App, indices: &[usize], theme: &Theme) -> Vec<TreeItem<'static, String>> {
    let mut order = indices.to_vec();
    order.sort_by(|left, right| {
        let left_outgoing = app.inventory.graph.dependencies[*left].len();
        let right_outgoing = app.inventory.graph.dependencies[*right].len();
        right_outgoing
            .cmp(&left_outgoing)
            .then_with(|| {
                app.inventory.graph.degree[*right].cmp(&app.inventory.graph.degree[*left])
            })
            .then_with(|| {
                app.inventory.packages[*left]
                    .name
                    .cmp(&app.inventory.packages[*right].name)
            })
    });

    order
        .into_iter()
        .filter_map(|index| build_package_item(app, index, 2, &BTreeSet::new(), theme))
        .collect()
}

fn build_package_item(
    app: &App,
    index: usize,
    depth: u8,
    ancestors: &BTreeSet<String>,
    theme: &Theme,
) -> Option<TreeItem<'static, String>> {
    let package = app.inventory.packages.get(index)?;
    if ancestors.contains(&package.name) {
        return None;
    }

    let identifier = package.name.clone();
    let text = package_text(package, theme);
    let mut next_ancestors = ancestors.clone();
    next_ancestors.insert(identifier.clone());

    let mut children = Vec::new();
    if depth > 0 {
        let mut seen = BTreeSet::new();
        for child_index in &app.inventory.graph.dependencies[index] {
            let child = &app.inventory.packages[*child_index];
            if !seen.insert(child.name.clone()) {
                continue;
            }
            if let Some(child_item) =
                build_package_item(app, *child_index, depth - 1, &next_ancestors, theme)
            {
                children.push(child_item);
            }
        }
    }

    if children.is_empty() {
        Some(TreeItem::new_leaf(identifier, text))
    } else {
        match TreeItem::new(identifier.clone(), text.clone(), children.clone()) {
            Ok(item) => Some(item),
            Err(_) => Some(TreeItem::new_leaf(identifier, text)),
        }
    }
}

fn package_text(package: &Package, theme: &Theme) -> Text<'static> {
    let name_color = if package.is_orphan() {
        theme.warning
    } else if package.foreign {
        theme.accent
    } else {
        theme.text
    };

    let mut spans = vec![
        Span::styled(
            package.name.clone(),
            Style::default().fg(name_color).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {}", package.size_label()),
            Style::default().fg(theme.info),
        ),
    ];

    if package.is_cleanup_candidate() {
        spans.push(Span::styled(
            "  cleanup",
            Style::default().fg(theme.warning),
        ));
    } else if package.foreign {
        spans.push(Span::styled("  foreign", Style::default().fg(theme.accent)));
    }

    Text::from(Line::from(spans))
}

fn ensure_selection(state: &mut tui_tree_widget::TreeState<String>, items: &[TreeItem<String>]) {
    if items.is_empty() {
        state.select(Vec::new());
        return;
    }

    if tree_path_exists(items, state.selected()) {
        return;
    }

    let first = items[0].identifier().clone();
    state.select(vec![first.clone()]);
    let _ = state.open(vec![first]);
    state.scroll_selected_into_view();
}

fn tree_path_exists(items: &[TreeItem<String>], path: &[String]) -> bool {
    if path.is_empty() {
        return false;
    }

    let mut current = items;
    for identifier in path {
        let Some(item) = current.iter().find(|item| item.identifier() == identifier) else {
            return false;
        };
        current = item.children();
    }
    true
}

fn render_lens(frame: &mut Frame, area: Rect, app: &App, selected_index: usize, theme: &Theme) {
    let block = widgets::panel("Dependency lens", theme);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let package = &app.inventory.packages[selected_index];
    let dependents = &app.inventory.graph.dependents[selected_index];
    let dependencies = &app.inventory.graph.dependencies[selected_index];
    let degree = app.inventory.graph.degree[selected_index];
    let half = inner.width / 2;
    let mut lines = Vec::new();

    lines.push(Line::from(vec![
        Span::styled(
            format!("← REQUIRED BY ({})", dependents.len()),
            Style::default().fg(theme.info),
        ),
        Span::raw(" ".repeat(half.saturating_sub(17) as usize)),
        Span::styled(
            format!("DEPENDS ON ({}) →", dependencies.len()),
            Style::default().fg(theme.accent_alt),
        ),
    ]));

    let neighbor_rows = inner.height.saturating_sub(2) as usize;
    for row in 0..neighbor_rows {
        let left = dependents
            .get(row)
            .map(|index| app.inventory.packages[*index].name.as_str())
            .unwrap_or("");
        let right = dependencies
            .get(row)
            .map(|index| app.inventory.packages[*index].name.as_str())
            .unwrap_or("");
        let left_text = if left.is_empty() {
            String::new()
        } else {
            format!("← {}", widgets::truncate(left, half.saturating_sub(3)))
        };
        let right_text = if right.is_empty() {
            String::new()
        } else {
            format!("{} →", widgets::truncate(right, half.saturating_sub(3)))
        };
        let padding = half.saturating_sub(left_text.chars().count() as u16);
        lines.push(Line::from(vec![
            Span::styled(left_text, Style::default().fg(theme.text)),
            Span::raw(" ".repeat(padding as usize)),
            Span::styled(right_text, Style::default().fg(theme.text)),
        ]));
    }

    lines.push(Line::from(vec![
        Span::styled(
            "◆ ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            package.name.clone(),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  ·  {degree} links"),
            Style::default().fg(theme.muted),
        ),
    ]));

    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::App;

    #[test]
    fn demo_dependency_tree_builds_with_unique_identifiers() {
        let app = App::new();
        let indices = app.graph_indices();
        assert!(!indices.is_empty());
        let items = build_items(&app, &indices, &app.theme);
        assert!(!items.is_empty());
        assert!(Tree::new(&items).is_ok());
    }

    #[test]
    fn tree_selection_paths_are_validated() {
        let app = App::new();
        let indices = app.graph_indices();
        let items = build_items(&app, &indices, &app.theme);
        let first = items[0].identifier().clone();
        assert!(tree_path_exists(&items, std::slice::from_ref(&first)));
        assert!(!tree_path_exists(&items, &["does-not-exist".to_string()]));
        assert!(!tree_path_exists(&items, &[]));
    }
}
