use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, List, ListItem, ListState, Paragraph, Scrollbar, ScrollbarOrientation,
        ScrollbarState, Wrap,
    },
    Frame,
};

use super::app::{App, Focus, Screen};
use crate::config::Theme;

pub fn draw(frame: &mut Frame, app: &App, theme: &Theme) {
    match app.screen {
        Screen::Search => draw_search(frame, app, theme),
        Screen::Episodes => draw_episodes(frame, app, theme),
    }
}

/// Builds a bordered block styled per `theme`, brighter (accent-colored) when `focused`.
fn themed_block<'a>(theme: &Theme, title: &'a str, focused: bool) -> Block<'a> {
    let border_color = if focused { theme.accent } else { theme.muted };
    Block::default()
        .borders(Borders::ALL)
        .border_type(theme.border_type.to_ratatui())
        .border_style(Style::default().fg(border_color))
        .title(title)
}

fn draw_search(frame: &mut Frame, app: &App, theme: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let query = Paragraph::new(format!("{}_", app.query))
        .style(Style::default().fg(theme.text))
        .block(themed_block(theme, "Search", app.focus == Focus::Query));
    frame.render_widget(query, chunks[0]);

    let results_title = if app.loading {
        "Results (loading...)"
    } else {
        "Results"
    };
    let items: Vec<ListItem> = app
        .results
        .iter()
        .map(|result| {
            let (label, color) = theme.source_style(&result.id.prefix);
            ListItem::new(Line::from(vec![
                Span::styled(
                    format!("[{label}] "),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
                Span::styled(result.title.clone(), Style::default().fg(theme.text)),
            ]))
        })
        .collect();

    let results_focused = app.focus == Focus::Results;
    let highlight_style = if results_focused {
        Style::default().fg(theme.selection_fg).bg(theme.selection_bg)
    } else {
        Style::default()
    };
    let results = List::new(items)
        .block(themed_block(theme, results_title, results_focused))
        .highlight_style(highlight_style);
    let mut results_state = ListState::default().with_selected(Some(app.results_selected));
    frame.render_stateful_widget(results, chunks[1], &mut results_state);
    render_scrollbar(frame, chunks[1], app.results.len(), app.results_selected);

    frame.render_widget(status_line(app, theme), chunks[2]);
}

fn draw_episodes(frame: &mut Frame, app: &App, theme: &Theme) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let languages_line = if app.anime_languages.is_empty() {
        "Languages: unknown".to_string()
    } else {
        format!("Languages: {}", app.anime_languages.join(", "))
    };

    let header = Paragraph::new(vec![
        Line::from(Span::styled(
            app.anime_title.clone(),
            Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            format!("{} episodes", app.episodes.len()),
            Style::default().fg(theme.text),
        )),
        Line::from(Span::styled(languages_line, Style::default().fg(theme.text))),
        Line::from(Span::styled(
            app.anime_description.clone(),
            Style::default().fg(theme.text),
        )),
    ])
    .wrap(Wrap { trim: true })
    .block(themed_block(theme, "Detail", false));
    frame.render_widget(header, chunks[0]);

    let items: Vec<ListItem> = app
        .episodes
        .iter()
        .map(|episode| {
            ListItem::new(Span::styled(episode.title.clone(), Style::default().fg(theme.text)))
        })
        .collect();

    let highlight_style = Style::default().fg(theme.selection_fg).bg(theme.selection_bg);
    let episodes = List::new(items)
        .block(themed_block(theme, "Episodes", true))
        .highlight_style(highlight_style);
    let mut episodes_state = ListState::default().with_selected(Some(app.episodes_selected));
    frame.render_stateful_widget(episodes, chunks[1], &mut episodes_state);
    render_scrollbar(frame, chunks[1], app.episodes.len(), app.episodes_selected);

    frame.render_widget(status_line(app, theme), chunks[2]);
}

/// Renders a vertical scrollbar over `area`'s right edge tracking `selected` out of `len`
/// items. A no-op when there's nothing to scroll to.
fn render_scrollbar(frame: &mut Frame, area: ratatui::layout::Rect, len: usize, selected: usize) {
    if len <= 1 {
        return;
    }
    let mut state = ScrollbarState::new(len).position(selected);
    frame.render_stateful_widget(
        Scrollbar::default().orientation(ScrollbarOrientation::VerticalRight),
        area,
        &mut state,
    );
}

fn status_line(app: &App, theme: &Theme) -> Paragraph<'static> {
    let (text, color) = if let Some(error) = &app.error {
        (format!("Error: {error}"), theme.error)
    } else if let Some(status) = &app.status {
        (status.clone(), theme.accent)
    } else if !app.warnings.is_empty() {
        (app.warnings.join("; "), theme.warning)
    } else {
        let hint = match (app.screen, app.focus) {
            (Screen::Search, Focus::Query) => {
                "type to search - enter: search - down/tab: browse results - esc: quit"
            }
            (Screen::Search, Focus::Results) => {
                "up/down or j/k: navigate - enter: select - /: search - q: quit"
            }
            (Screen::Episodes, _) => "up/down or j/k: navigate - enter: play - esc: back - q: quit",
        };
        (hint.to_string(), theme.muted)
    };

    Paragraph::new(text)
        .style(Style::default().fg(color))
        .block(themed_block(theme, "", false))
}
