//! GitHub release discovery and HTTPS asset downloads.
//!
//! Installation only accepts `https://github.com/{owner}/{repo}` and only the
//! latest published GitHub release. The GitHub API response is parsed
//! strictly, every selected asset must carry GitHub's `sha256:<hex>` digest,
//! and downloads are bounded and HTTPS-only.
//!
//! Network I/O uses `curl` with argument arrays (never a shell), matching the
//! existing updater. Downloads are transport only: [`GitHubClient`] does not
//! verify digests, because the plugin store verifies every asset itself before
//! writing it into the store.

use std::process::Command;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Maximum GitHub API response size.
const MAX_API_BYTES: usize = 2 * 1024 * 1024;

/// Network timeout for API and asset requests.
pub const NETWORK_TIMEOUT: Duration = Duration::from_secs(20);

/// A parsed GitHub `owner/repo` identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
    owner: String,
    name: String,
}

impl Repository {
    /// Parse exactly `https://github.com/{owner}/{repo}`.
    ///
    /// A single trailing slash is accepted. HTTP, other hosts, query strings,
    /// fragments, credentials, extra path segments and percent-encoding are
    /// rejected.
    pub fn parse_url(input: &str) -> Result<Self> {
        let trimmed = input.trim();
        if trimmed != input {
            bail!("GitHub URL must not have leading or trailing whitespace");
        }
        let Some(path) = input.strip_prefix("https://github.com/") else {
            bail!("plugin source must be an https://github.com/{{owner}}/{{repo}} URL");
        };
        let path = path.strip_suffix('/').unwrap_or(path);
        Self::parse_full_name(path)
    }

    /// Parse an `owner/repo` pair, as stored in an install record.
    pub fn parse_full_name(value: &str) -> Result<Self> {
        if value.contains(':')
            || value.contains('?')
            || value.contains('#')
            || value.contains('%')
            || value.contains('\\')
            || value.contains('@')
        {
            bail!("invalid GitHub repository `{value}`");
        }
        let mut parts = value.split('/');
        let owner = parts.next().unwrap_or_default();
        let name = parts.next().unwrap_or_default();
        if parts.next().is_some() || owner.is_empty() || name.is_empty() {
            bail!("plugin source must be an https://github.com/{{owner}}/{{repo}} URL");
        }
        validate_owner(owner)?;
        validate_repo_name(name)?;
        Ok(Self {
            owner: owner.to_string(),
            name: name.to_string(),
        })
    }

    /// Repository owner.
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// Repository name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// `owner/repo`.
    pub fn full_name(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }

    /// Canonical browser URL.
    pub fn html_url(&self) -> String {
        format!("https://github.com/{}", self.full_name())
    }

    fn api_url(&self) -> String {
        format!(
            "https://api.github.com/repos/{}/releases/latest",
            self.full_name()
        )
    }

    fn asset_prefix(&self) -> String {
        format!("https://github.com/{}/releases/download/", self.full_name())
    }
}

fn validate_owner(owner: &str) -> Result<()> {
    if owner.is_empty() || owner.len() > 39 {
        bail!("GitHub owner must be 1-39 characters");
    }
    if !owner
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    {
        bail!("GitHub owner may contain only ASCII letters, digits and '-'");
    }
    let first = owner.chars().next().unwrap_or('-');
    let last = owner.chars().last().unwrap_or('-');
    if !first.is_ascii_alphanumeric() || !last.is_ascii_alphanumeric() {
        bail!("GitHub owner must start and end with a letter or digit");
    }
    Ok(())
}

fn validate_repo_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 100 {
        bail!("GitHub repository name must be 1-100 characters");
    }
    if name == "." || name == ".." {
        bail!("invalid GitHub repository name");
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        bail!("GitHub repository name may contain only ASCII letters, digits, '-', '_' and '.'");
    }
    Ok(())
}

/// A verified-shape SHA-256 digest from the GitHub release API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sha256Digest(String);

impl Sha256Digest {
    /// Parse GitHub's `sha256:<64 hex>` digest form.
    pub fn parse(value: &str) -> Result<Self> {
        let Some(hex) = value.strip_prefix("sha256:") else {
            bail!("release asset digest must use the `sha256:<hex>` form");
        };
        if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            bail!("release asset digest must contain exactly 64 hexadecimal characters");
        }
        Ok(Self(hex.to_ascii_lowercase()))
    }

    /// Lowercase hexadecimal digest.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Verify bytes against this digest.
    pub fn verify(&self, bytes: &[u8]) -> Result<()> {
        let actual = sha256_hex(bytes);
        if actual != self.0 {
            bail!(
                "SHA-256 mismatch: GitHub declared {} but the downloaded asset is {}",
                self.0,
                actual
            );
        }
        Ok(())
    }
}

/// Compute a lowercase SHA-256 hex digest.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

/// One release asset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseAsset {
    pub name: String,
    pub url: String,
    pub size: u64,
    pub digest: Option<Sha256Digest>,
}

/// The latest GitHub release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub tag: String,
    pub assets: Vec<ReleaseAsset>,
}

impl Release {
    /// Find exactly one asset by file name.
    pub fn asset(&self, name: &str) -> Result<&ReleaseAsset> {
        let mut matches = self.assets.iter().filter(|asset| asset.name == name);
        let Some(asset) = matches.next() else {
            bail!("latest GitHub release `{}` has no `{name}` asset", self.tag);
        };
        if matches.next().is_some() {
            bail!(
                "latest GitHub release `{}` has duplicate `{name}` assets",
                self.tag
            );
        }
        Ok(asset)
    }
}

/// Transport abstraction used by the plugin store.
///
/// Implementations must return the exact bytes served for an asset. The store
/// always verifies the release digest independently before publishing a
/// plugin, so a faulty provider cannot bypass integrity checks.
pub trait ReleaseProvider {
    /// Fetch the latest non-draft, non-prerelease GitHub release.
    fn latest_release(&self, repository: &Repository) -> Result<Release>;

    /// Download one release asset, rejecting responses above `max_bytes`.
    fn download_asset(
        &self,
        repository: &Repository,
        asset: &ReleaseAsset,
        max_bytes: usize,
    ) -> Result<Vec<u8>>;
}

/// GitHub Releases implementation backed by `curl`.
#[derive(Debug, Default, Clone, Copy)]
pub struct GitHubClient;

impl GitHubClient {
    /// Create a client.
    pub fn new() -> Self {
        Self
    }
}

impl ReleaseProvider for GitHubClient {
    fn latest_release(&self, repository: &Repository) -> Result<Release> {
        let body = curl(
            &repository.api_url(),
            MAX_API_BYTES,
            &[
                ("Accept", "application/vnd.github+json"),
                ("X-GitHub-Api-Version", "2022-11-28"),
            ],
        )
        .with_context(|| {
            format!(
                "cannot query the latest release for {}",
                repository.full_name()
            )
        })?;
        parse_release(&body, repository)
    }

    fn download_asset(
        &self,
        repository: &Repository,
        asset: &ReleaseAsset,
        max_bytes: usize,
    ) -> Result<Vec<u8>> {
        if !asset.url.starts_with(&repository.asset_prefix())
            || !asset.url.starts_with("https://github.com/")
            || asset.url.chars().any(char::is_whitespace)
        {
            bail!(
                "asset `{}` is not a GitHub release asset for {}",
                asset.name,
                repository.full_name()
            );
        }
        if asset.size > max_bytes as u64 {
            bail!(
                "release asset `{}` is too large ({} bytes, limit {max_bytes})",
                asset.name,
                asset.size
            );
        }
        let bytes = curl(&asset.url, max_bytes, &[]).with_context(|| {
            format!(
                "cannot download release asset `{}` from {}",
                asset.name,
                repository.full_name()
            )
        })?;
        if bytes.len() as u64 != asset.size {
            bail!(
                "release asset `{}` size mismatch: GitHub declared {} bytes but {} were downloaded",
                asset.name,
                asset.size,
                bytes.len()
            );
        }
        Ok(bytes)
    }
}

#[derive(Debug, Deserialize)]
struct ApiRelease {
    tag_name: String,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Debug, Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
    #[serde(default)]
    size: u64,
    #[serde(default)]
    digest: Option<String>,
}

fn parse_release(body: &[u8], repository: &Repository) -> Result<Release> {
    if body.len() > MAX_API_BYTES {
        bail!("GitHub API response exceeded {MAX_API_BYTES} bytes");
    }
    let api: ApiRelease =
        serde_json::from_slice(body).context("cannot parse the GitHub release response")?;
    if api.draft || api.prerelease {
        bail!(
            "GitHub returned a draft or prerelease for {}; install the latest stable release only",
            repository.full_name()
        );
    }
    if api.tag_name.is_empty()
        || api.tag_name.len() > 128
        || !api
            .tag_name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '+' | '_'))
    {
        bail!("latest release has an unsupported tag name");
    }

    let mut assets = Vec::with_capacity(api.assets.len());
    for asset in api.assets {
        if asset.name.is_empty()
            || asset.name.len() > 128
            || asset.name.contains('/')
            || asset.name.contains('\\')
            || asset.name.chars().any(char::is_control)
        {
            bail!("latest release contains an invalid asset name");
        }
        if !asset
            .browser_download_url
            .starts_with(&repository.asset_prefix())
        {
            bail!(
                "release asset `{}` does not belong to {}",
                asset.name,
                repository.full_name()
            );
        }
        let digest = match asset.digest {
            Some(value) => Some(Sha256Digest::parse(&value).with_context(|| {
                format!("release asset `{}` has an invalid digest", asset.name)
            })?),
            None => None,
        };
        assets.push(ReleaseAsset {
            name: asset.name,
            url: asset.browser_download_url,
            size: asset.size,
            digest,
        });
    }

    Ok(Release {
        tag: api.tag_name,
        assets,
    })
}

fn curl(url: &str, max_bytes: usize, headers: &[(&str, &str)]) -> Result<Vec<u8>> {
    let timeout_secs = NETWORK_TIMEOUT.as_secs().to_string();
    let connect_secs = (NETWORK_TIMEOUT.as_secs() / 2).max(1).to_string();
    let limit = max_bytes;
    let max_bytes = max_bytes.to_string();

    let mut command = Command::new("curl");
    command
        .arg("--fail")
        .arg("--silent")
        .arg("--show-error")
        .arg("--location")
        .arg("--proto")
        .arg("=https")
        .arg("--proto-redir")
        .arg("=https")
        .arg("--connect-timeout")
        .arg(connect_secs)
        .arg("--max-time")
        .arg(timeout_secs)
        .arg("--max-filesize")
        .arg(max_bytes)
        .arg("--user-agent")
        .arg(concat!("amph/", env!("CARGO_PKG_VERSION")));
    for (name, value) in headers {
        command.arg("--header").arg(format!("{name}: {value}"));
    }
    command.arg(url);

    let output = command
        .output()
        .context("cannot run curl; install curl to use the plugin store")?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        let detail = detail.trim();
        if detail.is_empty() {
            bail!("GitHub request failed with {}", output.status);
        }
        bail!("GitHub request failed with {}: {detail}", output.status);
    }
    if output.stdout.len() > limit {
        bail!("GitHub response exceeded its size limit");
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_exact_https_github_repository_urls() {
        let repo = Repository::parse_url("https://github.com/owner/amph-plugin").unwrap();
        assert_eq!(repo.owner(), "owner");
        assert_eq!(repo.name(), "amph-plugin");
        assert_eq!(repo.full_name(), "owner/amph-plugin");
        assert_eq!(
            Repository::parse_url("https://github.com/owner/amph-plugin/")
                .unwrap()
                .full_name(),
            "owner/amph-plugin"
        );
    }

    #[test]
    fn rejects_unsafe_or_non_github_sources() {
        for url in [
            "http://github.com/owner/repo",
            "https://gitlab.com/owner/repo",
            "https://github.com/owner",
            "https://github.com/owner/repo/sub",
            "https://github.com/owner/repo?tab=readme",
            "https://github.com/owner/repo#readme",
            "https://user@github.com/owner/repo",
            "https://github.com/../repo",
            "https://github.com/owner/..",
            "https://github.com/owner/re%70o",
            " https://github.com/owner/repo",
            "https://github.com/owner/repo ",
        ] {
            assert!(
                Repository::parse_url(url).is_err(),
                "accepted unsafe URL {url:?}"
            );
        }
    }

    #[test]
    fn digests_must_be_sha256_and_verify_exactly() {
        let bytes = b"plugin bytes";
        let hex = sha256_hex(bytes);
        let digest = Sha256Digest::parse(&format!("sha256:{hex}")).unwrap();
        digest.verify(bytes).unwrap();
        assert!(Sha256Digest::parse("sha512:abcd").is_err());
        assert!(Sha256Digest::parse("sha256:abcd").is_err());
        assert!(digest.verify(b"different").is_err());
    }

    fn release_json(digest: &str) -> String {
        format!(
            r#"{{
                "tag_name": "v1.0.0",
                "draft": false,
                "prerelease": false,
                "assets": [
                    {{
                        "name": "plugin.json",
                        "browser_download_url": "https://github.com/owner/repo/releases/download/v1.0.0/plugin.json",
                        "size": 12,
                        "digest": "{digest}"
                    }}
                ]
            }}"#
        )
    }

    #[test]
    fn parses_latest_release_assets_and_digests() {
        let repo = Repository::parse_url("https://github.com/owner/repo").unwrap();
        let digest = format!("sha256:{}", "a".repeat(64));
        let release = parse_release(release_json(&digest).as_bytes(), &repo).unwrap();
        assert_eq!(release.tag, "v1.0.0");
        let asset = release.asset("plugin.json").unwrap();
        assert_eq!(asset.digest, Sha256Digest::parse(&digest).ok());
        assert!(release.asset("plugin.wasm").is_err());
    }

    #[test]
    fn missing_digest_is_preserved_for_the_store_to_reject() {
        let repo = Repository::parse_url("https://github.com/owner/repo").unwrap();
        let json = r#"{
            "tag_name": "v1.0.0",
            "assets": [{
                "name": "plugin.json",
                "browser_download_url": "https://github.com/owner/repo/releases/download/v1.0.0/plugin.json",
                "size": 12
            }]
        }"#;
        let release = parse_release(json.as_bytes(), &repo).unwrap();
        assert!(release.asset("plugin.json").unwrap().digest.is_none());
    }

    #[test]
    fn malformed_digest_is_an_api_error() {
        let repo = Repository::parse_url("https://github.com/owner/repo").unwrap();
        let error = parse_release(release_json("sha256:not-hex").as_bytes(), &repo)
            .unwrap_err()
            .to_string();
        assert!(error.contains("invalid digest"), "{error}");
    }

    #[test]
    fn release_assets_must_belong_to_the_requested_repository() {
        let repo = Repository::parse_url("https://github.com/owner/repo").unwrap();
        let json = release_json(&format!("sha256:{}", "a".repeat(64)))
            .replace("owner/repo/releases", "other/repo/releases");
        assert!(parse_release(json.as_bytes(), &repo).is_err());
    }

    #[test]
    fn draft_and_prerelease_responses_are_rejected() {
        let repo = Repository::parse_url("https://github.com/owner/repo").unwrap();
        let json = release_json(&format!("sha256:{}", "a".repeat(64)))
            .replace("\"prerelease\": false", "\"prerelease\": true");
        assert!(parse_release(json.as_bytes(), &repo).is_err());
    }
}
