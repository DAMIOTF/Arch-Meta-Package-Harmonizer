use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum InstallReason {
    Explicit,
    Dependency,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageRecord {
    pub name: String,
    pub version: String,
    pub description: Option<String>,
    pub installed_size_bytes: u64,
    pub install_date: Option<String>,
    pub install_reason: InstallReason,
    pub packager: Option<String>,
    pub groups: Vec<String>,
    pub depends_on: Vec<String>,
    pub required_by: Vec<String>,
    pub optional_for: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecentPackageAction {
    pub timestamp: String,
    pub action: String,
    pub name: String,
    pub version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemSnapshot {
    pub generated_at: String,
    pub packages: Vec<PackageRecord>,
    pub orphan_names: Vec<String>,
    pub recent_actions: Vec<RecentPackageAction>,
}

pub fn collect_snapshot() -> Result<SystemSnapshot> {
    let package_names = list_installed_package_names()?;
    let mut packages = Vec::with_capacity(package_names.len());

    for name in package_names {
        packages.push(query_package_info(&name)?);
    }

    let orphan_names = list_orphan_names().unwrap_or_default();
    let recent_actions = recent_package_actions(25).unwrap_or_default();

    Ok(SystemSnapshot {
        generated_at: current_timestamp(),
        packages,
        orphan_names,
        recent_actions,
    })
}

pub fn list_installed_package_names() -> Result<Vec<String>> {
    let output = run_pacman(&["-Qq"])?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

pub fn list_orphan_names() -> Result<Vec<String>> {
    let output = run_pacman(&["-Qdtq"])?;
    Ok(output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

pub fn list_installed_packages() -> Result<Vec<PackageRecord>> {
    let names = list_installed_package_names()?;
    let mut packages = Vec::with_capacity(names.len());

    for name in names {
        packages.push(query_package_info(&name)?);
    }

    Ok(packages)
}

pub fn query_package_info(name: &str) -> Result<PackageRecord> {
    let output = Command::new("pacman")
        .args(["-Qi", name])
        .output()
        .with_context(|| format!("Failed to query package info for {}", name))?;

    if !output.status.success() {
        bail!(
            "pacman -Qi {} failed with status {}: {}",
            name,
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(parse_package_info(&String::from_utf8_lossy(&output.stdout)))
}

pub fn recent_package_actions(limit: usize) -> Result<Vec<RecentPackageAction>> {
    let mut actions = Vec::new();

    for path in ["/var/log/pacman.log", "/var/log/pacman.log.1"] {
        let candidate = Path::new(path);
        if !candidate.exists() {
            continue;
        }

        let content = fs::read_to_string(candidate)
            .with_context(|| format!("Failed to read {}", candidate.display()))?;

        for line in content.lines().rev() {
            if let Some(action) = parse_pacman_log_line(line) {
                actions.push(action);
                if actions.len() >= limit {
                    return Ok(actions);
                }
            }
        }
    }

    Ok(actions)
}

pub fn pacman_cache_size_bytes() -> Option<u64> {
    let output = Command::new("du")
        .args(["-sb", "/var/cache/pacman/pkg"])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .split_whitespace()
        .next()
        .and_then(|value| value.parse::<u64>().ok())
}

fn run_pacman(args: &[&str]) -> Result<String> {
    let output = Command::new("pacman")
        .args(args)
        .output()
        .with_context(|| format!("Failed to execute pacman {:?}", args))?;

    if !output.status.success() {
        bail!(
            "pacman {:?} failed with status {}: {}",
            args,
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }

    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn parse_package_info(raw: &str) -> PackageRecord {
    let fields = parse_keyed_fields(raw);
    let name = fields.get("Name").cloned().unwrap_or_default();
    let version = fields.get("Version").cloned().unwrap_or_default();

    PackageRecord {
        name,
        version,
        description: fields.get("Description").cloned().filter(|value| !value.is_empty()),
        installed_size_bytes: fields
            .get("Installed Size")
            .map(|value| parse_size_to_bytes(value))
            .unwrap_or(0),
        install_reason: fields
            .get("Install Reason")
            .map(|value| parse_install_reason(value))
            .unwrap_or(InstallReason::Unknown),
        install_date: fields.get("Install Date").cloned().filter(|value| !value.is_empty()),
        packager: fields.get("Packager").cloned().filter(|value| !value.is_empty()),
        groups: split_tokens(fields.get("Groups").map(String::as_str).unwrap_or("")),
        depends_on: split_dependency_list(fields.get("Depends On").map(String::as_str).unwrap_or("")),
        required_by: split_dependency_list(fields.get("Required By").map(String::as_str).unwrap_or("")),
        optional_for: split_dependency_list(fields.get("Optional For").map(String::as_str).unwrap_or("")),
    }
}

fn parse_keyed_fields(raw: &str) -> HashMap<String, String> {
    let mut fields = HashMap::new();
    let mut current_key = String::new();

    for line in raw.lines() {
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim().to_string();
            let value = value.trim().to_string();
            current_key = key.clone();
            fields.insert(key, value);
        } else if !current_key.is_empty() {
            let entry = fields.entry(current_key.clone()).or_default();
            if !entry.is_empty() {
                entry.push(' ');
            }
            entry.push_str(line.trim());
        }
    }

    fields
}

fn split_tokens(raw: &str) -> Vec<String> {
    if raw.is_empty() || raw.eq_ignore_ascii_case("none") {
        return Vec::new();
    }

    raw.split_whitespace()
        .map(normalize_package_token)
        .filter(|token| !token.is_empty() && token != "None")
        .collect()
}

fn split_dependency_list(raw: &str) -> Vec<String> {
    if raw.is_empty() || raw.eq_ignore_ascii_case("none") {
        return Vec::new();
    }

    raw.split_whitespace()
        .map(normalize_dependency_token)
        .filter(|token| !token.is_empty() && token != "None")
        .collect()
}

pub fn normalize_dependency_token(token: &str) -> String {
    normalize_package_token(token)
}

fn normalize_package_token(token: &str) -> String {
    token
        .split(|c: char| matches!(c, '<' | '>' | '='))
        .next()
        .unwrap_or(token)
        .trim_matches(|c: char| c == ',' || c == ':' || c.is_whitespace())
        .to_string()
}

fn parse_install_reason(raw: &str) -> InstallReason {
    let lower = raw.to_lowercase();
    if lower.contains("dependency") {
        InstallReason::Dependency
    } else if lower.contains("explicit") {
        InstallReason::Explicit
    } else {
        InstallReason::Unknown
    }
}

fn parse_size_to_bytes(raw: &str) -> u64 {
    let mut parts = raw.split_whitespace();
    let value = parts
        .next()
        .map(|part| part.replace(',', "."))
        .and_then(|part| part.parse::<f64>().ok())
        .unwrap_or(0.0);
    let unit = parts.next().unwrap_or("B").to_ascii_lowercase();
    let multiplier = match unit.as_str() {
        "b" => 1.0,
        "kib" | "kb" => 1024.0,
        "mib" | "mb" => 1024.0 * 1024.0,
        "gib" | "gb" => 1024.0 * 1024.0 * 1024.0,
        "tib" | "tb" => 1024.0_f64.powi(4),
        _ => 1.0,
    };

    (value * multiplier).round().max(0.0) as u64
}

fn parse_pacman_log_line(line: &str) -> Option<RecentPackageAction> {
    let (timestamp_part, rest) = line.split_once("] [ALPM] ")?;
    let timestamp = timestamp_part.trim_start_matches('[').to_string();

    for verb in ["installed", "upgraded", "downgraded", "reinstalled"] {
        let prefix = format!("{} ", verb);
        if let Some(package_part) = rest.strip_prefix(&prefix) {
            let name = package_part.split_whitespace().next()?.to_string();
            let version = package_part
                .split_once('(')
                .and_then(|(_, tail)| tail.split_once(')'))
                .map(|(value, _)| value.trim().to_string())
                .filter(|value| !value.is_empty());

            return Some(RecentPackageAction {
                timestamp,
                action: verb.to_string(),
                name,
                version,
            });
        }
    }

    None
}

fn current_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};

    match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_secs().to_string(),
        Err(_) => String::from("0"),
    }
}