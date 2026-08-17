use crate::anime_repo::{self, AnimeRepository, AnimeRepositoryError};
use easy_scraper::Pattern;
use regex::Regex;
use reqwest::{
    header::{HeaderMap, USER_AGENT},
    Client,
};
use serde::Deserialize;

use QueryError::{ConnectionError, InvalidLink, ParsingError};

/// A link to the website
pub const BASE_URL: &str = "https://anidb.app";

/// A source prefix in the string representation of ids from this source
pub const REPR_PREFIX: &str = "ADB-1";

/// <https://anidb.app> API.
pub struct AnidbApp {
    /// HTTP client used for all requests to anidb.app
    web_client: Client,
}

impl AnidbApp {
    #[allow(missing_docs)]
    pub fn new() -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36"
                .parse()
                .expect("Could not set User Agent for web client"),
        );

        Self {
            web_client: Client::builder()
                .default_headers(headers)
                .build()
                .expect("Could not build a web client"),
        }
    }
}

impl Default for AnidbApp {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AnimeRepository for AnidbApp {
    fn prefix(&self) -> &'static str {
        REPR_PREFIX
    }

    async fn search(&self, query: &str) -> anime_repo::Result<Vec<anime_repo::SearchResult>> {
        let html = self
            .web_client
            .get(format!("{BASE_URL}/browse"))
            .query(&[("q", query)])
            .send()
            .await
            .or(Err(ConnectionError))?
            .error_for_status()
            .or(Err(ConnectionError))?
            .text()
            .await
            .or(Err(ConnectionError))?;

        Ok(parse_search_page(&html)
            .into_iter()
            .map(|(title, raw)| anime_repo::SearchResult {
                title,
                id: anime_repo::GlobalId {
                    prefix: REPR_PREFIX.to_string(),
                    raw,
                },
            })
            .collect())
    }

    async fn list_eps(&self, raw_id: &str) -> anime_repo::Result<Vec<anime_repo::Episode>> {
        if raw_id.is_empty() {
            return Err(AnimeRepositoryError::Unsupported);
        }
        let numeric_id = raw_id.rsplit('-').next().ok_or(InvalidLink)?;

        let json = self
            .web_client
            .get(format!(
                "{BASE_URL}/api/frontend/anime/{numeric_id}/episodes"
            ))
            .send()
            .await
            .or(Err(ConnectionError))?
            .error_for_status()
            .or(Err(ConnectionError))?
            .text()
            .await
            .or(Err(ConnectionError))?;

        Ok(parse_episodes_json(&json)?
            .into_iter()
            .map(|ep| anime_repo::Episode {
                title: format!("Episode {}", ep.number),
                id: anime_repo::GlobalId {
                    prefix: REPR_PREFIX.to_string(),
                    raw: ep.id.to_string(),
                },
            })
            .collect())
    }

    async fn detail(&self, raw_id: &str) -> anime_repo::Result<anime_repo::Detail> {
        if raw_id.is_empty() {
            return Err(AnimeRepositoryError::Unsupported);
        }

        let html = self
            .web_client
            .get(format!("{BASE_URL}/anime/{raw_id}"))
            .send()
            .await
            .or(Err(ConnectionError))?
            .error_for_status()
            .or(Err(ConnectionError))?
            .text()
            .await
            .or(Err(ConnectionError))?;
        let (title, description) = parse_detail_page(&html).ok_or(ParsingError)?;
        let episode_count = self.list_eps(raw_id).await?.len();

        Ok(anime_repo::Detail {
            title,
            description,
            episode_count,
        })
    }

    /// Resolves a direct HLS master playlist link for the Japanese-audio track. Only `jpn` is
    /// supported for now; there's no CLI flag yet to request `eng` (dub).
    async fn watch_link(&self, raw_id: &str) -> anime_repo::Result<String> {
        if raw_id.is_empty() {
            return Err(AnimeRepositoryError::Unsupported);
        }

        let json = self
            .web_client
            .get(format!(
                "{BASE_URL}/api/frontend/episode/{raw_id}/languages"
            ))
            .send()
            .await
            .or(Err(ConnectionError))?
            .error_for_status()
            .or(Err(ConnectionError))?
            .text()
            .await
            .or(Err(ConnectionError))?;
        let embed_url = parse_languages_json(&json, "jpn")?;

        let embed_html = self
            .web_client
            .get(&embed_url)
            .send()
            .await
            .or(Err(ConnectionError))?
            .error_for_status()
            .or(Err(ConnectionError))?
            .text()
            .await
            .or(Err(ConnectionError))?;

        let link = parse_embed_page(&embed_html).ok_or(ParsingError)?;
        Ok(link)
    }
}

/// Parses a search-results page's HTML into `(title, raw id)` pairs. Split out from
/// [`AnidbApp::search`] so the scraping logic can be tested against a fixture HTML file
/// without a network round-trip.
fn parse_search_page(html: &str) -> Vec<(String, String)> {
    let pattern = Pattern::new(
        r#"
<a href="{{link}}" class="anime-card block group" title="{{title}}">
...
</a>"#,
    )
    .unwrap();

    // `...` can match the same element more than once (e.g. a minimal and maximal match of the
    // same inner content), so dedupe by the exact (title, raw id) pair while keeping first-seen
    // order.
    let mut seen = std::collections::HashSet::new();
    pattern
        .matches(html)
        .into_iter()
        .filter_map(|m| {
            let link = m.get("link")?;
            let raw = link.strip_prefix(BASE_URL)?.strip_prefix("/anime/")?;
            Some((m.get("title")?.to_string(), raw.to_string()))
        })
        .filter(|entry| seen.insert(entry.clone()))
        .collect()
}

/// Parses an anime detail page's HTML into `(title, description)`. Split out from
/// [`AnidbApp::detail`] so the scraping logic can be tested against a fixture HTML file
/// without a network round-trip.
fn parse_detail_page(html: &str) -> Option<(String, String)> {
    let title_pattern = Pattern::new(
        r#"<h1 class="text-2xl sm:text-3xl font-bold text-white leading-tight flex-1">{{title}}</h1>"#,
    )
    .unwrap();
    let description_pattern =
        Pattern::new(r#"<p class="text-sm text-faint leading-relaxed">{{description}}</p>"#)
            .unwrap();

    let title = title_pattern
        .matches(html)
        .first()?
        .get("title")?
        .to_string();
    let description = description_pattern
        .matches(html)
        .first()?
        .get("description")?
        .to_string();

    Some((title, description))
}

/// Parses an embed page's HTML for its `file: '...'` JS literal, which points at the episode's
/// HLS master playlist. Split out from [`AnidbApp::watch_link`] so it can be tested against a
/// fixture HTML file without a network round-trip.
fn parse_embed_page(html: &str) -> Option<String> {
    let pattern = Regex::new(r"file: '([^']*)'").unwrap();
    Some(pattern.captures(html)?.get(1)?.as_str().to_string())
}

/// Response shape of `{BASE_URL}/api/frontend/anime/<id>/episodes`
#[derive(Debug, Deserialize)]
struct EpisodesResponse {
    /// The episode list
    episodes: Vec<EpisodeEntry>,
}

/// A single episode entry in [`EpisodesResponse`]
#[derive(Debug, Deserialize)]
struct EpisodeEntry {
    /// Opaque numeric episode ID, used to fetch a watch link
    id: u64,
    /// Episode number
    number: u64,
}

/// Parses the episodes JSON API response. Split out from [`AnidbApp::list_eps`] so it can be
/// tested against a fixture file without a network round-trip.
fn parse_episodes_json(json: &str) -> Result<Vec<EpisodeEntry>, QueryError> {
    serde_json::from_str::<EpisodesResponse>(json)
        .map(|response| response.episodes)
        .map_err(|_| ParsingError)
}

/// Response shape of `{BASE_URL}/api/frontend/episode/<id>/languages`
#[derive(Debug, Deserialize)]
struct LanguagesResponse {
    /// The available audio language tracks
    languages: Vec<LanguageEntry>,
}

/// A single language entry in [`LanguagesResponse`]
#[derive(Debug, Deserialize)]
struct LanguageEntry {
    /// Language code, e.g. `"jpn"` (sub) or `"eng"` (dub)
    code: String,
    /// Embed page URL for this language track
    embed_url: String,
}

/// Parses the languages JSON API response and returns the embed URL for `lang_code`. Split out
/// from [`AnidbApp::watch_link`] so it can be tested against a fixture file without a network
/// round-trip.
fn parse_languages_json(json: &str, lang_code: &str) -> Result<String, QueryError> {
    let response: LanguagesResponse = serde_json::from_str(json).map_err(|_| ParsingError)?;
    response
        .languages
        .into_iter()
        .find(|entry| entry.code == lang_code)
        .map(|entry| entry.embed_url)
        .ok_or(ParsingError)
}

/// An error that could occur as a result of a query
#[derive(Debug, PartialEq, Eq)]
pub enum QueryError {
    /// Connection to the server could not be established
    ConnectionError,
    /// Returned if the url supplied doesnt pass the validation checks
    InvalidLink,
    /// Occurs when a page could not be parsed into a struct
    ParsingError,
}

impl From<QueryError> for AnimeRepositoryError {
    fn from(source: QueryError) -> Self {
        match source {
            QueryError::ConnectionError => AnimeRepositoryError::DatasourceError,
            QueryError::InvalidLink => AnimeRepositoryError::Unsupported,
            QueryError::ParsingError => AnimeRepositoryError::Unsupported,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn adapter_reports_its_prefix() {
        assert_eq!(AnidbApp::new().prefix(), REPR_PREFIX);
    }

    #[tokio::test]
    async fn adapter_rejects_empty_raw_id_without_a_network_call() {
        let anidb = AnidbApp::new();

        assert!(matches!(
            anidb.list_eps("").await,
            Err(AnimeRepositoryError::Unsupported)
        ));
        assert!(matches!(
            anidb.detail("").await,
            Err(AnimeRepositoryError::Unsupported)
        ));
        assert!(matches!(
            anidb.watch_link("").await,
            Err(AnimeRepositoryError::Unsupported)
        ));
    }

    #[test]
    fn parses_search_page_fixture() {
        let html = include_str!("../../tests/fixtures/anidb-search.html");
        let results = parse_search_page(html);

        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "Bocchi the Rock!");
        assert_eq!(results[0].1, "bocchi-the-rock-729");
    }

    #[test]
    fn parses_detail_page_fixture() {
        let html = include_str!("../../tests/fixtures/anidb-detail.html");
        let (title, description) = parse_detail_page(html).expect("fixture should parse");

        assert_eq!(title, "Bocchi the Rock!");
        assert!(description.contains("Hitori"));
    }

    #[test]
    fn parses_episodes_json_fixture() {
        let json = include_str!("../../tests/fixtures/anidb-episodes.json");
        let episodes = parse_episodes_json(json).expect("fixture should parse");

        assert_eq!(episodes.len(), 2);
        assert_eq!(episodes[0].id, 23316);
        assert_eq!(episodes[0].number, 1);
    }

    #[test]
    fn parses_languages_json_fixture_and_finds_requested_language() {
        let json = include_str!("../../tests/fixtures/anidb-languages.json");

        assert_eq!(
            parse_languages_json(json, "jpn").expect("fixture should parse"),
            "https://anidb.app/embed/jpn-token"
        );
        assert!(parse_languages_json(json, "no-such-lang").is_err());
    }

    #[test]
    fn parses_embed_page_fixture() {
        let html = include_str!("../../tests/fixtures/anidb-embed.html");
        let link = parse_embed_page(html).expect("fixture should parse");

        assert_eq!(
            link,
            "https://hls.anidb.app/stream/example-token/master.m3u8"
        );
    }
}
