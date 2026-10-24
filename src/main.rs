//! `amph` command line interface and TUI host.
//!
//! The old diagnostic modules are intentionally undeclared.  This entry point
//! wires the current model/source/graph/analysis/store/config backend to either
//! the interactive dashboard or scriptable subcommands.

pub mod analysis;
pub mod config;
pub mod graph;
pub mod model;
pub mod source;
pub mod store;
pub mod tui;
pub mod update;

use std::collections::BTreeSet;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use anyhow::{Context, Result, anyhow};
use clap::{Args, Parser, Subcommand};
use comfy_table::{Cell, ContentArrangement, Table, presets::UTF8_FULL};
use serde_json::{Value, json};

use analysis::blueprint::{self, ApplyCommand, ApplyOptions, Blueprint};
use analysis::checks::{CheckStatus, ChecksReport};
use analysis::harmony::{self, HarmonyConfig, Suggestion};
use analysis::timeline::{self, PackageAction, PacmanTransaction};
use config::Config;
use graph::Graph;
use model::Snapshot;
use store::HistoryEntry;

#[derive(Parser)]
#[command(
    name = "amph",
    version = "v0.3.0-beta",
    about = "Arch Meta Package Harmonizer",
    long_about = "Inspect installed Arch Linux packages, trace dependency paths, preview safe removals, check system health, export blueprints and browse a terminal dashboard."
)]
struct Cli {
    /// Emit machine-readable JSON on stdout. Human banners and tables are suppressed.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Explain why a package is installed and how it is reached.
    Why {
        /// Installed package name.
        package: String,
    },

    /// Preview the exact removal impact of one or more packages.
    Impact {
        /// Package names to remove.
        #[arg(required = true, num_args = 1..)]
        packages: Vec<String>,
    },

    /// Show Harmony categories and suggestions.
    Harmony {
        /// Optional Harmony TOML file. Defaults to ~/.config/amph/harmony.toml.
        #[arg(long, value_name = "FILE")]
        file: Option<PathBuf>,
    },

    /// Export, diff or apply portable package blueprints.
    Blueprint {
        #[command(subcommand)]
        action: BlueprintAction,
    },

    /// Run checks and return cron-friendly exit status 0/1/2.
    Check(CheckArgs),

    /// Show detailed check results.
    Checks(CheckArgs),

    /// Show pacman transactions from current and rotated logs.
    Timeline {
        /// Only show transactions since this date or duration (for example 7d, 24h, 2026-01-01).
        #[arg(long)]
        since: Option<String>,

        /// Maximum number of transactions (0 means all).
        #[arg(long, default_value_t = 40)]
        limit: usize,

        /// Pacman log path.
        #[arg(long, value_name = "FILE")]
        log: Option<PathBuf>,
    },

    /// Show compact bounded scan history.
    History {
        /// Maximum number of entries (defaults to the configured limit).
        #[arg(long)]
        limit: Option<usize>,

        /// Append a compact post-transaction summary. Used by the optional pacman hook.
        #[arg(long, hide = true)]
        record_hook: bool,
    },

    /// Append a compact history entry without invoking pacman (hidden hook alias).
    #[command(hide = true)]
    RecordHistory,

    /// Check GitHub `main` for a newer version and install it with Cargo.
    Update {
        /// Install without asking for confirmation.
        #[arg(long)]
        yes: bool,
    },

    /// Preview orphan removal, confirm and optionally execute `sudo pacman -Rns`.
    Cleanup {
        /// Execute after the preview. Without this flag cleanup is a dry run.
        #[arg(long)]
        execute: bool,

        /// Skip the interactive confirmation phrase. Implies explicit consent.
        #[arg(long)]
        yes: bool,

        /// Additional protected package names (repeatable).
        #[arg(long = "keep", value_name = "PKG")]
        keep: Vec<String>,
    },
}

#[derive(Args, Clone, Copy)]
struct CheckArgs {
    /// Skip the `pacman -Qk` integrity check.
    #[arg(long)]
    no_integrity: bool,
}

#[derive(Subcommand)]
enum BlueprintAction {
    /// Export the installed package set as TOML.
    Export {
        /// Write the blueprint to a file instead of stdout.
        #[arg(short, long, value_name = "FILE")]
        output: Option<PathBuf>,

        /// Blueprint name (defaults to the machine hostname).
        #[arg(long)]
        name: Option<String>,
    },

    /// Compare the installed system with a blueprint.
    Diff {
        /// Blueprint TOML file.
        #[arg(value_name = "FILE")]
        file: PathBuf,
    },

    /// Plan a blueprint application. Mutation requires --execute.
    Apply {
        /// Blueprint TOML file.
        #[arg(value_name = "FILE")]
        file: PathBuf,

        /// Execute the plan. Without this flag the plan is printed only.
        #[arg(long)]
        execute: bool,

        /// Also remove installed packages absent from the blueprint.
        #[arg(long)]
        remove_extra: bool,

        /// Skip the interactive confirmation phrase. Implies explicit consent.
        #[arg(long)]
        yes: bool,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(code) => ExitCode::from(code),
        Err(error) => {
            eprintln!("error: {error:#}");
            ExitCode::from(1)
        }
    }
}

fn run(cli: Cli) -> Result<u8> {
    let notify_update = !cli.json
        && cli.command.is_some()
        && !matches!(&cli.command, Some(Commands::Update { .. }));
    let code = match cli.command {
        None => {
            if cli.json {
                return Err(anyhow!("--json requires a subcommand"));
            }
            run_tui()?;
            Ok(0)
        }
        Some(Commands::Why { package }) => command_why(&package, cli.json),
        Some(Commands::Impact { packages }) => command_impact(&packages, cli.json),
        Some(Commands::Harmony { file }) => command_harmony(file.as_deref(), cli.json),
        Some(Commands::Blueprint { action }) => command_blueprint(action, cli.json),
        Some(Commands::Check(args)) => command_check(args, cli.json, false),
        Some(Commands::Checks(args)) => command_check(args, cli.json, true),
        Some(Commands::Timeline { since, limit, log }) => {
            command_timeline(since.as_deref(), limit, log.as_deref(), cli.json)
        }
        Some(Commands::History { limit, record_hook }) => {
            command_history(limit, record_hook, cli.json)
        }
        Some(Commands::RecordHistory) => command_record_history(cli.json),
        Some(Commands::Update { yes }) => command_update(yes, cli.json),
        Some(Commands::Cleanup { execute, yes, keep }) => {
            command_cleanup(execute, yes, &keep, cli.json)
        }
    }?;

    if notify_update {
        update::notify_if_update_available();
    }
    Ok(code)
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn load_config() -> Result<Config> {
    Config::load_default()
}

fn scan_system() -> Result<(Snapshot, Vec<String>)> {
    let result = source::load_snapshot_detailed(&source::default_pacman_root(), None)?;
    Ok((result.snapshot, result.warnings))
}

fn print_json(value: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

fn report_warnings(warnings: &[String], json: bool) {
    if !json {
        for warning in warnings {
            eprintln!("warning: {warning}");
        }
    }
}

fn protected_packages(config: &Config, extra: &[String]) -> Vec<String> {
    let mut protected: BTreeSet<String> =
        config::default_protected_packages().into_iter().collect();
    // `base` is part of the README's protected contract even when the compact
    // backend default list does not mention it.
    protected.insert("base".to_string());
    protected.extend(config.protected_packages.iter().cloned());
    protected.extend(extra.iter().cloned());
    protected.into_iter().collect()
}

fn package_names(graph: &Graph, indices: &[usize]) -> Vec<String> {
    indices
        .iter()
        .filter_map(|index| graph.package_name(*index).map(ToString::to_string))
        .collect()
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

fn format_timestamp(timestamp: u64) -> String {
    let days = (timestamp / 86_400) as i64;
    let seconds = timestamp % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} UTC",
        seconds / 3_600,
        (seconds % 3_600) / 60
    )
}

fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month as u32, day as u32)
}

fn confirm(prompt: &str, yes: bool, json: bool) -> Result<()> {
    if yes {
        return Ok(());
    }
    if !io::stdin().is_terminal() {
        return Err(anyhow!(
            "refusing to run a destructive command without confirmation; pass --yes"
        ));
    }

    if json {
        eprintln!("{prompt}");
        eprint!("Type 'yes' to continue: ");
        io::stderr().flush()?;
    } else {
        println!("{prompt}");
        print!("Type 'yes' to continue: ");
        io::stdout().flush()?;
    }

    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    if answer.trim().eq_ignore_ascii_case("yes") {
        Ok(())
    } else {
        Err(anyhow!("cancelled"))
    }
}

fn default_blueprint_name() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty())
        })
        .unwrap_or_else(|| "amph-blueprint".to_string())
}

// ---------------------------------------------------------------------------
// why / impact
// ---------------------------------------------------------------------------

fn command_why(name: &str, json: bool) -> Result<u8> {
    let (snapshot, warnings) = scan_system()?;
    report_warnings(&warnings, json);
    let graph = Graph::from_snapshot(&snapshot);
    let index = graph
        .index_of(name)
        .ok_or_else(|| anyhow!("package '{name}' is not installed"))?;
    let package = &graph.packages[index];
    let reachable = graph.reachable_from_explicit();
    let is_reachable = reachable.contains(&index);
    let paths = graph.shortest_paths_from_explicit();
    let path = paths
        .get(index)
        .and_then(|path| path.as_ref())
        .map(|path| package_names(&graph, path))
        .unwrap_or_default();
    let required_by: Vec<String> = graph
        .dependents
        .get(index)
        .map(|dependents| package_names(&graph, dependents))
        .unwrap_or_default();

    // Enumerate dependency paths from every explicit root, then retain the
    // prefix of each path that reaches the requested package.  The shortest
    // path above remains the primary explanation; this gives the complete
    // reachability story.
    const MAX_WHY_PATHS: usize = 512;
    let root_paths = graph.paths_from_explicit(MAX_WHY_PATHS);
    let paths_truncated = root_paths.len() >= MAX_WHY_PATHS;
    let all_paths: Vec<Vec<String>> = {
        let mut unique: BTreeSet<Vec<usize>> = BTreeSet::new();
        for full_path in &root_paths {
            if let Some(position) = full_path.iter().position(|candidate| *candidate == index) {
                unique.insert(full_path[..=position].to_vec());
            }
        }
        unique
            .into_iter()
            .map(|path| package_names(&graph, &path))
            .collect()
    };

    let reason = if package.explicit {
        "explicitly installed".to_string()
    } else if !is_reachable {
        "not reachable from an explicit root; possible orphan".to_string()
    } else if let Some(first) = path.first() {
        format!("required through {first}")
    } else {
        "installed as a dependency".to_string()
    };

    if json {
        print_json(&json!({
            "kind": "why",
            "scanned_at": snapshot.scanned_at,
            "scanned_at_display": format_timestamp(snapshot.scanned_at),
            "package": package.name,
            "version": package.version,
            "description": package.description,
            "size": package.size,
            "explicit": package.explicit,
            "foreign": package.foreign,
            "reason": reason,
            "reachable_from_explicit": is_reachable,
            "dependency_path": path,
            "paths": all_paths,
            "paths_truncated": paths_truncated,
            "required_by": required_by,
            "warnings": warnings,
        }))?;
    } else {
        println!(
            "{} {} ({})",
            package.name,
            package.version,
            human_bytes(package.size)
        );
        if !package.description.is_empty() {
            println!("  {}", package.description);
        }
        println!("  reason: {reason}");
        if path.len() > 1 {
            println!("  shortest path: {}", path.join(" -> "));
        }
        if all_paths.is_empty() {
            println!("  paths from explicit roots: none");
        } else {
            println!("  paths from explicit roots ({}):", all_paths.len());
            for candidate in &all_paths {
                println!("    {}", candidate.join(" -> "));
            }
            if paths_truncated {
                println!("    … path enumeration truncated at {MAX_WHY_PATHS}");
            }
        }
        if required_by.is_empty() {
            println!("  required by: none");
        } else {
            println!("  required by: {}", required_by.join(", "));
        }
    }
    Ok(0)
}

fn command_impact(names: &[String], json: bool) -> Result<u8> {
    let config = load_config()?;
    let (snapshot, warnings) = scan_system()?;
    report_warnings(&warnings, json);
    let graph = Graph::from_snapshot(&snapshot);
    let protected = protected_packages(&config, &[]);
    let protected_set: BTreeSet<&str> = protected.iter().map(String::as_str).collect();

    let not_found: Vec<String> = names
        .iter()
        .filter(|name| graph.index_of(name).is_none())
        .cloned()
        .collect();
    let (impact, skipped) = analysis::removal_impact_for(&graph, names, &protected);

    // Rank every unprotected explicit package by the space its removal frees.
    // This is the "what can I actually reclaim?" companion to the combined
    // impact above.  Implicit dependencies are deliberately excluded: only an
    // explicitly installed package is a legitimate removal root.
    let mut ranked_explicit: Vec<(usize, graph::RemovalImpact)> = graph
        .packages
        .iter()
        .enumerate()
        .filter(|(_, package)| package.explicit && !protected_set.contains(package.name.as_str()))
        .map(|(index, _)| (index, graph.removal_impact(&[index])))
        .collect();
    ranked_explicit.sort_by(|a, b| {
        b.1.freed_bytes.cmp(&a.1.freed_bytes).then_with(|| {
            graph
                .package_name(a.0)
                .unwrap_or("")
                .cmp(graph.package_name(b.0).unwrap_or(""))
        })
    });

    if json {
        let impact_json = impact
            .as_ref()
            .map(|impact| removal_impact_json(&graph, impact))
            .unwrap_or(Value::Null);
        print_json(&json!({
            "kind": "impact",
            "scanned_at": snapshot.scanned_at,
            "scanned_at_display": format_timestamp(snapshot.scanned_at),
            "requested": names,
            "impact": impact_json,
            "not_found": not_found,
            "protected_skipped": skipped,
            "ranked_explicit": ranked_explicit.iter().map(|(index, impact)| {
                let name = graph.package_name(*index).unwrap_or("").to_string();
                json!({
                    "package": name,
                    "freed_bytes": impact.freed_bytes,
                    "freed_display": human_bytes(impact.freed_bytes),
                    "removed_count": impact.removed.len(),
                    "exclusive_closure_count": impact.exclusive_closure.len(),
                    "surviving_consumers_count": impact.surviving_consumers.len(),
                    "safe": impact.is_safe(),
                })
            }).collect::<Vec<_>>(),
            "warnings": warnings,
        }))?;
    } else {
        if !not_found.is_empty() {
            eprintln!("warning: not installed: {}", not_found.join(", "));
        }
        if !skipped.is_empty() {
            eprintln!("warning: protected and skipped: {}", skipped.join(", "));
        }
        match impact {
            Some(impact) => print_removal_impact(&graph, &impact),
            None => println!("Nothing to remove."),
        }
        print_explicit_ranking(&graph, &ranked_explicit);
    }

    Ok(0)
}

fn print_explicit_ranking(graph: &Graph, ranked: &[(usize, graph::RemovalImpact)]) {
    if ranked.is_empty() {
        return;
    }
    println!();
    println!("Explicit packages ranked by reclaimable space");
    let mut table = new_table();
    table.set_header(vec!["Package", "Freed", "Removed", "Surviving consumers"]);
    for (index, impact) in ranked {
        let name = graph.package_name(*index).unwrap_or("").to_string();
        table.add_row(vec![
            Cell::new(name),
            Cell::new(human_bytes(impact.freed_bytes)),
            Cell::new(impact.removed.len().to_string()),
            Cell::new(impact.surviving_consumers.len().to_string()),
        ]);
    }
    println!("{table}");
}

fn removal_impact_json(graph: &Graph, impact: &graph::RemovalImpact) -> Value {
    json!({
        "requested": package_names(graph, &impact.requested),
        "removed": package_names(graph, &impact.removed),
        "exclusive_closure": package_names(graph, &impact.exclusive_closure),
        "surviving_consumers": impact.surviving_consumers.iter().map(|consumer| json!({
            "package": consumer.name,
            "requirements": consumer.requirements,
            "optional": consumer.optional,
        })).collect::<Vec<_>>(),
        "optional_consumers": impact.optional_consumers.iter().map(|consumer| json!({
            "package": consumer.name,
            "requirements": consumer.requirements,
            "optional": consumer.optional,
        })).collect::<Vec<_>>(),
        "freed_bytes": impact.freed_bytes,
        "safe": impact.is_safe(),
    })
}

fn print_removal_impact(graph: &Graph, impact: &graph::RemovalImpact) {
    println!("Removal preview");
    println!(
        "  removed: {} package(s), {}",
        impact.removed.len(),
        human_bytes(impact.freed_bytes)
    );
    println!("    {}", package_names(graph, &impact.removed).join(", "));
    if !impact.exclusive_closure.is_empty() {
        println!(
            "  exclusive dependency closure: {}",
            package_names(graph, &impact.exclusive_closure).join(", ")
        );
    }
    if impact.surviving_consumers.is_empty() {
        println!("  surviving consumers: none");
    } else {
        println!("  surviving consumers (would break):");
        for consumer in &impact.surviving_consumers {
            println!(
                "    {} requires {}",
                consumer.name,
                consumer.requirements.join(", ")
            );
        }
    }
    if !impact.optional_consumers.is_empty() {
        println!("  optional consumers:");
        for consumer in &impact.optional_consumers {
            println!(
                "    {} optionally uses {}",
                consumer.name,
                consumer.requirements.join(", ")
            );
        }
    }
}

// ---------------------------------------------------------------------------
// harmony
// ---------------------------------------------------------------------------

fn command_harmony(file: Option<&Path>, json: bool) -> Result<u8> {
    let (snapshot, warnings) = scan_system()?;
    report_warnings(&warnings, json);
    let config = load_harmony(file)?;
    let suggestions = config.suggest(&snapshot.packages);
    let categories = config.categorize(&snapshot.packages);

    if json {
        print_json(&json!({
            "kind": "harmony",
            "scanned_at": snapshot.scanned_at,
            "scanned_at_display": format_timestamp(snapshot.scanned_at),
            "categories": config.categories.iter().map(|category| json!({
                "id": category.id,
                "name": category.name,
                "description": category.description,
                "priority": category.priority,
                "packages": category.packages,
                "installed": categories.iter().filter(|(_, ids)| ids.iter().any(|id| id == &category.id)).map(|(name, _)| name).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "suggestions": suggestions.iter().map(suggestion_json).collect::<Vec<_>>(),
            "warnings": warnings,
        }))?;
    } else {
        let mut table = new_table();
        table.set_header(vec!["Category", "Installed", "Suggested"]);
        for category in &config.categories {
            let installed = categories
                .iter()
                .filter(|(_, ids)| ids.iter().any(|id| id == &category.id))
                .map(|(name, _)| name.clone())
                .collect::<Vec<_>>();
            let suggested = suggestions
                .iter()
                .filter(|suggestion| suggestion.category.as_deref() == Some(&category.id))
                .map(|suggestion| suggestion.package.clone())
                .collect::<Vec<_>>();
            table.add_row(vec![
                Cell::new(&category.name),
                Cell::new(if installed.is_empty() {
                    "-".to_string()
                } else {
                    installed.join(", ")
                }),
                Cell::new(if suggested.is_empty() {
                    "-".to_string()
                } else {
                    suggested.join(", ")
                }),
            ]);
        }
        println!("{table}");
        if suggestions.is_empty() {
            println!("No Harmony suggestions for the installed package set.");
        }
    }
    Ok(0)
}

fn suggestion_json(suggestion: &Suggestion) -> Value {
    json!({
        "package": suggestion.package,
        "category": suggestion.category,
        "reason": suggestion.reason,
        "weight": suggestion.weight,
        "source": suggestion.source,
    })
}

fn load_harmony(file: Option<&Path>) -> Result<HarmonyConfig> {
    let mut base = harmony::default_harmony();
    let custom = match file {
        Some(path) => Some(harmony::load(path)?),
        None => Config::default_path()
            .map(|path| path.with_file_name("harmony.toml"))
            .filter(|path| path.is_file())
            .map(|path| harmony::load(&path))
            .transpose()?,
    };
    if let Some(custom) = custom {
        merge_harmony(&mut base, &custom);
    }
    Ok(base)
}

fn merge_harmony(base: &mut HarmonyConfig, extra: &HarmonyConfig) {
    for category in &extra.categories {
        match base
            .categories
            .iter_mut()
            .find(|existing| existing.id == category.id)
        {
            Some(existing) => *existing = category.clone(),
            None => base.categories.push(category.clone()),
        }
    }
    for rule in &extra.rules {
        match base
            .rules
            .iter_mut()
            .find(|existing| existing.id == rule.id)
        {
            Some(existing) => *existing = rule.clone(),
            None => base.rules.push(rule.clone()),
        }
    }
    base.categories
        .sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.id.cmp(&b.id)));
    base.rules.sort_by(|a, b| a.id.cmp(&b.id));
}

// ---------------------------------------------------------------------------
// blueprint
// ---------------------------------------------------------------------------

fn command_blueprint(action: BlueprintAction, json: bool) -> Result<u8> {
    match action {
        BlueprintAction::Export { output, name } => blueprint_export(output, name, json),
        BlueprintAction::Diff { file } => blueprint_diff(&file, json),
        BlueprintAction::Apply {
            file,
            execute,
            remove_extra,
            yes,
        } => blueprint_apply(&file, execute, remove_extra, yes, json),
    }
}

fn blueprint_export(output: Option<PathBuf>, name: Option<String>, json: bool) -> Result<u8> {
    let (snapshot, warnings) = scan_system()?;
    report_warnings(&warnings, json);
    let name = name.unwrap_or_else(default_blueprint_name);
    let blueprint = blueprint::export(&snapshot, &name);
    if let Some(path) = &output {
        blueprint::save(&blueprint, path)?;
    }

    if json {
        print_json(&json!({
            "kind": "blueprint",
            "action": "export",
            "scanned_at": snapshot.scanned_at,
            "scanned_at_display": format_timestamp(snapshot.scanned_at),
            "output": output.as_ref().map(|path| path.display().to_string()),
            "blueprint": blueprint_json(&blueprint),
            "warnings": warnings,
        }))?;
    } else if output.is_none() {
        print!("{}", blueprint::to_toml(&blueprint));
    } else if let Some(path) = output {
        println!(
            "wrote blueprint {} ({} packages)",
            path.display(),
            blueprint.packages.len()
        );
    }
    Ok(0)
}

fn blueprint_diff(file: &Path, json: bool) -> Result<u8> {
    let (snapshot, warnings) = scan_system()?;
    report_warnings(&warnings, json);
    let blueprint = blueprint::load(file)?;
    let difference = blueprint::diff(&snapshot, &blueprint);

    if json {
        print_json(&json!({
            "kind": "blueprint",
            "action": "diff",
            "scanned_at": snapshot.scanned_at,
            "scanned_at_display": format_timestamp(snapshot.scanned_at),
            "file": file.display().to_string(),
            "blueprint": blueprint.name,
            "diff": blueprint_diff_json(&difference),
            "warnings": warnings,
        }))?;
    } else if difference.is_empty() {
        println!("Blueprint matches the installed package set.");
    } else {
        println!("Blueprint diff for {} ({})", blueprint.name, file.display());
        if !difference.missing.is_empty() {
            println!("  missing: {}", difference.missing.join(", "));
        }
        if !difference.extra.is_empty() {
            println!("  extra: {}", difference.extra.join(", "));
        }
        for (package, installed, wanted) in &difference.version_changed {
            println!("  version: {package} {installed} -> {wanted}");
        }
        if !difference.to_explicit.is_empty() {
            println!("  mark explicit: {}", difference.to_explicit.join(", "));
        }
        if !difference.to_dependency.is_empty() {
            println!("  mark dependency: {}", difference.to_dependency.join(", "));
        }
    }
    Ok(0)
}

fn blueprint_apply(
    file: &Path,
    execute: bool,
    remove_extra: bool,
    yes: bool,
    json: bool,
) -> Result<u8> {
    let config = load_config()?;
    let (snapshot, warnings) = scan_system()?;
    report_warnings(&warnings, json);
    let blueprint = blueprint::load(file)?;
    let protected = protected_packages(&config, &blueprint.protected);

    let options = ApplyOptions {
        dry_run: true,
        remove_extra,
        noconfirm: true,
        protected,
    };
    let plan = blueprint::plan(&snapshot, &blueprint, &options);

    // Package metadata and the TUI's reason field are not a trust boundary.
    // Re-check every destructive target against default + configured + blueprint
    // protected packages immediately before any confirmation or execution.
    if let Some(blocked) = plan
        .remove
        .iter()
        .find(|name| options.protected.iter().any(|protected| protected == *name))
    {
        return Err(anyhow!(
            "refusing to remove protected package '{blocked}'; protected packages are never destructive targets"
        ));
    }

    if !execute {
        if json {
            print_json(&json!({
                "kind": "blueprint",
                "action": "apply",
                "scanned_at": snapshot.scanned_at,
                "scanned_at_display": format_timestamp(snapshot.scanned_at),
                "file": file.display().to_string(),
                "blueprint": blueprint.name,
                "execute": false,
                "dry_run": true,
                "plan": apply_plan_json(&plan),
                "warnings": warnings,
            }))?;
        } else {
            print_apply_plan(&plan);
            println!("Dry run. Re-run with --execute to apply.");
        }
        return Ok(0);
    }

    if plan.commands.is_empty() {
        if json {
            print_json(&json!({
                "kind": "blueprint",
                "action": "apply",
                "scanned_at": snapshot.scanned_at,
                "scanned_at_display": format_timestamp(snapshot.scanned_at),
                "file": file.display().to_string(),
                "blueprint": blueprint.name,
                "execute": true,
                "dry_run": false,
                "plan": apply_plan_json(&plan),
                "outcomes": [],
                "warnings": warnings,
            }))?;
        } else {
            println!("Blueprint already matches; no commands to run.");
        }
        return Ok(0);
    }

    confirm(
        &format!(
            "Apply blueprint '{}' with {} pacman command(s)?",
            blueprint.name,
            plan.commands.len()
        ),
        yes,
        json,
    )?;

    let outcomes = execute_apply_commands(&plan.commands, json)?;
    if json {
        print_json(&json!({
            "kind": "blueprint",
            "action": "apply",
            "scanned_at": snapshot.scanned_at,
            "scanned_at_display": format_timestamp(snapshot.scanned_at),
            "file": file.display().to_string(),
            "blueprint": blueprint.name,
            "execute": true,
            "dry_run": false,
            "plan": apply_plan_json(&plan),
            "outcomes": outcomes,
            "warnings": warnings,
        }))?;
    } else {
        println!("Blueprint applied.");
    }
    Ok(0)
}

fn blueprint_json(blueprint: &Blueprint) -> Value {
    json!({
        "version": blueprint.version,
        "name": blueprint.name,
        "generated_at": blueprint.generated_at,
        "explicit": blueprint.explicit,
        "protected": blueprint.protected,
        "packages": blueprint.packages.iter().map(|package| json!({
            "name": package.name,
            "version": package.version,
            "explicit": package.explicit,
            "reason": package.reason,
            "provides": package.provides,
            "depends": package.depends,
        })).collect::<Vec<_>>(),
    })
}

fn blueprint_diff_json(difference: &blueprint::BlueprintDiff) -> Value {
    json!({
        "missing": difference.missing,
        "extra": difference.extra,
        "version_changed": difference.version_changed.iter().map(|(package, installed, wanted)| json!({
            "package": package,
            "installed": installed,
            "blueprint": wanted,
        })).collect::<Vec<_>>(),
        "to_explicit": difference.to_explicit,
        "to_dependency": difference.to_dependency,
        "empty": difference.is_empty(),
    })
}

fn apply_plan_json(plan: &blueprint::ApplyPlan) -> Value {
    json!({
        "install": plan.install,
        "remove": plan.remove,
        "mark_explicit": plan.mark_explicit,
        "mark_dependency": plan.mark_dependency,
        "commands": plan.commands.iter().map(|command| json!({
            "command": command.display(),
            "description": command.description,
            "program": command.program,
            "args": command.args,
        })).collect::<Vec<_>>(),
        "warnings": plan.warnings,
        "dry_run": plan.dry_run,
        "noop": plan.is_noop(),
    })
}

fn print_apply_plan(plan: &blueprint::ApplyPlan) {
    println!("Blueprint plan");
    if plan.install.is_empty() {
        println!("  install: none");
    } else {
        println!("  install: {}", plan.install.join(", "));
    }
    if plan.remove.is_empty() {
        println!("  remove: none");
    } else {
        println!("  remove: {}", plan.remove.join(", "));
    }
    if !plan.mark_explicit.is_empty() {
        println!("  mark explicit: {}", plan.mark_explicit.join(", "));
    }
    if !plan.mark_dependency.is_empty() {
        println!("  mark dependency: {}", plan.mark_dependency.join(", "));
    }
    for warning in &plan.warnings {
        println!("  warning: {warning}");
    }
    for command in &plan.commands {
        println!("  $ sudo {}", command.display());
    }
    if plan.commands.is_empty() {
        println!("  no commands");
    }
}

fn execute_apply_commands(commands: &[ApplyCommand], json: bool) -> Result<Vec<Value>> {
    let mut outcomes = Vec::new();
    for command in commands {
        let mut process = Command::new("sudo");
        process
            .arg(&command.program)
            .args(&command.args)
            .env("LC_ALL", "C");
        if json {
            process
                .stdin(Stdio::inherit())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit());
        }
        let status = process
            .status()
            .with_context(|| format!("cannot run sudo {}", command.display()))?;
        outcomes.push(json!({
            "command": format!("sudo {}", command.display()),
            "description": command.description,
            "success": status.success(),
            "status": status.code(),
        }));
        if !status.success() {
            return Err(anyhow!(
                "command failed: sudo {} (status {:?})",
                command.display(),
                status.code()
            ));
        }
    }
    Ok(outcomes)
}

// ---------------------------------------------------------------------------
// checks
// ---------------------------------------------------------------------------

fn command_check(args: CheckArgs, json: bool, detailed: bool) -> Result<u8> {
    let config = load_config()?;
    let (snapshot, warnings) = scan_system()?;
    report_warnings(&warnings, json);
    let graph = Graph::from_snapshot(&snapshot);
    let mut options = analysis::AnalysisOptions::from_config(&config).checks;
    options.run_integrity = !args.no_integrity;
    let mut report = analysis::run_checks(&snapshot, &graph, &options);
    if args.no_integrity {
        // The backend emits an explicit Unknown placeholder for disabled
        // integrity checks. Remove it so a deliberate --no-integrity run does
        // not look like a warning to cron.
        report.results.retain(|result| result.id != "integrity");
    }
    let health = analysis::compute_health(
        &report,
        config.thresholds.health_warn,
        config.thresholds.health_critical,
    );
    let exit_code = check_exit_code(&report);

    if json {
        print_json(&json!({
            "kind": if detailed { "checks" } else { "check" },
            "generated_at": report.generated_at,
            "generated_at_display": format_timestamp(report.generated_at),
            "exit_code": exit_code,
            "status": overall_status(&report),
            "health": health_json(&health),
            "checks": report.results.iter().map(check_result_json).collect::<Vec<_>>(),
            "warnings": warnings,
        }))?;
    } else if detailed {
        print_checks_table(&report);
        print_check_details(&report);
        println!(
            "Health {:.1}/100 ({}) — {}",
            health.score, health.grade, health.formula
        );
    } else {
        println!(
            "{} — {} check(s), health {:.1}/100 ({})",
            overall_status(&report).to_uppercase(),
            report.results.len(),
            health.score,
            health.grade
        );
        for result in report.problems() {
            println!(
                "  [{}] {}: {}",
                result.status.as_str().to_uppercase(),
                result.title,
                result.summary
            );
        }
    }

    Ok(exit_code)
}

fn check_exit_code(report: &ChecksReport) -> u8 {
    if report.count(CheckStatus::Critical) > 0 {
        2
    } else if report.count(CheckStatus::Warning) > 0 || report.count(CheckStatus::Unknown) > 0 {
        1
    } else {
        0
    }
}

fn overall_status(report: &ChecksReport) -> &'static str {
    if report.count(CheckStatus::Critical) > 0 {
        "critical"
    } else if report.count(CheckStatus::Warning) > 0 {
        "warning"
    } else if report.count(CheckStatus::Unknown) > 0 {
        "unknown"
    } else {
        "ok"
    }
}

fn check_result_json(result: &analysis::checks::CheckResult) -> Value {
    json!({
        "id": result.id,
        "title": result.title,
        "category": result.category,
        "status": result.status.as_str(),
        "score": result.score,
        "weight": result.weight,
        "summary": result.summary,
        "details": result.details,
    })
}

fn health_json(health: &analysis::HealthReport) -> Value {
    json!({
        "score": health.score,
        "grade": health.grade,
        "formula": health.formula,
        "notes": health.notes,
        "components": health.components.iter().map(|component| json!({
            "id": component.id,
            "category": component.category,
            "title": component.title,
            "weight": component.weight,
            "score": component.score,
            "contributed": component.contributed,
            "status": component.status.as_str(),
            "check_ids": component.check_ids,
            "explanation": component.explanation,
        })).collect::<Vec<_>>(),
    })
}

fn print_checks_table(report: &ChecksReport) {
    let mut table = new_table();
    table.set_header(vec!["Check", "Status", "Weight", "Summary"]);
    for result in &report.results {
        table.add_row(vec![
            Cell::new(&result.title),
            Cell::new(result.status.as_str().to_uppercase()),
            Cell::new(format!("{:.2}", result.weight)),
            Cell::new(&result.summary),
        ]);
    }
    println!("{table}");
}

fn print_check_details(report: &ChecksReport) {
    for result in &report.results {
        if result.details.is_empty() {
            continue;
        }
        println!("{} ({}):", result.title, result.status.as_str());
        for detail in &result.details {
            println!("  - {detail}");
        }
    }
}

// ---------------------------------------------------------------------------
// update
// ---------------------------------------------------------------------------

fn command_update(yes: bool, json: bool) -> Result<u8> {
    let Some(available) = update::check_for_update()? else {
        if json {
            print_json(&json!({
                "kind": "update",
                "current": update::current_version(),
                "update_available": false,
                "installed": false,
                "branch": update::BRANCH,
                "repository": update::REPOSITORY,
            }))?;
        } else {
            println!(
                "amph v{} is up to date (GitHub branch {}).",
                update::current_version(),
                update::BRANCH
            );
        }
        return Ok(0);
    };

    if json && !yes {
        // `--json` is non-interactive: report the available update without
        // prompting or installing. Add --yes to install from a script.
        print_json(&json!({
            "kind": "update",
            "current": available.current,
            "latest": available.latest,
            "update_available": true,
            "installed": false,
            "branch": available.branch,
            "repository": available.repository,
        }))?;
        return Ok(0);
    }

    if !json {
        println!(
            "Update available: amph v{} -> v{} (GitHub branch {}).",
            available.current, available.latest, available.branch
        );
        println!(
            "This runs `cargo install --git {} --branch {} --locked --force`.",
            available.repository, available.branch
        );
        match confirm("Install this update now?", yes, false) {
            Ok(()) => {}
            Err(error) if error.to_string() == "cancelled" => {
                println!("Update cancelled.");
                return Ok(0);
            }
            Err(error) => return Err(error),
        }
    }

    if json {
        return match update::install_update() {
            Ok(()) => {
                print_json(&json!({
                    "kind": "update",
                    "current": available.current,
                    "latest": available.latest,
                    "update_available": true,
                    "installed": true,
                    "branch": available.branch,
                    "repository": available.repository,
                }))?;
                Ok(0)
            }
            Err(error) => {
                print_json(&json!({
                    "kind": "update",
                    "current": available.current,
                    "latest": available.latest,
                    "update_available": true,
                    "installed": false,
                    "branch": available.branch,
                    "repository": available.repository,
                    "error": format!("{error:#}"),
                }))?;
                Ok(1)
            }
        };
    }

    update::install_update()?;
    println!(
        "amph v{} installed. Restart any running amph sessions to use it.",
        available.latest
    );
    Ok(0)
}

// ---------------------------------------------------------------------------
// timeline
// ---------------------------------------------------------------------------

fn command_timeline(
    since: Option<&str>,
    limit: usize,
    log: Option<&Path>,
    json: bool,
) -> Result<u8> {
    let path = log
        .map(Path::to_path_buf)
        .unwrap_or_else(timeline::default_log_path);
    if !path.is_file() {
        return Err(anyhow!("pacman log not found: {}", path.display()));
    }
    let cutoff = since.map(parse_since).transpose()?;
    let mut transactions = timeline::load_timeline(&path)?;
    if let Some(cutoff) = cutoff {
        transactions.retain(|transaction| transaction.started_at >= cutoff);
    }
    if limit > 0 && transactions.len() > limit {
        transactions = transactions.split_off(transactions.len() - limit);
    }

    if json {
        print_json(&json!({
            "kind": "timeline",
            "log": path.display().to_string(),
            "since": cutoff,
            "since_display": cutoff.map(format_timestamp),
            "count": transactions.len(),
            "transactions": transactions.iter().map(transaction_json).collect::<Vec<_>>(),
        }))?;
    } else if transactions.is_empty() {
        println!("No pacman transactions found.");
    } else {
        let mut table = new_table();
        table.set_header(vec!["Started", "Packages", "Command / changes"]);
        for transaction in &transactions {
            let changes = transaction
                .actions
                .iter()
                .take(4)
                .map(action_label)
                .collect::<Vec<_>>()
                .join(", ");
            let suffix = if transaction.actions.len() > 4 {
                format!(" (+{} more)", transaction.actions.len() - 4)
            } else {
                String::new()
            };
            table.add_row(vec![
                Cell::new(format_timestamp(transaction.started_at)),
                Cell::new(transaction.actions.len().to_string()),
                Cell::new(
                    format!(
                        "{}{}{}",
                        transaction.command.as_deref().unwrap_or("transaction"),
                        if changes.is_empty() { "" } else { ": " },
                        changes
                    ) + &suffix,
                ),
            ]);
        }
        println!("{table}");
    }
    Ok(0)
}

fn transaction_json(transaction: &PacmanTransaction) -> Value {
    json!({
        "started_at": transaction.started_at,
        "started_at_display": format_timestamp(transaction.started_at),
        "finished_at": transaction.finished_at,
        "finished_at_display": transaction.finished_at.map(format_timestamp),
        "command": transaction.command,
        "duration_seconds": transaction.duration_seconds(),
        "actions": transaction.actions.iter().map(|action| json!({
            "name": action.name,
            "action": action.action.as_str(),
            "old_version": action.old_version,
            "new_version": action.new_version,
        })).collect::<Vec<_>>(),
    })
}

fn action_label(action: &timeline::LogPackageAction) -> String {
    match action.action {
        PackageAction::Install => format!("install {}", action.name),
        PackageAction::Upgrade => format!(
            "upgrade {} {} -> {}",
            action.name,
            action.old_version.as_deref().unwrap_or("?"),
            action.new_version.as_deref().unwrap_or("?")
        ),
        PackageAction::Downgrade => format!("downgrade {}", action.name),
        PackageAction::Reinstall => format!("reinstall {}", action.name),
        PackageAction::Remove => format!("remove {}", action.name),
    }
}

fn parse_since(value: &str) -> Result<u64> {
    let value = value.trim();
    if value.is_empty() {
        return Err(anyhow!("--since cannot be empty"));
    }

    if let Some(relative) = parse_relative_since(value) {
        return Ok(relative);
    }
    if let Some(timestamp) = timeline::parse_log_timestamp(value) {
        return Ok(timestamp);
    }
    if value.len() == 10
        && let Some(timestamp) = timeline::parse_log_timestamp(&format!("{value}T00:00:00+0000"))
    {
        return Ok(timestamp);
    }
    if value.contains('T')
        && !value.ends_with('Z')
        && !value.contains('+')
        && let Some(timestamp) = timeline::parse_log_timestamp(&format!("{value}+0000"))
    {
        return Ok(timestamp);
    }
    Err(anyhow!(
        "invalid --since value '{value}'; use a duration like 7d/24h or a date like 2026-01-01"
    ))
}

fn parse_relative_since(value: &str) -> Option<u64> {
    let (number, unit) = value.split_at(value.len().checked_sub(1)?);
    let amount: u64 = number.parse().ok()?;
    let seconds = match unit {
        "m" => 60,
        "h" => 3_600,
        "d" => 86_400,
        "w" => 604_800,
        _ => return None,
    };
    Some(model::unix_now().saturating_sub(amount.saturating_mul(seconds)))
}

// ---------------------------------------------------------------------------
// history
// ---------------------------------------------------------------------------

fn command_history(limit: Option<usize>, record_hook: bool, json: bool) -> Result<u8> {
    let config = load_config()?;
    if record_hook {
        return command_record_history_with_config(&config, json);
    }
    let limit = limit.unwrap_or(config.thresholds.max_history).max(1);
    let history = store::load_default(limit)?;

    if json {
        print_json(&json!({
            "kind": "history",
            "action": "list",
            "limit": history.limit,
            "entries": history.entries.iter().map(history_entry_json).collect::<Vec<_>>(),
        }))?;
    } else if history.entries.is_empty() {
        println!("No history entries yet.");
    } else {
        let mut table = new_table();
        table.set_header(vec![
            "When", "Packages", "Explicit", "Foreign", "Health", "Note",
        ]);
        for entry in &history.entries {
            table.add_row(vec![
                Cell::new(format_timestamp(entry.timestamp)),
                Cell::new(entry.packages.to_string()),
                Cell::new(entry.explicit.to_string()),
                Cell::new(entry.foreign.to_string()),
                Cell::new(format!("{:.1}", entry.health)),
                Cell::new(entry.note.clone().unwrap_or_default()),
            ]);
        }
        println!("{table}");
    }
    Ok(0)
}

fn command_record_history(json: bool) -> Result<u8> {
    let config = load_config()?;
    command_record_history_with_config(&config, json)
}

fn command_record_history_with_config(config: &Config, json: bool) -> Result<u8> {
    // The hook path must never invoke pacman. Reading the local description
    // database is enough for a compact post-transaction summary.
    let (snapshot, warnings) = scan_system()?;
    let entry = HistoryEntry {
        timestamp: model::unix_now(),
        packages: snapshot.packages.len(),
        explicit: snapshot.explicit_count(),
        foreign: snapshot.foreign_count(),
        health: 100.0,
        freed_bytes: 0,
        removed: 0,
        note: Some("pacman PostTransaction hook".to_string()),
    };
    store::record_default(entry.clone(), config.thresholds.max_history)?;

    if json {
        print_json(&json!({
            "kind": "history",
            "action": "record",
            "entry": history_entry_json(&entry),
            "warnings": warnings,
        }))?;
    } else {
        println!(
            "recorded history entry: {} packages, {} explicit, {} foreign",
            entry.packages, entry.explicit, entry.foreign
        );
    }
    Ok(0)
}

fn history_entry_json(entry: &HistoryEntry) -> Value {
    json!({
        "timestamp": entry.timestamp,
        "when": format_timestamp(entry.timestamp),
        "packages": entry.packages,
        "explicit": entry.explicit,
        "foreign": entry.foreign,
        "health": entry.health,
        "freed_bytes": entry.freed_bytes,
        "removed": entry.removed,
        "note": entry.note,
    })
}

// ---------------------------------------------------------------------------
// cleanup
// ---------------------------------------------------------------------------

fn command_cleanup(execute: bool, yes: bool, keep: &[String], json: bool) -> Result<u8> {
    let config = load_config()?;
    let (snapshot, warnings) = scan_system()?;
    report_warnings(&warnings, json);
    let graph = Graph::from_snapshot(&snapshot);
    let reachable = graph.reachable_from_explicit();
    let protected = protected_packages(&config, keep);
    let protected_set: BTreeSet<&str> = protected.iter().map(String::as_str).collect();

    let candidates: Vec<usize> = graph
        .packages
        .iter()
        .enumerate()
        .filter(|(index, package)| {
            !package.explicit
                && !reachable.contains(index)
                && !protected_set.contains(package.name.as_str())
        })
        .map(|(index, _)| index)
        .collect();
    let names = package_names(&graph, &candidates);

    // Never trust a candidate list assembled from UI metadata: re-check every
    // name against default + configured protection before pacman is invoked.
    ensure_not_protected(&names, &protected_set)?;

    let preview = if names.is_empty() {
        None
    } else {
        Some(run_pacman_print(&names)?)
    };

    let mut executed = false;
    let mut exit_status = 0i32;
    if execute && !names.is_empty() {
        if let Some(preview) = &preview
            && !preview.success
        {
            return Err(anyhow!(
                "pacman -Rns --print failed; refusing to execute: {}",
                preview.stderr
            ));
        }
        let command_text = format!("sudo pacman -Rns --noconfirm {}", names.join(" "));
        confirm(
            &format!(
                "About to run:\n  {command_text}\nThis removes {} package(s).",
                names.len()
            ),
            yes,
            json,
        )?;

        let mut command = Command::new("sudo");
        command
            .arg("pacman")
            .arg("-Rns")
            .arg("--noconfirm")
            .arg("--")
            .args(&names)
            .env("LC_ALL", "C");
        if json {
            command
                .stdin(Stdio::inherit())
                .stdout(Stdio::null())
                .stderr(Stdio::inherit());
        }
        let status = command.status().context("cannot run sudo pacman -Rns")?;
        executed = true;
        exit_status = status.code().unwrap_or(1);
    }

    if json {
        print_json(&json!({
            "kind": "cleanup",
            "scanned_at": snapshot.scanned_at,
            "scanned_at_display": format_timestamp(snapshot.scanned_at),
            "candidates": names,
            "candidate_sizes": candidates.iter().filter_map(|index| graph.packages.get(*index).map(|package| json!({
                "name": package.name,
                "size": package.size,
            }))).collect::<Vec<_>>(),
            "protected": protected,
            "preview": preview.as_ref().map(|preview| json!({
                "command": preview.command,
                "success": preview.success,
                "stdout": preview.stdout,
                "stderr": preview.stderr,
            })),
            "execute_requested": execute,
            "executed": executed,
            "status": if executed { Some(exit_status) } else { None },
            "warnings": warnings,
        }))?;
    } else {
        if names.is_empty() {
            println!("No orphan cleanup candidates found.");
        } else {
            println!("Cleanup candidates ({}):", names.len());
            for index in &candidates {
                if let Some(package) = graph.packages.get(*index) {
                    println!(
                        "  {} {} ({})",
                        package.name,
                        package.version,
                        human_bytes(package.size)
                    );
                }
            }
            if let Some(preview) = &preview {
                if !preview.stdout.is_empty() {
                    print!("{}", preview.stdout);
                    if !preview.stdout.ends_with('\n') {
                        println!();
                    }
                }
                if !preview.stderr.is_empty() {
                    eprint!("{}", preview.stderr);
                }
            }
            if executed {
                println!("Removal finished with status {exit_status}.");
            } else {
                println!("Dry run. Use --execute to confirm and run sudo pacman -Rns.");
            }
        }
    }

    if executed && exit_status != 0 {
        return Err(anyhow!("pacman removal failed with status {exit_status}"));
    }
    Ok(0)
}

/// Trust-boundary guard used by the CLI and any future TUI cleanup callback.
/// The TUI's reason metadata is display data only; every name is re-checked here
/// immediately before pacman is invoked.
fn ensure_not_protected(names: &[String], protected: &BTreeSet<&str>) -> Result<()> {
    if let Some(blocked) = names.iter().find(|name| protected.contains(name.as_str())) {
        return Err(anyhow!(
            "refusing cleanup because protected package '{blocked}' was selected"
        ));
    }
    Ok(())
}

struct PacmanPreview {
    command: String,
    success: bool,
    stdout: String,
    stderr: String,
}

fn run_pacman_print(names: &[String]) -> Result<PacmanPreview> {
    let command_text = format!("pacman -Rns --print -- {}", names.join(" "));
    let output = Command::new("pacman")
        .arg("-Rns")
        .arg("--print")
        .arg("--")
        .args(names)
        .env("LC_ALL", "C")
        .output()
        .context("cannot run pacman -Rns --print")?;
    Ok(PacmanPreview {
        command: command_text,
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn tui_cleanup_handler(config: &Config) -> impl tui::CleanupHandler {
    let protected: BTreeSet<String> = protected_packages(config, &[]).into_iter().collect();
    let mut approved_preview: Option<Vec<String>> = None;

    move |request: tui::CleanupRequest| -> tui::CleanupHandlerResult {
        let names = request.packages();
        let fail = |message: String| tui::CleanupError::new(message);
        if names.is_empty() {
            return Err(fail("select at least one package".to_string()));
        }
        if let Some(package) = names.iter().find(|name| protected.contains(*name)) {
            approved_preview = None;
            return Err(fail(format!(
                "refusing cleanup: protected package '{package}' is selected"
            )));
        }

        // Package lists are refreshed for both phases: a stale TUI selection
        // must never turn into a pacman argument after the local DB changes.
        let current = source::load_local_snapshot(None)
            .map_err(|error| fail(format!("cannot refresh installed packages: {error:#}")))?;
        let installed: BTreeSet<&str> = current
            .packages
            .iter()
            .map(|package| package.name.as_str())
            .collect();
        if let Some(package) = names.iter().find(|name| !installed.contains(name.as_str())) {
            approved_preview = None;
            return Err(fail(format!(
                "package '{package}' is no longer installed; refresh the package list"
            )));
        }

        match request {
            tui::CleanupRequest::Preview { packages } => {
                let preview = run_pacman_print(&packages)
                    .map_err(|error| fail(format!("cannot preview removal: {error:#}")))?;
                approved_preview = preview.success.then(|| packages.clone());
                let output = combine_command_output(&preview.stdout, &preview.stderr);
                Ok(tui::CleanupResponse::Preview(tui::CleanupPreview::new(
                    if output.is_empty() {
                        "Pacman returned an empty removal preview.".to_string()
                    } else {
                        output
                    },
                    preview.success,
                )))
            }
            tui::CleanupRequest::Remove { packages } => {
                if approved_preview.as_deref() != Some(packages.as_slice()) {
                    return Err(fail(
                        "removal refused: run and review a fresh preview for this exact selection"
                            .to_string(),
                    ));
                }
                let refreshed = run_pacman_print(&packages)
                    .map_err(|error| fail(format!("cannot refresh removal preview: {error:#}")))?;
                if !refreshed.success {
                    approved_preview = None;
                    return Ok(tui::CleanupResponse::Removed(tui::CleanupOutcome::new(
                        false,
                        combine_command_output(&refreshed.stdout, &refreshed.stderr),
                    )));
                }

                let status = Command::new("sudo")
                    .arg("pacman")
                    .arg("-Rns")
                    .arg("--noconfirm")
                    .arg("--")
                    .args(&packages)
                    .env("LC_ALL", "C")
                    .status()
                    .map_err(|error| fail(format!("cannot run sudo pacman -Rns: {error}")))?;
                approved_preview = None;
                Ok(tui::CleanupResponse::Removed(tui::CleanupOutcome::new(
                    status.success(),
                    format!("sudo pacman -Rns finished with status {status}."),
                )))
            }
        }
    }
}

fn combine_command_output(stdout: &str, stderr: &str) -> String {
    match (stdout.is_empty(), stderr.is_empty()) {
        (true, true) => String::new(),
        (false, true) => stdout.to_string(),
        (true, false) => stderr.to_string(),
        (false, false) => format!("{stdout}\n{stderr}"),
    }
}

// ---------------------------------------------------------------------------
// TUI
// ---------------------------------------------------------------------------

fn run_tui() -> Result<()> {
    let config = match Config::load_default() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("warning: could not load config ({error:#}); using default palette");
            Config::default()
        }
    };
    let theme = match theme_kind_from_palette(&config.theme.palette) {
        Some(theme) => theme,
        None => {
            eprintln!(
                "warning: unknown theme.palette '{}'; using catppuccin",
                config.theme.palette
            );
            tui::ThemeKind::Catppuccin
        }
    };

    let cleanup = tui_cleanup_handler(&config);
    let installer = |_request: tui::UpdateRequest| match update::install_update() {
        Ok(()) => Ok(tui::UpdateOutcome::new(
            true,
            "Update installed. Restart AMPH to load the new version.",
        )),
        Err(error) => Err(tui::UpdateError::new(format!("{error:#}"))),
    };
    tui::run_with_theme_and_update(tui_scan, theme, cleanup, installer)
}

fn theme_kind_from_palette(value: &str) -> Option<tui::ThemeKind> {
    match value.trim().to_ascii_lowercase().as_str() {
        "catppuccin" | "mocha" => Some(tui::ThemeKind::Catppuccin),
        "tokyo" | "tokyo-night" | "tokyonight" => Some(tui::ThemeKind::TokyoNight),
        "gruvbox" => Some(tui::ThemeKind::Gruvbox),
        "nord" => Some(tui::ThemeKind::Nord),
        _ => None,
    }
}

fn tui_scan(tx: std::sync::mpsc::Sender<tui::ScanMessage>) {
    let update_tx = tx.clone();
    std::thread::spawn(move || {
        if let Ok(Some(found)) = update::check_for_update() {
            let info = tui::UpdateInfo::new(found.current, found.latest)
                .with_url(update::REPOSITORY)
                .with_notes("Newer build available on GitHub main.");
            let _ = update_tx.send(tui::ScanMessage::update_available(info));
        }
    });

    let progress_tx = tx.clone();
    let progress = source::Progress::new(move |event| {
        let stage = match event.phase {
            source::ProgressPhase::LocalPackages => "Reading local package database",
            source::ProgressPhase::SyncDatabase => "Reading sync databases",
            source::ProgressPhase::Foreign => "Classifying foreign packages",
        };
        let detail = format!("{} / {} packages", event.done, event.total);
        let _ = progress_tx.send(tui::ScanMessage::progress_detail(
            stage,
            event.done,
            event.total,
            detail,
        ));
    });

    match source::load_snapshot_detailed(&source::default_pacman_root(), Some(&progress)) {
        Ok(result) => {
            for warning in &result.warnings {
                let _ = tx.send(tui::ScanMessage::warning(warning.clone()));
            }

            let _ = tx.send(tui::ScanMessage::progress(
                "Resolving dependency graph",
                0,
                1,
            ));
            let graph = Graph::from_snapshot(&result.snapshot);
            let reachable = graph.reachable_from_explicit();
            let _ = tx.send(tui::ScanMessage::progress(
                "Resolving dependency graph",
                1,
                1,
            ));

            let protected_names = match Config::load_default() {
                Ok(config) => protected_packages(&config, &[]),
                Err(error) => {
                    let _ = tx.send(tui::ScanMessage::warning(format!(
                        "could not load config ({error:#}); using default protected packages"
                    )));
                    config::default_protected_packages()
                }
            };
            let protected_set: BTreeSet<&str> =
                protected_names.iter().map(String::as_str).collect();

            let config = match Config::load_default() {
                Ok(config) => config,
                Err(error) => {
                    let _ = tx.send(tui::ScanMessage::warning(format!(
                        "could not load analysis configuration ({error:#}); using defaults"
                    )));
                    Config::default()
                }
            };
            let check_options = analysis::AnalysisOptions::from_config(&config).checks;
            let _ = tx.send(tui::ScanMessage::progress("Running system checks", 0, 1));
            let checks = analysis::run_checks(&result.snapshot, &graph, &check_options);
            let health = analysis::compute_health(
                &checks,
                config.thresholds.health_warn,
                config.thresholds.health_critical,
            );
            let _ = tx.send(tui::ScanMessage::progress("Running system checks", 1, 1));

            let history = match store::load_default(config.thresholds.max_history) {
                Ok(history) => history,
                Err(error) => {
                    let _ = tx.send(tui::ScanMessage::warning(format!(
                        "could not read stored scan history ({error:#})"
                    )));
                    store::History::new(config.thresholds.max_history)
                }
            };
            let mut health_history: Vec<u8> = history
                .entries
                .iter()
                .map(|entry| entry.health.round().clamp(0.0, 100.0) as u8)
                .collect();
            health_history.push(health.score.round().clamp(0.0, 100.0) as u8);
            let mut history_entry = HistoryEntry::new(result.snapshot.scanned_at);
            history_entry.packages = result.snapshot.packages.len();
            history_entry.explicit = result
                .snapshot
                .packages
                .iter()
                .filter(|package| package.explicit)
                .count();
            history_entry.foreign = result
                .snapshot
                .packages
                .iter()
                .filter(|package| package.foreign)
                .count();
            history_entry.health = health.score;
            if let Err(error) = store::record_default(history_entry, config.thresholds.max_history)
            {
                let _ = tx.send(tui::ScanMessage::warning(format!(
                    "could not save scan history ({error:#})"
                )));
            }

            let _ = tx.send(tui::ScanMessage::progress(
                "Reading pacman transaction history",
                0,
                1,
            ));
            let timeline_items = match timeline::load_timeline(&timeline::default_log_path()) {
                Ok(transactions) => transactions.iter().map(tui_timeline_item).collect(),
                Err(error) => {
                    let _ = tx.send(tui::ScanMessage::warning(format!(
                        "could not read pacman timeline ({error:#})"
                    )));
                    Vec::new()
                }
            };
            let _ = tx.send(tui::ScanMessage::progress(
                "Reading pacman transaction history",
                1,
                1,
            ));

            let packages = result
                .snapshot
                .packages
                .iter()
                .enumerate()
                .map(|(index, package)| tui::Package {
                    name: package.name.clone(),
                    version: package.version.clone(),
                    description: package.description.clone(),
                    size: package.size,
                    explicit: package.explicit,
                    reason: tui_reason(package, &reachable, index, &protected_set),
                    depends: package.depends.clone(),
                    provides: package.provides.clone(),
                    optdepends: package.optdepends.clone(),
                    foreign: package.foreign,
                })
                .collect();

            let snapshot =
                tui::Snapshot::new(packages, format_timestamp(result.snapshot.scanned_at))
                    .with_health_history(health_history)
                    .with_timeline(timeline_items);
            let _ = tx.send(tui::ScanMessage::snapshot(snapshot));
            let _ = tx.send(tui::ScanMessage::finished());
        }
        Err(error) => {
            let _ = tx.send(tui::ScanMessage::failed(format!("{error:#}")));
            let _ = tx.send(tui::ScanMessage::finished());
        }
    }
}

fn tui_timeline_item(transaction: &timeline::PacmanTransaction) -> tui::TimelineItem {
    let kind = transaction
        .actions
        .first()
        .map(|action| match action.action {
            timeline::PackageAction::Install => tui::TimelineKind::Install,
            timeline::PackageAction::Upgrade => tui::TimelineKind::Upgrade,
            timeline::PackageAction::Downgrade => tui::TimelineKind::Downgrade,
            timeline::PackageAction::Reinstall => tui::TimelineKind::Reinstall,
            timeline::PackageAction::Remove => tui::TimelineKind::Remove,
        })
        .unwrap_or(tui::TimelineKind::Other);
    let title = transaction
        .command
        .clone()
        .unwrap_or_else(|| "Pacman transaction".to_string());
    let detail = transaction
        .actions
        .iter()
        .map(|action| {
            let version = action
                .new_version
                .as_deref()
                .or(action.old_version.as_deref())
                .map(|version| format!(" ({version})"))
                .unwrap_or_default();
            format!("{} {}{version}", action.action.as_str(), action.name)
        })
        .collect::<Vec<_>>()
        .join(" · ");
    tui::TimelineItem::new(
        format_timestamp(transaction.started_at),
        title,
        detail,
        kind,
    )
}

fn tui_reason(
    package: &model::Package,
    reachable: &BTreeSet<usize>,
    index: usize,
    protected: &BTreeSet<&str>,
) -> String {
    if protected.contains(package.name.as_str()) {
        // The TUI only offers reason strings containing orphan/unused/removable
        // as cleanup candidates. "protected" is a defensive filter in addition
        // to the CLI executor re-check.
        "protected".to_string()
    } else if package.explicit {
        "explicit".to_string()
    } else if reachable.contains(&index) {
        "dependency".to_string()
    } else {
        "orphan".to_string()
    }
}

// ---------------------------------------------------------------------------
// presentation
// ---------------------------------------------------------------------------

fn new_table() -> Table {
    let mut table = Table::new();
    table
        .load_preset(UTF8_FULL)
        .set_content_arrangement(ContentArrangement::Dynamic);
    table
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::checks::{CheckResult, CheckStatus, ChecksReport};

    #[test]
    fn timestamps_convert_between_backend_u64_and_tui_string() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(format_timestamp(0), "1970-01-01 00:00 UTC");
        assert_eq!(format_timestamp(1_767_225_600), "2026-01-01 00:00 UTC");
        assert!(parse_since("7d").expect("relative") < model::unix_now());
        assert_eq!(parse_since("2026-01-01").expect("date"), 1_767_225_600);
    }

    #[test]
    fn json_is_a_boolean_flag_not_an_optional_value() {
        let cli = Cli::try_parse_from(["amph", "why", "bash", "--json"]).expect("json flag");
        assert!(cli.json);
        assert!(Cli::try_parse_from(["amph", "--json=1", "why", "bash"]).is_err());
    }

    #[test]
    fn update_command_parses_with_yes_flag() {
        let cli = Cli::try_parse_from(["amph", "update", "--yes"]).expect("update command");
        assert!(matches!(cli.command, Some(Commands::Update { yes: true })));

        let cli = Cli::try_parse_from(["amph", "update"]).expect("update command");
        assert!(matches!(cli.command, Some(Commands::Update { yes: false })));
    }

    #[test]
    fn hook_aliases_parse() {
        let cli = Cli::try_parse_from(["amph", "history", "--record-hook"]).expect("flag alias");
        assert!(matches!(
            cli.command,
            Some(Commands::History {
                record_hook: true,
                ..
            })
        ));

        let cli = Cli::try_parse_from(["amph", "record-history"]).expect("subcommand alias");
        assert!(matches!(cli.command, Some(Commands::RecordHistory)));
    }

    #[test]
    fn check_exit_status_contract_is_stable() {
        let report = |status| {
            ChecksReport::new(vec![CheckResult::new(
                "test", "Test", "test", status, 1.0, "summary",
            )])
        };
        assert_eq!(check_exit_code(&report(CheckStatus::Ok)), 0);
        assert_eq!(check_exit_code(&report(CheckStatus::Warning)), 1);
        assert_eq!(check_exit_code(&report(CheckStatus::Unknown)), 1);
        assert_eq!(check_exit_code(&report(CheckStatus::Critical)), 2);
    }

    #[test]
    fn config_palette_names_map_to_tui_themes() {
        assert_eq!(
            theme_kind_from_palette("catppuccin"),
            Some(tui::ThemeKind::Catppuccin)
        );
        assert_eq!(
            theme_kind_from_palette("tokyo-night"),
            Some(tui::ThemeKind::TokyoNight)
        );
        assert_eq!(
            theme_kind_from_palette("gruvbox"),
            Some(tui::ThemeKind::Gruvbox)
        );
        assert_eq!(theme_kind_from_palette("Nord"), Some(tui::ThemeKind::Nord));
        assert_eq!(theme_kind_from_palette("unknown"), None);
    }

    #[test]
    fn protected_packages_are_never_tui_cleanup_candidates() {
        let package = model::Package {
            name: "linux".to_string(),
            explicit: false,
            ..model::Package::default()
        };
        let reachable = BTreeSet::new();
        let protected: BTreeSet<&str> = ["linux"].into_iter().collect();
        let reason = tui_reason(&package, &reachable, 0, &protected);
        assert_eq!(reason, "protected");
        assert!(!reason.contains("orphan"));
    }
}
