use ani_tui::anime_repo::{self, Detail, Episode, GlobalId, SearchResult, WatchLink};
use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use std::collections::HashSet;

/// Rendered position of the currently visible scrollable list, used for mouse hit-testing.
/// Deliberately a plain rectangle rather than `ratatui::layout::Rect` — `App` stays free of
/// rendering-crate types so [`App::on_key`]/[`App::on_mouse`] stay pure and unit-testable, the
/// same reasoning already applied to keeping side effects out of `App` entirely.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListArea {
    pub x: u16,
    pub y: u16,
    pub width: u16,
    pub height: u16,
}

impl ListArea {
    fn contains_row(self, row: u16) -> bool {
        row >= self.y && row < self.y + self.height
    }

    /// The scrollbar's column, matching `ScrollbarOrientation::VerticalRight` in `ui.rs`: the
    /// rightmost column of the same area the list itself is rendered into.
    fn scrollbar_column(self) -> u16 {
        self.x + self.width.saturating_sub(1)
    }

    /// Maps a mouse row within this area to a proportional index into a list of `len` items.
    fn index_for_row(self, row: u16, len: usize) -> usize {
        if len == 0 {
            return 0;
        }
        let relative = row.saturating_sub(self.y) as f64;
        let height = self.height.max(1) as f64;
        let index = (relative / height * len as f64) as usize;
        index.min(len - 1)
    }
}

/// Clamps `current + delta` into `0..len`, saturating at either end. `len == 0` always yields
/// `0`.
fn clamp_index(current: usize, delta: i32, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    (current as i32 + delta).clamp(0, len as i32 - 1) as usize
}

/// A parsed jump-to-episode target: either a plain episode number, or a season+episode pair
/// for sources whose numbering restarts every season (see [`parse_jump_target`]).
enum JumpTarget {
    Number(u32),
    SeasonEpisode(u32, u32),
}

/// Parses a jump-to-episode input buffer. Accepts a plain episode number (`"55"`), matched
/// against [`Episode::number`] regardless of season — correct for sources with no season
/// concept (anidb.app) or single-season shows. Also accepts `"S<season>E<number>"`
/// (case-insensitive, e.g. `"s2e55"`), needed to disambiguate shows like Detective Conan on
/// aniworld.to where every season's episodes restart at 1, so a plain number would match
/// whichever season happens to come first in the list rather than the one the user meant.
fn parse_jump_target(buffer: &str) -> Option<JumpTarget> {
    let lower = buffer.to_ascii_lowercase();
    match lower.strip_prefix('s') {
        Some(rest) => {
            let (season, number) = rest.split_once('e')?;
            Some(JumpTarget::SeasonEpisode(season.parse().ok()?, number.parse().ok()?))
        }
        None => lower.parse().ok().map(JumpTarget::Number),
    }
}

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
    /// Carries the series' id alongside the result so `on_app_event` can remember which series
    /// is on screen, for manual refresh.
    Detail(GlobalId, anime_repo::Result<(Detail, Vec<Episode>)>),
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
    /// Id of the series currently shown on [`Screen::Episodes`], if any — set from a successful
    /// [`AppEvent::Detail`], used by the `r` (refresh) key to know what to re-fetch.
    pub current_anime_id: Option<GlobalId>,
    /// Screen position of the currently visible list (results or episodes, whichever the
    /// active screen shows), refreshed on every frame draw. Used to hit-test mouse clicks/drags
    /// against the scrollbar. `None` before the first frame is drawn.
    pub list_area: Option<ListArea>,
    /// Whether the user is currently dragging the scrollbar (mouse button held after a
    /// [`MouseEventKind::Down`] that landed on it), so subsequent `Drag` events keep scrolling
    /// even if the cursor strays off the scrollbar's exact column.
    pub dragging_scrollbar: bool,
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
    /// A manual refresh request (the `r` key): the event loop should bypass/invalidate the
    /// series cache for this id and re-fetch, rather than serve a cached result.
    pub pending_refresh: Option<GlobalId>,
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
            current_anime_id: None,
            list_area: None,
            dragging_scrollbar: false,
            watched: HashSet::new(),
            jump_input: None,
            status: None,
            error: None,
            loading: false,
            should_quit: false,
            pending_search: None,
            pending_detail: None,
            pending_refresh: None,
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

    /// Handles a mouse event: click-to-jump and drag-to-scroll on the currently visible list's
    /// scrollbar. Ignored while [`Self::jump_input`] is active, same as other non-jump keys.
    pub fn on_mouse(&mut self, event: MouseEvent) {
        if self.jump_input.is_some() {
            return;
        }

        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some(area) = self.list_area {
                    if event.column == area.scrollbar_column() && area.contains_row(event.row) {
                        self.dragging_scrollbar = true;
                        self.set_selection_from_row(area, event.row);
                    }
                }
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                if self.dragging_scrollbar {
                    if let Some(area) = self.list_area {
                        self.set_selection_from_row(area, event.row);
                    }
                }
            }
            MouseEventKind::Up(MouseButton::Left) => self.dragging_scrollbar = false,
            MouseEventKind::ScrollUp => self.nudge_current_selection(-3),
            MouseEventKind::ScrollDown => self.nudge_current_selection(3),
            _ => {}
        }
    }

    /// Sets the current screen's selected index from a mouse row, proportional to `area`.
    /// Clicking the results list's scrollbar also moves focus there, matching what `Down`/`Tab`
    /// already do when the results list has entries.
    fn set_selection_from_row(&mut self, area: ListArea, row: u16) {
        match self.screen {
            Screen::Search => {
                if !self.results.is_empty() {
                    self.focus = Focus::Results;
                    self.results_selected = area.index_for_row(row, self.results.len());
                }
            }
            Screen::Episodes => {
                if !self.episodes.is_empty() {
                    self.episodes_selected = area.index_for_row(row, self.episodes.len());
                }
            }
        }
    }

    /// Moves the current screen's selection by `delta` (negative = up), clamped to bounds. Used
    /// for mouse wheel scrolling.
    fn nudge_current_selection(&mut self, delta: i32) {
        match self.screen {
            Screen::Search if self.focus == Focus::Results => {
                self.results_selected = clamp_index(self.results_selected, delta, self.results.len());
            }
            Screen::Episodes => {
                self.episodes_selected = clamp_index(self.episodes_selected, delta, self.episodes.len());
            }
            _ => {}
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
            KeyCode::Char('r') => {
                if let Some(id) = self.current_anime_id.clone() {
                    self.pending_refresh = Some(id);
                    self.status = Some("Refreshing...".to_string());
                    self.error = None;
                }
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
            KeyCode::Char(c) if c.is_ascii_digit() || matches!(c, 's' | 'S' | 'e' | 'E') => {
                if let Some(buffer) = &mut self.jump_input {
                    buffer.push(c);
                }
            }
            KeyCode::Enter => {
                let buffer = self.jump_input.take().unwrap_or_default();
                match parse_jump_target(&buffer) {
                    Some(JumpTarget::Number(target)) => {
                        match self.episodes.iter().position(|ep| ep.number == target) {
                            Some(index) => {
                                self.episodes_selected = index;
                                self.error = None;
                            }
                            None => self.error = Some(format!("No episode {target}")),
                        }
                    }
                    Some(JumpTarget::SeasonEpisode(season, number)) => {
                        match self
                            .episodes
                            .iter()
                            .position(|ep| ep.season == Some(season) && ep.number == number)
                        {
                            Some(index) => {
                                self.episodes_selected = index;
                                self.error = None;
                            }
                            None => {
                                self.error = Some(format!("No episode S{season:02}E{number:02}"))
                            }
                        }
                    }
                    None => self.error = Some("Enter an episode number or SxxExx".to_string()),
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
            AppEvent::Detail(id, Ok((detail, episodes))) => {
                self.anime_title = detail.title;
                self.anime_description = detail.description;
                self.anime_languages = detail.languages;
                self.episodes = episodes;
                self.episodes_selected = 0;
                self.current_anime_id = Some(id);
                self.screen = Screen::Episodes;
                self.status = None;
            }
            AppEvent::Detail(_, Err(_)) => {
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

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent { kind, column, row, modifiers: KeyModifiers::NONE }
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
            season: None,
            number,
            id: GlobalId {
                prefix: prefix.to_string(),
                raw: raw.to_string(),
            },
        }
    }

    fn sample_episode_in_season(prefix: &str, raw: &str, season: u32, number: u32) -> Episode {
        Episode {
            title: format!("S{season:02}E{number:02}"),
            season: Some(season),
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
        let id = GlobalId { prefix: "ADB-1".to_string(), raw: "x".to_string() };
        app.on_app_event(AppEvent::Detail(
            id.clone(),
            Ok((
                Detail {
                    title: "Bocchi the Rock!".to_string(),
                    description: "...".to_string(),
                    episode_count: 12,
                    languages: vec!["jpn".to_string()],
                },
                vec![sample_episode("ADB-1", "x#1", 1)],
            )),
        ));

        assert_eq!(app.screen, Screen::Episodes);
        assert_eq!(app.anime_title, "Bocchi the Rock!");
        assert_eq!(app.anime_languages, vec!["jpn".to_string()]);
        assert_eq!(app.episodes.len(), 1);
        assert_eq!(app.current_anime_id, Some(id));
    }

    #[test]
    fn r_queues_a_refresh_for_the_current_series() {
        let mut app = App::new();
        app.screen = Screen::Episodes;
        let id = GlobalId { prefix: "ADB-1".to_string(), raw: "x".to_string() };
        app.current_anime_id = Some(id.clone());

        app.on_key(key(KeyCode::Char('r')));

        assert_eq!(app.pending_refresh, Some(id));
    }

    #[test]
    fn r_without_a_current_series_does_nothing() {
        let mut app = App::new();
        app.screen = Screen::Episodes;

        app.on_key(key(KeyCode::Char('r')));

        assert_eq!(app.pending_refresh, None);
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

    fn episode_area() -> ListArea {
        ListArea { x: 0, y: 0, width: 40, height: 10 }
    }

    #[test]
    fn clicking_the_scrollbar_column_jumps_to_the_proportional_episode() {
        let mut app = App::new();
        app.screen = Screen::Episodes;
        app.list_area = Some(episode_area());
        for n in 1..=10 {
            app.episodes.push(sample_episode("AWT-1", &format!("e{n}"), n));
        }

        // Row 5 of a 10-row area over 10 episodes -> index 5.
        app.on_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 39, 5));

        assert!(app.dragging_scrollbar);
        assert_eq!(app.episodes_selected, 5);
    }

    #[test]
    fn clicking_off_the_scrollbar_column_does_nothing() {
        let mut app = App::new();
        app.screen = Screen::Episodes;
        app.list_area = Some(episode_area());
        app.episodes.push(sample_episode("AWT-1", "e1", 1));
        app.episodes.push(sample_episode("AWT-1", "e2", 2));

        app.on_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 10, 5));

        assert!(!app.dragging_scrollbar);
        assert_eq!(app.episodes_selected, 0);
    }

    #[test]
    fn dragging_after_a_scrollbar_click_keeps_scrolling_even_off_column() {
        let mut app = App::new();
        app.screen = Screen::Episodes;
        app.list_area = Some(episode_area());
        for n in 1..=10 {
            app.episodes.push(sample_episode("AWT-1", &format!("e{n}"), n));
        }

        app.on_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 39, 0));
        assert_eq!(app.episodes_selected, 0);

        // Cursor drifts off the scrollbar's exact column mid-drag; should still track.
        app.on_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 20, 9));
        assert_eq!(app.episodes_selected, 9);

        app.on_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 20, 9));
        assert!(!app.dragging_scrollbar);

        // No longer dragging, so further moves at the same position don't do anything.
        app.on_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 20, 0));
        assert_eq!(app.episodes_selected, 9);
    }

    #[test]
    fn mouse_wheel_nudges_the_selection() {
        let mut app = App::new();
        app.screen = Screen::Episodes;
        for n in 1..=10 {
            app.episodes.push(sample_episode("AWT-1", &format!("e{n}"), n));
        }

        app.on_mouse(mouse(MouseEventKind::ScrollDown, 0, 0));
        assert_eq!(app.episodes_selected, 3);

        app.on_mouse(mouse(MouseEventKind::ScrollUp, 0, 0));
        assert_eq!(app.episodes_selected, 0);
    }

    #[test]
    fn mouse_is_ignored_while_jump_input_is_active() {
        let mut app = App::new();
        app.screen = Screen::Episodes;
        app.list_area = Some(episode_area());
        app.episodes.push(sample_episode("AWT-1", "e1", 1));
        app.episodes.push(sample_episode("AWT-1", "e2", 2));
        app.jump_input = Some(String::new());

        app.on_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 39, 9));

        assert_eq!(app.episodes_selected, 0);
        assert_eq!(app.jump_input.as_deref(), Some(""));
    }

    #[test]
    fn clicking_the_results_scrollbar_moves_focus_there() {
        let mut app = App::new();
        app.list_area = Some(episode_area());
        app.results.push(sample_result("ADB-1", "x#1", "A"));
        app.results.push(sample_result("ADB-1", "x#2", "B"));

        app.on_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 39, 9));

        assert_eq!(app.focus, Focus::Results);
        assert_eq!(app.results_selected, 1);
    }

    #[test]
    fn jump_with_a_season_prefix_disambiguates_episodes_sharing_a_number() {
        let mut app = App::new();
        app.screen = Screen::Episodes;
        app.episodes.push(sample_episode_in_season("AWT-1", "s1e5", 1, 5));
        app.episodes.push(sample_episode_in_season("AWT-1", "s2e5", 2, 5));

        app.on_key(key(KeyCode::Char('g')));
        for c in "s2e5".chars() {
            app.on_key(key(KeyCode::Char(c)));
        }
        app.on_key(key(KeyCode::Enter));

        assert_eq!(app.jump_input, None);
        assert_eq!(app.episodes_selected, 1, "should land on season 2's episode 5, not season 1's");
        assert_eq!(app.error, None);
    }

    #[test]
    fn jump_with_only_a_number_matches_the_first_episode_with_that_number() {
        let mut app = App::new();
        app.screen = Screen::Episodes;
        app.episodes.push(sample_episode_in_season("AWT-1", "s1e5", 1, 5));
        app.episodes.push(sample_episode_in_season("AWT-1", "s2e5", 2, 5));

        app.on_key(key(KeyCode::Char('g')));
        app.on_key(key(KeyCode::Char('5')));
        app.on_key(key(KeyCode::Enter));

        assert_eq!(app.episodes_selected, 0, "plain number matches by position in the list");
    }

    #[test]
    fn jump_with_a_season_prefix_to_a_missing_episode_sets_an_error() {
        let mut app = App::new();
        app.screen = Screen::Episodes;
        app.episodes.push(sample_episode_in_season("AWT-1", "s1e5", 1, 5));

        app.on_key(key(KeyCode::Char('g')));
        for c in "s2e5".chars() {
            app.on_key(key(KeyCode::Char(c)));
        }
        app.on_key(key(KeyCode::Enter));

        assert_eq!(app.error, Some("No episode S02E05".to_string()));
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
