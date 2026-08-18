use crate::anime_repo::{self, AnimeRepositoryError, WatchLink};
use crate::websites::curl_client::CurlClient;
use base64::{engine::general_purpose::STANDARD, Engine};
use regex::Regex;

/// Literal junk substrings VOE's obfuscation pipeline inserts into the base64 payload; must be
/// stripped before decoding.
const JUNK_PAIRS: &[&str] = &["@$", "^^", "~@", "%?", "*~", "!!", "#&"];

/// Rot13-encodes/decodes `s` (rot13 is its own inverse). Only touches ASCII letters.
fn rot13(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='z' => (((c as u8 - b'a' + 13) % 26) + b'a') as char,
            'A'..='Z' => (((c as u8 - b'A' + 13) % 26) + b'A') as char,
            _ => c,
        })
        .collect()
}

/// Shifts a single ASCII character's codepoint by `delta`, wrapping to itself on overflow. VOE's
/// pipeline only ever runs this over base64 output (all ASCII), so a per-`char` shift is
/// equivalent to the reference implementation's per-byte shift.
fn shift_char(c: char, delta: i32) -> char {
    char::from_u32((c as i32 + delta) as u32).unwrap_or(c)
}

/// Extracts the raw JSON-array-of-one-string blob from a VOE embed page's
/// `<script type="application/json">["..."]</script>` tag.
fn find_json_blob(html: &str) -> Option<String> {
    let pattern =
        Regex::new(r#"(?s)<script[^>]+type="application/json"[^>]*>(.*?)</script>"#).unwrap();
    Some(pattern.captures(html)?.get(1)?.as_str().trim().to_string())
}

/// Reverses VOE's obfuscation pipeline on `blob` (the raw JSON-array-of-one-string captured by
/// [`find_json_blob`]): rot13 -> strip junk -> base64 decode -> shift every char by -3 ->
/// reverse -> base64 decode -> JSON-parse -> `source` (preferred) or `direct_access_url`.
fn decode_stream_url(blob: &str) -> Option<String> {
    let outer: serde_json::Value = serde_json::from_str(blob).ok()?;
    let inner = outer.as_array()?.first()?.as_str()?;

    let mut s = rot13(inner);
    for junk in JUNK_PAIRS {
        s = s.replace(junk, "");
    }
    let decoded = String::from_utf8(STANDARD.decode(s.as_bytes()).ok()?).ok()?;
    let shifted: String = decoded.chars().map(|c| shift_char(c, -3)).collect();
    let reversed: String = shifted.chars().rev().collect();
    let final_str = String::from_utf8(STANDARD.decode(reversed.as_bytes()).ok()?).ok()?;

    let obj: serde_json::Value = serde_json::from_str(&final_str).ok()?;
    obj.get("source")
        .or_else(|| obj.get("direct_access_url"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Extracts the final stream URL from a VOE embed page. Pure and fixture-testable, split out
/// from [`extract`].
fn parse_embed(html: &str) -> Option<String> {
    decode_stream_url(&find_json_blob(html)?)
}

/// Returns `scheme://host` for `url`, used as VOE's required `Referer`.
fn origin(url: &str) -> String {
    match url.splitn(2, "://").collect::<Vec<_>>().as_slice() {
        [scheme, rest] => {
            let host = rest.split('/').next().unwrap_or(rest);
            format!("{scheme}://{host}")
        }
        _ => url.to_string(),
    }
}

/// Follows `redirect_url` through VOE's redirect chain: `/redirect/<id>` -> `voe.sx/e/<code>`
/// (a JS stub that bounces to a rotating mirror domain via `window.location.href = '...'`,
/// followed manually here since curl can't follow a JS redirect) -> the mirror's embed page.
/// VOE's CDN requires a `Referer` set to that final page's own origin.
pub async fn extract(client: &CurlClient, redirect_url: &str) -> anime_repo::Result<WatchLink> {
    let hop_re = Regex::new(r#"window\.location\.href\s*=\s*['"]([^'"]+)['"]"#).unwrap();

    let mut url = redirect_url.to_string();
    let mut html = client.get(&url, &[]).await?;
    for _ in 0..3 {
        let Some(hop) = hop_re.captures(&html).and_then(|c| c.get(1)) else {
            break;
        };
        url = hop.as_str().to_string();
        html = client.get(&url, &[]).await?;
    }

    let stream_url = parse_embed(&html).ok_or(AnimeRepositoryError::NotFound)?;

    Ok(WatchLink {
        url: stream_url,
        headers: vec![("Referer".to_string(), origin(&url))],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rot13_is_its_own_inverse() {
        assert_eq!(rot13(&rot13("Hello, World!")), "Hello, World!");
        assert_eq!(rot13("abc"), "nop");
    }

    #[test]
    fn parses_embed_page_fixture() {
        let html = include_str!("../../../../tests/fixtures/aniworld-voe-embed.html");
        let url = parse_embed(html).expect("fixture should decode");
        assert_eq!(url, "https://voe-cdn.example.com/hls/master.m3u8");
    }

    #[test]
    fn extracts_origin() {
        assert_eq!(
            origin("https://example.com/e/abc?x=1"),
            "https://example.com"
        );
    }
}
