use crate::websites::curl_client::CurlClient;
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

/// Base URL of Jikan, the community-run MyAnimeList API used to fill in per-episode titles
/// that anidb.app itself doesn't expose.
const JIKAN_BASE_URL: &str = "https://api.jikan.moe/v4";

/// Hard cap on paginated episode-title requests per anime, so one absurdly long show can't
/// hang a lookup indefinitely. At ~100 episodes/page this covers up to ~2000 episodes.
const MAX_PAGES: u32 = 20;

/// Delay between paginated requests, comfortably under Jikan's public rate limit.
const PAGE_DELAY: Duration = Duration::from_millis(400);

/// Extracts the MyAnimeList numeric id from an anidb.app detail page, if it links to one.
/// Split out so it can be tested against a fixture file without a network round-trip.
pub fn extract_mal_id(html: &str) -> Option<u32> {
    let pattern = Regex::new(r"myanimelist\.net/anime/(\d+)").unwrap();
    pattern.captures(html)?.get(1)?.as_str().parse().ok()
}

/// Fetches every episode title for `mal_id`, from the local cache if present, or from Jikan
/// (caching the result) otherwise. Never fails outward: any problem (no cache, Jikan
/// unreachable, a malformed response) just yields an empty map, since titles are cosmetic and
/// never a hard dependency for the rest of the app to work.
pub async fn episode_titles(client: &CurlClient, mal_id: u32) -> HashMap<u32, String> {
    let mal_key = mal_id.to_string();
    let mut cache = load_cache();

    if let Some(cached) = cache.get(&mal_key) {
        return to_numeric_keys(cached);
    }

    let mut titles: HashMap<u32, String> = HashMap::new();
    for page in 1..=MAX_PAGES {
        let url = format!("{JIKAN_BASE_URL}/anime/{mal_id}/episodes?page={page}");
        let Ok(json) = client.get(&url, &[]).await else {
            break;
        };
        let Some((entries, has_next_page)) = parse_episodes_page(&json) else {
            break;
        };
        titles.extend(entries);

        if !has_next_page {
            break;
        }
        tokio::time::sleep(PAGE_DELAY).await;
    }

    if !titles.is_empty() {
        cache.insert(mal_key, to_string_keys(&titles));
        save_cache(&cache);
    }

    titles
}

/// Path to the local episode-title cache file: this is a pure performance cache
/// (regeneratable, safe to delete), so it belongs under the platform's cache dir, distinct
/// from config or user data.
fn cache_path() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "ani-tui")
        .map(|dirs| dirs.cache_dir().join("jikan_episode_titles.json"))
}

/// `mal_id -> episode_number -> title`, both keys stringified so the map round-trips through
/// JSON unambiguously.
type TitleCache = HashMap<String, HashMap<String, String>>;

/// Loads the cache file, or an empty cache if it's missing/unreadable/malformed.
fn load_cache() -> TitleCache {
    let Some(path) = cache_path() else {
        return TitleCache::new();
    };
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return TitleCache::new();
    };
    serde_json::from_str(&contents).unwrap_or_default()
}

/// Writes the cache file, creating its parent directory if needed. Best-effort: a write
/// failure is silently ignored, since losing the cache just means a future lookup re-fetches.
fn save_cache(cache: &TitleCache) {
    let Some(path) = cache_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string(cache) {
        let _ = std::fs::write(&path, json);
    }
}

/// Stringifies episode-number keys for storage in a [`TitleCache`] entry.
fn to_string_keys(map: &HashMap<u32, String>) -> HashMap<String, String> {
    map.iter().map(|(number, title)| (number.to_string(), title.clone())).collect()
}

/// Parses episode-number keys back out of a [`TitleCache`] entry, dropping any that aren't
/// valid numbers.
fn to_numeric_keys(map: &HashMap<String, String>) -> HashMap<u32, String> {
    map.iter()
        .filter_map(|(number, title)| number.parse().ok().map(|n| (n, title.clone())))
        .collect()
}

/// Response shape of `{JIKAN_BASE_URL}/anime/<id>/episodes?page=<n>`
#[derive(Debug, Deserialize, Serialize)]
struct EpisodesPage {
    /// Pagination info
    pagination: Pagination,
    /// This page's episode entries
    data: Vec<EpisodeEntry>,
}

/// Pagination info in [`EpisodesPage`]
#[derive(Debug, Deserialize, Serialize)]
struct Pagination {
    /// Whether a further page of episodes exists
    has_next_page: bool,
}

/// A single episode entry in [`EpisodesPage`]
#[derive(Debug, Deserialize, Serialize)]
struct EpisodeEntry {
    /// The episode's number within the series (Jikan's naming, not a MyAnimeList-wide id)
    mal_id: u32,
    /// English episode title
    title: String,
}

/// Parses one page of Jikan's episodes response into `(episode_number, title)` pairs plus
/// whether a further page exists. Split out from [`episode_titles`] so it can be tested
/// against a fixture file without a network round-trip.
fn parse_episodes_page(json: &str) -> Option<(Vec<(u32, String)>, bool)> {
    let response: EpisodesPage = serde_json::from_str(json).ok()?;
    let entries = response.data.into_iter().map(|ep| (ep.mal_id, ep.title)).collect();
    Some((entries, response.pagination.has_next_page))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_mal_id_fixture() {
        let html = include_str!("../../../tests/fixtures/anidb-detail.html");
        assert_eq!(extract_mal_id(html), Some(47917));
    }

    #[test]
    fn extract_mal_id_returns_none_without_a_link() {
        assert_eq!(extract_mal_id("<p>no mal link here</p>"), None);
    }

    #[test]
    fn parses_episodes_page_fixture() {
        let json = include_str!("../../../tests/fixtures/jikan-episodes-page1.json");
        let (entries, has_next_page) = parse_episodes_page(json).expect("fixture should parse");

        assert!(!has_next_page);
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0],
            (1, "To You, 2,000 Years in the Future: The Fall of Shiganshina (1)".to_string())
        );
    }

    #[test]
    fn title_cache_key_round_trip() {
        let mut numeric = HashMap::new();
        numeric.insert(1, "Episode One".to_string());
        numeric.insert(2, "Episode Two".to_string());

        let stringified = to_string_keys(&numeric);
        let back = to_numeric_keys(&stringified);
        assert_eq!(back, numeric);
    }
}
