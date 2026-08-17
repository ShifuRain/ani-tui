mod tui;

use ani_tui::{anime_repo::GlobalId, cli_args::*, registry::Registry};

use clap::Parser;
use std::process::{ExitCode, Stdio};
use std::sync::Arc;
use tokio::process::Command;

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    let registry = Registry::new();

    let result = match args.command {
        Some(command) => run(command, &registry).await,
        None => tui::run(Arc::new(registry))
            .await
            .map_err(|e| e.to_string()),
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

            println!(
                r#"{title}
{eps} episodes, {ident}

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

            println!("Launching MPV");
            let mut mpv = Command::new("mpv")
                .arg(link)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
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
