use clap::Parser;

#[derive(Debug, Parser)]
#[clap(author, version)]
#[clap(propagate_version = true)]
/// A terminal app to search and watch anime from multiple sources in MPV
pub struct Args {
    /// A command. If omitted, launches the interactive TUI instead.
    #[clap(subcommand)]
    pub command: Option<Commands>,
}

/// Supported CLI commnands
#[allow(missing_docs)]
#[derive(Debug, Subcommand)]
pub enum Commands {
    /// Search for an anime by title
    Search {
        /// Anime title
        title: String,
    },
    /// Get a list of episodes for anime identifier.
    EpCount {
        /// An anime identifier
        ident: String,
    },
    /// Get detailed information: description + episode list
    Detail {
        /// An anime identifier
        ident: String,
    },
    /// Watch episode in MPV
    Watch {
        /// Anime identifier
        ident: String,
        /// Episode number
        ep: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_search() {
        let args = Args::try_parse_from(["ani-tui", "search", "bocchi the rock"]).unwrap();
        match args.command {
            Some(Commands::Search { title }) => assert_eq!(title, "bocchi the rock"),
            other => panic!("expected Search, got {other:?}"),
        }
    }

    #[test]
    fn parses_ep_count() {
        let args = Args::try_parse_from(["ani-tui", "ep-count", "<GLP-1:x#1>"]).unwrap();
        match args.command {
            Some(Commands::EpCount { ident }) => assert_eq!(ident, "<GLP-1:x#1>"),
            other => panic!("expected EpCount, got {other:?}"),
        }
    }

    #[test]
    fn parses_detail() {
        let args = Args::try_parse_from(["ani-tui", "detail", "<GLP-1:x#1>"]).unwrap();
        match args.command {
            Some(Commands::Detail { ident }) => assert_eq!(ident, "<GLP-1:x#1>"),
            other => panic!("expected Detail, got {other:?}"),
        }
    }

    #[test]
    fn parses_watch_with_episode_number() {
        let args = Args::try_parse_from(["ani-tui", "watch", "<GLP-1:x#1>", "3"]).unwrap();
        match args.command {
            Some(Commands::Watch { ident, ep }) => {
                assert_eq!(ident, "<GLP-1:x#1>");
                assert_eq!(ep, 3);
            }
            other => panic!("expected Watch, got {other:?}"),
        }
    }

    #[test]
    fn rejects_watch_missing_episode_number() {
        assert!(Args::try_parse_from(["ani-tui", "watch", "<GLP-1:x#1>"]).is_err());
    }

    #[test]
    fn rejects_unknown_subcommand() {
        assert!(Args::try_parse_from(["ani-tui", "not-a-command"]).is_err());
    }

    #[test]
    fn bare_invocation_has_no_command() {
        let args = Args::try_parse_from(["ani-tui"]).unwrap();
        assert!(args.command.is_none());
    }
}
