//! Configuration, a dependency-free TOML subset reader/writer, and defaults.
//!
//! The subset covers everything the application writes: tables, arrays of
//! tables, strings (basic and literal), booleans, integers, floats and arrays.
//! It is intentionally forgiving about unknown keys so Harmony categories can
//! grow without a code change.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};

/// A parsed TOML value.
#[derive(Debug, Clone, PartialEq)]
pub enum TomlValue {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Array(Vec<TomlValue>),
    Table(TomlTable),
}

impl TomlValue {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::Integer(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Float(value) => Some(*value),
            Self::Integer(value) => Some(*value as f64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Boolean(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[TomlValue]> {
        match self {
            Self::Array(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_table(&self) -> Option<&TomlTable> {
        match self {
            Self::Table(value) => Some(value),
            _ => None,
        }
    }

    pub fn string_list(&self) -> Vec<String> {
        match self {
            Self::String(value) => vec![value.clone()],
            Self::Array(values) => values
                .iter()
                .filter_map(|value| value.as_str().map(ToString::to_string))
                .collect(),
            _ => Vec::new(),
        }
    }
}

/// A TOML table, ordered for deterministic output.
pub type TomlTable = BTreeMap<String, TomlValue>;

#[derive(Debug, Clone)]
enum SectionTarget {
    Root,
    Table(Vec<String>),
    ArrayItem { path: Vec<String>, index: usize },
}

/// Parse the supported TOML subset.
pub fn parse_toml(input: &str) -> Result<TomlTable> {
    let mut root = TomlTable::new();
    let mut target = SectionTarget::Root;
    let mut pending: Option<(String, String)> = None;

    for (line_number, raw) in input.lines().enumerate() {
        if let Some((key, mut value)) = pending.take() {
            value.push(' ');
            value.push_str(strip_comment(raw).trim());
            if brackets_balanced(&value) {
                let parsed =
                    parse_value(&value).with_context(|| format!("line {}", line_number + 1))?;
                insert_value(&mut root, &target, &key, parsed)?;
            } else {
                pending = Some((key, value));
            }
            continue;
        }

        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }

        if let Some(header) = line
            .strip_prefix("[[")
            .and_then(|rest| rest.strip_suffix("]]"))
        {
            let path =
                parse_key_path(header).with_context(|| format!("line {}", line_number + 1))?;
            target = start_array_table(&mut root, &path)
                .with_context(|| format!("line {}", line_number + 1))?;
            continue;
        }

        if let Some(header) = line
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            let path =
                parse_key_path(header).with_context(|| format!("line {}", line_number + 1))?;
            ensure_table(&mut root, &path).with_context(|| format!("line {}", line_number + 1))?;
            target = SectionTarget::Table(path);
            continue;
        }

        let (key, value) =
            split_key_value(line).with_context(|| format!("line {}", line_number + 1))?;
        let value = value.trim();
        if value.starts_with('[') && !brackets_balanced(value) {
            pending = Some((key, value.to_string()));
            continue;
        }

        let parsed = parse_value(value).with_context(|| format!("line {}", line_number + 1))?;
        insert_value(&mut root, &target, &key, parsed)
            .with_context(|| format!("line {}", line_number + 1))?;
    }

    if pending.is_some() {
        return Err(anyhow!("unterminated array at end of TOML input"));
    }

    Ok(root)
}

fn strip_comment(line: &str) -> &str {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (index, character) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quote == Some('"') {
            escaped = true;
            continue;
        }
        match quote {
            Some(active) if character == active => quote = None,
            Some(_) => {}
            None if character == '"' || character == '\'' => quote = Some(character),
            None if character == '#' => return &line[..index],
            None => {}
        }
    }
    line
}

fn split_key_value(line: &str) -> Result<(String, String)> {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (index, character) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quote == Some('"') {
            escaped = true;
            continue;
        }
        match quote {
            Some(active) if character == active => quote = None,
            Some(_) => {}
            None if character == '"' || character == '\'' => quote = Some(character),
            None if character == '=' => {
                let key = line[..index].trim();
                let value = line[index + 1..].trim();
                if key.is_empty() {
                    return Err(anyhow!("empty TOML key"));
                }
                return Ok((key.to_string(), value.to_string()));
            }
            None => {}
        }
    }
    Err(anyhow!("expected key = value"))
}

fn brackets_balanced(value: &str) -> bool {
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for character in value.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quote == Some('"') {
            escaped = true;
            continue;
        }
        match quote {
            Some(active) if character == active => quote = None,
            Some(_) => {}
            None if character == '"' || character == '\'' => quote = Some(character),
            None => match character {
                '[' => depth += 1,
                ']' => depth -= 1,
                _ => {}
            },
        }
    }
    depth <= 0
}

fn parse_key_path(input: &str) -> Result<Vec<String>> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut quote: Option<char> = None;
    let mut escaped = false;

    for character in input.trim().chars() {
        if escaped {
            current.push(character);
            escaped = false;
            continue;
        }
        if character == '\\' && quote == Some('"') {
            escaped = true;
            continue;
        }
        match quote {
            Some(active) if character == active => quote = None,
            Some(_) => current.push(character),
            None if character == '"' || character == '\'' => quote = Some(character),
            None if character == '.' => {
                let part = current.trim().to_string();
                if part.is_empty() {
                    return Err(anyhow!("empty key segment in '{input}'"));
                }
                parts.push(part);
                current.clear();
            }
            None => current.push(character),
        }
    }

    let part = current.trim().to_string();
    if !part.is_empty() {
        parts.push(part);
    }
    if parts.is_empty() {
        return Err(anyhow!("empty table path"));
    }
    Ok(parts)
}

fn ensure_table<'a>(root: &'a mut TomlTable, path: &[String]) -> Result<&'a mut TomlTable> {
    let mut current = root;
    for component in path {
        let entry = current
            .entry(component.clone())
            .or_insert_with(|| TomlValue::Table(TomlTable::new()));
        current = match entry {
            TomlValue::Table(table) => table,
            _ => return Err(anyhow!("'{component}' is not a table")),
        };
    }
    Ok(current)
}

fn start_array_table(root: &mut TomlTable, path: &[String]) -> Result<SectionTarget> {
    if path.is_empty() {
        return Err(anyhow!("empty array table path"));
    }
    let (parents, last) = path.split_at(path.len() - 1);
    let parent = ensure_table(root, parents)?;
    let entry = parent
        .entry(last[0].clone())
        .or_insert_with(|| TomlValue::Array(Vec::new()));
    let array = match entry {
        TomlValue::Array(array) => array,
        _ => return Err(anyhow!("'{}' is not an array of tables", last[0])),
    };
    array.push(TomlValue::Table(TomlTable::new()));
    Ok(SectionTarget::ArrayItem {
        path: path.to_vec(),
        index: array.len() - 1,
    })
}

fn insert_value(
    root: &mut TomlTable,
    target: &SectionTarget,
    key: &str,
    value: TomlValue,
) -> Result<()> {
    let table = match target {
        SectionTarget::Root => root,
        SectionTarget::Table(path) => ensure_table(root, path)?,
        SectionTarget::ArrayItem { path, index } => {
            let (parents, last) = path.split_at(path.len() - 1);
            let parent = ensure_table(root, parents)?;
            let entry = parent
                .get_mut(&last[0])
                .ok_or_else(|| anyhow!("missing array table"))?;
            let array = match entry {
                TomlValue::Array(array) => array,
                _ => return Err(anyhow!("'{}' is not an array", last[0])),
            };
            match array.get_mut(*index) {
                Some(TomlValue::Table(table)) => table,
                _ => return Err(anyhow!("missing array table item")),
            }
        }
    };

    let key_path = parse_key_path(key)?;
    let (leaf, parents) = key_path.split_last().ok_or_else(|| anyhow!("empty key"))?;
    let mut current = table;
    for component in parents {
        let entry = current
            .entry(component.clone())
            .or_insert_with(|| TomlValue::Table(TomlTable::new()));
        current = match entry {
            TomlValue::Table(table) => table,
            _ => return Err(anyhow!("'{component}' is not a table")),
        };
    }
    current.insert(leaf.clone(), value);
    Ok(())
}

fn parse_value(input: &str) -> Result<TomlValue> {
    let input = input.trim();
    if input.is_empty() {
        return Err(anyhow!("empty TOML value"));
    }

    if let Some(inner) = input
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    {
        return Ok(TomlValue::String(unescape_basic(inner)?));
    }
    if let Some(inner) = input
        .strip_prefix('\'')
        .and_then(|rest| rest.strip_suffix('\''))
    {
        return Ok(TomlValue::String(inner.to_string()));
    }
    if input.starts_with('[') {
        if !input.ends_with(']') {
            return Err(anyhow!("unterminated array"));
        }
        return Ok(TomlValue::Array(parse_array(&input[1..input.len() - 1])?));
    }
    match input {
        "true" => return Ok(TomlValue::Boolean(true)),
        "false" => return Ok(TomlValue::Boolean(false)),
        _ => {}
    }

    let numeric = input.replace('_', "");
    if let Ok(value) = numeric.parse::<i64>() {
        return Ok(TomlValue::Integer(value));
    }
    if let Ok(value) = numeric.parse::<f64>() {
        return Ok(TomlValue::Float(value));
    }

    // Be lenient with unquoted enum-like strings.
    Ok(TomlValue::String(input.to_string()))
}

fn parse_array(input: &str) -> Result<Vec<TomlValue>> {
    let mut values = Vec::new();
    let mut start = 0usize;
    let mut depth = 0i32;
    let mut quote: Option<char> = None;
    let mut escaped = false;

    for (index, character) in input.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if character == '\\' && quote == Some('"') {
            escaped = true;
            continue;
        }
        match quote {
            Some(active) if character == active => quote = None,
            Some(_) => {}
            None if character == '"' || character == '\'' => quote = Some(character),
            None => match character {
                '[' => depth += 1,
                ']' => depth -= 1,
                ',' if depth == 0 => {
                    let item = input[start..index].trim();
                    if !item.is_empty() {
                        values.push(parse_value(item)?);
                    }
                    start = index + 1;
                }
                _ => {}
            },
        }
    }

    let item = input[start..].trim();
    if !item.is_empty() {
        values.push(parse_value(item)?);
    }
    Ok(values)
}

fn unescape_basic(input: &str) -> Result<String> {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars();
    while let Some(character) = chars.next() {
        if character != '\\' {
            out.push(character);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => return Err(anyhow!("trailing escape in string")),
        }
    }
    Ok(out)
}

/// Quote a string for TOML output.
pub fn toml_string(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for character in value.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// Render an array of strings for TOML output.
pub fn toml_array_strings(values: &[String]) -> String {
    let rendered: Vec<String> = values.iter().map(|value| toml_string(value)).collect();
    format!("[{}]", rendered.join(", "))
}

/// Quote a table key when it is not a bare key.
pub fn toml_key(value: &str) -> String {
    if !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '-'
        })
    {
        value.to_string()
    } else {
        toml_string(value)
    }
}

/// Read a string list from a table key.
pub fn table_strings(table: &TomlTable, key: &str) -> Vec<String> {
    table
        .get(key)
        .map(TomlValue::string_list)
        .unwrap_or_default()
}

/// Visual theme names; actual colour rendering belongs to the TUI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    /// One of the TUI palettes: catppuccin, tokyo-night, gruvbox, or nord.
    pub palette: String,
    pub accent: String,
    pub good: String,
    pub warn: String,
    pub bad: String,
    pub muted: String,
    pub info: String,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            palette: "catppuccin".into(),
            accent: "cyan".into(),
            good: "green".into(),
            warn: "yellow".into(),
            bad: "red".into(),
            muted: "bright black".into(),
            info: "blue".into(),
        }
    }
}

/// Tunable thresholds shared by checks, health and history.
#[derive(Debug, Clone, PartialEq)]
pub struct Thresholds {
    pub sync_db_max_age_days: u64,
    pub cache_old_days: u64,
    pub health_warn: f32,
    pub health_critical: f32,
    pub max_history: usize,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            sync_db_max_age_days: 30,
            cache_old_days: 30,
            health_warn: 80.0,
            health_critical: 60.0,
            max_history: 100,
        }
    }
}

/// Application configuration loaded from `~/.config/amph/config.toml`.
#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    pub theme: Theme,
    pub protected_packages: Vec<String>,
    pub thresholds: Thresholds,
    /// Unknown top-level tables are preserved for forward compatibility.
    pub extra: TomlTable,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme: Theme::default(),
            protected_packages: default_protected_packages(),
            thresholds: Thresholds::default(),
            extra: TomlTable::new(),
        }
    }
}

impl Config {
    /// `$XDG_CONFIG_HOME/amph/config.toml` or `~/.config/amph/config.toml`.
    pub fn default_path() -> Option<PathBuf> {
        config_home().map(|home| home.join("amph").join("config.toml"))
    }

    /// Load the default file, or return defaults when it does not exist.
    pub fn load_default() -> Result<Self> {
        match Self::default_path() {
            Some(path) if path.exists() => Self::load(&path),
            _ => Ok(Self::default()),
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("cannot read config {}", path.display()))?;
        Self::from_toml(&text).with_context(|| format!("invalid config {}", path.display()))
    }

    pub fn from_toml(text: &str) -> Result<Self> {
        let root = parse_toml(text)?;
        Self::from_table(&root)
    }

    pub fn from_table(root: &TomlTable) -> Result<Self> {
        let mut config = Self::default();

        if let Some(theme) = root.get("theme").and_then(TomlValue::as_table) {
            if let Some(palette) = theme
                .get("palette")
                .or_else(|| theme.get("name"))
                .and_then(TomlValue::as_str)
            {
                config.theme.palette = palette.to_string();
            }
            set_string(&mut config.theme.accent, theme.get("accent"));
            set_string(&mut config.theme.good, theme.get("good"));
            set_string(&mut config.theme.warn, theme.get("warn"));
            set_string(&mut config.theme.bad, theme.get("bad"));
            set_string(&mut config.theme.muted, theme.get("muted"));
            set_string(&mut config.theme.info, theme.get("info"));
        }

        let protected = match root.get("protected") {
            Some(TomlValue::Table(table)) => table_strings(table, "packages"),
            Some(value) => value.string_list(),
            None => root
                .get("protected_packages")
                .map(TomlValue::string_list)
                .unwrap_or_default(),
        };
        if !protected.is_empty() {
            config.protected_packages = protected;
        }

        if let Some(thresholds) = root.get("thresholds").and_then(TomlValue::as_table) {
            if let Some(value) = thresholds
                .get("sync_db_max_age_days")
                .and_then(TomlValue::as_i64)
            {
                config.thresholds.sync_db_max_age_days = value.max(0) as u64;
            }
            if let Some(value) = thresholds.get("cache_old_days").and_then(TomlValue::as_i64) {
                config.thresholds.cache_old_days = value.max(0) as u64;
            }
            if let Some(value) = thresholds.get("health_warn").and_then(TomlValue::as_f64) {
                config.thresholds.health_warn = value.clamp(0.0, 100.0) as f32;
            }
            if let Some(value) = thresholds
                .get("health_critical")
                .and_then(TomlValue::as_f64)
            {
                config.thresholds.health_critical = value.clamp(0.0, 100.0) as f32;
            }
            if let Some(value) = thresholds.get("max_history").and_then(TomlValue::as_i64) {
                config.thresholds.max_history = value.max(1) as usize;
            }
        }

        for (key, value) in root {
            if !matches!(
                key.as_str(),
                "theme" | "protected" | "protected_packages" | "thresholds"
            ) {
                config.extra.insert(key.clone(), value.clone());
            }
        }

        Ok(config)
    }

    pub fn to_toml(&self) -> String {
        let mut out = String::new();
        let _ = writeln!(
            out,
            "protected = {}",
            toml_array_strings(&self.protected_packages)
        );
        let _ = writeln!(out);
        let _ = writeln!(out, "[theme]");
        let _ = writeln!(out, "palette = {}", toml_string(&self.theme.palette));
        let _ = writeln!(out, "accent = {}", toml_string(&self.theme.accent));
        let _ = writeln!(out, "good = {}", toml_string(&self.theme.good));
        let _ = writeln!(out, "warn = {}", toml_string(&self.theme.warn));
        let _ = writeln!(out, "bad = {}", toml_string(&self.theme.bad));
        let _ = writeln!(out, "muted = {}", toml_string(&self.theme.muted));
        let _ = writeln!(out, "info = {}", toml_string(&self.theme.info));
        let _ = writeln!(out);

        let _ = writeln!(out, "[thresholds]");
        let _ = writeln!(
            out,
            "sync_db_max_age_days = {}",
            self.thresholds.sync_db_max_age_days
        );
        let _ = writeln!(out, "cache_old_days = {}", self.thresholds.cache_old_days);
        let _ = writeln!(out, "health_warn = {}", self.thresholds.health_warn);
        let _ = writeln!(out, "health_critical = {}", self.thresholds.health_critical);
        let _ = writeln!(out, "max_history = {}", self.thresholds.max_history);
        out
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
        fs::write(&temporary, self.to_toml())
            .with_context(|| format!("cannot write {}", temporary.display()))?;
        fs::rename(&temporary, path)
            .with_context(|| format!("cannot replace {}", path.display()))?;
        Ok(())
    }

    pub fn is_protected(&self, name: &str) -> bool {
        self.protected_packages.iter().any(|entry| entry == name)
    }
}

fn set_string(target: &mut String, value: Option<&TomlValue>) {
    if let Some(value) = value.and_then(TomlValue::as_str) {
        *target = value.to_string();
    }
}

fn config_home() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_CONFIG_HOME") {
        let path = PathBuf::from(path);
        if !path.as_os_str().is_empty() {
            return Some(path);
        }
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config"))
}

/// Packages that should never be proposed for removal by default.
pub fn default_protected_packages() -> Vec<String> {
    [
        "bash",
        "base",
        "base-devel",
        "coreutils",
        "filesystem",
        "glibc",
        "linux",
        "linux-lts",
        "networkmanager",
        "openssh",
        "pacman",
        "pam",
        "sudo",
        "systemd",
        "util-linux",
    ]
    .into_iter()
    .map(ToString::to_string)
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_theme_protected_and_thresholds() {
        let text = r#"
protected = ["linux", "systemd", "my-vpn"]

[theme]
palette = "nord"
accent = "magenta"
good = "green"
warn = "yellow"
bad = "red"
muted = "bright black"
info = "blue"

[thresholds]
sync_db_max_age_days = 7
cache_old_days = 14
health_warn = 85.5
health_critical = 55
max_history = 250

[future]
enabled = true
"#;

        let config = Config::from_toml(text).expect("config should parse");
        assert_eq!(config.theme.accent, "magenta");
        assert_eq!(config.theme.palette, "nord");
        assert_eq!(
            config.protected_packages,
            vec!["linux", "systemd", "my-vpn"]
        );
        assert!(config.is_protected("systemd"));
        assert_eq!(config.thresholds.sync_db_max_age_days, 7);
        assert_eq!(config.thresholds.cache_old_days, 14);
        assert!((config.thresholds.health_warn - 85.5).abs() < 0.001);
        assert_eq!(config.thresholds.max_history, 250);
        assert!(config.extra.contains_key("future"));
    }

    #[test]
    fn round_trips_written_config() {
        let mut config = Config::default();
        config.theme.palette = "nord".to_string();
        config.protected_packages = vec!["my-critical-package".to_string()];
        let text = config.to_toml();
        let parsed = Config::from_toml(&text).expect("serialized config should parse");
        assert_eq!(parsed.theme, config.theme);
        assert_eq!(parsed.protected_packages, config.protected_packages);
        assert_eq!(parsed.thresholds, config.thresholds);
    }

    #[test]
    fn parses_arrays_of_tables_and_nested_tables() {
        let text = r#"
name = "demo"

[[packages]]
name = "a"
tags = ["x", "y"]

[[packages]]
name = "b"

[categories.dev]
packages = ["gcc", "make"]
priority = 3
"#;
        let root = parse_toml(text).expect("toml");
        let packages = root
            .get("packages")
            .and_then(TomlValue::as_array)
            .expect("packages array");
        assert_eq!(packages.len(), 2);
        assert_eq!(
            packages[0]
                .as_table()
                .and_then(|table| table.get("name"))
                .and_then(TomlValue::as_str),
            Some("a")
        );
        let dev = root
            .get("categories")
            .and_then(TomlValue::as_table)
            .and_then(|table| table.get("dev"))
            .and_then(TomlValue::as_table)
            .expect("dev category");
        assert_eq!(table_strings(dev, "packages"), vec!["gcc", "make"]);
        assert_eq!(dev.get("priority").and_then(TomlValue::as_i64), Some(3));
    }

    #[test]
    fn rejects_malformed_input_without_panicking() {
        assert!(parse_toml("key = [1, 2").is_err());
        assert!(parse_toml("[[[").is_err());
        assert!(Config::from_toml("thresholds = 3").is_ok());
    }
}
