//! Reusable AMPH backend.
//!
//! The binary keeps its historical module layout in `src/main.rs`. New domain
//! code that needs to be shared with the CLI and TUI is added here instead of
//! making `main.rs` larger. The plugin backend is the first such module.

pub mod plugin;
