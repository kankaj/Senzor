//! Whether a newer Senzor is out: the latest release on GitHub, asked about at most once a day.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// A published release: its version (without a leading `v`) and the page that describes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    pub url: String,
}

/// Where the latest release comes from, behind a trait so tests never touch the network.
pub trait ReleaseSource: Send {
    /// The latest published release, or `None` when there is none yet.
    fn latest(&self) -> Result<Option<Release>, String>;
}

/// How long after a check the next one waits.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Now, in seconds since 1970, as the settings remember the last check.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Whether to ask again at `now`, after the last check at `checked` (both in seconds since
/// 1970). A last check in the future (a clock set back) does not hold the next one off.
pub fn due(checked: Option<u64>, now: u64) -> bool {
    match checked {
        None => true,
        Some(checked) => now < checked || now - checked >= CHECK_INTERVAL.as_secs(),
    }
}

/// `tag` without a leading `v`.
fn without_v(tag: &str) -> &str {
    tag.strip_prefix(['v', 'V']).unwrap_or(tag)
}

/// `major.minor.patch` of a stable release's tag, with or without a leading `v`; `None` for a
/// pre-release (`1.1.0-rc.1`) or a tag that is not a version.
fn stable_version(tag: &str) -> Option<(u64, u64, u64)> {
    let version = without_v(tag.trim());
    if version.contains('-') {
        return None;
    }
    crate::whats_new::version_number(version)
}

/// Whether `release` is a stable version newer than `current`.
pub fn is_newer(release: &str, current: &str) -> bool {
    match (
        stable_version(release),
        crate::whats_new::version_number(current),
    ) {
        (Some(release), Some(current)) => release > current,
        _ => false,
    }
}

/// GitHub's latest stable release of the repository in `Cargo.toml` (pre-releases and drafts
/// are never the latest). One HTTPS request, without cookies or anything about the user: the
/// program and its version in the `User-Agent`, which GitHub requires.
pub struct GitHubReleases;

/// The host of GitHub's API.
const API_HOST: &str = "api.github.com";
/// How long the check may take, from connecting to the last byte.
const TIMEOUT: Duration = Duration::from_secs(5);
/// More than a release's description ever needs; a longer answer is an error.
const MAX_ANSWER: usize = 1024 * 1024;

/// The API path of the repository's latest release, for example
/// `/repos/kankaj/Senzor/releases/latest`.
fn latest_path() -> String {
    let repository = env!("CARGO_PKG_REPOSITORY");
    let owner_and_name = repository
        .strip_prefix("https://github.com/")
        .unwrap_or(repository);
    format!("/repos/{owner_and_name}/releases/latest")
}

/// The program, its version and where it comes from.
fn user_agent() -> String {
    format!(
        "Senzor/{} (+{})",
        env!("CARGO_PKG_VERSION"),
        env!("CARGO_PKG_REPOSITORY")
    )
}

impl ReleaseSource for GitHubReleases {
    fn latest(&self) -> Result<Option<Release>, String> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|error| error.to_string())?;
        runtime.block_on(async {
            tokio::time::timeout(TIMEOUT, ask_github())
                .await
                .map_err(|_| format!("{API_HOST} did not answer within {TIMEOUT:?}"))?
        })
    }
}

/// Asks for the latest release; GitHub answers 404 while the repository has none.
async fn ask_github() -> Result<Option<Release>, String> {
    let (status, body) = get(API_HOST, &latest_path()).await?;
    match status {
        hyper::StatusCode::OK => parse_release(&body).map(Some),
        hyper::StatusCode::NOT_FOUND => Ok(None),
        status => Err(format!("{API_HOST} answered {status}")),
    }
}

/// `GET https://{host}{path}` over HTTP/1.1, trusting the system's certificates: the status and
/// the body.
async fn get(host: &str, path: &str) -> Result<(hyper::StatusCode, String), String> {
    use http_body_util::{BodyExt, Empty, Limited};
    use hyper::body::Bytes;
    use hyper::header;
    use tokio_rustls::rustls::{self, pki_types::ServerName};

    let mut roots = rustls::RootCertStore::empty();
    roots.add_parsable_certificates(rustls_native_certs::load_native_certs().certs);
    if roots.is_empty() {
        return Err("no trusted certificates found on this system".into());
    }
    let provider = std::sync::Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut tls = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| error.to_string())?
        .with_root_certificates(roots)
        .with_no_client_auth();
    tls.alpn_protocols = vec![b"http/1.1".to_vec()];
    let server_name = ServerName::try_from(host.to_string()).map_err(|error| error.to_string())?;

    let tcp = tokio::net::TcpStream::connect((host, 443))
        .await
        .map_err(|error| format!("cannot connect to {host}: {error}"))?;
    let stream = tokio_rustls::TlsConnector::from(std::sync::Arc::new(tls))
        .connect(server_name, tcp)
        .await
        .map_err(|error| format!("TLS with {host} failed: {error}"))?;
    let (mut sender, connection) =
        hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(stream))
            .await
            .map_err(|error| error.to_string())?;
    tokio::spawn(connection);

    let request = hyper::Request::get(path)
        .header(header::HOST, host)
        .header(header::USER_AGENT, user_agent())
        .header(header::ACCEPT, "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .body(Empty::<Bytes>::new())
        .map_err(|error| error.to_string())?;
    let response = sender
        .send_request(request)
        .await
        .map_err(|error| error.to_string())?;
    let status = response.status();
    let body = Limited::new(response.into_body(), MAX_ANSWER)
        .collect()
        .await
        .map_err(|error| error.to_string())?
        .to_bytes();
    Ok((status, String::from_utf8_lossy(&body).into_owned()))
}

/// The release in a response of GitHub's latest-release API.
fn parse_release(json: &str) -> Result<Release, String> {
    #[derive(Deserialize)]
    struct Response {
        tag_name: String,
        html_url: String,
    }
    let response: Response = serde_json::from_str(json)
        .map_err(|error| format!("unexpected answer from GitHub: {error}"))?;
    Ok(Release {
        version: without_v(response.tag_name.trim()).to_string(),
        url: response.html_url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_newer_stable_tag_is_newer() {
        assert!(is_newer("v1.1.0", "1.0.0"));
        assert!(is_newer("1.0.1", "1.0.0"));
        assert!(is_newer("v2.0.0", "1.9.9"));
        assert!(is_newer("V1.10.0", "1.9.0"), "numbers, not text");
    }

    #[test]
    fn the_same_or_an_older_version_is_not_newer() {
        assert!(!is_newer("v1.0.0", "1.0.0"));
        assert!(!is_newer("0.9.0", "1.0.0"));
    }

    #[test]
    fn a_pre_release_or_garbage_is_never_newer() {
        assert!(!is_newer("v1.1.0-rc.1", "1.0.0"));
        assert!(!is_newer("nightly", "1.0.0"));
        assert!(!is_newer("v1.1", "1.0.0"));
        assert!(!is_newer("", "1.0.0"));
        assert!(!is_newer("v1.1.0", "not a version"));
    }

    #[test]
    fn checks_are_a_day_apart() {
        let day = CHECK_INTERVAL.as_secs();
        let now = 1_800_000_000;
        assert!(due(None, now), "never checked");
        assert!(!due(Some(now - 60), now), "a minute ago");
        assert!(!due(Some(now - day + 1), now), "almost a day ago");
        assert!(due(Some(now - day), now), "a day ago");
        assert!(due(Some(now + 3 * day), now), "a clock set back");
    }

    #[test]
    fn the_request_names_the_repository_and_only_the_program() {
        assert_eq!(latest_path(), "/repos/kankaj/Senzor/releases/latest");
        assert_eq!(
            user_agent(),
            concat!(
                "Senzor/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/kankaj/Senzor)"
            )
        );
    }

    #[test]
    fn a_latest_release_response_gives_the_version_and_the_page() {
        let json = r#"{
            "url": "https://api.github.com/repos/kankaj/Senzor/releases/1",
            "html_url": "https://github.com/kankaj/Senzor/releases/tag/v1.1.0",
            "tag_name": "v1.1.0",
            "name": "Senzor 1.1.0",
            "prerelease": false,
            "assets": []
        }"#;
        assert_eq!(
            parse_release(json),
            Ok(Release {
                version: "1.1.0".into(),
                url: "https://github.com/kankaj/Senzor/releases/tag/v1.1.0".into(),
            })
        );
        assert!(parse_release("{}").is_err());
        assert!(parse_release("<html>").is_err());
    }
}
