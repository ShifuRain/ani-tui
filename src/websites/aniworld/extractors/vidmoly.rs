use crate::anime_repo::{self, AnimeRepositoryError, WatchLink};
use crate::websites::curl_client::CurlClient;
use regex::Regex;

/// aniworld.to's own origin, sent as `Referer` since Vidmoly's CDN requires one.
const REFERER: &str = "https://aniworld.to/";

/// Extracts the `sources: [{ file: '...' }]` jwplayer source URL from a Vidmoly embed page.
/// Pure and fixture-testable, split out from [`extract`].
fn parse_embed(html: &str) -> Option<String> {
    let pattern = Regex::new(r#"sources\s*:\s*\[\s*\{\s*file\s*:\s*["']([^"']+)["']"#).unwrap();
    Some(pattern.captures(html)?.get(1)?.as_str().to_string())
}

/// Follows `redirect_url` (aniworld.to's `/redirect/<id>` link, which curl `-L` already
/// resolves through Vidmoly's chained redirects) and resolves the final stream link. Vidmoly's
/// CDN requires a `Referer` header on the stream request or it rejects playback.
pub async fn extract(client: &CurlClient, redirect_url: &str) -> anime_repo::Result<WatchLink> {
    let referer_header = format!("Referer: {REFERER}");
    let html = client.get(redirect_url, &[referer_header.as_str()]).await?;
    let url = parse_embed(&html).ok_or(AnimeRepositoryError::NotFound)?;

    Ok(WatchLink {
        url,
        headers: vec![("Referer".to_string(), REFERER.to_string())],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_embed_page_fixture() {
        let html = include_str!("../../../../tests/fixtures/aniworld-vidmoly-embed.html");
        let url = parse_embed(html).expect("fixture should parse");
        assert!(url.contains("master.m3u8"));
    }
}
