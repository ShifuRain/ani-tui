use ani_tui::websites::curl_client::CurlClient;
use serde::Deserialize;

/// GitHub's "latest release" API for this repo. Redirects past pre-releases/drafts on its own.
const RELEASES_URL: &str = "https://api.github.com/repos/ShifuRain/ani-tui/releases/latest";

/// The version this binary was built as, e.g. `"1.3.0"`.
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Shape of the fields we need from GitHub's release JSON.
#[derive(Deserialize)]
struct LatestRelease {
    /// e.g. `"v1.3.0"`
    tag_name: String,
}

/// Checks GitHub for the latest release, returning `Some(version)` (without the leading `v`)
/// if it's newer than this build. Never fails outward: offline, rate-limited, or a malformed
/// response all just yield `None`, since this is a nice-to-have notice, never a hard dependency
/// — the same rule the Jikan episode-title lookups follow.
pub async fn check_for_update() -> Option<String> {
    let client = CurlClient::new(format!("ani-tui/{CURRENT_VERSION}"));
    let json = client
        .get(RELEASES_URL, &["Accept: application/vnd.github+json"])
        .await
        .ok()?;
    let release: LatestRelease = serde_json::from_str(&json).ok()?;
    let latest = release.tag_name.strip_prefix('v').unwrap_or(&release.tag_name);

    is_newer(latest, CURRENT_VERSION).then(|| latest.to_string())
}

/// Whether `candidate` is a newer `major.minor.patch` version than `current`. Anything that
/// doesn't parse as three dot-separated numbers is treated as "not newer" — fails safe (no
/// notification) rather than erroring.
fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_semver(candidate), parse_semver(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

/// Parses a plain `major.minor.patch` version string, ignoring any pre-release/build suffix
/// (e.g. `"1.3.0-rc1"` parses as `(1, 3, 0)`).
fn parse_semver(version: &str) -> Option<(u32, u32, u32)> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()?;
    Some((major, minor, patch))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newer_patch_version_is_newer() {
        assert!(is_newer("1.3.1", "1.3.0"));
    }

    #[test]
    fn newer_minor_version_is_newer() {
        assert!(is_newer("1.4.0", "1.3.9"));
    }

    #[test]
    fn same_version_is_not_newer() {
        assert!(!is_newer("1.3.0", "1.3.0"));
    }

    #[test]
    fn older_version_is_not_newer() {
        assert!(!is_newer("1.2.0", "1.3.0"));
    }

    #[test]
    fn malformed_version_is_never_newer() {
        assert!(!is_newer("not-a-version", "1.3.0"));
        assert!(!is_newer("1.3.0", "not-a-version"));
    }

    #[test]
    fn ignores_a_pre_release_suffix() {
        assert_eq!(parse_semver("1.3.0-rc1"), Some((1, 3, 0)));
    }
}
