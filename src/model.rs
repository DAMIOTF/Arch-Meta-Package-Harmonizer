//! Core indexed data model for the Arch Meta Package Harmonizer.
//!
//! The model deliberately contains no process spawning and no filesystem I/O
//! beyond the small helpers in [`Snapshot`].  Reading pacman databases lives in
//! [`crate::source`]; consumers can therefore build snapshots from fixtures or
//! from a remote machine without pulling in the scanner.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

/// One installed package as recorded by pacman's local database.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub description: String,
    /// Installed size in bytes (`%SIZE%`).
    pub size: u64,
    /// `%REASON% == "1"` means explicitly installed.
    pub explicit: bool,
    /// Raw `%REASON%` value, when present.
    pub reason: Option<String>,
    /// Hard dependencies, exactly as written in `/var/lib/pacman/local/*/desc`.
    pub depends: Vec<String>,
    /// Provided capabilities, including sonames and version suffixes.
    pub provides: Vec<String>,
    /// Optional dependencies. Optdepends are intentionally kept separate from
    /// hard dependencies everywhere in the graph.
    pub optdepends: Vec<String>,
    /// True when the package name is absent from every sync database.
    pub foreign: bool,
}

impl Package {
    /// Parse a single `desc` file.  The directory name is used as a fallback
    /// when `%NAME%` is missing, which makes old/partially written databases
    /// recoverable.
    pub fn from_desc(fallback_name: Option<&str>, text: &str) -> Result<Self> {
        parse_desc_with_name(text, fallback_name)
    }

    /// Parse a single `desc` file without a fallback name.
    pub fn parse_desc(text: &str) -> Result<Self> {
        parse_desc_with_name(text, None)
    }

    /// Normalised name used for dependency matching.
    pub fn normalized_name(&self) -> String {
        normalize_dependency_name(&self.name)
    }
}

/// A point-in-time view of the installed package set.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub packages: Vec<Package>,
    /// Unix timestamp (seconds) when the snapshot was produced.
    pub scanned_at: u64,
}

impl Snapshot {
    pub fn new(packages: Vec<Package>) -> Self {
        Self {
            packages,
            scanned_at: unix_now(),
        }
    }

    pub fn find(&self, name: &str) -> Option<&Package> {
        self.packages.iter().find(|p| p.name == name)
    }

    pub fn find_index(&self, name: &str) -> Option<usize> {
        self.packages.iter().position(|p| p.name == name)
    }

    pub fn total_size(&self) -> u64 {
        self.packages
            .iter()
            .fold(0u64, |sum, p| sum.saturating_add(p.size))
    }

    pub fn explicit_count(&self) -> usize {
        self.packages.iter().filter(|p| p.explicit).count()
    }

    pub fn foreign_count(&self) -> usize {
        self.packages.iter().filter(|p| p.foreign).count()
    }

    /// Sort by package name and remove duplicate names (last entry wins).
    pub fn normalize(&mut self) {
        self.packages.sort_by(|a, b| a.name.cmp(&b.name));
        let mut deduped: Vec<Package> = Vec::with_capacity(self.packages.len());
        for package in self.packages.drain(..) {
            match deduped.last_mut() {
                Some(previous) if previous.name == package.name => *previous = package,
                _ => deduped.push(package),
            }
        }
        self.packages = deduped;
    }
}

/// Parse `%SECTION%` style pacman description files.
///
/// Blank lines are ignored.  Lines after a header belong to that header until
/// the next header.  This is tolerant enough for malformed/truncated files and
/// never panics on unusual input.
pub fn parse_sections(text: &str) -> BTreeMap<String, Vec<String>> {
    let mut sections: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut current: Option<String> = None;

    for raw_line in text.lines() {
        let line = raw_line.trim_end_matches('\r');
        let trimmed = line.trim();

        if trimmed.len() >= 2 && trimmed.starts_with('%') && trimmed.ends_with('%') {
            let key = trimmed
                .trim_start_matches('%')
                .trim_end_matches('%')
                .trim()
                .to_ascii_uppercase();
            if !key.is_empty() {
                current = Some(key);
            }
            continue;
        }

        if let Some(key) = current.as_deref()
            && !trimmed.is_empty()
        {
            sections
                .entry(key.to_string())
                .or_default()
                .push(trimmed.to_string());
        }
    }

    sections
}

fn first_value<'a>(sections: &'a BTreeMap<String, Vec<String>>, key: &str) -> Option<&'a str> {
    sections
        .get(key)
        .and_then(|values| values.first())
        .map(String::as_str)
}

fn joined_value(sections: &BTreeMap<String, Vec<String>>, key: &str) -> Option<String> {
    let values = sections.get(key)?;
    if values.is_empty() {
        return None;
    }
    Some(values.join(" "))
}

/// Parse a desc file, optionally supplying the on-disk directory name.
pub fn parse_desc_with_name(text: &str, fallback_name: Option<&str>) -> Result<Package> {
    let sections = parse_sections(text);

    let name = first_value(&sections, "NAME")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| {
            fallback_name
                .map(str::trim)
                .filter(|value| !value.is_empty())
        })
        .ok_or_else(|| anyhow!("desc is missing %NAME% and no fallback name was supplied"))?
        .to_string();

    let version = first_value(&sections, "VERSION")
        .unwrap_or_default()
        .trim()
        .to_string();

    let description = joined_value(&sections, "DESC")
        .unwrap_or_default()
        .trim()
        .to_string();

    let size = first_value(&sections, "SIZE")
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(0);

    let reason = first_value(&sections, "REASON")
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToString::to_string);

    // Pacman stores 1 for explicitly installed packages and 0 for dependencies.
    // Old databases may omit REASON entirely; treat those as explicit to avoid
    // accidentally suggesting removal.
    let explicit = match reason.as_deref() {
        Some(value) => value != "0",
        None => true,
    };

    let depends = section_tokens(&sections, "DEPENDS");
    let provides = section_tokens(&sections, "PROVIDES");
    let optdepends = section_lines(&sections, "OPTDEPENDS");

    Ok(Package {
        name,
        version,
        description,
        size,
        explicit,
        reason,
        depends,
        provides,
        optdepends,
        foreign: false,
    })
}

fn section_tokens(sections: &BTreeMap<String, Vec<String>>, key: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(values) = sections.get(key) {
        for value in values {
            for token in value.split_whitespace() {
                if !token.is_empty() {
                    out.push(token.to_string());
                }
            }
        }
    }
    out
}

fn section_lines(sections: &BTreeMap<String, Vec<String>>, key: &str) -> Vec<String> {
    sections
        .get(key)
        .map(|values| {
            values
                .iter()
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// Split a dependency spec into `(name, optional constraint)`.
///
/// Examples:
/// * `bar>=2.0` -> `("bar", Some(">=2.0"))`
/// * `libfoo.so=1-64` -> `("libfoo.so", Some("=1-64"))`
/// * `extra: optional feature` -> `("extra", None)`
pub fn split_dependency(spec: &str) -> (&str, Option<&str>) {
    let spec = spec.trim();
    let description_cut = spec.find(':').unwrap_or(spec.len());
    let head = &spec[..description_cut];
    let operator = head.find(['<', '>', '=']);

    match operator {
        Some(index) => (&head[..index], Some(&head[index..])),
        None => (head, None),
    }
}

/// The package/capability name part of a dependency or provides entry.
pub fn dependency_name(spec: &str) -> &str {
    split_dependency(spec).0.trim()
}

/// Normalised package/capability name.
///
/// Version suffixes are stripped, so `libfoo.so=1-64`, `libfoo.so>=1` and a
/// plain `libfoo.so` provide all map to the same index key.
pub fn normalize_dependency_name(spec: &str) -> String {
    dependency_name(spec).to_string()
}

/// The version constraint part of a dependency, if any (operator included).
pub fn version_constraint(spec: &str) -> Option<&str> {
    split_dependency(spec).1
}

/// The version offered by a provides entry (`foo=1.2-1` -> `1.2-1`).
pub fn provided_version(spec: &str) -> Option<&str> {
    let constraint = split_dependency(spec).1?;
    let version = constraint.trim_start_matches(['<', '>', '=']).trim();
    if version.is_empty() {
        None
    } else {
        Some(version)
    }
}

/// Return true when `version` satisfies `constraint` such as `>=1.2`.
pub fn version_satisfies(version: &str, constraint: &str) -> bool {
    let Some(constraint) = version_constraint(constraint) else {
        return true;
    };
    let operator_len = constraint
        .chars()
        .take_while(|c| matches!(c, '<' | '>' | '='))
        .count();
    let (operator, required) = constraint.split_at(operator_len);
    let required = required.trim();
    if required.is_empty() {
        return true;
    }

    let ordering = version_cmp(version, required);
    match operator {
        "=" | "==" => ordering == Ordering::Equal,
        ">" => ordering == Ordering::Greater,
        ">=" => ordering != Ordering::Less,
        "<" => ordering == Ordering::Less,
        "<=" => ordering != Ordering::Greater,
        _ => true,
    }
}

/// Compare two pacman-style versions.
///
/// This is a compact rpmvercmp-style comparison: numeric runs compare
/// numerically, alphabetic runs compare lexicographically and separators are
/// ignored.  It is intentionally dependency-free and deterministic.
pub fn version_cmp(left: &str, right: &str) -> Ordering {
    let left = left.trim();
    let right = right.trim();
    if left == right {
        return Ordering::Equal;
    }

    let left_chars: Vec<char> = left.chars().collect();
    let right_chars: Vec<char> = right.chars().collect();
    let mut li = 0usize;
    let mut ri = 0usize;

    while li < left_chars.len() || ri < right_chars.len() {
        while li < left_chars.len() && !left_chars[li].is_alphanumeric() {
            li += 1;
        }
        while ri < right_chars.len() && !right_chars[ri].is_alphanumeric() {
            ri += 1;
        }

        if li >= left_chars.len() || ri >= right_chars.len() {
            break;
        }

        let left_numeric = left_chars[li].is_ascii_digit();
        let right_numeric = right_chars[ri].is_ascii_digit();

        if left_numeric != right_numeric {
            // Numeric segments traditionally sort newer than alphabetic ones.
            return if left_numeric {
                Ordering::Greater
            } else {
                Ordering::Less
            };
        }

        let mut left_end = li;
        let mut right_end = ri;
        if left_numeric {
            while left_end < left_chars.len() && left_chars[left_end].is_ascii_digit() {
                left_end += 1;
            }
            while right_end < right_chars.len() && right_chars[right_end].is_ascii_digit() {
                right_end += 1;
            }
        } else {
            while left_end < left_chars.len() && left_chars[left_end].is_ascii_alphabetic() {
                left_end += 1;
            }
            while right_end < right_chars.len() && right_chars[right_end].is_ascii_alphabetic() {
                right_end += 1;
            }
        }

        let left_segment: String = left_chars[li..left_end].iter().collect();
        let right_segment: String = right_chars[ri..right_end].iter().collect();

        let ordering = if left_numeric {
            let left_trimmed = left_segment.trim_start_matches('0');
            let right_trimmed = right_segment.trim_start_matches('0');
            match left_trimmed.len().cmp(&right_trimmed.len()) {
                Ordering::Equal => left_trimmed.cmp(right_trimmed),
                other => other,
            }
        } else {
            left_segment.cmp(&right_segment)
        };

        if ordering != Ordering::Equal {
            return ordering;
        }

        li = left_end;
        ri = right_end;
    }

    // Any remaining alphanumeric content makes the longer version newer.
    let left_rest = left_chars
        .get(li..)
        .map(|slice| slice.iter().any(|c| c.is_alphanumeric()))
        .unwrap_or(false);
    let right_rest = right_chars
        .get(ri..)
        .map(|slice| slice.iter().any(|c| c.is_alphanumeric()))
        .unwrap_or(false);

    match (left_rest, right_rest) {
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        _ => Ordering::Equal,
    }
}

/// Current Unix time in seconds.  Falls back to 0 on platforms with a clock
/// before the Unix epoch instead of panicking.
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const APP_DESC: &str = include_str!("../tests/fixtures/desc/app/desc");
    const LIBFOO_DESC: &str = include_str!("../tests/fixtures/desc/libfoo/desc");

    #[test]
    fn parses_desc_fixture_with_all_fields() {
        let package = Package::from_desc(Some("app"), APP_DESC).expect("app desc should parse");

        assert_eq!(package.name, "app");
        assert_eq!(package.version, "1.0-1");
        assert_eq!(package.size, 2_048);
        assert!(package.explicit);
        assert_eq!(package.reason.as_deref(), Some("1"));
        assert_eq!(package.depends, vec!["libfoo.so=1-64", "bar>=2.0", "tool"]);
        assert_eq!(package.optdepends, vec!["extra: optional features"]);
        assert!(!package.foreign);
    }

    #[test]
    fn parses_desc_with_missing_and_invalid_fields() {
        let text = "%NAME%\nbroken\n%VERSION%\n\n%SIZE%\nnot-a-number\n%DEPENDS%\n";
        let package = Package::parse_desc(text).expect("broken but usable desc");

        assert_eq!(package.name, "broken");
        assert_eq!(package.version, "");
        assert_eq!(package.size, 0);
        assert!(package.explicit);
        assert!(package.depends.is_empty());
    }

    #[test]
    fn falls_back_to_directory_name() {
        let text = "%VERSION%\n9.9-1\n";
        let package = Package::from_desc(Some("fallback-pkg"), text).expect("fallback name");
        assert_eq!(package.name, "fallback-pkg");
    }

    #[test]
    fn missing_name_is_an_error_not_a_panic() {
        let result = Package::parse_desc("%VERSION%\n1.0\n");
        assert!(result.is_err());
    }

    #[test]
    fn normalises_soname_and_provider_versions() {
        assert_eq!(normalize_dependency_name("libfoo.so=1-64"), "libfoo.so");
        assert_eq!(normalize_dependency_name("libfoo.so>=1"), "libfoo.so");
        assert_eq!(normalize_dependency_name("extra: description"), "extra");
        assert_eq!(provided_version("libfoo.so=1-64"), Some("1-64"));
        assert_eq!(version_constraint("foo>=2.0"), Some(">=2.0"));
    }

    #[test]
    fn compares_versions_and_constraints() {
        assert_eq!(version_cmp("1.10-1", "1.9-2"), Ordering::Greater);
        assert_eq!(version_cmp("2.0-1", "2.0-1"), Ordering::Equal);
        assert!(version_satisfies("2.1-1", ">=2.0"));
        assert!(!version_satisfies("1.9-1", ">=2.0"));
        assert!(version_satisfies("1-64", "=1-64"));
    }

    #[test]
    fn libfoo_fixture_provides_a_soname() {
        let package =
            Package::from_desc(Some("libfoo"), LIBFOO_DESC).expect("libfoo desc should parse");
        assert!(
            package
                .provides
                .iter()
                .any(|provide| normalize_dependency_name(provide) == "libfoo.so")
        );
    }

    #[test]
    fn snapshot_helpers_are_saturating() {
        let mut snapshot = Snapshot::new(vec![
            Package {
                name: "a".into(),
                size: u64::MAX - 1,
                explicit: true,
                ..Package::default()
            },
            Package {
                name: "b".into(),
                size: 10,
                ..Package::default()
            },
        ]);
        snapshot.normalize();
        assert_eq!(snapshot.total_size(), u64::MAX);
        assert_eq!(snapshot.explicit_count(), 1);
    }
}
