use anyhow::Result;
use clap::{Parser, Subcommand};
use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;

pub mod analyzer;
pub mod cleanup;
pub mod deps;
pub mod pacman;
pub mod report;
pub mod scan;
pub mod stats;
pub mod storage;

use pacman::SystemSnapshot;

#[derive(Parser)]
#[command(
    name = "amph",
    author,
    version,
    about = "Arch Meta Package Harmonizer",
    long_about = "A practical Arch Linux package management assistant for understanding installed packages, cleaning safely, analyzing disk usage, detecting issues, and generating reports."
)]
struct Cli {
    #[arg(long)]
    no_banner: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    Scan {
        #[arg(long)]
        json: Option<String>,
        #[arg(long)]
        markdown: Option<String>,
    },
    Analyze {
        #[arg(long)]
        json: Option<String>,
        #[arg(long)]
        markdown: Option<String>,
    },
    Report {
        #[arg(long)]
        json: Option<String>,
        #[arg(long)]
        markdown: Option<String>,
    },
    Clean {
        #[arg(long)]
        json: Option<String>,
        #[arg(long)]
        markdown: Option<String>,
    },
    Stats,
    FullScan {
        #[arg(long)]
        json: Option<String>,
        #[arg(long)]
        markdown: Option<String>,
    },
    History,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if !cli.no_banner {
        report::print_header();
    }

    match cli.command {
        Commands::Scan { json, markdown } => run_scan(json, markdown)?,
        Commands::Analyze { json, markdown } => run_analyze(json, markdown)?,
        Commands::Report { json, markdown } => run_report(json, markdown)?,
        Commands::Clean { json, markdown } => run_clean(json, markdown)?,
        Commands::Stats => run_stats()?,
        Commands::FullScan { json, markdown } => run_full_scan(json, markdown)?,
        Commands::History => run_history()?,
    }

    Ok(())
}

fn run_scan(json: Option<String>, markdown: Option<String>) -> Result<()> {
    let snapshot = fetch_snapshot("Scanning packages...")?;
    let quick = scan::quick_scan(&snapshot);

    report::print_scan_brief(&quick);

    if let Some(path) = json {
        report::export_json(&quick, &path)?;
    }
    if let Some(path) = markdown {
        report::export_scan_markdown(&quick, &path)?;
    }

    Ok(())
}

fn run_clean(json: Option<String>, markdown: Option<String>) -> Result<()> {
    let snapshot = fetch_snapshot("Scanning cleanup candidates...")?;
    let pkg_stats = stats::compute_package_stats(&snapshot);
    let cleanup = cleanup::build_cleanup_assessment(&snapshot, &pkg_stats);

    report::print_clean_table(&cleanup);

    if let Some(path) = json {
        report::export_json(&cleanup, &path)?;
    }
    if let Some(path) = markdown {
        report::export_clean_markdown(&cleanup, &path)?;
    }

    Ok(())
}

fn run_stats() -> Result<()> {
    let snapshot = fetch_snapshot("Building package statistics...")?;
    let pkg_stats = stats::compute_package_stats(&snapshot);

    report::print_stats_table(&pkg_stats);

    Ok(())
}

fn run_analyze(json: Option<String>, markdown: Option<String>) -> Result<()> {
    let snapshot = fetch_snapshot("Scanning packages...")?;
    let analysis = run_deep_analysis(&snapshot, "Analyzing dependencies and disk usage...")?;

    report::print_deep_analysis(&analysis);

    if let Some(path) = json {
        report::export_json(&analysis, &path)?;
    }
    if let Some(path) = markdown {
        report::export_full_markdown(&analysis, &path)?;
    }
    storage::record_scan(&analysis)?;

    Ok(())
}


fn run_report(json: Option<String>, markdown: Option<String>) -> Result<()> {
    let snapshot = fetch_snapshot("Generating system diagnostic report...")?;
    let analysis = run_deep_analysis(&snapshot, "Analyzing dependencies and disk usage...")?;

    report::print_full_report(&analysis);

    if let Some(path) = json {
        report::export_json(&analysis, &path)?;
    }
    if let Some(path) = markdown {
        report::export_full_markdown(&analysis, &path)?;
    }
    storage::record_scan(&analysis)?;

    Ok(())
}

fn run_full_scan(json: Option<String>, markdown: Option<String>) -> Result<()> {
    let snapshot = fetch_snapshot("Running full system scan...")?;
    let analysis = run_deep_analysis(&snapshot, "Aggregating health, disk usage and cleanup data...")?;

    report::print_dashboard(&analysis);

    if let Some(path) = json {
        report::export_json(&analysis, &path)?;
    }
    if let Some(path) = markdown {
        report::export_full_markdown(&analysis, &path)?;
    }
    storage::record_scan(&analysis)?;

    Ok(())
}

fn run_history() -> Result<()> {
    report::print_history_report(&storage::load_history()?)
}

fn fetch_snapshot(message: &str) -> Result<SystemSnapshot> {
    let spinner = spinner(message);
    let snapshot = spinner_wrap(&spinner, pacman::collect_snapshot);
    spinner.finish_and_clear();
    snapshot
}


fn run_deep_analysis(snapshot: &SystemSnapshot, message: &str) -> Result<analyzer::SystemAnalysis> {
    let spinner = spinner(message);
    let analysis = spinner_wrap(&spinner, || Ok(analyzer::analyze(snapshot)));
    spinner.finish_and_clear();
    analysis
}

fn spinner(message: &str) -> ProgressBar {
    let spinner = ProgressBar::new_spinner();
    let style = ProgressStyle::with_template("{spinner:.cyan} {msg}")
        .unwrap_or_else(|_| ProgressStyle::default_spinner())
        .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏");
    spinner.set_style(style);
    spinner.set_message(message.to_string());
    spinner.enable_steady_tick(Duration::from_millis(80));
    spinner
}

fn spinner_wrap<T>(spinner: &ProgressBar, work: impl FnOnce() -> Result<T>) -> Result<T> {
    let result = work();
    if result.is_err() {
        spinner.finish_and_clear();
    }
    result
}
