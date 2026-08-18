mod config;
mod series_cache;
mod tui;
mod update_check;
mod watch_history;

use ani_tui::{anime_repo::GlobalId, cli_args::*, registry::Registry};
use config::Config;
use watch_history::WatchHistory;

use clap::Parser;
use std::process::ExitCode;
use std::sync::Arc;
use tokio::process::Command;

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    let registry = Registry::new();

    let result = match args.command {
        Some(command) => run(command, &registry).await,
        None => {
            let theme = Config::load().theme;
            let watch_history = WatchHistory::load();
            tui::run(Arc::new(registry), theme, watch_history)
                .await
                .map_err(|e| e.to_string())
        }
    };

    if let Err(message) = result {
        eprintln!("Error: {message}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// Runs the requested subcommand, returning a user-facing error message on failure.
async fn run(command: Commands, registry: &Registry) -> Result<(), String> {
    match command {
        Commands::Search { title } => {
            let mut any_results = false;

            for (prefix, results) in registry.search(&title).await {
                match results {
                    Ok(results) => {
                        for result in results {
                            any_results = true;
                            println!(
                                r#"
 • {title} [{prefix}]
   {ident}"#,
                                ident = result.id.as_repr(),
                                title = result.title,
                            );
                        }
                    }
                    Err(_) => eprintln!("Warning: source {prefix} could not be searched"),
                }
            }

            if !any_results {
                println!("No results found.");
            }
        }
        Commands::EpCount { ident } => {
            let id = GlobalId::from_repr(&ident).ok_or("invalid identifier")?;
            let detail = registry
                .detail(&id)
                .await
                .map_err(|_| "could not fetch anime details")?;

            println!(
                r#""{title}" has {ep_count} episodes."#,
                title = detail.title,
                ep_count = detail.episode_count
            );
        }
        Commands::Detail { ident } => {
            let id = GlobalId::from_repr(&ident).ok_or("invalid identifier")?;
            let detail = registry
                .detail(&id)
                .await
                .map_err(|_| "could not fetch anime details")?;

            let languages = if detail.languages.is_empty() {
                "unknown".to_string()
            } else {
                detail.languages.join(", ")
            };
            println!(
                r#"{title}
{eps} episodes, {ident}
Languages: {languages}

{description}"#,
                title = detail.title,
                eps = detail.episode_count,
                description = detail.description
            );
        }
        Commands::Watch { ident, ep } => {
            let id = GlobalId::from_repr(&ident).ok_or("invalid identifier")?;
            let ep_index = ep
                .checked_sub(1)
                .ok_or("episode number must be 1 or greater")?;
            let episodes = registry
                .list_eps(&id)
                .await
                .map_err(|_| "could not fetch episode list")?;
            let episode = episodes
                .get(ep_index)
                .ok_or("episode number out of range")?;
            let link = registry
                .watch_link(&episode.id)
                .await
                .map_err(|_| "could not resolve a watch link")?;
            let mut watch_history = WatchHistory::load();
            watch_history.set_watched(&episode.id, true);

            println!("Launching MPV");
            let mut mpv_cmd = Command::new("mpv");
            if let Some(header_fields) = link.mpv_header_fields() {
                mpv_cmd.arg(format!("--http-header-fields={header_fields}"));
            }
            // Inherited, not piped: this is a foreground, blocking command with no TUI of our
            // own to protect, so let mpv use the real terminal directly — the user sees its
            // actual output, and there's no pipe buffer to fill up unread while we wait.
            let mut mpv = mpv_cmd
                .arg(&link.url)
                .spawn()
                .map_err(|_| "could not launch mpv: is it installed and on PATH?")?;
            mpv.wait().await.map_err(|_| "mpv exited unexpectedly")?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// These all fail validation before `run` ever touches the registry, so they're fast and
    /// deterministic even without network access.
    #[tokio::test]
    async fn rejects_malformed_identifier() {
        let registry = Registry::new();
        let command = Commands::Detail {
            ident: "not-a-valid-id".to_string(),
        };

        assert_eq!(
            run(command, &registry).await,
            Err("invalid identifier".to_string())
        );
    }

    #[tokio::test]
    async fn rejects_episode_number_zero() {
        let registry = Registry::new();
        let command = Commands::Watch {
            ident: "<ADB-1:some-anime#1>".to_string(),
            ep: 0,
        };

        assert_eq!(
            run(command, &registry).await,
            Err("episode number must be 1 or greater".to_string())
        );
    }
}
