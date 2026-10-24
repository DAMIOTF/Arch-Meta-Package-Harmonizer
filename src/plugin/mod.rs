//! Sandboxed plugin backend.
//!
//! AMPH supports two plugin kinds:
//!
//! - **theme** plugins are declarative data (`theme` in `plugin.json`) and
//!   contain no executable code;
//! - **functionality** plugins are stateless Wasm analyze hooks that receive
//!   one JSON value and return one JSON value.
//!
//! A functionality module imports nothing. It cannot access files, the
//! network, processes, environment variables, the clock, or host callbacks,
//! because `wasmi` is started with an empty linker and the module is rejected
//! if it declares any import. Execution is bounded by module, memory, input,
//! output, recursion and fuel limits.
//!
//! Plugins are distributed through GitHub Releases. Installation accepts only
//! `https://github.com/{owner}/{repo}` and the latest stable release, and every
//! asset must have a GitHub-provided SHA-256 digest that matches the download.
//! Plugins live under the per-user XDG data directory and are installed and
//! removed with directory renames.
//!
//! This module is deliberately host-agnostic. A later TUI/CLI wiring lane can
//! call [`store::PluginStore`] and [`wasm`] without reimplementing any policy
//! here. It does not read configuration or start network work on import.

pub mod github;
pub mod manifest;
pub mod store;
pub mod wasm;

pub use github::{
    GitHubClient, Release, ReleaseAsset, ReleaseProvider, Repository, Sha256Digest, sha256_hex,
};
pub use manifest::{
    MANIFEST_SCHEMA, MAX_MANIFEST_BYTES, PluginKind, PluginManifest, ThemeSpec, validate_entry,
    validate_id,
};
pub use store::{
    CheckOutcome, DISABLED_FILE, InstallRecord, InstalledPlugin, MANIFEST_FILE, PluginStore,
    PluginUpdate, RECORD_FILE, RECORD_SCHEMA, UpdateOutcome, default_root,
};
pub use wasm::{
    ABI_VERSION, DEFAULT_FUEL, MAX_MODULE_BYTES, ModuleInfo, RuntimeLimits, WASM_PAGE_BYTES,
    analyze, analyze_json, analyze_json_with, analyze_with, validate_module, validate_module_with,
};
