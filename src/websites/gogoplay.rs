use crate::anime_repo::{self, AnimeRepository, AnimeRepositoryError};
use aes::Aes256;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use cbc::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use easy_scraper::Pattern;
use regex::{escape, Regex};
use reqwest::{
    header::{HeaderMap, USER_AGENT},
    Client,
};

use QueryError::{ConnectionError, InvalidLink, ParsingError};

/// A link to the website
pub const BASE_URL: &str = "https://goload.pro";

/// <https://goload.pro> API.
pub struct Gogoplay {
    /// HTTP client used for all requests to goload.pro
    web_client: Client,
}

impl Gogoplay {
    #[allow(missing_docs)]
    pub fn new() -> Self {
        let mut headers = HeaderMap::new();
        headers.insert(
            USER_AGENT,
            "Mozilla/5.0 (X11; Linux x86_64; rv:101.0) Gecko/20100101 Firefox/101.0"
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

impl Default for Gogoplay {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AnimeRepository for Gogoplay {
    fn prefix(&self) -> &'static str {
        REPR_PREFIX
    }

    async fn search(&self, query: &str) -> anime_repo::Result<Vec<anime_repo::SearchResult>> {
        let results = self
            .search(query)
            .await
            .ok_or(AnimeRepositoryError::DatasourceError)?;

        Ok(results
            .into_iter()
            .map(|ep| anime_repo::SearchResult {
                title: ep.title,
                id: anime_repo::GlobalId {
                    prefix: REPR_PREFIX.to_string(),
                    raw: ep.link.as_raw(),
                },
            })
            .collect())
    }

    async fn list_eps(&self, raw_id: &str) -> anime_repo::Result<Vec<anime_repo::Episode>> {
        let id = Identifier::from_raw(raw_id).ok_or(AnimeRepositoryError::Unsupported)?;
        let eps = self.episode_page(id).await?.ep_list;

        Ok(eps
            .into_iter()
            .map(|ep| anime_repo::Episode {
                title: ep.title,
                id: anime_repo::GlobalId {
                    prefix: REPR_PREFIX.to_string(),
                    raw: ep.link.as_raw(),
                },
            })
            .collect())
    }

    async fn detail(&self, raw_id: &str) -> anime_repo::Result<anime_repo::Detail> {
        let id = Identifier::from_raw(raw_id).ok_or(AnimeRepositoryError::Unsupported)?;
        let page = self.episode_page(id).await?;

        Ok(anime_repo::Detail {
            title: page.anime_title,
            description: page.description,
            episode_count: page.ep_list.len(),
        })
    }

    /// Resolves a direct video link by decrypting goload.pro's iframe payload.
    async fn watch_link(&self, raw_id: &str) -> anime_repo::Result<String> {
        let id = Identifier::from_raw(raw_id).ok_or(AnimeRepositoryError::Unsupported)?;
        let iframe_link = self.episode_page(id).await?.iframe;
        let iframe = self.iframe_page(&iframe_link).await?;

        let token = String::from_utf8(
            aes256_cbc_decrypt(&iframe.token, &iframe.secret_key, &iframe.iv)
                .ok_or(ParsingError)?,
        )
        .map_err(|_| ParsingError)?;

        let ajax_id = STANDARD.encode(
            aes256_cbc_encrypt(iframe.id.as_bytes(), &iframe.secret_key, &iframe.iv)
                .ok_or(ParsingError)?,
        );

        let json = self
            .web_client
            .get(format!(
                "https://goload.pro/encrypt-ajax.php?id={ajax_id}&alias={id}&{token}",
                ajax_id = ajax_id,
                id = iframe.id,
                token = token.split_at(token.find("token").ok_or(ParsingError)?).1,
            ))
            .header("X-Requested-With", "XMLHttpRequest")
            .send()
            .await
            .map_err(|_| ConnectionError)?
            .error_for_status()
            .map_err(|_| ConnectionError)?
            .text()
            .await
            .map_err(|_| ConnectionError)?;

        let regex = regex::Regex::new(r#""data":"(.*?)""#).unwrap();
        let enc_link = STANDARD
            .decode(
                regex
                    .captures(&json)
                    .ok_or(ParsingError)?
                    .get(1)
                    .ok_or(ParsingError)?
                    .as_str()
                    .replace('\\', ""),
            )
            .map_err(|_| ParsingError)?;

        let json = String::from_utf8(
            aes256_cbc_decrypt(&enc_link, &iframe.second_key, &iframe.iv).ok_or(ParsingError)?,
        )
        .map_err(|_| ParsingError)?;

        let regex = regex::Regex::new(r#""file":"(.*?)""#).unwrap();
        let link = regex
            .captures(&json)
            .ok_or(ParsingError)?
            .get(1)
            .ok_or(ParsingError)?
            .as_str()
            .replace('\\', "");

        Ok(link)
    }
}

/// AES-256-CBC decryptor, equivalent to `openssl enc -d -aes256`
type Aes256CbcDec = cbc::Decryptor<Aes256>;
/// AES-256-CBC encryptor, equivalent to `openssl enc -aes256`
type Aes256CbcEnc = cbc::Encryptor<Aes256>;

/// Decrypts `data` with AES-256 in CBC mode, PKCS7 padded. Mirrors `openssl enc -d -aes256 -K
/// key -iv iv`, but operating on raw key/iv bytes instead of a hex-encoded CLI argument.
fn aes256_cbc_decrypt(data: &[u8], key: &[u8], iv: &[u8]) -> Option<Vec<u8>> {
    Aes256CbcDec::new_from_slices(key, iv)
        .ok()?
        .decrypt_padded_vec_mut::<Pkcs7>(data)
        .ok()
}

/// Encrypts `data` with AES-256 in CBC mode, PKCS7 padded. Mirrors `openssl enc -aes256 -K key
/// -iv iv`, but operating on raw key/iv bytes instead of a hex-encoded CLI argument.
fn aes256_cbc_encrypt(data: &[u8], key: &[u8], iv: &[u8]) -> Option<Vec<u8>> {
    Some(
        Aes256CbcEnc::new_from_slices(key, iv)
            .ok()?
            .encrypt_padded_vec_mut::<Pkcs7>(data),
    )
}

impl Gogoplay {
    /// Returns content on a search page, given a title to search for
    ///
    /// # Return value
    ///
    /// Returns None in case of connection errors with the server
    pub async fn search(&self, title: &str) -> Option<SearchPage> {
        let html = self
            .web_client
            .get("https://goload.pro/search.html")
            .query(&[("keyword", title)])
            .send()
            .await
            .ok()?
            .error_for_status()
            .ok()?
            .text()
            .await
            .ok()?;

        parse_search_page(&html)
    }

    /// Get all the relevant info on a page
    pub async fn episode_page(&self, url: Identifier) -> Result<EpisodePage, QueryError> {
        let html = self
            .web_client
            .get(url.as_link())
            .send()
            .await
            .or(Err(ConnectionError))?
            .error_for_status()
            .or(Err(ConnectionError))?
            .text()
            .await
            .or(Err(ConnectionError))?;

        parse_episode_page(&html, url)
    }

    /// Returns content on player iframe page
    pub async fn iframe_page(&self, link: &str) -> Result<IframePage, QueryError> {
        if !link.starts_with(&format!("{}/streaming.php", BASE_URL)) {
            return Err(QueryError::InvalidLink);
        }

        let html = self
            .web_client
            .get(link)
            .send()
            .await
            .or(Err(QueryError::ConnectionError))?
            .error_for_status()
            .or(Err(QueryError::ConnectionError))?
            .text()
            .await
            .or(Err(QueryError::ConnectionError))?;

        parse_iframe_page(&html)
    }
}

/// Parses a search-results page's HTML into a list of results. Split out from
/// [`Gogoplay::search`] so the scraping logic can be tested against a fixture HTML file
/// without a network round-trip.
fn parse_search_page(html: &str) -> Option<SearchPage> {
    let pattern = Pattern::new(
        r#"
<div class="video_player followed default">
    <ul class="listing items">
        <li class="video-block ">
            <a href="{{link}}">
                <div class="name">
                  {{title}}
                </div>
            </a>
        </li>
    </ul>
</div>"#,
    )
    .unwrap();

    let mut eps = Vec::new();
    for ep in pattern.matches(html) {
        eps.push(EpisodeLink {
            title: ep.get("title").unwrap().to_string(),
            link: Identifier::from_link(&format!("{BASE_URL}{}", ep.get("link").unwrap()))?,
        })
    }
    Some(eps)
}

/// Parses an anime/episode page's HTML into its structured content. Split out from
/// [`Gogoplay::episode_page`] so the scraping logic can be tested against a fixture HTML file
/// without a network round-trip.
fn parse_episode_page(html: &str, url: Identifier) -> Result<EpisodePage, QueryError> {
    let info_pattern = Pattern::new(
        r#"
<div class="video-info">
  <div class="video-info-left">
    <h1>{{ep_title}}</h1>
    ...
    <div class="video-details">
      <span class="date">{{anime_title}}</span>
      <div class="post-entry">
        <div class="content-more-js" id="rmjs-1">{{description}}</div>
      </div>
    </div>
  </div>
</div>"#,
    )
    .unwrap();

    let episode_pattern = Pattern::new(
        r#"
<div class="video-info">
  <div class="video-info-left">
    <ul class="listing items lists">
      <li class="video-block ">
        <a href="{{ep_link}}">
          <div class="name">
            {{ep_title}}
          </div>
        </a>
      </li>
    </ul>
  </div>
</div>"#,
    )
    .unwrap();

    let iframe_pattern = Pattern::new(r#"<iframe src="{{link}}" allowfullscreen="true" frameborder="0" marginwidth="0" marginheight="0" scrolling="no" />"#).unwrap();

    let m = info_pattern.matches(html);
    let info = m.first().ok_or(ParsingError)?;
    let episodes = episode_pattern.matches(html);

    Ok(EpisodePage {
        link: url,
        ep_title: info["ep_title"].to_string(),
        anime_title: info["anime_title"].to_string(),
        description: info["description"].to_string(),
        ep_list: {
            let mut eps = Vec::new();
            for ep in episodes {
                eps.push(EpisodeLink {
                    title: ep["ep_title"].to_string(),
                    link: Identifier::from_link(&format!("{BASE_URL}{}", ep["ep_link"]))
                        .ok_or(InvalidLink)?,
                })
            }
            eps
        },
        iframe: format!(
            "https:{}",
            iframe_pattern.matches(html).first().ok_or(InvalidLink)?["link"]
        ),
    })
}

/// Parses a player iframe page's HTML into its structured content. Split out from
/// [`Gogoplay::iframe_page`] so the scraping logic can be tested against a fixture HTML file
/// without a network round-trip.
fn parse_iframe_page(html: &str) -> Result<IframePage, QueryError> {
    let pattern = Pattern::new(r#"
<head>
   <script type="text/javascript" src="https://goload.pro/js/crypto-js/crypto-js.js?v=9.988" data-name="episode" data-value="{{token}}"></script>
</head>
<body class="container-{{secret_key}}">
    <input type="hidden" id="id" value="{{id}}">
    ...
    <div class="wrapper container-{{iv}}">
        <div class="videocontent videocontent-{{second_key}}">
        </div>
    </div>
</body>
"#).unwrap();

    let matches = pattern.matches(html);
    let matches = matches.first().ok_or(QueryError::ParsingError)?;

    let get =
        |name: &str| -> Result<&str, QueryError> { Ok(matches.get(name).ok_or(ParsingError)?) };

    Ok(IframePage {
        token: STANDARD.decode(get("token")?).map_err(|_| ParsingError)?,
        secret_key: get("secret_key")?.as_bytes().to_vec(),
        second_key: get("second_key")?.as_bytes().to_vec(),
        iv: get("iv")?.as_bytes().to_vec(),
        id: get("id")?.to_string(),
    })
}

/// An identifier for an episode
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identifier {
    /// ID of the anime
    pub id: String,
    /// Episode number of the anime
    pub ep: usize,
}

/// A source prefix in the string representation of Identifier
pub const REPR_PREFIX: &str = "GLP-1";

impl Identifier {
    /// Takes a URL link `"https://goload.pro/..."` and returns a parsed object
    pub fn from_link(url: &str) -> Option<Self> {
        let cap = Regex::new(&format!(
            "^{}/videos/(?P<id>.*?)-episode-(?P<ep>.*?)$",
            escape(BASE_URL)
        ))
        .unwrap()
        .captures(url)?;
        Some(Self {
            id: cap.name("id")?.as_str().to_string(),
            ep: cap.name("ep")?.as_str().parse().ok()?,
        })
    }

    /// Parses this source's raw ID format (`id#ep`) — the `raw` part of a
    /// [`crate::anime_repo::GlobalId`] this source produced.
    pub fn from_raw(raw: &str) -> Option<Self> {
        let (id, ep) = raw.split_once('#')?;
        Some(Self {
            id: id.to_string(),
            ep: ep.parse().ok()?,
        })
    }

    /// Makes a new URL from self
    pub fn as_link(&self) -> String {
        format!(
            "{base}/videos/{id}-episode-{ep}",
            base = BASE_URL,
            id = self.id,
            ep = self.ep
        )
    }

    /// Formats as this source's raw ID format (`id#ep`) — the `raw` part of a
    /// [`crate::anime_repo::GlobalId`].
    pub fn as_raw(&self) -> String {
        format!("{id}#{ep}", id = self.id, ep = self.ep)
    }
}

/// A search page type
pub type SearchPage = Vec<EpisodeLink>;

/// An element of a result list on a search page
#[derive(Debug, Clone)]
pub struct EpisodeLink {
    /// Title of the element
    pub title: String,
    /// Link to the content
    pub link: Identifier,
}

/// Content on the page for an anime episode
#[derive(Debug)]
pub struct EpisodePage {
    /// A link to this page
    pub link: Identifier,
    /// Title of the episode
    pub ep_title: String,
    /// Title of the anime
    pub anime_title: String,
    /// Description on this page. Could be shared between all episodes of an anime,
    /// or it could be diferent for every episode.
    pub description: String,
    /// List of other episodes on this page
    pub ep_list: Vec<EpisodeLink>,
    /// Link to the player in the iframe
    pub iframe: String,
}

/// Anime details
pub struct Detail {
    /// Anime title
    pub anime_title: String,
    /// Anime description
    pub description: String,
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

/// Parsed content from the player iframe
#[derive(Debug)]
pub struct IframePage {
    /// Contains encrypted data used for fetching download URL
    pub token: Vec<u8>,
    /// 1st encryption key, as raw bytes (not hex-encoded)
    pub secret_key: Vec<u8>,
    /// 2nd encryption key, as raw bytes (not hex-encoded)
    pub second_key: Vec<u8>,
    /// Encryption IV (initialization vector), as raw bytes (not hex-encoded)
    pub iv: Vec<u8>,
    /// Anime ID
    pub id: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identifier_link_round_trip() {
        let ident = Identifier::from_link("https://goload.pro/videos/some-anime-episode-1")
            .expect("valid link should parse");
        assert_eq!(ident.id, "some-anime");
        assert_eq!(ident.ep, 1);
        assert_eq!(
            ident.as_link(),
            "https://goload.pro/videos/some-anime-episode-1"
        );
    }

    #[test]
    fn identifier_raw_round_trip() {
        let ident = Identifier::from_raw("some-anime#1").expect("valid raw id should parse");
        assert_eq!(ident.id, "some-anime");
        assert_eq!(ident.ep, 1);
        assert_eq!(ident.as_raw(), "some-anime#1");
    }

    #[tokio::test]
    async fn adapter_reports_its_prefix() {
        assert_eq!(Gogoplay::new().prefix(), REPR_PREFIX);
    }

    #[tokio::test]
    async fn adapter_rejects_malformed_raw_id_without_a_network_call() {
        // These all fail to parse as `id#ep` before any request is made, so this test is fast
        // and deterministic even without network access.
        let gogoplay = Gogoplay::new();

        assert!(matches!(
            gogoplay.list_eps("no-hash-here").await,
            Err(AnimeRepositoryError::Unsupported)
        ));
        assert!(matches!(
            gogoplay.detail("no-hash-here").await,
            Err(AnimeRepositoryError::Unsupported)
        ));
        assert!(matches!(
            gogoplay.watch_link("no-hash-here").await,
            Err(AnimeRepositoryError::Unsupported)
        ));
    }

    #[test]
    fn identifier_rejects_garbage() {
        assert!(Identifier::from_link("https://example.com/nope").is_none());
        assert!(Identifier::from_raw("no-hash-here").is_none());
        assert!(Identifier::from_raw("some-anime#not-a-number").is_none());
    }

    #[test]
    fn parses_search_page_fixture() {
        let html = include_str!("../../tests/fixtures/search.html");
        let results = parse_search_page(html).expect("fixture should parse");

        assert!(!results.is_empty());
        let first = &results[0];
        assert_eq!(first.title.trim(), "Some Anime Episode 12");
        assert_eq!(first.link.id, "some-anime");
        assert_eq!(first.link.ep, 12);
    }

    #[test]
    fn parses_episode_page_fixture() {
        let html = include_str!("../../tests/fixtures/some-anime-episode-1.html");
        let url = Identifier {
            id: "some-ident".to_string(),
            ep: 1,
        };
        let page = parse_episode_page(html, url).expect("fixture should parse");

        assert_eq!(page.ep_title.trim(), "Episode title");
        assert_eq!(page.anime_title.trim(), "Anime title");
        assert!(page.description.contains("Multiline"));
        assert!(page.iframe.starts_with("https://goload.pro/streaming.php"));
        assert_eq!(page.ep_list.len(), 2);
        assert_eq!(page.ep_list[0].link.id, "some-ident");
        assert_eq!(page.ep_list[0].link.ep, 2);
    }

    #[test]
    fn parses_iframe_page_fixture() {
        let html = include_str!("../../tests/fixtures/iframe.html");
        let page = parse_iframe_page(html).expect("fixture should parse");

        assert_eq!(page.id, "MTUwMjU3");
        assert!(!page.token.is_empty());
        assert_eq!(page.secret_key, b"37911490979715163134003223491201");
        assert_eq!(page.second_key, b"54674138327930866480207815084989");
        assert_eq!(page.iv, b"3134003223491201");
    }

    #[test]
    fn aes256_cbc_round_trips() {
        let key = [0x11u8; 32];
        let iv = [0x22u8; 16];
        let plaintext = b"the quick brown fox jumps over the lazy dog";

        let encrypted =
            aes256_cbc_encrypt(plaintext, &key, &iv).expect("encryption should succeed");
        let decrypted =
            aes256_cbc_decrypt(&encrypted, &key, &iv).expect("decryption should succeed");
        assert_eq!(decrypted, plaintext);
        assert_eq!(encrypted.len(), 48); // 44 bytes of plaintext padded to a multiple of the 16 byte block size
    }

    #[test]
    fn aes256_cbc_matches_openssl_cli_output() {
        // Cross-checks this port against the system `openssl` binary (the tool this code used
        // to shell out to), so we know the native rewrite is byte-for-byte equivalent. Skips
        // if `openssl` isn't installed rather than failing an environment that lacks it.
        use std::process::{Command, Stdio};

        let Ok(version_check) = Command::new("openssl").arg("version").output() else {
            eprintln!("openssl CLI not found, skipping cross-check");
            return;
        };
        if !version_check.status.success() {
            eprintln!("openssl CLI not usable, skipping cross-check");
            return;
        }

        let key_hex = hex::encode([0x11u8; 32]);
        let iv_hex = hex::encode([0x22u8; 16]);
        let plaintext = b"the quick brown fox jumps over the lazy dog";

        let mut child = Command::new("openssl")
            .args(["enc", "-e", "-aes256", "-K", &key_hex, "-iv", &iv_hex])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("failed to spawn openssl");
        {
            use std::io::Write;
            child
                .stdin
                .take()
                .unwrap()
                .write_all(plaintext)
                .expect("failed to write to openssl stdin");
        }
        let openssl_output = child
            .wait_with_output()
            .expect("failed to read openssl output")
            .stdout;

        let our_output = aes256_cbc_encrypt(plaintext, &[0x11u8; 32], &[0x22u8; 16])
            .expect("encryption should succeed");

        assert_eq!(
            our_output, openssl_output,
            "native AES-256-CBC output must match openssl CLI output"
        );
    }
}
