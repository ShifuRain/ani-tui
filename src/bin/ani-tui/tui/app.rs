use ani_tui::anime_repo::{self, Detail, Episode, GlobalId, SearchResult, WatchLink};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use std::collections::HashSet;

/// Which screen is currently shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Search,
    Episodes,
}

/// Which widget on the [`Screen::Search`] screen currently receives key input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Query,
    Results,
}

/// Result of a background network action, delivered back to the event loop.
pub enum AppEvent {
    SearchResults(Vec<(&'static str, anime_repo::Result<Vec<SearchResult>>)>),
    Detail(anime_repo::Result<(Detail, Vec<Episode>)>),
    /// Carries the episode's id alongside the result so `on_app_event` knows what to mark
    /// watched.
    WatchLinkResolved(GlobalId, anime_repo::Result<WatchLink>),
    /// The mpv process launched for the current episode has exited (however it exited —
    /// finished, was closed by the user, or crashed).
    PlaybackFinished,
}

/// All interactive TUI state, plus "intents" (the `pending_*` fields) that the event loop
/// consumes to actually talk to the network/launch mpv. Keeping those side effects out of
/// `App` itself is what makes [`App::on_key`]/[`App::on_app_event`] pure and unit-testable.
pub struct App {
    pub screen: Screen,
    pub focus: Focus,
    pub query: String,
    pub results: Vec<SearchResult>,
    pub results_selected: usize,
    pub warnings: Vec<String>,
    pub anime_title: String,
    pub anime_description: String,
    pub anime_languages: Vec<String>,
    pub episodes: Vec<Episode>,
    pub episodes_selected: usize,
    /// Watched episode ids ([`GlobalId::as_repr`] strings), hydrated once at startup from
    /// persisted watch history and kept in sync with it via [`Self::pending_watch_history`].
    pub watched: HashSet<String>,
    /// Numeric-entry buffer for jump-to-episode. `None` = normal browsing, `Some(digits)` =
    /// actively typing a target episode number.
    pub jump_input: Option<String>,
    pub status: Option<String>,
    pub error: Option<String>,
    pub loading: bool,
    pub should_quit: bool,
    pub pending_search: Option<String>,
    pub pending_detail: Option<GlobalId>,
    pub pending_watch: Option<GlobalId>,
    pub pending_mpv_link: Option<WatchLink>,
    /// A watched/unwatched change (from an auto-mark on launch, or a manual `x` toggle) that
    /// still needs to be persisted to disk.
    pub pending_watch_history: Option<(GlobalId, bool)>,
}

impl App {
    pub fn new() -> Self {
        Self {
            screen: Screen::Search,
            focus: Focus::Query,
            query: String::new(),
            results: Vec::new(),
            results_selected: 0,
            warnings: Vec::new(),
            anime_title: String::new(),
            anime_description: String::new(),
            anime_languages: Vec::new(),
            episodes: Vec::new(),
            episodes_selected: 0,
            watched: HashSet::new(),
            jump_input: None,
            status: None,
            error: None,
            loading: false,
            should_quit: false,
            pending_search: None,
            pending_detail: None,
            pending_watch: None,
            pending_mpv_link: None,
            pending_watch_history: None,
        }
    }

    /// Handles a single key press.
    pub fn on_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.should_quit = true;
            return;
        }

        match self.screen {
            Screen::Search => self.on_key_search(key),
            Screen::Episodes => self.on_key_episodes(key),
        }
    }

    fn on_key_search(&mut self, key: KeyEvent) {
        match self.focus {
            Focus::Query => match key.code {
                KeyCode::Enter => {
                    let query = self.query.trim();
                    if !query.is_empty() {
                        self.pending_search = Some(query.to_string());
                        self.loading = true;
                        self.error = None;
                    }
                }
                KeyCode::Down | KeyCode::Tab => {
                    if !self.results.is_empty() {
                        self.focus = Focus::Results;
                    }
                }
                KeyCode::Esc => self.should_quit = true,
                KeyCode::Backspace => {
                    self.query.pop();
                }
                KeyCode::Char(c) => self.query.push(c),
                _ => {}
            },
            Focus::Results => match key.code {
                KeyCode::Char('q') => self.should_quit = true,
                KeyCode::Char('/') => self.focus = Focus::Query,
                KeyCode::Up | KeyCode::Char('k') => {
                    self.results_selected = self.results_selected.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if self.results_selected + 1 < self.results.len() {
                        self.results_selected += 1;
                    }
                }
                KeyCode::Enter => {
                    if let Some(result) = self.results.get(self.results_selected) {
                        self.pending_detail = Some(result.id.clone());
                        self.loading = true;
                        self.error = None;
                    }
                }
                _ => {}
            },
        }
    }

    fn on_key_episodes(&mut self, key: KeyEvent) {
        if self.jump_input.is_some() {
            self.on_key_episodes_jump(key);
            return;
        }

        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Esc | KeyCode::Backspace => {
                self.screen = Screen::Search;
                self.status = None;
                self.error = None;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.episodes_selected = self.episodes_selected.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if self.episodes_selected + 1 < self.episodes.len() {
                    self.episodes_selected += 1;
                }
            }
            KeyCode::Enter => {
                if let Some(episode) = self.episodes.get(self.episodes_selected) {
                    self.pending_watch = Some(episode.id.clone());
                    self.status = Some("Resolving watch link...".to_string());
                    self.error = None;
                }
            }
            KeyCode::Char('x') => {
                if let Some(episode) = self.episodes.get(self.episodes_selected) {
                    let repr = episode.id.as_repr();
                    let now_watched = if self.watched.remove(&repr) {
                        false
                    } else {
                        self.watched.insert(repr);
                        true
                    };
                    self.pending_watch_history = Some((episode.id.clone(), now_watched));
                }
            }
            KeyCode::Char('g') => {
                self.jump_input = Some(String::new());
                self.error = None;
            }
            _ => {}
        }
    }

    /// Handles a key press while [`Self::jump_input`] is active (jump-to-episode mode).
    fn on_key_episodes_jump(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => self.jump_input = None,
            KeyCode::Backspace => {
                if let Some(buffer) = &mut self.jump_input {
                    buffer.pop();
                }
            }
            KeyCode::Char(c) if c.is_ascii_digit() => {
                if let Some(buffer) = &mut self.jump_input {
                    buffer.push(c);
                }
            }
            KeyCode::Enter => {
                let buffer = self.jump_input.take().unwrap_or_default();
                match buffer.parse::<u32>() {
                    Ok(target) => match self.episodes.iter().position(|ep| ep.number == target) {
                        Some(index) => {
                            self.episodes_selected = index;
                            self.error = None;
                        }
                        None => self.error = Some(format!("No episode {target}")),
                    },
                    Err(_) => self.error = Some("Enter an episode number".to_string()),
                }
            }
            _ => {}
        }
    }

    /// Handles a background network action completing.
    pub fn on_app_event(&mut self, event: AppEvent) {
        self.loading = false;

        match event {
            AppEvent::SearchResults(per_source) => {
                self.results.clear();
                self.warnings.clear();
                self.results_selected = 0;

                for (prefix, result) in per_source {
                    match result {
                        Ok(results) => self.results.extend(results),
                        Err(_) => self
                            .warnings
                            .push(format!("{prefix} could not be searched")),
                    }
                }

                self.status = if self.results.is_empty() && self.warnings.is_empty() {
                    Some("No results found.".to_string())
                } else {
                    None
                };
            }
            AppEvent::Detail(Ok((detail, episodes))) => {
                self.anime_title = detail.title;
                self.anime_description = detail.description;
                self.anime_languages = detail.languages;
                self.episodes = episodes;
                self.episodes_selected = 0;
                self.screen = Screen::Episodes;
                self.status = None;
            }
            AppEvent::Detail(Err(_)) => {
                self.error = Some("could not fetch anime details".to_string());
            }
            AppEvent::WatchLinkResolved(id, Ok(link)) => {
                self.status = Some("Launching mpv...".to_string());
                self.watched.insert(id.as_repr());
                self.pending_watch_history = Some((id, true));
                self.pending_mpv_link = Some(link);
            }
            AppEvent::WatchLinkResolved(_, Err(_)) => {
                self.error = Some("could not resolve a watch link".to_string());
                self.status = None;
            }
            AppEvent::PlaybackFinished => {
                self.status = None;
            }
        }
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn sample_result(prefix: &str, raw: &str, title: &str) -> SearchResult {
        SearchResult {
            title: title.to_string(),
            id: GlobalId {
                prefix: prefix.to_string(),
                raw: raw.to_string(),
            },
        }
    }

    fn sample_episode(prefix: &str, raw: &str, number: u32) -> Episode {
        Episode {
            title: format!("Episode {number}"),
            number,
            id: GlobalId {
                prefix: prefix.to_string(),
                raw: raw.to_string(),
            },
        }
    }

    #[test]
    fn typing_appends_to_query() {
        let mut app = App::new();
        app.on_key(key(KeyCode::Char('a')));
        app.on_key(key(KeyCode::Char('b')));
        assert_eq!(app.query, "ab");
        app.on_key(key(KeyCode::Backspace));
        assert_eq!(app.query, "a");
    }

    #[test]
    fn enter_on_empty_query_does_not_search() {
        let mut app = App::new();
        app.on_key(key(KeyCode::Enter));
        assert!(app.pending_search.is_none());
        assert!(!app.loading);
    }

    #[test]
    fn enter_on_nonempty_query_sets_pending_search() {
        let mut app = App::new();
        app.query = "bocchi".to_string();
        app.on_key(key(KeyCode::Enter));
        assert_eq!(app.pending_search.as_deref(), Some("bocchi"));
        assert!(app.loading);
    }

    #[test]
    fn down_moves_focus_to_results_only_when_nonempty() {
        let mut app = App::new();
        app.on_key(key(KeyCode::Down));
        assert_eq!(
            app.focus,
            Focus::Query,
            "no results yet, focus shouldn't move"
        );

        app.results.push(sample_result("ADB-1", "x#1", "Title"));
        app.on_key(key(KeyCode::Down));
        assert_eq!(app.focus, Focus::Results);
    }

    #[test]
    fn results_navigation_clamps_at_bounds() {
        let mut app = App::new();
        app.focus = Focus::Results;
        app.results.push(sample_result("ADB-1", "x#1", "A"));
        app.results.push(sample_result("ADB-1", "x#2", "B"));

        app.on_key(key(KeyCode::Up));
        assert_eq!(app.results_selected, 0, "shouldn't go below 0");

        app.on_key(key(KeyCode::Down));
        assert_eq!(app.results_selected, 1);
        app.on_key(key(KeyCode::Down));
        assert_eq!(app.results_selected, 1, "shouldn't go past the last item");
    }

    #[test]
    fn enter_on_empty_results_is_a_noop() {
        let mut app = App::new();
        app.focus = Focus::Results;
        app.on_key(key(KeyCode::Enter));
        assert!(app.pending_detail.is_none());
    }

    #[test]
    fn enter_on_results_sets_pending_detail() {
        let mut app = App::new();
        app.focus = Focus::Results;
        app.results.push(sample_result("ADB-1", "x#1", "A"));
        app.on_key(key(KeyCode::Enter));
        assert_eq!(
            app.pending_detail,
            Some(GlobalId {
                prefix: "ADB-1".to_string(),
                raw: "x#1".to_string(),
            })
        );
    }

    #[test]
    fn search_results_event_populates_results_and_warnings() {
        let mut app = App::new();
        app.loading = true;
        app.on_app_event(AppEvent::SearchResults(vec![
            ("ADB-1", Ok(vec![sample_result("ADB-1", "x#1", "A")])),
            (
                "ADB-1",
                Err(anime_repo::AnimeRepositoryError::DatasourceError),
            ),
        ]));

        assert!(!app.loading);
        assert_eq!(app.results.len(), 1);
        assert_eq!(
            app.warnings,
            vec!["ADB-1 could not be searched".to_string()]
        );
    }

    #[test]
    fn detail_event_switches_to_episodes_screen() {
        let mut app = App::new();
        app.on_app_event(AppEvent::Detail(Ok((
            Detail {
                title: "Bocchi the Rock!".to_string(),
                description: "...".to_string(),
                episode_count: 12,
                languages: vec!["jpn".to_string()],
            },
            vec![sample_episode("ADB-1", "x#1", 1)],
        ))));

        assert_eq!(app.screen, Screen::Episodes);
        assert_eq!(app.anime_title, "Bocchi the Rock!");
        assert_eq!(app.anime_languages, vec!["jpn".to_string()]);
        assert_eq!(app.episodes.len(), 1);
    }

    #[test]
    fn esc_from_episodes_returns_to_search_preserving_query() {
        let mut app = App::new();
        app.query = "bocchi".to_string();
        app.screen = Screen::Episodes;
        app.episodes.push(sample_episode("ADB-1", "x#1", 1));

        app.on_key(key(KeyCode::Esc));

        assert_eq!(app.screen, Screen::Search);
        assert_eq!(app.query, "bocchi");
        assert_eq!(
            app.episodes.len(),
            1,
            "going back shouldn't discard fetched data"
        );
    }

    #[test]
    fn ctrl_c_quits_from_any_screen() {
        let mut app = App::new();
        app.on_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(app.should_quit);
    }

    #[test]
    fn watch_link_resolved_sets_pending_mpv_link() {
        let mut app = App::new();
        let id = GlobalId { prefix: "ADB-1".to_string(), raw: "x#1".to_string() };
        app.on_app_event(AppEvent::WatchLinkResolved(
            id.clone(),
            Ok(WatchLink {
                url: "https://example.com/x.m3u8".to_string(),
                headers: vec![("Referer".to_string(), "https://example.com/".to_string())],
            }),
        ));
        let link = app.pending_mpv_link.expect("watch link should be pending");
        assert_eq!(link.url, "https://example.com/x.m3u8");
        assert_eq!(
            link.mpv_header_fields().as_deref(),
            Some("Referer: https://example.com/")
        );
    }

    #[test]
    fn watch_link_resolved_marks_episode_watched() {
        let mut app = App::new();
        let id = GlobalId { prefix: "ADB-1".to_string(), raw: "x#1".to_string() };
        app.on_app_event(AppEvent::WatchLinkResolved(
            id.clone(),
            Ok(WatchLink { url: "https://example.com".to_string(), headers: vec![] }),
        ));

        assert!(app.watched.contains(&id.as_repr()));
        assert_eq!(app.pending_watch_history, Some((id, true)));
    }

    #[test]
    fn playback_finished_clears_the_launching_status() {
        let mut app = App::new();
        app.status = Some("Launching mpv...".to_string());

        app.on_app_event(AppEvent::PlaybackFinished);

        assert_eq!(app.status, None);
    }

    #[test]
    fn x_toggles_watched_status_on_the_selected_episode() {
        let mut app = App::new();
        app.screen = Screen::Episodes;
        app.episodes.push(sample_episode("ADB-1", "x#1", 1));
        let repr = app.episodes[0].id.as_repr();

        app.on_key(key(KeyCode::Char('x')));
        assert!(app.watched.contains(&repr));
        assert_eq!(
            app.pending_watch_history,
            Some((app.episodes[0].id.clone(), true))
        );

        app.on_key(key(KeyCode::Char('x')));
        assert!(!app.watched.contains(&repr));
        assert_eq!(
            app.pending_watch_history,
            Some((app.episodes[0].id.clone(), false))
        );
    }

    #[test]
    fn g_then_digits_then_enter_jumps_to_the_matching_episode_number() {
        let mut app = App::new();
        app.screen = Screen::Episodes;
        app.episodes.push(sample_episode("ADB-1", "x#1", 1));
        app.episodes.push(sample_episode("ADB-1", "x#500", 500));
        app.episodes.push(sample_episode("ADB-1", "x#2", 2));

        app.on_key(key(KeyCode::Char('g')));
        assert_eq!(app.jump_input.as_deref(), Some(""));

        app.on_key(key(KeyCode::Char('5')));
        app.on_key(key(KeyCode::Char('0')));
        app.on_key(key(KeyCode::Char('0')));
        assert_eq!(app.jump_input.as_deref(), Some("500"));

        app.on_key(key(KeyCode::Enter));
        assert_eq!(app.jump_input, None);
        assert_eq!(app.episodes_selected, 1);
        assert_eq!(app.error, None);
    }

    #[test]
    fn jump_to_a_missing_episode_number_sets_an_error() {
        let mut app = App::new();
        app.screen = Screen::Episodes;
        app.episodes.push(sample_episode("ADB-1", "x#1", 1));

        app.on_key(key(KeyCode::Char('g')));
        app.on_key(key(KeyCode::Char('9')));
        app.on_key(key(KeyCode::Enter));

        assert_eq!(app.episodes_selected, 0, "selection shouldn't move on a miss");
        assert_eq!(app.error, Some("No episode 9".to_string()));
    }

    #[test]
    fn esc_cancels_jump_mode_without_moving_selection() {
        let mut app = App::new();
        app.screen = Screen::Episodes;
        app.episodes.push(sample_episode("ADB-1", "x#1", 1));
        app.episodes.push(sample_episode("ADB-1", "x#2", 2));

        app.on_key(key(KeyCode::Char('g')));
        app.on_key(key(KeyCode::Char('2')));
        app.on_key(key(KeyCode::Esc));

        assert_eq!(app.jump_input, None);
        assert_eq!(app.episodes_selected, 0);
    }
}
