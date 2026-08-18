mod app;
mod ui;

use std::io;
use std::sync::Arc;

use ani_tui::registry::Registry;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture, Event, EventStream},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::sync::mpsc;

use crate::config::Theme;
use crate::series_cache::SeriesCache;
use crate::watch_history::WatchHistory;
use app::{App, AppEvent};

/// Runs the interactive TUI until the user quits, then restores the terminal.
pub async fn run(
    registry: Arc<Registry>,
    theme: Theme,
    mut watch_history: WatchHistory,
) -> io::Result<()> {
    install_panic_hook();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, registry, &theme, &mut watch_history).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;

    result
}

/// Restores the terminal before the default panic handler prints, so a panic while in raw
/// mode/the alternate screen doesn't leave the user's terminal broken.
fn install_panic_hook() {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture);
        original_hook(panic_info);
    }));
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    registry: Arc<Registry>,
    theme: &Theme,
    watch_history: &mut WatchHistory,
) -> io::Result<()> {
    let mut app = App::new();
    app.watched = watch_history.watched_ids();
    let series_cache = Arc::new(SeriesCache::new());
    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
    let mut events = EventStream::new();

    let mut list_area = None;
    terminal.draw(|frame| list_area = Some(ui::draw(frame, &app, theme)))?;
    app.list_area = list_area;

    loop {
        tokio::select! {
            maybe_event = events.next() => {
                match maybe_event {
                    Some(Ok(Event::Key(key))) => app.on_key(key),
                    Some(Ok(Event::Mouse(mouse))) => app.on_mouse(mouse),
                    Some(Ok(_)) => {}
                    Some(Err(_)) => {}
                    None => app.should_quit = true,
                }
            }
            maybe_app_event = rx.recv() => {
                if let Some(event) = maybe_app_event {
                    app.on_app_event(event);
                }
            }
        }

        dispatch_pending(&mut app, &registry, &series_cache, &tx, watch_history);

        if app.should_quit {
            break;
        }

        let mut list_area = None;
        terminal.draw(|frame| list_area = Some(ui::draw(frame, &app, theme)))?;
        app.list_area = list_area;
    }

    Ok(())
}

/// Consumes any `pending_*` intents set by the last state update, spawning the corresponding
/// network request (or launching mpv) in the background.
fn dispatch_pending(
    app: &mut App,
    registry: &Arc<Registry>,
    series_cache: &Arc<SeriesCache>,
    tx: &mpsc::UnboundedSender<AppEvent>,
    watch_history: &mut WatchHistory,
) {
    if let Some(query) = app.pending_search.take() {
        let registry = Arc::clone(registry);
        let tx = tx.clone();
        tokio::spawn(async move {
            let results = registry.search(&query).await;
            let _ = tx.send(AppEvent::SearchResults(results));
        });
    }

    if let Some(id) = app.pending_refresh.take() {
        series_cache.invalidate(&id);
        app.pending_detail = Some(id);
        app.loading = true;
    }

    if let Some(id) = app.pending_detail.take() {
        if let Some((detail, episodes)) = series_cache.get(&id) {
            let _ = tx.send(AppEvent::Detail(id, Ok((detail, episodes))));
        } else {
            let registry = Arc::clone(registry);
            let series_cache = Arc::clone(series_cache);
            let tx = tx.clone();
            tokio::spawn(async move {
                let (detail, episodes) =
                    tokio::join!(registry.detail(&id), registry.list_eps(&id));
                let combined = match (detail, episodes) {
                    (Ok(detail), Ok(episodes)) => {
                        series_cache.insert(&id, detail.clone(), episodes.clone());
                        Ok((detail, episodes))
                    }
                    (Err(err), _) | (_, Err(err)) => Err(err),
                };
                let _ = tx.send(AppEvent::Detail(id, combined));
            });
        }
    }

    if let Some(id) = app.pending_watch.take() {
        let registry = Arc::clone(registry);
        let tx = tx.clone();
        tokio::spawn(async move {
            let link = registry.watch_link(&id).await;
            let _ = tx.send(AppEvent::WatchLinkResolved(id, link));
        });
    }

    if let Some((id, watched)) = app.pending_watch_history.take() {
        watch_history.set_watched(&id, watched);
    }

    if let Some(link) = app.pending_mpv_link.take() {
        let mut mpv = tokio::process::Command::new("mpv");
        if let Some(header_fields) = link.mpv_header_fields() {
            mpv.arg(format!("--http-header-fields={header_fields}"));
        }
        // `null`, not `piped`: nothing ever reads these, and holding a `Child` open across a
        // whole video's playback while its stdout/stderr pipe buffer fills up unread would
        // block mpv the moment that buffer's full. `null` just discards the output instead —
        // still keeps it off the TUI's own alternate-screen terminal.
        let spawned = mpv
            .arg(&link.url)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        match spawned {
            Ok(mut child) => {
                let tx = tx.clone();
                tokio::spawn(async move {
                    let _ = child.wait().await;
                    let _ = tx.send(AppEvent::PlaybackFinished);
                });
            }
            Err(_) => {
                app.error = Some("could not launch mpv: is it installed and on PATH?".to_string());
            }
        }
    }
}
