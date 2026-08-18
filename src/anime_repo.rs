/// An identifier for an anime/episode that's addressable across any source. Formats as
/// `<prefix:raw>`, e.g. `<ADB-1:some-anime#1>`. `prefix` identifies which
/// [`AnimeRepository`] produced it (see [`AnimeRepository::prefix`]); `raw` is an opaque
/// string that only that source's implementation understands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalId {
    /// Identifies which source this ID belongs to, e.g. `"ADB-1"`
    pub prefix: String,
    /// Opaque, source-defined identifier
    pub raw: String,
}

impl GlobalId {
    /// Makes a new user-facing representation of this ID: `<prefix:raw>`
    pub fn as_repr(&self) -> String {
        format!("<{}:{}>", self.prefix, self.raw)
    }

    /// Parses a user-facing `<prefix:raw>` representation back into a [`GlobalId`]
    pub fn from_repr(repr: &str) -> Option<Self> {
        let repr = repr.strip_prefix('<')?.strip_suffix('>')?;
        let (prefix, raw) = repr.split_once(':')?;
        Some(Self {
            prefix: prefix.to_string(),
            raw: raw.to_string(),
        })
    }
}

/// A single search result: a title plus the ID needed to look up more about it.
#[derive(Debug, Clone)]
pub struct SearchResult {
    /// Title of the anime
    pub title: String,
    /// ID that can be passed to [`AnimeRepository::list_eps`] or [`AnimeRepository::detail`]
    pub id: GlobalId,
}

/// A single episode: a title plus the ID needed to fetch a watch link for it.
#[derive(Debug, Clone)]
pub struct Episode {
    /// Title of the episode
    pub title: String,
    /// ID that can be passed to [`AnimeRepository::watch_link`]
    pub id: GlobalId,
}

/// Detailed info about an anime.
#[derive(Debug, Clone)]
pub struct Detail {
    /// Anime title
    pub title: String,
    /// Anime description
    pub description: String,
    /// Number of episodes
    pub episode_count: usize,
    /// Available audio/subtitle languages, informational only (there's no way to request a
    /// specific one yet). Empty if the source couldn't determine any.
    pub languages: Vec<String>,
}

/// A resolved, playable link to an episode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchLink {
    /// Direct URL to hand to the video player
    pub url: String,
    /// Extra HTTP headers (name, value) the player must send when requesting `url`, e.g. a
    /// `Referer` some hosters require to actually serve the stream rather than just resolving
    /// the link. Empty if none are needed.
    pub headers: Vec<(String, String)>,
}

impl WatchLink {
    /// Formats [`Self::headers`] as mpv's `--http-header-fields=...` argument value, or `None`
    /// if there are no headers to send.
    pub fn mpv_header_fields(&self) -> Option<String> {
        if self.headers.is_empty() {
            return None;
        }
        Some(
            self.headers
                .iter()
                .map(|(name, value)| format!("{name}: {value}"))
                .collect::<Vec<_>>()
                .join(","),
        )
    }
}

/// Interface for all anime search and watch implementors. Unlike an earlier version of this
/// trait, every method here uses the same concrete types (no associated types), so
/// implementors can be stored as `Box<dyn AnimeRepository>` and dispatched to at runtime —
/// see [`crate::registry::Registry`].
#[async_trait]
pub trait AnimeRepository: Send + Sync {
    /// A short, stable prefix identifying this source in a [`GlobalId`], e.g. `"ADB-1"`.
    /// Must be unique across all registered sources.
    fn prefix(&self) -> &'static str;

    /// Performs a search in the data source.
    async fn search(&self, query: &str) -> Result<Vec<SearchResult>>;
    /// Lists the episodes in a series. `raw_id` is the `raw` part of a [`GlobalId`] this
    /// source produced.
    async fn list_eps(&self, raw_id: &str) -> Result<Vec<Episode>>;
    /// Returns details about a series. `raw_id` is the `raw` part of a [`GlobalId`] this
    /// source produced.
    async fn detail(&self, raw_id: &str) -> Result<Detail>;
    /// Returns a watch link that can be played in a video player. `raw_id` is the `raw` part
    /// of a [`GlobalId`] this source produced.
    async fn watch_link(&self, raw_id: &str) -> Result<WatchLink>;
}

/// An error returned from [`AnimeRepository`]
#[derive(Debug)]
pub enum AnimeRepositoryError {
    /// Nothing was found
    NotFound,
    /// This operation could not be performed by this implementor, or no registered source
    /// matches the requested [`GlobalId`] prefix.
    /// Keep in mind that it doesnt mean that you will get `Unsopported` error for every query parameter.
    Unsupported,
    /// Datasource could not process your request. This could be caused by an internet connection error or a missing file.
    DatasourceError,
}

/// A result type shortcut returned by AnimeRepository
pub type Result<T> = std::result::Result<T, AnimeRepositoryError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_id_round_trips() {
        let id = GlobalId {
            prefix: "ADB-1".to_string(),
            raw: "some-anime#1".to_string(),
        };
        assert_eq!(id.as_repr(), "<ADB-1:some-anime#1>");
        assert_eq!(GlobalId::from_repr(&id.as_repr()).as_ref(), Some(&id));
    }

    #[test]
    fn global_id_rejects_garbage() {
        assert!(GlobalId::from_repr("not-a-valid-repr").is_none());
        assert!(GlobalId::from_repr("<no-colon>").is_none());
        assert!(GlobalId::from_repr("missing-brackets:raw").is_none());
    }

    #[test]
    fn mpv_header_fields_are_none_when_empty() {
        let link = WatchLink { url: "https://example.com".to_string(), headers: vec![] };
        assert_eq!(link.mpv_header_fields(), None);
    }

    #[test]
    fn mpv_header_fields_joins_multiple_headers() {
        let link = WatchLink {
            url: "https://example.com".to_string(),
            headers: vec![
                ("Referer".to_string(), "https://example.com/".to_string()),
                ("X-Custom".to_string(), "value".to_string()),
            ],
        };
        assert_eq!(
            link.mpv_header_fields().as_deref(),
            Some("Referer: https://example.com/,X-Custom: value")
        );
    }
}
