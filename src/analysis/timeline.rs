//! Pacman log timeline parsing by transaction.
//!
//! Supports the current `pacman.log`, numeric rotations (`.1`, `.2`, ...), and
//! gzip/zstd-compressed rotations. Transactions are returned oldest first.

use std::collections::HashSet;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use flate2::read::GzDecoder;

/// Package action recorded inside a transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PackageAction {
    Install,
    Upgrade,
    Downgrade,
    Reinstall,
    Remove,
}

impl PackageAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Install => "install",
            Self::Upgrade => "upgrade",
            Self::Downgrade => "downgrade",
            Self::Reinstall => "reinstall",
            Self::Remove => "remove",
        }
    }
}

/// One package change in a transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogPackageAction {
    pub name: String,
    pub action: PackageAction,
    pub old_version: Option<String>,
    pub new_version: Option<String>,
}

/// One pacman transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PacmanTransaction {
    pub started_at: u64,
    pub finished_at: Option<u64>,
    pub command: Option<String>,
    pub actions: Vec<LogPackageAction>,
}

impl PacmanTransaction {
    pub fn package_names(&self) -> Vec<&str> {
        self.actions
            .iter()
            .map(|action| action.name.as_str())
            .collect()
    }

    pub fn duration_seconds(&self) -> Option<u64> {
        self.finished_at
            .map(|finished| finished.saturating_sub(self.started_at))
    }
}

/// Default pacman log path.
pub fn default_log_path() -> PathBuf {
    PathBuf::from("/var/log/pacman.log")
}

/// Parse a pacman log text into transactions.
pub fn parse_pacman_log(text: &str) -> Vec<PacmanTransaction> {
    let mut transactions = Vec::new();
    let mut current: Option<PacmanTransaction> = None;
    let mut pending_command: Option<String> = None;

    for raw_line in text.lines() {
        let Some((timestamp, rest)) = split_log_line(raw_line) else {
            continue;
        };
        let rest = rest.trim();

        if rest.starts_with("[PACMAN] Running '") || rest.starts_with("[PACMAN] Running \"") {
            let command = rest
                .split_once("Running '")
                .map(|(_, command)| command)
                .or_else(|| rest.split_once("Running \"").map(|(_, command)| command))
                .map(|command| command.trim_end_matches(['\'', '"']).to_string());
            pending_command = command;
            if current
                .as_ref()
                .is_some_and(|transaction| !transaction.actions.is_empty())
            {
                let finished = current.take();
                if let Some(mut transaction) = finished {
                    transaction.finished_at = Some(timestamp);
                    transactions.push(transaction);
                }
            }
            continue;
        }

        if rest.contains("[ALPM] transaction started") {
            if let Some(mut transaction) = current.take()
                && !transaction.actions.is_empty()
            {
                transaction.finished_at = Some(timestamp);
                transactions.push(transaction);
            }
            current = Some(PacmanTransaction {
                started_at: timestamp,
                finished_at: None,
                command: pending_command.take(),
                actions: Vec::new(),
            });
            continue;
        }

        if rest.contains("[ALPM] transaction completed") {
            if let Some(mut transaction) = current.take() {
                transaction.finished_at = Some(timestamp);
                transactions.push(transaction);
            }
            continue;
        }

        let Some(action) = parse_alpm_action(rest) else {
            continue;
        };

        if current.is_none() {
            current = Some(PacmanTransaction {
                started_at: timestamp,
                finished_at: None,
                command: pending_command.take(),
                actions: Vec::new(),
            });
        }
        if let Some(transaction) = current.as_mut() {
            transaction.actions.push(action);
        }
    }

    if let Some(transaction) = current
        && !transaction.actions.is_empty()
    {
        transactions.push(transaction);
    }

    transactions
}

/// Load the current log and its numeric rotations.
pub fn load_timeline(path: &Path) -> Result<Vec<PacmanTransaction>> {
    let mut rotations: Vec<(usize, PathBuf)> = Vec::new();

    let mut rotation = 1usize;
    let mut consecutive_missing = 0usize;
    while rotation <= 50 && consecutive_missing < 2 {
        let candidates = [
            PathBuf::from(format!("{}.{rotation}", path.display())),
            PathBuf::from(format!("{}.{rotation}.gz", path.display())),
            PathBuf::from(format!("{}.{rotation}.zst", path.display())),
        ];
        let existing: Vec<PathBuf> = candidates
            .into_iter()
            .filter(|candidate| candidate.is_file())
            .collect();
        if !existing.is_empty() {
            rotations.extend(existing.into_iter().map(|candidate| (rotation, candidate)));
            consecutive_missing = 0;
        } else {
            consecutive_missing += 1;
        }
        rotation += 1;
    }

    if !path.is_file() && rotations.is_empty() {
        return Ok(Vec::new());
    }

    // Oldest first: highest rotation number, then the current file.
    rotations.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    let mut candidates: Vec<PathBuf> = rotations
        .into_iter()
        .map(|(_, candidate)| candidate)
        .collect();
    if path.is_file() {
        candidates.push(path.to_path_buf());
    }

    let mut transactions = Vec::new();
    for candidate in candidates {
        let text = read_log_file(&candidate)
            .with_context(|| format!("cannot read pacman log {}", candidate.display()))?;
        transactions.extend(parse_pacman_log(&text));
    }

    let mut seen = HashSet::new();
    transactions.retain(|transaction| {
        let key = transaction_key(transaction);
        seen.insert(key)
    });
    transactions.sort_by(|a, b| {
        a.started_at
            .cmp(&b.started_at)
            .then_with(|| a.command.cmp(&b.command))
    });
    Ok(transactions)
}

fn read_log_file(path: &Path) -> Result<String> {
    let bytes = fs::read(path)?;
    let decoded = if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut decoded = Vec::new();
        GzDecoder::new(bytes.as_slice()).read_to_end(&mut decoded)?;
        decoded
    } else if bytes.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        zstd::stream::decode_all(bytes.as_slice())?
    } else {
        bytes
    };
    String::from_utf8(decoded)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error).into())
}

fn transaction_key(transaction: &PacmanTransaction) -> String {
    let first = transaction
        .actions
        .first()
        .map(|action| format!("{}:{}", action.name, action.action.as_str()))
        .unwrap_or_default();
    format!(
        "{}:{}:{}:{}:{}",
        transaction.started_at,
        transaction.finished_at.unwrap_or(0),
        transaction.command.as_deref().unwrap_or(""),
        transaction.actions.len(),
        first
    )
}

fn split_log_line(line: &str) -> Option<(u64, &str)> {
    let line = line.trim_start();
    let rest = line.strip_prefix('[')?;
    let end = rest.find(']')?;
    let timestamp = parse_log_timestamp(&rest[..end])?;
    Some((timestamp, &rest[end + 1..]))
}

fn parse_alpm_action(rest: &str) -> Option<LogPackageAction> {
    let payload = rest.split_once("[ALPM]")?.1.trim();

    for (prefix, action) in [
        ("installed ", PackageAction::Install),
        ("upgraded ", PackageAction::Upgrade),
        ("downgraded ", PackageAction::Downgrade),
        ("reinstalled ", PackageAction::Reinstall),
        ("removed ", PackageAction::Remove),
    ] {
        if let Some(body) = payload.strip_prefix(prefix) {
            return parse_action_body(body, action);
        }
    }
    None
}

fn parse_action_body(body: &str, action: PackageAction) -> Option<LogPackageAction> {
    let open = body.rfind('(')?;
    let close = body.rfind(')')?;
    if close <= open {
        return None;
    }
    let name = body[..open].trim();
    if name.is_empty() {
        return None;
    }
    let versions = body[open + 1..close].trim();

    let (old_version, new_version) = if let Some((old, new)) = versions.split_once(" -> ") {
        (Some(old.trim().to_string()), Some(new.trim().to_string()))
    } else if matches!(action, PackageAction::Remove) {
        (Some(versions.to_string()), None)
    } else {
        (None, Some(versions.to_string()))
    };

    Some(LogPackageAction {
        name: name.to_string(),
        action,
        old_version,
        new_version,
    })
}

/// Parse `YYYY-MM-DDTHH:MM:SS+ZZZZ` (or `Z`) to Unix seconds.
pub fn parse_log_timestamp(input: &str) -> Option<u64> {
    let input = input.trim();
    let datetime = input
        .strip_suffix('Z')
        .map(|value| (value, 0i64))
        .or_else(|| parse_offset(input))?;
    let (datetime, offset_seconds) = datetime;

    let (date, time) = datetime.split_once('T')?;
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: u32 = date_parts.next()?.parse().ok()?;
    let day: u32 = date_parts.next()?.parse().ok()?;
    if date_parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    let mut time_parts = time.split(':');
    let hour: i64 = time_parts.next()?.parse().ok()?;
    let minute: i64 = time_parts.next()?.parse().ok()?;
    let second: i64 = time_parts.next()?.parse().ok()?;
    if time_parts.next().is_some() || hour > 23 || minute > 59 || second > 60 {
        return None;
    }

    let days = days_from_civil(year, month, day);
    let seconds = days
        .checked_mul(86_400)?
        .checked_add(hour * 3_600 + minute * 60 + second)?
        .checked_sub(offset_seconds)?;
    u64::try_from(seconds).ok()
}

fn parse_offset(input: &str) -> Option<(&str, i64)> {
    // Find the timezone sign after the time component.
    let time_start = input.find('T')?;
    let sign_offset = input[time_start..]
        .char_indices()
        .find(|(_, character)| *character == '+' || *character == '-')
        .map(|(index, _)| time_start + index)?;
    let sign = if input.as_bytes().get(sign_offset) == Some(&b'-') {
        -1i64
    } else {
        1i64
    };
    let offset = &input[sign_offset + 1..];
    if offset.len() != 4 && offset.len() != 6 {
        return None;
    }
    let hours: i64 = offset.get(0..2)?.parse().ok()?;
    let minutes: i64 = offset.get(2..4)?.parse().ok()?;
    let seconds: i64 = if offset.len() == 6 {
        offset.get(4..6)?.parse().ok()?
    } else {
        0
    };
    Some((
        &input[..sign_offset],
        sign * (hours * 3_600 + minutes * 60 + seconds),
    ))
}

/// Howard Hinnant's days-from-civil algorithm.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month = month as i64;
    let day = day as i64;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const LOG: &str = r#"
[2024-01-01T10:00:00+0000] [PACMAN] Running 'pacman -Syu'
[2024-01-01T10:00:01+0000] [ALPM] transaction started
[2024-01-01T10:00:02+0000] [ALPM] upgraded linux (6.6.1-1 -> 6.7.0-1)
[2024-01-01T10:00:03+0000] [ALPM] installed new-tool (1.0-1)
[2024-01-01T10:00:04+0000] [ALPM] transaction completed
[2024-01-01T10:05:00+0000] [PACMAN] Running 'pacman -Rns old'
[2024-01-01T10:05:01+0000] [ALPM] transaction started
[2024-01-01T10:05:02+0000] [ALPM] removed old (2.0-1)
[2024-01-01T10:05:03+0000] [ALPM] transaction completed
[2024-01-01T10:05:04+0000] [ALPM] warning: something unrelated
"#;

    #[test]
    fn parses_transactions_and_actions() {
        let transactions = parse_pacman_log(LOG);
        assert_eq!(transactions.len(), 2);

        let first = &transactions[0];
        assert_eq!(first.command.as_deref(), Some("pacman -Syu"));
        assert_eq!(first.actions.len(), 2);
        assert_eq!(first.actions[0].name, "linux");
        assert_eq!(first.actions[0].action, PackageAction::Upgrade);
        assert_eq!(first.actions[0].old_version.as_deref(), Some("6.6.1-1"));
        assert_eq!(first.actions[0].new_version.as_deref(), Some("6.7.0-1"));
        assert_eq!(first.duration_seconds(), Some(3));

        let second = &transactions[1];
        assert_eq!(second.actions[0].action, PackageAction::Remove);
        assert_eq!(second.actions[0].old_version.as_deref(), Some("2.0-1"));
    }

    #[test]
    fn parses_iso_timestamp_with_timezone() {
        let timestamp = parse_log_timestamp("2024-01-01T10:00:00+0000").expect("timestamp");
        assert_eq!(timestamp, 1_704_103_200);
        assert_eq!(
            parse_log_timestamp("2024-01-01T11:00:00+0100"),
            Some(timestamp)
        );
    }

    #[test]
    fn rejects_malformed_timestamps_without_panicking() {
        assert!(parse_log_timestamp("").is_none());
        assert!(parse_log_timestamp("not-a-date").is_none());
        assert!(parse_log_timestamp("2024-13-01T10:00:00+0000").is_none());
    }

    #[test]
    fn unfinished_transaction_is_kept() {
        let text = "[2024-01-01T10:00:00+0000] [ALPM] transaction started\n\
                    [2024-01-01T10:00:01+0000] [ALPM] installed dangling (1.0-1)\n";
        let transactions = parse_pacman_log(text);
        assert_eq!(transactions.len(), 1);
        assert_eq!(transactions[0].finished_at, None);
        assert_eq!(transactions[0].actions[0].name, "dangling");
    }

    #[test]
    fn load_timeline_merges_rotations_oldest_first() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|value| value.as_nanos())
            .unwrap_or(0);
        let base = std::env::temp_dir().join(format!(
            "amph-log-{}-{unique}/pacman.log",
            std::process::id()
        ));
        if let Some(parent) = base.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let rotated = PathBuf::from(format!("{}.1.gz", base.display()));
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder
            .write_all(
                b"[2023-12-31T10:00:00+0000] [ALPM] transaction started\n\
                  [2023-12-31T10:00:01+0000] [ALPM] installed old (1.0-1)\n\
                  [2023-12-31T10:00:02+0000] [ALPM] transaction completed\n",
            )
            .expect("write compressed rotated log");
        fs::write(&rotated, encoder.finish().expect("finish gzip log")).expect("rotated log");
        fs::write(&base, LOG).expect("current log");

        let transactions = load_timeline(&base).expect("timeline");
        assert_eq!(transactions.len(), 3);
        assert!(transactions[0].started_at < transactions[1].started_at);

        let _ = fs::remove_dir_all(base.parent().unwrap_or(Path::new("/tmp")));
    }
}
