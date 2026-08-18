use crate::anime_repo::{self, AnimeRepository, AnimeRepositoryError};
use easy_scraper::Pattern;
use regex::Regex;
use serde::Deserialize;
use std::process::Stdio;
use tokio::process::Command;

use QueryError::{ConnectionError, InvalidLink, ParsingError};

/// A link to the website
pub const BASE_URL: &str = "https://anidb.app";

/// A source prefix in the string representation of ids from this source
pub const REPR_PREFIX: &str = "ADB-1";

/// User agent sent with every request, matched to the Chrome build that the curl-impersonate
/// binaries in [`IMPERSONATE_CANDIDATES`] emulate.
const AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// `curl` executable names to try, in order, before falling back to plain `curl`. Each is a
/// [curl-impersonate](https://github.com/lexiforest/curl-impersonate) build that reproduces a
/// real browser's TLS/JA3 and HTTP2 fingerprint, which anidb.app's Cloudflare challenge checks
/// for. Plain `curl` (and any plain HTTP client, e.g. `reqwest`) gets blocked regardless of
/// what headers it sends. This list and the fallback behavior mirror
/// [ani-cli](https://github.com/pystardust/ani-cli)'s `curl_exe` detection.
const IMPERSONATE_CANDIDATES: &[&str] =
    &["curl_chrome136", "curl_firefox135", "curl_chrome116", "curl_ff117"];

/// TLS 1.2 cipher suite order used on macOS when no curl-impersonate binary is available,
/// mimicking a browser's ordering closely enough to sometimes get past the Cloudflare check
/// with plain `curl`. Ported from ani-cli's `ciphers`.
const MACOS_CIPHERS: &str = "ECDHE-ECDSA-AES128-GCM-SHA256:ECDHE-RSA-AES128-GCM-SHA256:ECDHE-ECDSA-AES256-GCM-SHA384:ECDHE-RSA-AES256-GCM-SHA384:ECDHE-ECDSA-CHACHA20-POLY1305:ECDHE-RSA-CHACHA20-POLY1305";

/// TLS 1.3 cipher suite order used alongside [`MACOS_CIPHERS`]. Ported from ani-cli's
/// `tls13_ciphers`.
const MACOS_TLS13_CIPHERS: &str =
    "TLS_AES_128_GCM_SHA256:TLS_AES_256_GCM_SHA384:TLS_CHACHA20_POLY1305_SHA256";

/// <https://anidb.app> API.
pub struct AnidbApp {
    /// Name (or path) of the `curl`-compatible executable used for all requests
    curl_exe: String,
    /// Whether [`Self::curl_exe`] is one of [`IMPERSONATE_CANDIDATES`] rather than plain `curl`
    impersonating: bool,
    /// Extra flags appended to every request, e.g. macOS's cipher-suite mimicry
    extra_args: Vec<String>,
}

impl AnidbApp {
    #[allow(missing_docs)]
    pub fn new() -> Self {
        let (curl_exe, impersonating) = resolve_curl_exe();

        // Plain curl's cipher order can still be nudged closer to a browser's on macOS; every
        // other platform gets nothing extra since it doesn't reliably help there.
        let extra_args = if !impersonating && cfg!(target_os = "macos") {
            vec![
                "--ciphers".to_string(),
                MACOS_CIPHERS.to_string(),
                "--tls13-ciphers".to_string(),
                MACOS_TLS13_CIPHERS.to_string(),
            ]
        } else {
            Vec::new()
        };

        Self { curl_exe, impersonating, extra_args }
    }

    /// Fetches `url` via [`Self::curl_exe`] and returns the response body as text.
    ///
    /// Deliberately doesn't fail on non-2xx status (no `--fail`), matching ani-cli's
    /// `anidb_curl`: anidb.app's Cloudflare challenge page is itself served with a non-2xx
    /// status sometimes, and we need to inspect its body to tell that apart from a real error.
    async fn curl_get(&self, url: &str) -> Result<String, QueryError> {
        let output = Command::new(&self.curl_exe)
            .args(["-sL", "-A", AGENT, "--max-time", "10"])
            .args(&self.extra_args)
            .arg(url)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .output()
            .await
            .or(Err(ConnectionError))?;

        if !output.status.success() {
            return Err(ConnectionError);
        }

        let body = String::from_utf8(output.stdout).or(Err(ConnectionError))?;

        if body.contains("Just a moment") {
            return Err(QueryError::Blocked { impersonating: self.impersonating });
        }

        Ok(body)
    }
}

impl Default for AnidbApp {
    fn default() -> Self {
        Self::new()
    }
}

/// Picks which `curl`-compatible executable to use: the first of [`IMPERSONATE_CANDIDATES`]
/// found on `PATH`, or plain `curl` if none are installed. Returns the executable name and
/// whether it's an impersonating build.
fn resolve_curl_exe() -> (String, bool) {
    for candidate in IMPERSONATE_CANDIDATES {
        if binary_exists(candidate) {
            return (candidate.to_string(), true);
        }
    }
    ("curl".to_string(), false)
}

/// Checks whether `name` can be spawned at all, i.e. whether it exists on `PATH`.
fn binary_exists(name: &str) -> bool {
    std::process::Command::new(name)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}

/// Percent-encodes `value` for use as a URL query parameter.
fn encode_query_param(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[async_trait]
impl AnimeRepository for AnidbApp {
    fn prefix(&self) -> &'static str {
        REPR_PREFIX
    }

    async fn search(&self, query: &str) -> anime_repo::Result<Vec<anime_repo::SearchResult>> {
        let url = format!("{BASE_URL}/browse?q={}", encode_query_param(query));
        let html = self.curl_get(&url).await?;

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

        let url = format!("{BASE_URL}/api/frontend/anime/{numeric_id}/episodes");
        let json = self.curl_get(&url).await?;

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

        let url = format!("{BASE_URL}/anime/{raw_id}");
        let html = self.curl_get(&url).await?;
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

        let url = format!("{BASE_URL}/api/frontend/episode/{raw_id}/languages");
        let json = self.curl_get(&url).await?;
        let embed_url = parse_languages_json(&json, "jpn")?;

        let embed_html = self.curl_get(&embed_url).await?;

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
    /// The request went through, but anidb.app's Cloudflare challenge intercepted it instead of
    /// returning the real page.
    Blocked {
        /// Whether the request was already sent with a curl-impersonate binary (in which case
        /// installing one won't help further) or plain `curl` (in which case it might).
        impersonating: bool,
    },
}

impl From<QueryError> for AnimeRepositoryError {
    fn from(source: QueryError) -> Self {
        match source {
            QueryError::ConnectionError => AnimeRepositoryError::DatasourceError,
            QueryError::InvalidLink => AnimeRepositoryError::Unsupported,
            QueryError::ParsingError => AnimeRepositoryError::Unsupported,
            QueryError::Blocked { impersonating } => {
                if impersonating {
                    eprintln!(
                        "Blocked by Cloudflare, even with a curl-impersonate binary. anidb.app may have tightened its check."
                    );
                } else {
                    eprintln!(
                        "Blocked by Cloudflare. Install curl-impersonate (https://github.com/lexiforest/curl-impersonate) and make sure a binary like curl_chrome136 is on PATH."
                    );
                }
                AnimeRepositoryError::DatasourceError
            }
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

    #[test]
    fn encodes_query_params() {
        assert_eq!(encode_query_param("bocchi the rock!"), "bocchi%20the%20rock%21");
        assert_eq!(encode_query_param("a-b_c.d~e"), "a-b_c.d~e");
    }
}
