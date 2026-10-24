//! GitHub update checks and Cargo self-install.
//!
//! The update channel is the `main` branch of the public GitHub repository.
//! The version check downloads a bounded copy of that branch's `Cargo.toml`
//! with `curl` (using argument arrays, never a shell) and compares its package
//! version with the running binary's Cargo version.
//!
//! [`check_for_update`] and [`install_update`] are intentionally small and
//! synchronous so the TUI can call them from a worker thread later.

use std::cmp::Ordering;
use std::fs;
use std::io::{self, IsTerminal};
use std::os::fd::AsFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, anyhow, bail};

/// Public repository used for update checks and Cargo installs.
pub const REPOSITORY: &str = "https://github.com/DAMIOTF/Arch-Meta-Package-Harmonizer";
/// Branch treated as the update channel.
pub const BRANCH: &str = "main";

const MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/DAMIOTF/Arch-Meta-Package-Harmonizer/main/Cargo.toml";
const CHECK_TIMEOUT: Duration = Duration::from_secs(10);
const NOTIFY_TIMEOUT: Duration = Duration::from_secs(3);
const MAX_MANIFEST_BYTES: usize = 256 * 1024;
const UPDATE_CACHE_TTL_SECS: u64 = 24 * 60 * 60;

/// A newer version published on the update branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateAvailable {
    /// Version of the running binary.
    pub current: String,
    /// Version found on the update branch.
    pub latest: String,
    /// Branch the update would be installed from.
    pub branch: &'static str,
    /// Repository the update would be installed from.
    pub repository: &'static str,
}

/// Version compiled into the running binary.
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// Check the update branch for a version newer than the running binary.
///
/// Network access is bounded by [`CHECK_TIMEOUT`]. Errors are returned so the
/// caller can decide whether to report them; [`notify_if_update_available`]
/// treats the same failures as non-fatal.
pub fn check_for_update() -> Result<Option<UpdateAvailable>> {
    let latest = fetch_latest_version(CHECK_TIMEOUT)?;
    update_available_from(current_version(), &latest)
}

/// Install the latest `main` branch with Cargo.
///
/// The current executable's Cargo root is used when it can be identified
/// (for example `~/.cargo/bin/amph` -> `~/.cargo`). This function never runs
/// `sudo`; permission failures are reported with an actionable message.
pub fn install_update() -> Result<()> {
    let root = cargo_root_for_current_exe();
    if let Some(root) = root.as_deref()
        && let Ok(metadata) = fs::metadata(root)
        && metadata.permissions().readonly()
    {
        bail!(
            "cannot install the update into {}: the directory is read-only. \
             amph never runs sudo; use a writable Cargo root or install with \
             the appropriate system privileges.",
            root.display()
        );
    }

    let mut command = Command::new("cargo");
    command.args([
        "install", "--git", REPOSITORY, "--branch", BRANCH, "--locked", "--force", "--bin", "amph",
    ]);
    if let Some(root) = root.as_deref() {
        command.arg("--root").arg(root);
    }

    // Keep stdout clean for `--json` callers: Cargo's progress is diagnostic,
    // so send everything to stderr (or discard stdout if stderr is unavailable).
    match io::stderr().as_fd().try_clone_to_owned() {
        Ok(stderr) => {
            command.stdout(Stdio::from(stderr));
        }
        Err(_) => {
            command.stdout(Stdio::null());
        }
    }
    command.stderr(Stdio::inherit());

    let status = command.status().with_context(|| {
        format!(
            "cannot run cargo to install the update; ensure Cargo is installed and {REPOSITORY} is reachable"
        )
    })?;
    if !status.success() {
        return Err(anyhow!(
            "cargo install failed with {status}. {}",
            permission_hint(root.as_deref())
        ));
    }
    Ok(())
}

/// Best-effort update notice for interactive human CLI runs.
///
/// The notice is written to stderr, is skipped when stderr is not a terminal,
/// and can be disabled with `AMPH_NO_UPDATE_CHECK=1`. Check results are cached
/// for one day so ordinary CLI commands do not hit the network every time.
pub fn notify_if_update_available() {
    if update_check_disabled() || !io::stderr().is_terminal() {
        return;
    }
    let Some(available) = cached_or_fetch_update() else {
        return;
    };
    eprintln!();
    eprintln!(
        "Update available: amph v{} -> v{} (GitHub {}).",
        available.current, available.latest, available.branch
    );
    eprintln!("Run `amph update` to install it.");
}

fn update_check_disabled() -> bool {
    match std::env::var("AMPH_NO_UPDATE_CHECK") {
        Ok(value) => {
            let value = value.trim();
            !(value.is_empty()
                || value.eq_ignore_ascii_case("0")
                || value.eq_ignore_ascii_case("false")
                || value.eq_ignore_ascii_case("no"))
        }
        Err(_) => false,
    }
}

fn cached_or_fetch_update() -> Option<UpdateAvailable> {
    let now = now_secs();
    if let Some((checked_at, latest)) = read_cached_version()
        && now.saturating_sub(checked_at) < UPDATE_CACHE_TTL_SECS
        && let Ok(available) = update_available_from(current_version(), &latest)
    {
        return available;
    }

    let latest = fetch_latest_version(NOTIFY_TIMEOUT).ok()?;
    write_cached_version(&latest);
    update_available_from(current_version(), &latest)
        .ok()
        .flatten()
}

fn cache_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
    Some(base.join("amph").join("update-check"))
}

fn read_cached_version() -> Option<(u64, String)> {
    let text = fs::read_to_string(cache_path()?).ok()?;
    let mut lines = text.lines();
    let checked_at = lines.next()?.trim().parse::<u64>().ok()?;
    let version = lines.next()?.trim();
    if version.is_empty() {
        return None;
    }
    Some((checked_at, version.to_string()))
}

fn write_cached_version(version: &str) {
    let Some(path) = cache_path() else {
        return;
    };
    if let Some(parent) = path.parent()
        && fs::create_dir_all(parent).is_err()
    {
        return;
    }
    let _ = fs::write(path, format!("{}\n{}\n", now_secs(), version));
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn fetch_latest_version(timeout: Duration) -> Result<String> {
    let manifest = fetch_manifest(timeout)?;
    parse_package_version(&manifest)
}

fn fetch_manifest(timeout: Duration) -> Result<String> {
    let timeout_secs = timeout.as_secs().max(1).to_string();
    let connect_secs = (timeout.as_secs().max(1) / 2).max(1).to_string();
    let max_bytes = MAX_MANIFEST_BYTES.to_string();

    let output = Command::new("curl")
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
        .arg(concat!("amph/", env!("CARGO_PKG_VERSION")))
        .arg(MANIFEST_URL)
        .output()
        .context(
            "cannot run curl to check for updates; install curl or set AMPH_NO_UPDATE_CHECK=1",
        )?;

    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr);
        let detail = detail.trim();
        if detail.is_empty() {
            bail!("update check failed with {}", output.status);
        }
        bail!("update check failed with {}: {detail}", output.status);
    }
    if output.stdout.len() > MAX_MANIFEST_BYTES {
        bail!("update check response exceeded {MAX_MANIFEST_BYTES} bytes");
    }
    String::from_utf8(output.stdout).context("update check returned invalid UTF-8")
}

fn parse_package_version(manifest: &str) -> Result<String> {
    let mut in_package = false;
    for raw_line in manifest.lines() {
        let line = strip_toml_comment(raw_line).trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "version" {
            continue;
        }
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .or_else(|| {
                value
                    .strip_prefix('\'')
                    .and_then(|value| value.strip_suffix('\''))
            })
            .ok_or_else(|| anyhow!("package version in the update manifest is not a string"))?;
        if value.is_empty() {
            bail!("package version in the update manifest is empty");
        }
        return Ok(value.to_string());
    }
    bail!("could not find a [package] version in the update manifest")
}

fn strip_toml_comment(line: &str) -> &str {
    let mut quote = None;
    let mut escaped = false;
    for (index, character) in line.char_indices() {
        match quote {
            Some(current) => {
                if escaped {
                    escaped = false;
                } else if character == '\\' && current == '"' {
                    escaped = true;
                } else if character == current {
                    quote = None;
                }
            }
            None => {
                if character == '"' || character == '\'' {
                    quote = Some(character);
                } else if character == '#' {
                    return &line[..index];
                }
            }
        }
    }
    line
}

fn cargo_root_for_current_exe() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?.canonicalize().ok()?;
    let bin = executable.parent()?;
    if bin.file_name()? != "bin" {
        return None;
    }
    bin.parent().map(Path::to_path_buf)
}

fn permission_hint(root: Option<&Path>) -> String {
    match root {
        Some(root) => format!(
            "If Cargo reported a permission error, {} is not writable by the current user. \
             amph never runs sudo: use a writable user Cargo root or run the install with the \
             privileges that own that directory.",
            root.display()
        ),
        None => "If Cargo reported a permission error, the default Cargo install root is not \
                 writable by the current user. amph never runs sudo: configure a writable user \
                 Cargo root or install as the owner of that directory."
            .to_string(),
    }
}

fn update_available_from(current: &str, latest: &str) -> Result<Option<UpdateAvailable>> {
    let current_version = Version::parse(current)?;
    let latest_version = Version::parse(latest)?;
    if latest_version > current_version {
        Ok(Some(UpdateAvailable {
            current: current.to_string(),
            latest: latest.to_string(),
            branch: BRANCH,
            repository: REPOSITORY,
        }))
    } else {
        Ok(None)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Version {
    major: u64,
    minor: u64,
    patch: u64,
    pre: Vec<PreRelease>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PreRelease {
    Numeric(u64),
    Text(String),
}

impl Version {
    fn parse(input: &str) -> Result<Self> {
        let input = input.trim();
        let input = input.strip_prefix('v').unwrap_or(input);
        let input = input.split_once('+').map_or(input, |(version, _)| version);
        let (core, pre) = input
            .split_once('-')
            .map_or((input, None), |(core, pre)| (core, Some(pre)));

        let mut parts = core.split('.');
        let major = parse_core_number(parts.next(), input)?;
        let minor = parse_core_number(parts.next(), input)?;
        let patch = parse_core_number(parts.next(), input)?;
        if parts.next().is_some() {
            bail!("invalid version '{input}': expected major.minor.patch");
        }

        let pre = match pre {
            Some(pre) => pre
                .split('.')
                .map(|part| {
                    if part.is_empty() {
                        bail!("invalid version '{input}': empty pre-release identifier");
                    }
                    if part.bytes().all(|byte| byte.is_ascii_digit()) {
                        Ok(PreRelease::Numeric(part.parse::<u64>().with_context(
                            || format!("invalid version '{input}': numeric pre-release overflow"),
                        )?))
                    } else {
                        Ok(PreRelease::Text(part.to_string()))
                    }
                })
                .collect::<Result<Vec<_>>>()?,
            None => Vec::new(),
        };

        Ok(Self {
            major,
            minor,
            patch,
            pre,
        })
    }
}

fn parse_core_number(part: Option<&str>, input: &str) -> Result<u64> {
    let part =
        part.ok_or_else(|| anyhow!("invalid version '{input}': expected major.minor.patch"))?;
    part.parse::<u64>()
        .with_context(|| format!("invalid version '{input}': non-numeric core component"))
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        self.major
            .cmp(&other.major)
            .then_with(|| self.minor.cmp(&other.minor))
            .then_with(|| self.patch.cmp(&other.patch))
            .then_with(|| compare_pre_release(&self.pre, &other.pre))
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn compare_pre_release(left: &[PreRelease], right: &[PreRelease]) -> Ordering {
    match (left.is_empty(), right.is_empty()) {
        (true, true) => return Ordering::Equal,
        // A release without a pre-release suffix has higher precedence.
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        (false, false) => {}
    }

    for (left, right) in left.iter().zip(right) {
        let ordering = match (left, right) {
            (PreRelease::Numeric(left), PreRelease::Numeric(right)) => left.cmp(right),
            (PreRelease::Numeric(_), PreRelease::Text(_)) => Ordering::Less,
            (PreRelease::Text(_), PreRelease::Numeric(_)) => Ordering::Greater,
            (PreRelease::Text(left), PreRelease::Text(right)) => left.cmp(right),
        };
        if ordering != Ordering::Equal {
            return ordering;
        }
    }
    left.len().cmp(&right.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_version_is_read_from_the_package_section() {
        let manifest = r#"
[package]
name = "amph"
version = "0.3.0-beta" # release version
edition = "2024"

[dependencies]
serde = { version = "1", features = ["derive"] }
"#;
        assert_eq!(parse_package_version(manifest).unwrap(), "0.3.0-beta");
    }

    #[test]
    fn package_version_requires_a_package_section() {
        let manifest = "[dependencies]\nversion = \"9.9.9\"\n";
        assert!(parse_package_version(manifest).is_err());
    }

    #[test]
    fn version_ordering_follows_semver_precedence() {
        let ordered = [
            "0.2.0-beta",
            "0.2.0",
            "0.3.0-alpha",
            "0.3.0-alpha.1",
            "0.3.0-alpha.beta",
            "0.3.0-beta",
            "0.3.0-beta.2",
            "0.3.0-beta.11",
            "0.3.0-rc.1",
            "0.3.0",
            "1.0.0",
        ];
        for pair in ordered.windows(2) {
            let left = Version::parse(pair[0]).unwrap();
            let right = Version::parse(pair[1]).unwrap();
            assert!(left < right, "{} should be older than {}", pair[0], pair[1]);
        }
    }

    #[test]
    fn build_metadata_and_leading_v_do_not_affect_order() {
        assert_eq!(
            Version::parse("v0.3.0+build.1").unwrap(),
            Version::parse("0.3.0").unwrap()
        );
    }

    #[test]
    fn updates_are_only_reported_for_a_newer_version() {
        assert!(
            update_available_from("0.2.0-beta", "0.3.0-beta")
                .unwrap()
                .is_some()
        );
        assert!(
            update_available_from("0.3.0-beta", "0.3.0-beta")
                .unwrap()
                .is_none()
        );
        assert!(
            update_available_from("0.3.0", "0.3.0-beta")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn malformed_versions_are_rejected() {
        for value in ["", "1", "1.2", "1.2.3.4", "x.2.3", "1.2.3-"] {
            assert!(Version::parse(value).is_err(), "{value} should be invalid");
        }
    }
}
