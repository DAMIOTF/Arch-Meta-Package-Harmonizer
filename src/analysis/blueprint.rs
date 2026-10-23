//! Blueprint export, diff and apply.
//!
//! A blueprint is a portable TOML description of the package set.  Applying one
//! reconciles installs and explicit/dependency marks, with optional removal of
//! extras.  Commands are generated first and only executed when explicitly
//! requested; every pacman invocation sets `LC_ALL=C`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::process::Command;

use anyhow::{Context, Result, anyhow};

use crate::config::{
    TomlTable, TomlValue, parse_toml, table_strings, toml_array_strings, toml_string,
};
use crate::model::{Package, Snapshot, unix_now};

/// One package entry in a blueprint.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlueprintPackage {
    pub name: String,
    pub version: String,
    pub explicit: bool,
    pub reason: Option<String>,
    pub provides: Vec<String>,
    pub depends: Vec<String>,
}

/// Portable blueprint document.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Blueprint {
    pub version: u32,
    pub name: String,
    pub generated_at: u64,
    pub packages: Vec<BlueprintPackage>,
    pub explicit: Vec<String>,
    pub protected: Vec<String>,
}

/// Differences between the installed system and a blueprint.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BlueprintDiff {
    /// In the blueprint, not installed.
    pub missing: Vec<String>,
    /// Installed, not in the blueprint.
    pub extra: Vec<String>,
    /// Installed packages whose version differs from the blueprint.
    pub version_changed: Vec<(String, String, String)>,
    /// Installed packages that should be marked explicit.
    pub to_explicit: Vec<String>,
    /// Installed packages that should be marked as dependencies.
    pub to_dependency: Vec<String>,
}

impl BlueprintDiff {
    pub fn is_empty(&self) -> bool {
        self.missing.is_empty()
            && self.extra.is_empty()
            && self.version_changed.is_empty()
            && self.to_explicit.is_empty()
            && self.to_dependency.is_empty()
    }
}

/// Export the installed package set.
pub fn export(snapshot: &Snapshot, name: &str) -> Blueprint {
    let mut packages: Vec<BlueprintPackage> = snapshot
        .packages
        .iter()
        .map(|package| BlueprintPackage {
            name: package.name.clone(),
            version: package.version.clone(),
            explicit: package.explicit,
            reason: package.reason.clone(),
            provides: package.provides.clone(),
            depends: package.depends.clone(),
        })
        .collect();
    packages.sort_by(|a, b| a.name.cmp(&b.name));

    let explicit: Vec<String> = snapshot
        .packages
        .iter()
        .filter(|package| package.explicit)
        .map(|package| package.name.clone())
        .collect();

    Blueprint {
        version: 1,
        name: name.to_string(),
        generated_at: unix_now(),
        packages,
        explicit,
        protected: Vec::new(),
    }
}

/// Alias retained for callers that prefer the longer name.
pub fn export_blueprint(snapshot: &Snapshot, name: &str) -> Blueprint {
    export(snapshot, name)
}

/// Serialize a blueprint to TOML.
pub fn to_toml(blueprint: &Blueprint) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "version = {}", blueprint.version);
    let _ = writeln!(out, "name = {}", toml_string(&blueprint.name));
    let _ = writeln!(out, "generated_at = {}", blueprint.generated_at);
    let _ = writeln!(
        out,
        "explicit = {}",
        toml_array_strings(&blueprint.explicit)
    );
    let _ = writeln!(
        out,
        "protected = {}",
        toml_array_strings(&blueprint.protected)
    );
    let _ = writeln!(out);

    for package in &blueprint.packages {
        let _ = writeln!(out, "[[packages]]");
        let _ = writeln!(out, "name = {}", toml_string(&package.name));
        let _ = writeln!(out, "version = {}", toml_string(&package.version));
        let _ = writeln!(out, "explicit = {}", package.explicit);
        let _ = writeln!(
            out,
            "reason = {}",
            toml_string(package.reason.as_deref().unwrap_or(""))
        );
        let _ = writeln!(out, "provides = {}", toml_array_strings(&package.provides));
        let _ = writeln!(out, "depends = {}", toml_array_strings(&package.depends));
        let _ = writeln!(out);
    }
    out
}

/// Alias retained for callers that prefer the longer name.
pub fn blueprint_to_toml(blueprint: &Blueprint) -> String {
    to_toml(blueprint)
}

/// Parse a blueprint TOML document.
pub fn parse(text: &str) -> Result<Blueprint> {
    let root = parse_toml(text)?;
    blueprint_from_table(&root)
}

/// Alias retained for callers that prefer the longer name.
pub fn parse_blueprint(text: &str) -> Result<Blueprint> {
    parse(text)
}

fn blueprint_from_table(root: &TomlTable) -> Result<Blueprint> {
    let version = root
        .get("version")
        .and_then(TomlValue::as_i64)
        .unwrap_or(1)
        .max(0) as u32;
    let name = root
        .get("name")
        .and_then(TomlValue::as_str)
        .unwrap_or("blueprint")
        .to_string();
    let generated_at = root
        .get("generated_at")
        .and_then(TomlValue::as_i64)
        .unwrap_or(0)
        .max(0) as u64;
    let explicit = table_strings(root, "explicit");
    let protected = table_strings(root, "protected");

    let mut packages = Vec::new();
    if let Some(entries) = root.get("packages").and_then(TomlValue::as_array) {
        for entry in entries {
            let Some(table) = entry.as_table() else {
                continue;
            };
            let Some(name) = table.get("name").and_then(TomlValue::as_str) else {
                continue;
            };
            if name.trim().is_empty() {
                continue;
            }
            packages.push(BlueprintPackage {
                name: name.to_string(),
                version: table
                    .get("version")
                    .and_then(TomlValue::as_str)
                    .unwrap_or("")
                    .to_string(),
                explicit: table
                    .get("explicit")
                    .and_then(TomlValue::as_bool)
                    .unwrap_or_else(|| explicit.iter().any(|entry| entry == name)),
                reason: table
                    .get("reason")
                    .and_then(TomlValue::as_str)
                    .filter(|value| !value.is_empty())
                    .map(ToString::to_string),
                provides: table_strings(table, "provides"),
                depends: table_strings(table, "depends"),
            });
        }
    }

    packages.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Blueprint {
        version,
        name,
        generated_at,
        packages,
        explicit,
        protected,
    })
}

/// Compare the installed snapshot with a blueprint.
pub fn diff(current: &Snapshot, blueprint: &Blueprint) -> BlueprintDiff {
    let current_by_name: BTreeMap<&str, &Package> = current
        .packages
        .iter()
        .map(|package| (package.name.as_str(), package))
        .collect();
    let blueprint_by_name: BTreeMap<&str, &BlueprintPackage> = blueprint
        .packages
        .iter()
        .map(|package| (package.name.as_str(), package))
        .collect();

    let mut diff = BlueprintDiff::default();

    for (name, package) in &blueprint_by_name {
        match current_by_name.get(name) {
            None => diff.missing.push((*name).to_string()),
            Some(installed) => {
                if !package.version.is_empty() && installed.version != package.version {
                    diff.version_changed.push((
                        (*name).to_string(),
                        installed.version.clone(),
                        package.version.clone(),
                    ));
                }
                if package.explicit && !installed.explicit {
                    diff.to_explicit.push((*name).to_string());
                } else if !package.explicit && installed.explicit {
                    diff.to_dependency.push((*name).to_string());
                }
            }
        }
    }

    for name in current_by_name.keys() {
        if !blueprint_by_name.contains_key(name) {
            diff.extra.push((*name).to_string());
        }
    }

    diff.missing.sort();
    diff.extra.sort();
    diff.version_changed.sort();
    diff.to_explicit.sort();
    diff.to_dependency.sort();
    diff
}

/// Alias retained for callers that prefer the longer name.
pub fn diff_blueprint(current: &Snapshot, blueprint: &Blueprint) -> BlueprintDiff {
    diff(current, blueprint)
}

/// Options for applying a blueprint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyOptions {
    /// When true (the default) only the plan is generated.
    pub dry_run: bool,
    /// Remove installed packages absent from the blueprint.
    pub remove_extra: bool,
    pub noconfirm: bool,
    /// Packages that must never be removed.
    pub protected: Vec<String>,
}

impl Default for ApplyOptions {
    fn default() -> Self {
        Self {
            dry_run: true,
            remove_extra: false,
            noconfirm: true,
            protected: Vec::new(),
        }
    }
}

/// One command in an apply plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplyCommand {
    pub program: String,
    pub args: Vec<String>,
    pub description: String,
}

impl ApplyCommand {
    pub fn display(&self) -> String {
        let mut out = self.program.clone();
        for arg in &self.args {
            out.push(' ');
            if arg.contains(' ') {
                out.push('"');
                out.push_str(arg);
                out.push('"');
            } else {
                out.push_str(arg);
            }
        }
        out
    }
}

/// Outcome of one executed command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutcome {
    pub command: String,
    pub success: bool,
    pub status: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// Generated (and optionally executed) apply plan.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ApplyPlan {
    pub install: Vec<String>,
    pub remove: Vec<String>,
    pub mark_explicit: Vec<String>,
    pub mark_dependency: Vec<String>,
    pub commands: Vec<ApplyCommand>,
    pub warnings: Vec<String>,
    pub outcomes: Vec<CommandOutcome>,
    pub dry_run: bool,
}

impl ApplyPlan {
    pub fn is_noop(&self) -> bool {
        self.commands.is_empty()
    }
}

/// Build an apply plan without executing it.
pub fn plan(current: &Snapshot, blueprint: &Blueprint, options: &ApplyOptions) -> ApplyPlan {
    let differences = diff(current, blueprint);
    let protected: BTreeSet<&str> = options
        .protected
        .iter()
        .chain(blueprint.protected.iter())
        .map(String::as_str)
        .collect();

    let mut plan = ApplyPlan {
        install: differences.missing.clone(),
        remove: Vec::new(),
        mark_explicit: differences.to_explicit.clone(),
        mark_dependency: differences.to_dependency.clone(),
        commands: Vec::new(),
        warnings: Vec::new(),
        outcomes: Vec::new(),
        dry_run: options.dry_run,
    };

    if options.remove_extra {
        for package in &differences.extra {
            if protected.contains(package.as_str()) {
                plan.warnings
                    .push(format!("not removing protected package {package}"));
            } else {
                plan.remove.push(package.clone());
            }
        }
    }

    if !plan.install.is_empty() {
        let mut args = vec!["-S".to_string(), "--needed".to_string()];
        if options.noconfirm {
            args.push("--noconfirm".to_string());
        }
        args.extend(plan.install.iter().cloned());
        plan.commands.push(ApplyCommand {
            program: "pacman".to_string(),
            args,
            description: format!("install {} package(s)", plan.install.len()),
        });
    }

    if !plan.mark_explicit.is_empty() {
        let mut args = vec!["-D".to_string(), "--asexplicit".to_string()];
        args.extend(plan.mark_explicit.iter().cloned());
        plan.commands.push(ApplyCommand {
            program: "pacman".to_string(),
            args,
            description: format!("mark {} package(s) explicit", plan.mark_explicit.len()),
        });
    }

    if !plan.mark_dependency.is_empty() {
        let mut args = vec!["-D".to_string(), "--asdeps".to_string()];
        args.extend(plan.mark_dependency.iter().cloned());
        plan.commands.push(ApplyCommand {
            program: "pacman".to_string(),
            args,
            description: format!(
                "mark {} package(s) as dependencies",
                plan.mark_dependency.len()
            ),
        });
    }

    if !plan.remove.is_empty() {
        let mut args = vec!["-Rns".to_string()];
        if options.noconfirm {
            args.push("--noconfirm".to_string());
        }
        args.extend(plan.remove.iter().cloned());
        plan.commands.push(ApplyCommand {
            program: "pacman".to_string(),
            args,
            description: format!("remove {} extra package(s)", plan.remove.len()),
        });
    }

    plan
}

/// Plan and (unless `dry_run`) execute a blueprint application.
pub fn apply(
    current: &Snapshot,
    blueprint: &Blueprint,
    options: &ApplyOptions,
) -> Result<ApplyPlan> {
    let mut plan = plan(current, blueprint, options);
    if options.dry_run {
        return Ok(plan);
    }

    for command in &plan.commands {
        let output = Command::new(&command.program)
            .args(&command.args)
            .env("LC_ALL", "C")
            .output()
            .with_context(|| format!("cannot run {}", command.display()))?;
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let outcome = CommandOutcome {
            command: command.display(),
            success: output.status.success(),
            status: output.status.code(),
            stdout,
            stderr,
        };
        plan.outcomes.push(outcome);

        if !output.status.success() {
            return Err(anyhow!(
                "command failed: {} (status {:?})",
                command.display(),
                output.status.code()
            ));
        }
    }

    Ok(plan)
}

/// Alias retained for callers that prefer the longer name.
pub fn apply_blueprint(
    current: &Snapshot,
    blueprint: &Blueprint,
    options: &ApplyOptions,
) -> Result<ApplyPlan> {
    apply(current, blueprint, options)
}

/// Save a blueprint to disk.
pub fn save(blueprint: &Blueprint, path: &std::path::Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;
    }
    std::fs::write(path, to_toml(blueprint))
        .with_context(|| format!("cannot write blueprint {}", path.display()))
}

/// Load a blueprint from disk.
pub fn load(path: &std::path::Path) -> Result<Blueprint> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read blueprint {}", path.display()))?;
    parse(&text).with_context(|| format!("invalid blueprint {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> Snapshot {
        Snapshot::new(vec![
            Package {
                name: "app".into(),
                version: "1.0-1".into(),
                explicit: true,
                reason: Some("1".into()),
                provides: vec!["app-api=1".into()],
                depends: vec!["libfoo>=1".into()],
                ..Package::default()
            },
            Package {
                name: "libfoo".into(),
                version: "1.2-1".into(),
                explicit: false,
                reason: Some("0".into()),
                ..Package::default()
            },
        ])
    }

    #[test]
    fn round_trips_blueprint_toml() {
        let blueprint = export(&snapshot(), "workstation");
        let text = to_toml(&blueprint);
        let parsed = parse(&text).expect("blueprint round trip");

        assert_eq!(parsed.name, "workstation");
        assert_eq!(parsed.packages.len(), 2);
        assert_eq!(parsed.explicit, vec!["app"]);
        let app = parsed
            .packages
            .iter()
            .find(|package| package.name == "app")
            .expect("app");
        assert!(app.explicit);
        assert_eq!(app.provides, vec!["app-api=1"]);
        assert_eq!(app.depends, vec!["libfoo>=1"]);
    }

    #[test]
    fn diffs_versions_and_marks() {
        let blueprint = Blueprint {
            version: 1,
            name: "target".into(),
            generated_at: 0,
            packages: vec![
                BlueprintPackage {
                    name: "app".into(),
                    version: "2.0-1".into(),
                    explicit: false,
                    reason: Some("0".into()),
                    ..BlueprintPackage::default()
                },
                BlueprintPackage {
                    name: "new-package".into(),
                    version: "1.0-1".into(),
                    explicit: true,
                    ..BlueprintPackage::default()
                },
            ],
            explicit: vec!["new-package".into()],
            protected: Vec::new(),
        };

        let current = snapshot();
        let difference = diff(&current, &blueprint);
        assert_eq!(difference.missing, vec!["new-package"]);
        assert_eq!(difference.extra, vec!["libfoo"]);
        assert_eq!(
            difference.version_changed,
            vec![("app".to_string(), "1.0-1".to_string(), "2.0-1".to_string())]
        );
        assert_eq!(difference.to_dependency, vec!["app"]);
    }

    #[test]
    fn plans_commands_with_lc_all_only_when_applied() {
        let current = snapshot();
        let blueprint = Blueprint {
            version: 1,
            name: "target".into(),
            generated_at: 0,
            packages: vec![BlueprintPackage {
                name: "new-package".into(),
                version: "1.0-1".into(),
                explicit: true,
                ..BlueprintPackage::default()
            }],
            explicit: vec!["new-package".into()],
            protected: vec!["libfoo".into()],
        };

        let options = ApplyOptions {
            dry_run: true,
            remove_extra: true,
            noconfirm: true,
            protected: Vec::new(),
        };
        let plan = plan(&current, &blueprint, &options);
        assert!(plan.install.contains(&"new-package".to_string()));
        assert!(
            !plan.remove.contains(&"libfoo".to_string()),
            "libfoo is blueprint-protected"
        );
        assert!(plan.remove.contains(&"app".to_string()));
        assert!(
            plan.warnings
                .iter()
                .any(|warning| warning.contains("libfoo"))
        );
        assert!(
            plan.commands
                .iter()
                .any(|command| command.args.iter().any(|arg| arg == "-S"))
        );
    }

    #[test]
    fn dry_run_never_executes_pacman() {
        let current = snapshot();
        let blueprint = export(&current, "same");
        let plan = apply(&current, &blueprint, &ApplyOptions::default()).expect("dry run");
        assert!(plan.outcomes.is_empty());
    }
}
