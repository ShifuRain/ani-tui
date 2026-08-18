use crate::anime_repo::AnimeRepositoryError;
use std::process::Stdio;
use tokio::process::Command;

/// `curl` executable names to try, in order, before falling back to plain `curl`. Each is a
/// [curl-impersonate](https://github.com/lexiforest/curl-impersonate) build that reproduces a
/// real browser's TLS/JA3 and HTTP2 fingerprint, which some sources' Cloudflare challenges
/// check for. Plain `curl` (and any plain HTTP client, e.g. `reqwest`) can get blocked
/// regardless of what headers it sends. This list and the fallback behavior mirror
/// [ani-cli](https://github.com/pystardust/ani-cli)'s `curl_exe` detection.
const IMPERSONATE_CANDIDATES: &[&str] =
    &["curl_chrome136", "curl_firefox135", "curl_chrome116", "curl_ff117"];

/// TLS 1.2 cipher suite order used on macOS when no curl-impersonate binary is available,
/// mimicking a browser's ordering closely enough to sometimes get past a Cloudflare check with
/// plain `curl`. Ported from ani-cli's `ciphers`.
const MACOS_CIPHERS: &str = "ECDHE-ECDSA-AES128-GCM-SHA256:ECDHE-RSA-AES128-GCM-SHA256:ECDHE-ECDSA-AES256-GCM-SHA384:ECDHE-RSA-AES256-GCM-SHA384:ECDHE-ECDSA-CHACHA20-POLY1305:ECDHE-RSA-CHACHA20-POLY1305";

/// TLS 1.3 cipher suite order used alongside [`MACOS_CIPHERS`]. Ported from ani-cli's
/// `tls13_ciphers`.
const MACOS_TLS13_CIPHERS: &str =
    "TLS_AES_128_GCM_SHA256:TLS_AES_256_GCM_SHA384:TLS_CHACHA20_POLY1305_SHA256";

/// A `curl`-backed HTTP client shared by every [`crate::anime_repo::AnimeRepository`]. Handles
/// curl-impersonate binary resolution and Cloudflare-challenge detection once, so individual
/// sources don't need to reimplement either.
pub struct CurlClient {
    /// Name (or path) of the `curl`-compatible executable used for all requests
    curl_exe: String,
    /// Whether [`Self::curl_exe`] is one of [`IMPERSONATE_CANDIDATES`] rather than plain `curl`
    impersonating: bool,
    /// Extra flags appended to every request, e.g. macOS's cipher-suite mimicry
    extra_args: Vec<String>,
    /// `User-Agent` header sent with every request
    user_agent: String,
}

impl CurlClient {
    /// Resolves the best available `curl`-compatible executable once, and sends `user_agent`
    /// with every subsequent request.
    pub fn new(user_agent: impl Into<String>) -> Self {
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

        Self { curl_exe, impersonating, extra_args, user_agent: user_agent.into() }
    }

    /// Fetches `url` via GET, sending each of `headers` (formatted `"Name: value"`) verbatim.
    /// Returns the response body as text.
    pub async fn get(&self, url: &str, headers: &[&str]) -> Result<String, QueryError> {
        self.run(url, headers, None).await
    }

    /// Sends `form` as an `application/x-www-form-urlencoded` POST body to `url`, alongside
    /// each of `headers` verbatim. Returns the response body as text.
    pub async fn post_form(
        &self,
        url: &str,
        form: &[(&str, &str)],
        headers: &[&str],
    ) -> Result<String, QueryError> {
        let body = form
            .iter()
            .map(|(key, value)| format!("{}={}", encode_query_param(key), encode_query_param(value)))
            .collect::<Vec<_>>()
            .join("&");
        self.run(url, headers, Some(&body)).await
    }

    /// Runs `curl` against `url`, optionally as a POST with `body`.
    ///
    /// Deliberately doesn't fail on non-2xx status (no `--fail`), matching ani-cli's
    /// `anidb_curl`: a Cloudflare challenge page can itself be served with a non-2xx status,
    /// and we need to inspect its body to tell that apart from a real error.
    async fn run(&self, url: &str, headers: &[&str], body: Option<&str>) -> Result<String, QueryError> {
        let mut cmd = Command::new(&self.curl_exe);
        cmd.args(["-sL", "-A", &self.user_agent, "--max-time", "10"])
            .args(&self.extra_args);
        for header in headers {
            cmd.arg("-H").arg(header);
        }
        if let Some(body) = body {
            cmd.arg("--data").arg(body);
        }
        cmd.arg(url).stdout(Stdio::piped()).stderr(Stdio::null());

        let output = cmd.output().await.or(Err(QueryError::ConnectionError))?;

        if !output.status.success() {
            return Err(QueryError::ConnectionError);
        }

        let body = String::from_utf8(output.stdout).or(Err(QueryError::ConnectionError))?;

        if body.contains("Just a moment") {
            return Err(QueryError::Blocked { impersonating: self.impersonating });
        }

        Ok(body)
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

/// Percent-encodes `value` for use as a URL query parameter or form field.
pub fn encode_query_param(value: &str) -> String {
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

/// An error that could occur while making a request through a [`CurlClient`]
#[derive(Debug, PartialEq, Eq)]
pub enum QueryError {
    /// Connection to the server could not be established
    ConnectionError,
    /// Returned if the url supplied doesnt pass the validation checks
    InvalidLink,
    /// Occurs when a page could not be parsed into a struct
    ParsingError,
    /// The request went through, but a Cloudflare challenge intercepted it instead of
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
                        "Blocked by Cloudflare, even with a curl-impersonate binary. The site may have tightened its check."
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

    #[test]
    fn encodes_query_params() {
        assert_eq!(encode_query_param("bocchi the rock!"), "bocchi%20the%20rock%21");
        assert_eq!(encode_query_param("a-b_c.d~e"), "a-b_c.d~e");
    }
}
