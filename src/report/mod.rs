use crate::analyzer::SystemAnalysis;
use crate::cleanup::{format_cleanup_line, CleanupAssessment};
use crate::scan::QuickScan;
use crate::storage::{compare_recent, HistoryDelta, HistoryFile};
use crate::stats::{format_bytes, format_signed_bytes, PackageSizeEntry, PackageStats, SizeBucket};
use anyhow::{Context, Result};
use colored::*;
use comfy_table::{presets::UTF8_NO_BORDERS, Cell, Color as TableColor, ContentArrangement, Table as ComfyTable};
use serde::Serialize;
use std::fs;
use tabled::{Table, Tabled};

const GIB: u64 = 1024 * 1024 * 1024;
const SHOW_TOP_N: usize = 5;

fn lean_table(headers: &[&str]) -> ComfyTable {
    let mut table = ComfyTable::new();
    table
        .load_preset(UTF8_NO_BORDERS)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(headers.to_vec());
    table
}

fn orphan_color(count: usize) -> TableColor {
    match count {
        0 => TableColor::Green,
        1..=4 => TableColor::Yellow,
        _ => TableColor::Red,
    }
}

fn cache_color(bytes: u64) -> TableColor {
    if bytes >= 10 * GIB {
        TableColor::Red
    } else if bytes >= 5 * GIB {
        TableColor::Yellow
    } else {
        TableColor::Green
    }
}

fn score_color(score: u8) -> TableColor {
    match score {
        90..=100 => TableColor::Green,
        70..=89 => TableColor::DarkGreen,
        50..=69 => TableColor::Yellow,
        25..=49 => TableColor::Red,
        _ => TableColor::DarkRed,
    }
}

pub fn print_scan_brief(scan: &QuickScan) {
    print_report_header("SCAN");

    let mut table = lean_table(&["Metric", "Value"]);
    table.add_row(vec![Cell::new("Packages"), Cell::new(scan.total_packages)]);
    table.add_row(vec![Cell::new("Explicit"), Cell::new(scan.explicit_packages)]);
    table.add_row(vec![Cell::new("Dependencies"), Cell::new(scan.dependency_packages)]);
    table.add_row(vec![
        Cell::new("Orphans"),
        Cell::new(scan.orphan_count).fg(orphan_color(scan.orphan_count)),
    ]);
    if let Some(cache) = scan.cache_size_bytes {
        table.add_row(vec![
            Cell::new("Pacman cache"),
            Cell::new(format_bytes(cache)).fg(cache_color(cache)),
        ]);
    }
    println!("{table}");

    println!();
    if scan.missing_dependencies.is_empty() {
        println!("{}", "✔ No missing dependencies detected.".green());
    } else {
        println!(
            "{}",
            format!("✖ {} missing dependency link(s):", scan.missing_dependencies.len())
                .red()
                .bold()
        );
        for issue in scan.missing_dependencies.iter().take(SHOW_TOP_N) {
            println!("  {} {} -> {}", "•".red(), issue.package.white(), issue.missing.bright_red());
        }
        if scan.missing_dependencies.len() > SHOW_TOP_N {
            println!(
                "{}",
                format!("  … and {} more (run `amph scan --json out.json` for the full list)", scan.missing_dependencies.len() - SHOW_TOP_N)
                    .bright_black()
            );
        }
    }
}

pub fn print_clean_table(cleanup: &CleanupAssessment) {
    print_report_header("CLEAN");

    let mut summary = lean_table(&["Metric", "Value"]);
    summary.add_row(vec![
        Cell::new("Orphans"),
        Cell::new(cleanup.orphans.len()).fg(orphan_color(cleanup.orphans.len())),
    ]);
    summary.add_row(vec![
        Cell::new("Unused dependencies"),
        Cell::new(cleanup.unnecessary_dependencies.len())
            .fg(orphan_color(cleanup.unnecessary_dependencies.len())),
    ]);
    if let Some(cache) = cleanup.cache_size_bytes {
        summary.add_row(vec![
            Cell::new("Pacman cache"),
            Cell::new(format_bytes(cache)).fg(cache_color(cache)),
        ]);
    }
    summary.add_row(vec![
        Cell::new("Reclaimable (packages)"),
        Cell::new(format_bytes(cleanup.reclaimable_bytes())).fg(TableColor::Green),
    ]);
    println!("{summary}");

    let candidates = cleanup.candidates();
    if candidates.is_empty() {
        println!();
        println!("{}", "No safe cleanup candidates detected.".green());
        return;
    }

    let shown = candidates.len().min(SHOW_TOP_N);
    println!();
    println!(
        "{}",
        format!("Top candidates ({} total, showing {})", candidates.len(), shown)
            .white()
            .bold()
    );

    let mut table = lean_table(&["Package", "Version", "Size", "Reason"]);
    for item in candidates.iter().take(SHOW_TOP_N) {
        table.add_row(vec![
            Cell::new(&item.name),
            Cell::new(&item.version),
            Cell::new(format_bytes(item.size_bytes)),
            Cell::new(&item.reason),
        ]);
    }
    println!("{table}");

    if candidates.len() > SHOW_TOP_N {
        println!(
            "{}",
            format!("… and {} more (run `amph clean --json out.json` for the full list)", candidates.len() - SHOW_TOP_N)
                .bright_black()
        );
    }

    println!();
    println!("{} {}", "Dry-run:".white().bold(), cleanup.dry_run_command.bright_black());
}

pub fn print_stats_table(stats: &PackageStats) {
    print_report_header("STATS");

    let ratio = stats
        .explicit_to_dependency_ratio()
        .map(|r| format!("{:.2} : 1", r))
        .unwrap_or_else(|| String::from("n/a"));

    let mut summary = lean_table(&["Metric", "Value"]);
    summary.add_row(vec![Cell::new("Total packages"), Cell::new(stats.total_packages)]);
    summary.add_row(vec![Cell::new("Total size"), Cell::new(format_bytes(stats.total_size_bytes))]);
    summary.add_row(vec![Cell::new("Average size"), Cell::new(format_bytes(stats.average_size_bytes))]);
    summary.add_row(vec![Cell::new("Median size"), Cell::new(format_bytes(stats.median_size_bytes))]);
    summary.add_row(vec![
        Cell::new("Explicit / Dependency"),
        Cell::new(format!(
            "{} / {}  ({ratio})",
            stats.explicit_packages, stats.dependency_packages
        )),
    ]);
    println!("{summary}");

    println!();
    println!("{}", "Size distribution".white().bold());
    let mut buckets = lean_table(&["Bucket", "Count", "Total"]);
    for bucket in &stats.size_buckets {
        buckets.add_row(vec![
            Cell::new(&bucket.label),
            Cell::new(bucket.count),
            Cell::new(format_bytes(bucket.total_bytes)),
        ]);
    }
    println!("{buckets}");

    println!();
    println!("{}", format!("Heaviest packages (top {SHOW_TOP_N})").white().bold());
    let mut heavy = lean_table(&["Package", "Version", "Size"]);
    for package in stats.largest_packages.iter().take(SHOW_TOP_N) {
        heavy.add_row(vec![
            Cell::new(&package.name),
            Cell::new(&package.version),
            Cell::new(format_bytes(package.size_bytes)),
        ]);
    }
    println!("{heavy}");
}

pub fn print_dashboard(analysis: &SystemAnalysis) {
    print_report_header("FULL SCAN DASHBOARD");

    println!("{}", "Health".white().bold());
    let mut health = lean_table(&["Metric", "Value"]);
    health.add_row(vec![
        Cell::new("Health score"),
        Cell::new(analysis.health_score).fg(score_color(analysis.health_score)),
    ]);
    health.add_row(vec![
        Cell::new("Bloat score"),
        Cell::new(analysis.bloat_score).fg(score_color(100u8.saturating_sub(analysis.bloat_score))),
    ]);
    health.add_row(vec![
        Cell::new("Unused dep. confidence"),
        Cell::new(&analysis.unused_dependency_confidence),
    ]);
    health.add_row(vec![
        Cell::new("Suspicious dep. chains"),
        Cell::new(analysis.dependencies.suspicious_chains.len()),
    ]);
    println!("{health}");

    println!();
    println!("{}", "Disk Usage".white().bold());
    let mut disk = lean_table(&["Metric", "Value"]);
    disk.add_row(vec![Cell::new("Packages"), Cell::new(analysis.stats.total_packages)]);
    disk.add_row(vec![
        Cell::new("Total size"),
        Cell::new(format_bytes(analysis.stats.total_size_bytes)),
    ]);
    if let Some(cache) = analysis.cleanup.cache_size_bytes {
        disk.add_row(vec![
            Cell::new("Pacman cache"),
            Cell::new(format_bytes(cache)).fg(cache_color(cache)),
        ]);
    }
    if let Some(heaviest) = analysis.stats.heavy_packages.first() {
        disk.add_row(vec![
            Cell::new("Heaviest package"),
            Cell::new(format!("{} ({})", heaviest.name, format_bytes(heaviest.size_bytes))),
        ]);
    }
    println!("{disk}");

    println!();
    println!("{}", "Cleanup Candidates".white().bold());
    let mut cleanup = lean_table(&["Metric", "Value"]);
    cleanup.add_row(vec![
        Cell::new("Orphans"),
        Cell::new(analysis.cleanup.orphans.len()).fg(orphan_color(analysis.cleanup.orphans.len())),
    ]);
    cleanup.add_row(vec![
        Cell::new("Unused dependencies"),
        Cell::new(analysis.cleanup.unnecessary_dependencies.len()),
    ]);
    cleanup.add_row(vec![
        Cell::new("Reclaimable space"),
        Cell::new(format_bytes(analysis.cleanup.reclaimable_bytes())).fg(TableColor::Green),
    ]);
    println!("{cleanup}");

    if let Some(top) = analysis.recommendations.first() {
        println!();
        println!("{} {}", "→".bright_cyan().bold(), top.white());
    }
}

pub fn export_json<T: Serialize>(value: &T, path: &str) -> Result<()> {
    write_serialized(path, value)
}

pub fn export_scan_markdown(scan: &QuickScan, path: &str) -> Result<()> {
    let mut markdown = String::from("# amph scan\n\n");
    markdown.push_str(&format!("- Packages: {}\n", scan.total_packages));
    markdown.push_str(&format!("- Explicit: {}\n", scan.explicit_packages));
    markdown.push_str(&format!("- Dependencies: {}\n", scan.dependency_packages));
    markdown.push_str(&format!("- Orphans: {}\n", scan.orphan_count));
    if let Some(cache) = scan.cache_size_bytes {
        markdown.push_str(&format!("- Pacman cache: {}\n", format_bytes(cache)));
    }
    markdown.push_str(&format!(
        "- Missing dependency links: {}\n",
        scan.missing_dependencies.len()
    ));
    fs::write(path, markdown).with_context(|| format!("Failed to write markdown report to {}", path))?;
    Ok(())
}

pub fn export_clean_markdown(cleanup: &CleanupAssessment, path: &str) -> Result<()> {
    let mut markdown = String::from("# amph clean\n\n");
    markdown.push_str(&format!("- Orphans: {}\n", cleanup.orphans.len()));
    markdown.push_str(&format!(
        "- Unused dependencies: {}\n",
        cleanup.unnecessary_dependencies.len()
    ));
    markdown.push_str(&format!(
        "- Reclaimable: {}\n\n",
        format_bytes(cleanup.reclaimable_bytes())
    ));
    markdown.push_str("## Candidates\n\n| Package | Version | Size | Reason |\n| --- | --- | --- | --- |\n");
    for item in cleanup.candidates() {
        markdown.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            item.name,
            item.version,
            format_bytes(item.size_bytes),
            item.reason
        ));
    }
    markdown.push_str(&format!("\nDry-run: `{}`\n", cleanup.dry_run_command));
    fs::write(path, markdown).with_context(|| format!("Failed to write markdown report to {}", path))?;
    Ok(())
}

#[derive(Tabled)]
struct PackageRow {
    #[tabled(rename = "Package")]
    package: String,
    #[tabled(rename = "Version")]
    version: String,
    #[tabled(rename = "Size")]
    size: String,
}

#[derive(Tabled)]
struct SimpleRow {
    #[tabled(rename = "Item")]
    item: String,
    #[tabled(rename = "Value")]
    value: String,
}

#[derive(Tabled)]
struct BucketRow {
    #[tabled(rename = "Bucket")]
    bucket: String,
    #[tabled(rename = "Count")]
    count: usize,
    #[tabled(rename = "Total Size")]
    total_size: String,
}

pub fn print_header() {
    let banner = r#"
  __     _  _    ____    _  _ 
 / _\   ( \/ )  (  _ \  / )( \
/    \  / \/ \   ) __/  ) __ (
\_/\_/  \_)(_/  (__)    \_)(_/
"#;

    println!("{}", banner.bright_cyan());
    println!("{}", "Arch Meta Package Harmonizer".bright_white().bold());
    println!("{}", "Practical Arch package analysis and maintenance assistant".bright_black());
    println!();
}

pub fn print_deep_analysis(analysis: &SystemAnalysis) {
    print_report_header("ANALYZE REPORT");
    print_system_health(analysis);
    print_stats(analysis);
    print_cleanup_summary(analysis);
    print_dependency_summary(analysis);
    print_system_warnings(analysis);
    print_recommendations(analysis);
}

pub fn print_full_report(analysis: &SystemAnalysis) {
    print_report_header("FULL REPORT");
    print_system_summary(analysis);
    print_system_health(analysis);
    print_stats(analysis);
    print_recent_packages(analysis);
    print_cleanup_summary(analysis);
    print_dependency_summary(analysis);
    print_system_warnings(analysis);
    print_recommendations(analysis);
}

pub fn print_history_report(history: &HistoryFile) -> Result<()> {
    print_report_header("HISTORY");

    if history.entries.is_empty() {
        println!("{}", "No saved scans found.".bright_black());
        return Ok(());
    }

    let rows: Vec<SimpleRow> = history
        .entries
        .iter()
        .rev()
        .take(20)
        .map(|entry| SimpleRow {
            item: entry.collected_at.clone(),
            value: format!(
                "{} packages | {} orphans | score {}",
                entry.analysis.stats.total_packages,
                entry.analysis.cleanup.orphans.len(),
                entry.analysis.health_score
            ),
        })
        .collect();

    println!("{}", Table::new(rows));

    if let Some(delta) = compare_recent(history) {
        println!();
        print_delta(&delta);
    }

    Ok(())
}

pub fn export_full_markdown(analysis: &SystemAnalysis, path: &str) -> Result<()> {
    let mut markdown = String::new();
    markdown.push_str("# amph report\n\n");
    markdown.push_str(&format!("- Generated at: {}\n", analysis.snapshot.generated_at));
    markdown.push_str(&format!("- Packages: {}\n", analysis.stats.total_packages));
    markdown.push_str(&format!("- Total size: {}\n", format_bytes(analysis.stats.total_size_bytes)));
    markdown.push_str(&format!("- Health score: {}\n\n", analysis.health_score));

    markdown.push_str("## Largest packages\n\n");
    markdown.push_str(&markdown_table_for_sizes(&analysis.stats.largest_packages));
    markdown.push_str("\n\n");

    markdown.push_str("## Cleanup candidates\n\n");
    if analysis.cleanup.safe_remove_candidates.is_empty() {
        markdown.push_str("No cleanup candidates detected.\n\n");
    } else {
        for name in &analysis.cleanup.safe_remove_candidates {
            markdown.push_str(&format!("- {}\n", name));
        }
        markdown.push_str("\n");
    }

    markdown.push_str("## Recommendations\n\n");
    for recommendation in &analysis.recommendations {
        markdown.push_str(&format!("- {}\n", recommendation));
    }

    fs::write(path, markdown).with_context(|| format!("Failed to write markdown report to {}", path))?;
    Ok(())
}

fn print_report_header(title: &str) {
    println!();
    println!("{}", title.bright_cyan().bold());
    println!("{}", "═".repeat(60).bright_black());
}

fn print_system_summary(analysis: &SystemAnalysis) {
    println!("{}", "📦 System Summary".bright_cyan().bold());
    println!("{}", "─".repeat(24).bright_black());

    let rows = vec![
        SimpleRow {
            item: String::from("Packages"),
            value: analysis.stats.total_packages.to_string(),
        },
        SimpleRow {
            item: String::from("Total size"),
            value: format_bytes(analysis.stats.total_size_bytes),
        },
        SimpleRow {
            item: String::from("Average size"),
            value: format_bytes(analysis.stats.average_size_bytes),
        },
        SimpleRow {
            item: String::from("Median size"),
            value: format_bytes(analysis.stats.median_size_bytes),
        },
        SimpleRow {
            item: String::from("Orphans"),
            value: analysis.cleanup.orphans.len().to_string(),
        },
        SimpleRow {
            item: String::from("Cleanup candidates"),
            value: analysis.cleanup.safe_remove_candidates.len().to_string(),
        },
        SimpleRow {
            item: String::from("Health score"),
            value: analysis.health_score.to_string(),
        },
    ];

    println!("{}", Table::new(rows));
}

fn print_system_health(analysis: &SystemAnalysis) {
    println!();
    println!("{}", "🧠 Health".bright_cyan().bold());
    println!("{}", "─".repeat(16).bright_black());

    let score_color = match analysis.health_score {
        90..=100 => analysis.health_score.to_string().green().bold(),
        70..=89 => analysis.health_score.to_string().bright_green().bold(),
        50..=69 => analysis.health_score.to_string().yellow().bold(),
        25..=49 => analysis.health_score.to_string().red().bold(),
        _ => analysis.health_score.to_string().bright_red().bold(),
    };

    let bloat_color = match analysis.bloat_score {
        0..=24 => analysis.bloat_score.to_string().green().bold(),
        25..=49 => analysis.bloat_score.to_string().yellow().bold(),
        50..=74 => analysis.bloat_score.to_string().red().bold(),
        _ => analysis.bloat_score.to_string().bright_red().bold(),
    };

    println!("{} {}", "Health score:".bright_white(), score_color);
    println!("{} {}", "Bloat score:".bright_white(), bloat_color);
    println!("{} {}", "Unused dependency confidence:".bright_white(), analysis.unused_dependency_confidence.bright_cyan());

    if let Some(cache_warning) = &analysis.cache_warning {
        println!("{} {}", "Cache warning:".bright_white(), cache_warning.bright_yellow());
    }

    if let Some(primary) = analysis.recommendations.first() {
        println!("{} {}", "Primary insight:".bright_white(), primary.white());
    }
}

fn print_stats(analysis: &SystemAnalysis) {
    println!();
    println!("{}", "💾 Disk Usage".bright_cyan().bold());
    println!("{}", "─".repeat(16).bright_black());
    println!("{}", "Package Size Breakdown".bright_white().bold());
    println!("{}", Table::new(size_bucket_rows(&analysis.stats.size_buckets)));

    println!();
    println!("{}", "Top Offenders".bright_white().bold());
    println!("{}", Table::new(largest_package_rows(&analysis.stats.largest_packages)));
}

fn print_recent_packages(analysis: &SystemAnalysis) {
    if analysis.stats.recent_packages.is_empty() {
        return;
    }

    println!();
    println!("{}", "Recently Installed / Updated".bright_white().bold());
    let rows: Vec<SimpleRow> = analysis
        .stats
        .recent_packages
        .iter()
        .map(|entry| SimpleRow {
            item: entry.name.clone(),
            value: format!("{} | {}", entry.action, entry.timestamp),
        })
        .collect();
    println!("{}", Table::new(rows));
}

fn print_cleanup_summary(analysis: &SystemAnalysis) {
    println!();
    println!("{}", "🧹 Cleanup Suggestions".bright_cyan().bold());
    println!("{}", "─".repeat(23).bright_black());

    if analysis.cleanup.safe_remove_candidates.is_empty() {
        println!("{}", "No safe removal candidates detected.".bright_black());
        return;
    }

    for item in analysis
        .cleanup
        .orphans
        .iter()
        .chain(analysis.cleanup.unnecessary_dependencies.iter())
    {
        println!("  {}", format_cleanup_line(item).bright_yellow());
    }

    println!();
    println!("{} {}", "Dry-run:".bright_white(), analysis.cleanup.dry_run_command.bright_black());
}

fn print_dependency_summary(analysis: &SystemAnalysis) {
    println!();
    println!("{}", "⚠️ Warnings".bright_cyan().bold());
    println!("{}", "─".repeat(15).bright_black());

    if analysis.dependencies.suspicious_chains.is_empty() {
        println!("{}", "No suspicious dependency chains detected.".bright_black());
        return;
    }

    for warning in &analysis.dependencies.suspicious_chains {
        println!("  {}", warning.bright_yellow());
    }
}

fn print_system_warnings(analysis: &SystemAnalysis) {
    println!();
    println!("{}", "⚠️ Warnings".bright_cyan().bold());
    println!("{}", "─".repeat(15).bright_black());

    if let Some(cache_warning) = &analysis.cache_warning {
        println!("  {}", cache_warning.bright_yellow());
    } else {
        println!("  {}", "Pacman cache looks acceptable.".bright_black());
    }

    if analysis.dependencies.suspicious_chains.is_empty() {
        println!("  {}", "No deep dependency chains detected.".bright_black());
    }
}

fn print_recommendations(analysis: &SystemAnalysis) {
    println!();
    println!("{}", "🧠 Analysis".bright_cyan().bold());
    println!("{}", "─".repeat(14).bright_black());
    for recommendation in &analysis.recommendations {
        println!("  {}", recommendation.bright_cyan());
    }
    println!();
}

fn print_delta(delta: &HistoryDelta) {
    let rows = vec![
        SimpleRow {
            item: String::from("Package delta"),
            value: format!("{} -> {}", delta.previous_packages, delta.current_packages),
        },
        SimpleRow {
            item: String::from("Health delta"),
            value: format_delta(delta.health_delta as i128),
        },
        SimpleRow {
            item: String::from("Orphan delta"),
            value: format_delta(delta.orphan_delta as i128),
        },
        SimpleRow {
            item: String::from("Size delta"),
            value: format_signed_bytes(delta.size_delta_bytes),
        },
    ];

    println!("{}", Table::new(rows));
}

fn size_bucket_rows(buckets: &[SizeBucket]) -> Vec<BucketRow> {
    buckets
        .iter()
        .map(|bucket| BucketRow {
            bucket: bucket.label.clone(),
            count: bucket.count,
            total_size: format_bytes(bucket.total_bytes),
        })
        .collect()
}

fn largest_package_rows(packages: &[PackageSizeEntry]) -> Vec<PackageRow> {
    packages
        .iter()
        .map(|package| PackageRow {
            package: package.name.clone(),
            version: package.version.clone(),
            size: format_bytes(package.size_bytes),
        })
        .collect()
}

fn markdown_table_for_sizes(packages: &[PackageSizeEntry]) -> String {
    let mut out = String::from("| Package | Version | Size |\n| --- | --- | --- |\n");
    for package in packages {
        out.push_str(&format!(
            "| {} | {} | {} |\n",
            package.name,
            package.version,
            format_bytes(package.size_bytes)
        ));
    }
    out
}

fn format_delta(value: i128) -> String {
    if value > 0 {
        format!("+{}", value)
    } else {
        value.to_string()
    }
}

fn write_serialized<T: Serialize>(path: &str, value: &T) -> Result<()> {
    let json = serde_json::to_string_pretty(value).context("Failed to serialize report")?;
    fs::write(path, json).with_context(|| format!("Failed to write report to {}", path))?;
    Ok(())
}
