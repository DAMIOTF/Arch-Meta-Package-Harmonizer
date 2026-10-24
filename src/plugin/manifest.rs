//! Strict parsing for plugin package manifests.
//!
//! A plugin release contains a `plugin.json` manifest as a GitHub release
//! asset. The manifest is untrusted input: unknown fields are rejected, all
//! identifiers are path-safe, and theme-only/functionality-only fields are
//! mutually exclusive.
//!
//! Manifests contain no permissions or capability declarations. The only
//! supported functionality plugin model is a stateless Wasm analyze hook; it
//! cannot request filesystem, network, process, environment, clock, or host
//! callback access.

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};

use super::wasm::ABI_VERSION;

/// Current manifest schema.
pub const MANIFEST_SCHEMA: u32 = 1;

/// Maximum accepted manifest size.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;

/// A plugin kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginKind {
    /// Declarative theme data. No Wasm entry point is allowed.
    Theme,
    /// A stateless Wasm analyze hook.
    Functionality,
}

/// Declarative theme values.
///
/// The fields mirror the existing `config::Theme` section so a later TUI
/// wiring lane can apply an installed theme without translating the manifest.
/// `palette` must name one of the four built-in AMPH palettes. Color fields
/// accept only the color set implemented by the TUI's `parse_ansi_color`:
/// case-insensitive `black`, `red`, `green`, `yellow`, `blue`, `magenta`,
/// `cyan`, `white`, `gray`, `grey`, `dark gray`, `dark grey`, `bright black`,
/// `bright red`, `bright green`, `bright yellow`, `bright blue`,
/// `bright magenta`, `bright cyan`, `bright white`, or an exact `#RRGGBB`
/// value. Surrounding whitespace is trimmed, matching the TUI parser.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThemeSpec {
    pub palette: String,
    #[serde(default = "default_accent")]
    pub accent: String,
    #[serde(default = "default_good")]
    pub good: String,
    #[serde(default = "default_warn")]
    pub warn: String,
    #[serde(default = "default_bad")]
    pub bad: String,
    #[serde(default = "default_muted")]
    pub muted: String,
    #[serde(default = "default_info")]
    pub info: String,
}

fn default_accent() -> String {
    "cyan".to_string()
}

fn default_good() -> String {
    "green".to_string()
}

fn default_warn() -> String {
    "yellow".to_string()
}

fn default_bad() -> String {
    "red".to_string()
}

fn default_muted() -> String {
    "bright black".to_string()
}

fn default_info() -> String {
    "blue".to_string()
}

impl Default for ThemeSpec {
    fn default() -> Self {
        Self {
            palette: "catppuccin".to_string(),
            accent: default_accent(),
            good: default_good(),
            warn: default_warn(),
            bad: default_bad(),
            muted: default_muted(),
            info: default_info(),
        }
    }
}

impl ThemeSpec {
    /// Validate a declarative theme.
    pub fn validate(&self) -> Result<()> {
        let palette = self.palette.trim().to_ascii_lowercase();
        if !matches!(
            palette.as_str(),
            "catppuccin" | "mocha" | "tokyo" | "tokyo-night" | "tokyonight" | "gruvbox" | "nord"
        ) {
            bail!("theme.palette must be one of catppuccin, tokyo-night, gruvbox or nord");
        }
        for (field, value) in [
            ("accent", &self.accent),
            ("good", &self.good),
            ("warn", &self.warn),
            ("bad", &self.bad),
            ("muted", &self.muted),
            ("info", &self.info),
        ] {
            if !is_supported_theme_color(value) {
                bail!(
                    "theme.{field} must be a supported ANSI color name or an exact #RRGGBB value"
                );
            }
        }
        Ok(())
    }
}

fn is_supported_theme_color(value: &str) -> bool {
    let value = value.trim().to_ascii_lowercase();
    if value.len() == 7 && value.starts_with('#') {
        return value[1..].bytes().all(|byte| byte.is_ascii_hexdigit());
    }
    matches!(
        value.as_str(),
        "black"
            | "red"
            | "green"
            | "yellow"
            | "blue"
            | "magenta"
            | "cyan"
            | "white"
            | "gray"
            | "grey"
            | "dark gray"
            | "dark grey"
            | "bright black"
            | "bright red"
            | "bright green"
            | "bright yellow"
            | "bright blue"
            | "bright magenta"
            | "bright cyan"
            | "bright white"
    )
}

/// A strict `plugin.json` manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub schema: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub kind: PluginKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Required for functionality plugins; must be [`ABI_VERSION`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<u32>,
    /// Wasm release asset name, required for functionality plugins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
    /// Required for theme plugins and forbidden otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<ThemeSpec>,
}

impl PluginManifest {
    /// Parse and validate a manifest from untrusted bytes.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        if bytes.len() > MAX_MANIFEST_BYTES {
            bail!(
                "plugin manifest is too large ({} bytes, limit {MAX_MANIFEST_BYTES})",
                bytes.len()
            );
        }
        let manifest: Self = serde_json::from_slice(bytes)
            .map_err(|error| anyhow::anyhow!("cannot parse plugin manifest as JSON: {error}"))?;
        manifest.validate()?;
        Ok(manifest)
    }

    /// Parse and validate a manifest from a JSON string.
    pub fn from_json(text: &str) -> Result<Self> {
        Self::parse(text.as_bytes())
    }

    /// Validate schema, identifiers and kind-specific fields.
    pub fn validate(&self) -> Result<()> {
        if self.schema != MANIFEST_SCHEMA {
            bail!(
                "unsupported plugin manifest schema {}; this build supports schema {MANIFEST_SCHEMA}",
                self.schema
            );
        }
        validate_id(&self.id)?;
        validate_text("name", &self.name, 80, 1)?;
        validate_version(&self.version)?;
        if let Some(description) = &self.description {
            validate_text("description", description, 512, 0)?;
        }
        if let Some(author) = &self.author {
            validate_text("author", author, 80, 0)?;
        }

        match self.kind {
            PluginKind::Theme => {
                if self.api.is_some() {
                    bail!("theme plugins must not declare `api`");
                }
                if self.entry.is_some() {
                    bail!("theme plugins must not declare a Wasm `entry`");
                }
                let theme = self.theme.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("theme plugins must declare a `theme` object")
                })?;
                theme.validate()?;
            }
            PluginKind::Functionality => {
                if self.theme.is_some() {
                    bail!("functionality plugins must not declare `theme` data");
                }
                match self.api {
                    Some(api) if api == ABI_VERSION => {}
                    Some(api) => {
                        bail!("unsupported plugin ABI {api}; this build supports ABI {ABI_VERSION}")
                    }
                    None => bail!("functionality plugins must declare `api`"),
                }
                let entry = self
                    .entry
                    .as_deref()
                    .ok_or_else(|| anyhow::anyhow!("functionality plugins must declare `entry`"))?;
                validate_entry(entry)?;
            }
        }
        Ok(())
    }

    /// Whether this manifest describes a theme plugin.
    pub fn is_theme(&self) -> bool {
        self.kind == PluginKind::Theme
    }

    /// Whether this manifest describes a functionality plugin.
    pub fn is_functionality(&self) -> bool {
        self.kind == PluginKind::Functionality
    }
}

/// Validate a plugin identifier used as a directory name and lookup key.
///
/// Identifiers are lowercase ASCII, 1-64 characters, and contain only letters,
/// digits, `-`, `_` and `.`. The first and last characters must be
/// alphanumeric and `..` is forbidden, so an identifier can never traverse
/// outside the plugin store.
pub fn validate_id(id: &str) -> Result<()> {
    if id.is_empty() || id.len() > 64 {
        bail!("plugin id must be 1-64 characters");
    }
    if id.contains("..") {
        bail!("plugin id must not contain `..`");
    }
    let first = id.chars().next().unwrap_or(' ');
    let last = id.chars().last().unwrap_or(' ');
    if !first.is_ascii_alphanumeric() || !last.is_ascii_alphanumeric() {
        bail!("plugin id must start and end with an ASCII letter or digit");
    }
    if !id
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '-' | '_' | '.'))
    {
        bail!("plugin id may contain only lowercase letters, digits, '-', '_' and '.'");
    }
    Ok(())
}

/// Validate a Wasm entry asset name.
///
/// Entries are plain release asset file names, never paths.
pub fn validate_entry(entry: &str) -> Result<()> {
    if entry.is_empty() || entry.len() > 128 {
        bail!("plugin entry must be 1-128 characters");
    }
    if entry == "." || entry == ".." {
        bail!("plugin entry must be a file name");
    }
    if entry.contains('/') || entry.contains('\\') || entry.contains('\0') {
        bail!("plugin entry must not contain path separators");
    }
    if !entry.ends_with(".wasm") {
        bail!("plugin entry must end in `.wasm`");
    }
    if !entry
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | '+'))
    {
        bail!("plugin entry may contain only ASCII letters, digits, '.', '-', '_' and '+'");
    }
    if matches!(entry, "plugin.json" | "install.json" | "disabled") {
        bail!("plugin entry uses a reserved store file name");
    }
    Ok(())
}

fn validate_version(version: &str) -> Result<()> {
    if version.is_empty() || version.len() > 64 {
        bail!("plugin version must be 1-64 characters");
    }
    if !version
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '+' | '_'))
    {
        bail!("plugin version may contain only ASCII letters, digits, '.', '-', '+' and '_'");
    }
    Ok(())
}

fn validate_text(field: &str, value: &str, max: usize, min: usize) -> Result<()> {
    let length = value.chars().count();
    if length < min || length > max {
        bail!("plugin {field} must be {min}-{max} characters");
    }
    if value.trim() != value {
        bail!("plugin {field} must not start or end with whitespace");
    }
    if value.chars().any(char::is_control) {
        bail!("plugin {field} must not contain control characters");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn functionality_manifest_text() -> String {
        r#"{
            "schema": 1,
            "id": "example.thermal",
            "name": "Thermal",
            "version": "1.2.0",
            "kind": "functionality",
            "description": "Example plugin",
            "author": "Example",
            "api": 1,
            "entry": "plugin.wasm"
        }"#
        .to_string()
    }

    #[test]
    fn parses_a_theme_manifest() {
        let text = r##"{
            "schema": 1,
            "id": "example.thermal",
            "name": "Thermal",
            "version": "1.2.0",
            "kind": "theme",
            "theme": {"palette": "gruvbox", "accent": "#ff8800"}
        }"##;
        let parsed = PluginManifest::from_json(text).unwrap();
        assert!(parsed.is_theme());
        assert_eq!(parsed.theme.unwrap().palette, "gruvbox");
    }

    #[test]
    fn parses_a_functionality_manifest() {
        let text = functionality_manifest_text();
        let parsed = PluginManifest::from_json(&text).unwrap();
        assert!(parsed.is_functionality());
        assert_eq!(parsed.entry.as_deref(), Some("plugin.wasm"));
        assert_eq!(parsed.api, Some(ABI_VERSION));
    }

    #[test]
    fn unknown_fields_are_rejected() {
        let text = r#"{
            "schema": 1,
            "id": "example.thermal",
            "name": "Thermal",
            "version": "1.0.0",
            "kind": "theme",
            "theme": {"palette": "nord"},
            "permissions": ["network"]
        }"#;
        let error = PluginManifest::from_json(text).unwrap_err().to_string();
        assert!(error.contains("unknown field `permissions`"), "{error}");
    }

    #[test]
    fn theme_and_functionality_fields_are_mutually_exclusive() {
        let mut theme: PluginManifest = serde_json::from_str(
            r#"{
                "schema": 1,
                "id": "example.thermal",
                "name": "Thermal",
                "version": "1.0.0",
                "kind": "theme",
                "theme": {"palette": "nord"}
            }"#,
        )
        .unwrap();
        theme.entry = Some("plugin.wasm".into());
        assert!(theme.validate().is_err());

        let mut functionality: PluginManifest = serde_json::from_str(
            r#"{
                "schema": 1,
                "id": "example.analyze",
                "name": "Analyze",
                "version": "1.0.0",
                "kind": "functionality",
                "api": 1,
                "entry": "plugin.wasm"
            }"#,
        )
        .unwrap();
        functionality.theme = Some(ThemeSpec::default());
        assert!(functionality.validate().is_err());
    }

    #[test]
    fn functionality_requires_the_supported_abi_and_wasm_entry() {
        let mut parsed: PluginManifest = serde_json::from_str(
            r#"{
                "schema": 1,
                "id": "example.analyze",
                "name": "Analyze",
                "version": "1.0.0",
                "kind": "functionality",
                "api": 1,
                "entry": "plugin.wasm"
            }"#,
        )
        .unwrap();
        parsed.api = Some(99);
        assert!(parsed.validate().is_err());

        parsed.api = Some(1);
        parsed.entry = Some("payload.bin".into());
        assert!(parsed.validate().is_err());
    }

    #[test]
    fn ids_cannot_traverse_the_plugin_store() {
        for id in [
            "../escape",
            "..",
            ".hidden",
            "Upper",
            "with/slash",
            "with\\slash",
            "-leading",
            "trailing-",
            "con..dot",
            "",
            "a\u{0}b",
        ] {
            assert!(validate_id(id).is_err(), "accepted unsafe id {id:?}");
        }
        for id in ["a", "example.thermal", "owner-plugin_2", "z9"] {
            assert!(validate_id(id).is_ok(), "rejected safe id {id:?}");
        }
    }

    #[test]
    fn theme_palette_must_be_built_in() {
        assert!(
            ThemeSpec {
                palette: "custom".into(),
                ..ThemeSpec::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            ThemeSpec {
                palette: "Nord".into(),
                ..ThemeSpec::default()
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn theme_colors_accept_only_documented_names_and_hex() {
        let mut theme = ThemeSpec::default();
        for valid in [
            "black",
            "red",
            "green",
            "yellow",
            "blue",
            "magenta",
            "cyan",
            "white",
            "gray",
            "grey",
            "dark gray",
            "dark grey",
            "bright black",
            "bright red",
            "bright green",
            "bright yellow",
            "bright blue",
            "bright magenta",
            "bright cyan",
            "bright white",
            "BRIGHT BLACK",
            "Gray",
            "#ff8800",
            "#ABCDEF",
            "#000000",
            " cyan ",
            "\tcyan\t",
        ] {
            theme.accent = valid.into();
            assert!(theme.validate().is_ok(), "rejected valid color {valid:?}");
        }

        for invalid in [
            "#broken",
            "#123",
            "#12345g",
            "#1234567",
            "#12345678",
            "orange",
            "bright",
            "bright orange",
            "bright gray",
            "bright grey",
            "bright  red",
            "bright\tblack",
            "dark red",
            "light cyan",
            "",
        ] {
            theme.accent = invalid.into();
            assert!(
                theme.validate().is_err(),
                "accepted invalid color {invalid:?}"
            );
        }
    }

    #[test]
    fn oversized_manifests_are_rejected_before_json_parsing() {
        let oversized = vec![b' '; MAX_MANIFEST_BYTES + 1];
        assert!(PluginManifest::parse(&oversized).is_err());
    }
}
