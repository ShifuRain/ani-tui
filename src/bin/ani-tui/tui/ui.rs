use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Wrap},
    Frame,
};

use super::app::{App, Focus, Screen};

pub fn draw(frame: &mut Frame, app: &App) {
    match app.screen {
        Screen::Search => draw_search(frame, app),
        Screen::Episodes => draw_episodes(frame, app),
    }
}

fn draw_search(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let query_style = if app.focus == Focus::Query {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default()
    };
    let query = Paragraph::new(format!("{}_", app.query))
        .style(query_style)
        .block(Block::default().borders(Borders::ALL).title("Search"));
    frame.render_widget(query, chunks[0]);

    let results_title = if app.loading {
        "Results (loading...)"
    } else {
        "Results"
    };
    let items: Vec<ListItem> = app
        .results
        .iter()
        .enumerate()
        .map(|(i, result)| {
            let line = format!("{} [{}]", result.title, result.id.prefix);
            if i == app.results_selected && app.focus == Focus::Results {
                ListItem::new(line).style(Style::default().add_modifier(Modifier::REVERSED))
            } else {
                ListItem::new(line)
            }
        })
        .collect();
    let results =
        List::new(items).block(Block::default().borders(Borders::ALL).title(results_title));
    frame.render_widget(results, chunks[1]);

    frame.render_widget(status_line(app), chunks[2]);
}

fn draw_episodes(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Min(3),
            Constraint::Length(3),
        ])
        .split(frame.area());

    let header = Paragraph::new(vec![
        Line::from(Span::styled(
            app.anime_title.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(format!("{} episodes", app.episodes.len())),
        Line::from(app.anime_description.clone()),
    ])
    .wrap(Wrap { trim: true })
    .block(Block::default().borders(Borders::ALL).title("Detail"));
    frame.render_widget(header, chunks[0]);

    let items: Vec<ListItem> = app
        .episodes
        .iter()
        .enumerate()
        .map(|(i, episode)| {
            if i == app.episodes_selected {
                ListItem::new(episode.title.clone())
                    .style(Style::default().add_modifier(Modifier::REVERSED))
            } else {
                ListItem::new(episode.title.clone())
            }
        })
        .collect();
    let episodes = List::new(items).block(Block::default().borders(Borders::ALL).title("Episodes"));
    frame.render_widget(episodes, chunks[1]);

    frame.render_widget(status_line(app), chunks[2]);
}

fn status_line(app: &App) -> Paragraph<'static> {
    let text = if let Some(error) = &app.error {
        format!("Error: {error}")
    } else if let Some(status) = &app.status {
        status.clone()
    } else if !app.warnings.is_empty() {
        app.warnings.join("; ")
    } else {
        match (app.screen, app.focus) {
            (Screen::Search, Focus::Query) => {
                "type to search - enter: search - down/tab: browse results - esc: quit".to_string()
            }
            (Screen::Search, Focus::Results) => {
                "up/down or j/k: navigate - enter: select - /: search - q: quit".to_string()
            }
            (Screen::Episodes, _) => {
                "up/down or j/k: navigate - enter: play - esc: back - q: quit".to_string()
            }
        }
    };

    Paragraph::new(text).block(Block::default().borders(Borders::ALL))
}
