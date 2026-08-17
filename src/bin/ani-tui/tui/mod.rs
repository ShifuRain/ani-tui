mod app;
mod ui;

use std::io;
use std::sync::Arc;

use ani_tui::registry::Registry;
use crossterm::{
    event::{Event, EventStream},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use futures::StreamExt;
use ratatui::{backend::CrosstermBackend, Terminal};
use tokio::sync::mpsc;

use app::{App, AppEvent};

/// Runs the interactive TUI until the user quits, then restores the terminal.
pub async fn run(registry: Arc<Registry>) -> io::Result<()> {
    install_panic_hook();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal, registry).await;

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

/// Restores the terminal before the default panic handler prints, so a panic while in raw
/// mode/the alternate screen doesn't leave the user's terminal broken.
fn install_panic_hook() {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(panic_info);
    }));
}

async fn run_app(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    registry: Arc<Registry>,
) -> io::Result<()> {
    let mut app = App::new();
    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
    let mut events = EventStream::new();

    terminal.draw(|frame| ui::draw(frame, &app))?;

    loop {
        tokio::select! {
            maybe_event = events.next() => {
                match maybe_event {
                    Some(Ok(Event::Key(key))) => app.on_key(key),
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

        dispatch_pending(&mut app, &registry, &tx);

        if app.should_quit {
            break;
        }

        terminal.draw(|frame| ui::draw(frame, &app))?;
    }

    Ok(())
}

/// Consumes any `pending_*` intents set by the last state update, spawning the corresponding
/// network request (or launching mpv) in the background.
fn dispatch_pending(app: &mut App, registry: &Arc<Registry>, tx: &mpsc::UnboundedSender<AppEvent>) {
    if let Some(query) = app.pending_search.take() {
        let registry = Arc::clone(registry);
        let tx = tx.clone();
        tokio::spawn(async move {
            let results = registry.search(&query).await;
            let _ = tx.send(AppEvent::SearchResults(results));
        });
    }

    if let Some(id) = app.pending_detail.take() {
        let registry = Arc::clone(registry);
        let tx = tx.clone();
        tokio::spawn(async move {
            let (detail, episodes) = tokio::join!(registry.detail(&id), registry.list_eps(&id));
            let combined = match (detail, episodes) {
                (Ok(detail), Ok(episodes)) => Ok((detail, episodes)),
                (Err(err), _) | (_, Err(err)) => Err(err),
            };
            let _ = tx.send(AppEvent::Detail(combined));
        });
    }

    if let Some(id) = app.pending_watch.take() {
        let registry = Arc::clone(registry);
        let tx = tx.clone();
        tokio::spawn(async move {
            let link = registry.watch_link(&id).await;
            let _ = tx.send(AppEvent::WatchLinkResolved(link));
        });
    }

    if let Some(link) = app.pending_mpv_link.take() {
        let spawned = tokio::process::Command::new("mpv")
            .arg(&link)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn();
        if spawned.is_err() {
            app.error = Some("could not launch mpv: is it installed and on PATH?".to_string());
        }
    }
}
