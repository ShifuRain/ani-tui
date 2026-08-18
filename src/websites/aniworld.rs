use crate::anime_repo::{self, AnimeRepository, AnimeRepositoryError, WatchLink};
use crate::websites::curl_client::{CurlClient, QueryError};
use regex::Regex;
use serde::Deserialize;
use std::collections::{BTreeSet, HashMap, HashSet};

use QueryError::ParsingError;

/// Hoster-specific stream-URL extraction for aniworld.to's `/redirect/<id>` targets.
mod extractors;

/// A link to the website
pub const BASE_URL: &str = "https://aniworld.to";

/// A source prefix in the string representation of ids from this source
pub const REPR_PREFIX: &str = "AWT-1";

/// User agent sent with every request. aniworld.to has no Cloudflare TLS-fingerprint gate
/// (unlike anidb.app), so this only needs to look like a normal browser.
const AGENT: &str =
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0.0.0 Safari/537.36";

/// Language priority `watch_link` resolves against, highest first. Informational only for now —
/// there's no way to request a specific one yet.
const LANGUAGE_PRIORITY: &[&str] = &["German (dub)", "German (sub)", "English (sub)"];

/// Hoster priority `watch_link` resolves against, highest first. Only hosters with a working
/// extractor in [`extractors`] are listed; others are still parsed (so [`AniWorld::detail`]'s
/// language list stays honest about what's on the page) but never dispatched to.
const HOSTER_PRIORITY: &[&str] = &["voe", "vidmoly"];

/// <https://aniworld.to> API.
pub struct AniWorld {
    /// HTTP client used for all requests to aniworld.to
    client: CurlClient,
}

impl AniWorld {
    #[allow(missing_docs)]
    pub fn new() -> Self {
        Self { client: CurlClient::new(AGENT) }
    }
}

impl Default for AniWorld {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AnimeRepository for AniWorld {
    fn prefix(&self) -> &'static str {
        REPR_PREFIX
    }

    async fn search(&self, query: &str) -> anime_repo::Result<Vec<anime_repo::SearchResult>> {
        let json = self
            .client
            .post_form(
                &format!("{BASE_URL}/ajax/search"),
                &[("keyword", query)],
                &["X-Requested-With: XMLHttpRequest"],
            )
            .await?;

        Ok(parse_search_json(&json)?
            .into_iter()
            .map(|(title, slug)| anime_repo::SearchResult {
                title,
                id: anime_repo::GlobalId {
                    prefix: REPR_PREFIX.to_string(),
                    raw: slug,
                },
            })
            .collect())
    }

    async fn list_eps(&self, raw_id: &str) -> anime_repo::Result<Vec<anime_repo::Episode>> {
        if raw_id.is_empty() {
            return Err(AnimeRepositoryError::Unsupported);
        }
        let slug = raw_id;

        let series_html = self
            .client
            .get(&format!("{BASE_URL}/anime/stream/{slug}"), &[])
            .await?;
        let seasons = parse_seasons(&series_html, slug);

        let mut episodes = Vec::new();
        for season in seasons {
            let season_html = self.client.get(&season_url(slug, season), &[]).await?;
            let titles = parse_episode_titles(&season_html);
            for number in parse_episode_numbers(&season_html, slug, season) {
                let base = if season == 0 {
                    format!("Film {number}")
                } else {
                    format!("S{season:02} E{number:02}")
                };
                let title = match titles.get(&number) {
                    Some(real_title) => format!("{base} — {real_title}"),
                    None => base,
                };
                episodes.push(anime_repo::Episode {
                    title,
                    number,
                    id: anime_repo::GlobalId {
                        prefix: REPR_PREFIX.to_string(),
                        raw: episode_raw_id(slug, season, number),
                    },
                });
            }
        }
        Ok(episodes)
    }

    async fn detail(&self, raw_id: &str) -> anime_repo::Result<anime_repo::Detail> {
        if raw_id.is_empty() {
            return Err(AnimeRepositoryError::Unsupported);
        }
        let slug = raw_id;

        let series_html = self
            .client
            .get(&format!("{BASE_URL}/anime/stream/{slug}"), &[])
            .await?;
        let (title, description) = parse_detail_page(&series_html).ok_or(ParsingError)?;

        let episodes = self.list_eps(raw_id).await?;
        let episode_count = episodes.len();

        // Best-effort: a series' available languages are consistent across its episodes, so
        // one extra request against the first episode is enough. Don't fail `detail()` over it.
        let languages = episodes
            .first()
            .and_then(|first| parse_episode_raw_id(&first.id.raw))
            .map(|(slug, season, number)| (slug.to_string(), season, number));
        let languages = match languages {
            Some((slug, season, number)) => {
                let url = episode_url(&slug, season, number);
                self.client
                    .get(&url, &[])
                    .await
                    .ok()
                    .map(|html| {
                        let mut langs: Vec<String> = parse_language_map(&html)
                            .into_iter()
                            .map(|(_, lang)| lang)
                            .collect();
                        langs.sort();
                        langs.dedup();
                        langs
                    })
                    .unwrap_or_default()
            }
            None => Vec::new(),
        };

        Ok(anime_repo::Detail {
            title,
            description,
            episode_count,
            languages,
        })
    }

    async fn watch_link(&self, raw_id: &str) -> anime_repo::Result<WatchLink> {
        let (slug, season, number) =
            parse_episode_raw_id(raw_id).ok_or(AnimeRepositoryError::Unsupported)?;

        let episode_html = self.client.get(&episode_url(slug, season, number), &[]).await?;
        let lang_map = parse_language_map(&episode_html);
        let hosters = parse_hosters(&episode_html, &lang_map);

        for &language in LANGUAGE_PRIORITY {
            for &hoster_name in HOSTER_PRIORITY {
                let Some(hoster) = hosters
                    .iter()
                    .find(|h| h.language == language && h.name.eq_ignore_ascii_case(hoster_name))
                else {
                    continue;
                };

                let redirect_url = format!("{BASE_URL}{}", hoster.redirect_path);
                let resolved = match hoster_name {
                    "voe" => extractors::voe::extract(&self.client, &redirect_url).await,
                    "vidmoly" => extractors::vidmoly::extract(&self.client, &redirect_url).await,
                    _ => continue,
                };
                if let Ok(link) = resolved {
                    return Ok(link);
                }
            }
        }
        Err(AnimeRepositoryError::NotFound)
    }
}

/// Builds the opaque episode-level [`anime_repo::GlobalId::raw`] id from a series slug, season
/// number (`0` == the site's `/filme` movies section) and episode/film number. Round-trips with
/// [`parse_episode_raw_id`].
fn episode_raw_id(slug: &str, season: u32, number: u32) -> String {
    format!("{slug}#s{season}#e{number}")
}

/// Parses an episode-level raw id built by [`episode_raw_id`] back into
/// `(slug, season, number)`. Rejects anything malformed rather than guessing.
fn parse_episode_raw_id(raw: &str) -> Option<(&str, u32, u32)> {
    let mut parts = raw.split('#');
    let slug = parts.next()?;
    if slug.is_empty() {
        return None;
    }
    let season = parts.next()?.strip_prefix('s')?.parse().ok()?;
    let number = parts.next()?.strip_prefix('e')?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((slug, season, number))
}

/// URL of a season's own page (or the films section, for `season == 0`).
fn season_url(slug: &str, season: u32) -> String {
    if season == 0 {
        format!("{BASE_URL}/anime/stream/{slug}/filme")
    } else {
        format!("{BASE_URL}/anime/stream/{slug}/staffel-{season}")
    }
}

/// URL of a single episode's (or film's, for `season == 0`) page.
fn episode_url(slug: &str, season: u32, number: u32) -> String {
    if season == 0 {
        format!("{BASE_URL}/anime/stream/{slug}/filme/film-{number}")
    } else {
        format!("{BASE_URL}/anime/stream/{slug}/staffel-{season}/episode-{number}")
    }
}

/// One entry of aniworld.to's `POST /ajax/search` JSON array response.
#[derive(Debug, Deserialize)]
struct SearchEntry {
    /// Raw (still HTML-escaped, may contain `<em>` highlight tags) title
    title: String,
    /// Site-relative link, e.g. `/anime/stream/<slug>` for a series or
    /// `/anime/stream/<slug>/staffel-<n>/episode-<m>` for an episode-level hit
    link: String,
}

/// Parses aniworld.to's search JSON into de-duplicated `(title, slug)` pairs, keeping only
/// series-root hits (dropping episode-level hits and unrelated pages mixed into the same
/// response). Split out from [`AniWorld::search`] so it can be tested against a fixture file
/// without a network round-trip.
fn parse_search_json(json: &str) -> Result<Vec<(String, String)>, QueryError> {
    let entries: Vec<SearchEntry> = serde_json::from_str(json).map_err(|_| ParsingError)?;

    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for entry in entries {
        let Some(slug) = series_root_slug(&entry.link) else {
            continue;
        };
        if !seen.insert(slug.clone()) {
            continue;
        }
        out.push((clean_search_text(&entry.title), slug));
    }
    Ok(out)
}

/// Returns the series slug if `link` is exactly a series root (`/anime/stream/<slug>`), or
/// `None` for episode-level links (`/anime/stream/<slug>/staffel-N/episode-M`) and unrelated
/// pages.
fn series_root_slug(link: &str) -> Option<String> {
    let parts: Vec<&str> = link.split('/').filter(|part| !part.is_empty()).collect();
    match parts.as_slice() {
        ["anime", "stream", slug] => Some((*slug).to_string()),
        _ => None,
    }
}

/// Strips `<em>`/`</em>` highlight tags and unescapes HTML entities from search-result text.
fn clean_search_text(text: &str) -> String {
    let tag_re = Regex::new(r"</?em>").unwrap();
    unescape_entities(&tag_re.replace_all(text, ""))
}

/// Unescapes the handful of HTML entities aniworld.to's search JSON can contain (named XML
/// entities plus numeric `&#NNN;`/`&#xHH;` forms). Real accented characters (ü, ß, ...) arrive
/// as native JSON `\uXXXX` escapes already handled by `serde_json`, so this minimal set covers
/// what actually shows up rather than a full HTML5 entity table.
fn unescape_entities(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < text.len() {
        if text.as_bytes()[i] == b'&' {
            if let Some(rel_end) = text[i..].find(';') {
                let end = i + rel_end;
                if let Some(ch) = decode_entity(&text[i + 1..end]) {
                    out.push(ch);
                    i = end + 1;
                    continue;
                }
            }
        }
        let ch = text[i..].chars().next().expect("i is a valid char boundary");
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Decodes a single HTML entity name (without the surrounding `&`/`;`) into its character.
fn decode_entity(entity: &str) -> Option<char> {
    match entity {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        _ => {
            let digits = entity.strip_prefix('#')?;
            let code = if let Some(hex) = digits.strip_prefix('x').or_else(|| digits.strip_prefix('X')) {
                u32::from_str_radix(hex, 16).ok()?
            } else {
                digits.parse().ok()?
            };
            char::from_u32(code)
        }
    }
}

/// Parses an anime series page's HTML into `(title, description)`. Split out from
/// [`AniWorld::detail`] so it can be tested against a fixture file without a network
/// round-trip.
fn parse_detail_page(html: &str) -> Option<(String, String)> {
    let title_pattern = easy_scraper::Pattern::new(
        r#"<div class="series-title"><h1 itemprop="name"><span>{{title}}</span></h1></div>"#,
    )
    .unwrap();
    let description_pattern =
        easy_scraper::Pattern::new(r#"<p class="seri_des" data-full-description="{{description}}">"#)
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

/// Parses a series page's season nav block into season numbers (`0` == the films section),
/// sorted and de-duplicated. Split out from [`AniWorld::list_eps`] so it can be tested against
/// a fixture file without a network round-trip.
fn parse_seasons(html: &str, slug: &str) -> Vec<u32> {
    let escaped = regex::escape(slug);
    let staffel_re =
        Regex::new(&format!(r#"href="/anime/stream/{escaped}/staffel-(\d+)""#)).unwrap();
    let filme_re = Regex::new(&format!(r#"href="/anime/stream/{escaped}/filme""#)).unwrap();

    let mut seasons: BTreeSet<u32> = staffel_re
        .captures_iter(html)
        .filter_map(|cap| cap[1].parse().ok())
        .collect();
    if filme_re.is_match(html) {
        seasons.insert(0);
    }
    seasons.into_iter().collect()
}

/// Parses a season page's episode nav block into episode (or film, for `season == 0`) numbers,
/// sorted and de-duplicated. Split out from [`AniWorld::list_eps`] so it can be tested against
/// a fixture file without a network round-trip.
fn parse_episode_numbers(html: &str, slug: &str, season: u32) -> Vec<u32> {
    let escaped = regex::escape(slug);
    let pattern = if season == 0 {
        format!(r#"href="/anime/stream/{escaped}/filme/film-(\d+)""#)
    } else {
        format!(r#"href="/anime/stream/{escaped}/staffel-{season}/episode-(\d+)""#)
    };
    let re = Regex::new(&pattern).unwrap();

    let numbers: BTreeSet<u32> = re
        .captures_iter(html)
        .filter_map(|cap| cap[1].parse().ok())
        .collect();
    numbers.into_iter().collect()
}

/// Parses a season page's episode table into `episode_number -> real German title` (falls
/// back to the plain "S01 E01"-style label when a number has no entry here). This is the same
/// season page already fetched in [`AniWorld::list_eps`] for episode numbers — the titles
/// live in a `<table>` with `class="seasonEpisodeTitle"` cells that a narrower earlier scrape
/// of just the nav `<ul>` missed, so this needs no extra request. Split out so it can be
/// tested against a fixture file without a network round-trip.
fn parse_episode_titles(html: &str) -> HashMap<u32, String> {
    let re = Regex::new(
        r#"(?s)<meta itemprop="episodeNumber" content="(\d+)"\s*/>.*?<td class="seasonEpisodeTitle"><a[^>]*>\s*<strong>([^<]*)</strong>"#,
    )
    .unwrap();

    re.captures_iter(html)
        .filter_map(|cap| {
            let number: u32 = cap[1].parse().ok()?;
            Some((number, unescape_entities(cap[2].trim())))
        })
        .collect()
}

/// Classifies a language flag's combined `src`+`alt`+`title` text into one of our canonical
/// labels, falling back to a `lang-<key>` placeholder for anything unrecognized. Sub-variants
/// are checked before the plain dub case, since they also contain "german"/"deutsch".
fn classify_language(blob: &str, key: u32) -> String {
    let blob = blob.to_lowercase();
    if blob.contains("japanese-german") || blob.contains("untertitel deutsch") || blob.contains("ger-sub") {
        "German (sub)".to_string()
    } else if blob.contains("japanese-english")
        || blob.contains("untertitel englisch")
        || blob.contains("english")
        || blob.contains("englisch")
    {
        "English (sub)".to_string()
    } else if blob.contains("german") || blob.contains("deutsch") {
        "German (dub)".to_string()
    } else {
        format!("lang-{key}")
    }
}

/// Parses an episode page's `.changeLanguageBox` into `(lang_key, language)` pairs. Split out
/// from [`AniWorld::watch_link`]/[`AniWorld::detail`] so it can be tested against a fixture
/// file without a network round-trip.
fn parse_language_map(html: &str) -> Vec<(u32, String)> {
    let box_re = Regex::new(r#"(?s)<div class="changeLanguageBox">(.*?)</div>"#).unwrap();
    let Some(box_content) = box_re.captures(html).and_then(|cap| cap.get(1)) else {
        return Vec::new();
    };

    let img_re = Regex::new(r"<img\b[^>]*>").unwrap();
    let key_re = Regex::new(r#"data-lang-key="(\d+)""#).unwrap();
    let attr_re = Regex::new(r#"(?:src|alt|title)="([^"]*)""#).unwrap();

    img_re
        .find_iter(box_content.as_str())
        .filter_map(|tag_match| {
            let tag = tag_match.as_str();
            let key: u32 = key_re.captures(tag)?.get(1)?.as_str().parse().ok()?;
            let blob = attr_re
                .captures_iter(tag)
                .map(|cap| cap[1].to_string())
                .collect::<Vec<_>>()
                .join(" ");
            Some((key, classify_language(&blob, key)))
        })
        .collect()
}

/// A hoster entry on an episode page, joined against its resolved language.
#[derive(Debug, PartialEq, Eq)]
struct HosterEntry {
    /// Display name shown on the page, e.g. `"VOE"`
    name: String,
    /// Canonical language label resolved via [`parse_language_map`]
    language: String,
    /// Site-relative `/redirect/<id>` path
    redirect_path: String,
}

/// Parses an episode page's hoster list, joining each entry against `lang_map` for its
/// language. Entries for hosters with no working extractor (see [`extractors`]) are still
/// returned — only [`AniWorld::watch_link`]'s dispatch step skips them — so callers can report
/// the full set of languages actually offered. Split out from [`AniWorld::watch_link`] so it
/// can be tested against a fixture file without a network round-trip.
fn parse_hosters(html: &str, lang_map: &[(u32, String)]) -> Vec<HosterEntry> {
    let li_re = Regex::new(r"(?s)<li\b([^>]*)>(.*?)</li>").unwrap();
    let key_re = Regex::new(r#"data-lang-key="(\d+)""#).unwrap();
    let target_re = Regex::new(r#"data-link-target="(/redirect/[^"]*)""#).unwrap();
    let name_re = Regex::new(r"<h4[^>]*>([^<]*)</h4>").unwrap();

    li_re
        .captures_iter(html)
        .filter_map(|cap| {
            let attrs = &cap[1];
            let body = &cap[2];

            let lang_key: u32 = key_re.captures(attrs)?.get(1)?.as_str().parse().ok()?;
            let redirect_path = target_re.captures(attrs)?.get(1)?.as_str().to_string();
            let name = name_re.captures(body)?.get(1)?.as_str().trim().to_string();
            if name.is_empty() {
                return None;
            }

            let language = lang_map
                .iter()
                .find(|(key, _)| *key == lang_key)
                .map(|(_, lang)| lang.clone())
                .unwrap_or_else(|| format!("lang-{lang_key}"));

            Some(HosterEntry { name, language, redirect_path })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn adapter_reports_its_prefix() {
        assert_eq!(AniWorld::new().prefix(), REPR_PREFIX);
    }

    #[tokio::test]
    async fn adapter_rejects_empty_raw_id_without_a_network_call() {
        let aniworld = AniWorld::new();

        assert!(matches!(
            aniworld.list_eps("").await,
            Err(AnimeRepositoryError::Unsupported)
        ));
        assert!(matches!(
            aniworld.detail("").await,
            Err(AnimeRepositoryError::Unsupported)
        ));
        assert!(matches!(
            aniworld.watch_link("").await,
            Err(AnimeRepositoryError::Unsupported)
        ));
    }

    #[test]
    fn episode_raw_id_round_trips() {
        assert_eq!(
            parse_episode_raw_id(&episode_raw_id("attack-on-titan", 1, 5)),
            Some(("attack-on-titan", 1, 5))
        );
        assert_eq!(
            parse_episode_raw_id(&episode_raw_id("attack-on-titan", 0, 2)),
            Some(("attack-on-titan", 0, 2))
        );
    }

    #[test]
    fn episode_raw_id_rejects_garbage() {
        assert_eq!(parse_episode_raw_id(""), None);
        assert_eq!(parse_episode_raw_id("attack-on-titan"), None);
        assert_eq!(parse_episode_raw_id("attack-on-titan#s1"), None);
        assert_eq!(parse_episode_raw_id("attack-on-titan#s1#e5#extra"), None);
        assert_eq!(parse_episode_raw_id("attack-on-titan#x1#e5"), None);
    }

    #[test]
    fn parses_search_json_fixture() {
        let json = include_str!("../../tests/fixtures/aniworld-search.json");
        let results = parse_search_json(json).expect("fixture should parse");

        assert_eq!(results.len(), 2);
        assert_eq!(
            results[0],
            ("Attack on Titan".to_string(), "attack-on-titan".to_string())
        );
        assert_eq!(
            results[1],
            (
                "Attack & Defense: Do You Love Your Mom and Her Two-Hit Multi-Target Attacks?"
                    .to_string(),
                "attack-and-defense".to_string()
            )
        );
    }

    #[test]
    fn parses_detail_page_fixture() {
        let html = include_str!("../../tests/fixtures/aniworld-detail.html");
        let (title, description) = parse_detail_page(html).expect("fixture should parse");

        assert_eq!(title, "Attack on Titan");
        assert!(description.contains("Titanen"));
    }

    #[test]
    fn parses_seasons_fixture() {
        let html = include_str!("../../tests/fixtures/aniworld-detail.html");
        assert_eq!(parse_seasons(html, "attack-on-titan"), vec![0, 1, 2]);
    }

    #[test]
    fn parses_episode_numbers_fixture() {
        let html = include_str!("../../tests/fixtures/aniworld-season.html");
        assert_eq!(
            parse_episode_numbers(html, "attack-on-titan", 1),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn parses_episode_titles_fixture() {
        let html = include_str!("../../tests/fixtures/aniworld-season.html");
        let titles = parse_episode_titles(html);

        assert_eq!(titles.len(), 3);
        assert_eq!(
            titles.get(&1),
            Some(&"An dich in 2000 Jahren - Der Fall von Shiganshina, Teil 1".to_string())
        );
    }

    #[test]
    fn parses_language_map_fixture() {
        let html = include_str!("../../tests/fixtures/aniworld-episode.html");
        let map = parse_language_map(html);

        assert_eq!(
            map,
            vec![
                (1, "German (dub)".to_string()),
                (3, "German (sub)".to_string()),
                (2, "English (sub)".to_string()),
            ]
        );
    }

    #[test]
    fn parses_hosters_fixture() {
        let html = include_str!("../../tests/fixtures/aniworld-episode.html");
        let lang_map = parse_language_map(html);
        let hosters = parse_hosters(html, &lang_map);

        assert_eq!(hosters.len(), 3, "unsupported hosters are still parsed");

        let voe = hosters.iter().find(|h| h.name == "VOE").unwrap();
        assert_eq!(voe.language, "German (dub)");
        assert_eq!(voe.redirect_path, "/redirect/3540458");

        let vidmoly = hosters.iter().find(|h| h.name == "Vidmoly").unwrap();
        assert_eq!(vidmoly.redirect_path, "/redirect/3381704");

        let unsupported = hosters.iter().find(|h| h.name == "Doodstream").unwrap();
        assert_eq!(unsupported.language, "English (sub)");
    }
}
