use ani_tui::{anime_repo::AnimeRepository, cli_args::*, websites::gogoplay::*};

use clap::Parser;
use std::process::{Command, ExitCode, Stdio};

#[tokio::main]
async fn main() -> ExitCode {
    let args = Args::parse();
    let repo = Gogoplay::new();

    if let Err(message) = run(args, &repo).await {
        eprintln!("Error: {message}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

/// Runs the requested subcommand, returning a user-facing error message on failure.
async fn run(args: Args, repo: &Gogoplay) -> Result<(), String> {
    match args.command {
        Commands::Search { title } => {
            let results = repo
                .search(&title)
                .await
                .ok_or("could not search: the site may be down or unreachable")?;

            for result in results {
                println!(
                    r#"
 • {title}
   {ident}"#,
                    ident = result.link.as_repr(),
                    title = result.title
                );
            }
        }
        Commands::EpCount { ident } => {
            let id = Identifier::from_repr(&ident).ok_or("invalid identifier")?;
            let title = repo
                .detail(EpisodeLink {
                    link: id.clone(),
                    title: String::new(),
                })
                .await
                .map_err(|_| "could not fetch anime details")?
                .anime_title;
            let ep_count = repo
                .list_eps(id)
                .await
                .map_err(|_| "could not fetch episode list")?
                .len();

            println!(r#""{title}" has {ep_count} episodes."#);
        }
        Commands::Detail { ident } => {
            let id = Identifier::from_repr(&ident).ok_or("invalid identifier")?;
            let detail = repo
                .detail(EpisodeLink {
                    link: id.clone(),
                    title: String::new(),
                })
                .await
                .map_err(|_| "could not fetch anime details")?;
            let ep_count = repo
                .list_eps(id)
                .await
                .map_err(|_| "could not fetch episode list")?
                .len();

            println!(
                r#"{title}
{eps} episodes, {ident}

{description}"#,
                title = detail.anime_title,
                eps = ep_count,
                description = detail.description
            );
        }
        Commands::Watch { ident, ep } => {
            let id = Identifier::from_repr(&ident).ok_or("invalid identifier")?;
            let episodes = repo
                .list_eps(id)
                .await
                .map_err(|_| "could not fetch episode list")?;
            let episode = episodes
                .get(
                    ep.checked_sub(1)
                        .ok_or("episode number must be 1 or greater")?,
                )
                .ok_or("episode number out of range")?;
            let link = repo
                .watch_link(episode.clone())
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
            mpv.wait().map_err(|_| "mpv exited unexpectedly")?;
        }
    }

    Ok(())
}
