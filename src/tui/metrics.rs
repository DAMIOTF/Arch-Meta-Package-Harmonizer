use std::collections::{BTreeSet, HashMap};

use crate::tui::app::{PackageSort, TimelineEntry};
use crate::tui::fuzzy::FuzzyFilter;
use crate::tui::model::{Package, format_size, normalize_dependency};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckStatus {
    Pass,
    Info,
    Warn,
    Fail,
}

impl CheckStatus {
    pub fn label(self) -> &'static str {
        match self {
            CheckStatus::Pass => "PASS",
            CheckStatus::Info => "INFO",
            CheckStatus::Warn => "WARN",
            CheckStatus::Fail => "FAIL",
        }
    }

    pub fn is_attention(self) -> bool {
        matches!(self, CheckStatus::Warn | CheckStatus::Fail)
    }
}

#[derive(Debug, Clone)]
pub struct Check {
    pub title: String,
    pub status: CheckStatus,
    pub summary: String,
    pub detail: String,
    pub action: String,
}

/// A dependency edge, stored as indices into the package vector.
#[derive(Debug, Clone, Default)]
pub struct GraphData {
    pub edges: usize,
    pub missing_edges: usize,
    pub degree: Vec<usize>,
    pub dependents: Vec<Vec<usize>>,
    pub dependencies: Vec<Vec<usize>>,
}

#[derive(Debug, Clone, Default)]
pub struct Metrics {
    pub total_packages: usize,
    pub total_size: u64,
    pub average_size: u64,
    pub explicit: usize,
    pub dependency: usize,
    pub foreign: usize,
    pub orphans: usize,
    pub cleanup_candidates: usize,
    pub reclaimable: u64,
    pub heavy_packages: usize,
    pub largest: Vec<Package>,
    pub size_buckets: [u64; 5],
    pub health: u8,
    pub harmony: u8,
    pub structure_score: u8,
    pub dependency_score: u8,
    pub cleanliness_score: u8,
    pub provenance_score: u8,
    pub footprint_score: u8,
    pub explicit_ratio: f32,
    pub missing_descriptions: usize,
    pub missing_versions: usize,
    pub duplicate_families: Vec<String>,
    pub dangling: Vec<(String, String)>,
}

#[derive(Debug, Clone)]
pub struct Analysis {
    pub metrics: Metrics,
    pub graph: GraphData,
    pub checks: Vec<Check>,
}

pub fn analyze(packages: &[Package]) -> Analysis {
    let mut metrics = compute_metrics(packages);
    let graph = build_graph(packages);
    metrics.dangling = collect_dangling(packages);
    let mut analysis = Analysis {
        metrics,
        graph,
        checks: Vec::new(),
    };
    analysis.metrics.dangling = collect_dangling(packages);
    score(&mut analysis.metrics);
    analysis.checks = build_checks(packages, &analysis.metrics, &analysis.graph);
    analysis
}

fn compute_metrics(packages: &[Package]) -> Metrics {
    let total_packages = packages.len();
    let total_size = packages.iter().map(|package| package.size).sum::<u64>();
    let average_size = if total_packages == 0 {
        0
    } else {
        total_size / total_packages as u64
    };

    let explicit = packages.iter().filter(|package| package.explicit).count();
    let dependency = total_packages.saturating_sub(explicit);
    let foreign = packages.iter().filter(|package| package.foreign).count();
    let orphans = packages
        .iter()
        .filter(|package| package.is_orphan())
        .count();
    let cleanup = packages
        .iter()
        .filter(|package| package.is_cleanup_candidate())
        .collect::<Vec<_>>();
    let cleanup_candidates = cleanup.len();
    let reclaimable = cleanup.iter().map(|package| package.size).sum::<u64>();
    let heavy_packages = packages
        .iter()
        .filter(|package| package.size >= 1024 * 1024 * 1024)
        .count();
    let missing_descriptions = packages
        .iter()
        .filter(|package| package.description.trim().is_empty())
        .count();
    let missing_versions = packages
        .iter()
        .filter(|package| package.version.trim().is_empty())
        .count();

    let mut largest = packages.to_vec();
    largest.sort_by(|left, right| {
        right
            .size
            .cmp(&left.size)
            .then_with(|| left.name.cmp(&right.name))
    });
    largest.truncate(6);

    let buckets = [
        packages
            .iter()
            .filter(|package| package.size < 1024 * 1024)
            .count() as u64,
        packages
            .iter()
            .filter(|package| (1024 * 1024..25 * 1024 * 1024).contains(&package.size))
            .count() as u64,
        packages
            .iter()
            .filter(|package| (25 * 1024 * 1024..100 * 1024 * 1024).contains(&package.size))
            .count() as u64,
        packages
            .iter()
            .filter(|package| (100 * 1024 * 1024..1024 * 1024 * 1024).contains(&package.size))
            .count() as u64,
        packages
            .iter()
            .filter(|package| package.size >= 1024 * 1024 * 1024)
            .count() as u64,
    ];

    let explicit_ratio = if total_packages == 0 {
        0.0
    } else {
        explicit as f32 / total_packages as f32
    };

    let duplicates = duplicate_families(packages);

    let mut metrics = Metrics {
        total_packages,
        total_size,
        average_size,
        explicit,
        dependency,
        foreign,
        orphans,
        cleanup_candidates,
        reclaimable,
        heavy_packages,
        largest,
        size_buckets: buckets,
        health: 0,
        harmony: 0,
        structure_score: 0,
        dependency_score: 0,
        cleanliness_score: 0,
        provenance_score: 0,
        footprint_score: 0,
        explicit_ratio,
        missing_descriptions,
        missing_versions,
        duplicate_families: duplicates,
        dangling: Vec::new(),
    };

    // Fill dependency-dependent components before the final score pass.
    metrics.dependency_score = 100;
    metrics.cleanliness_score = 100u8.saturating_sub((metrics.orphans as u8).saturating_mul(10));
    metrics
}

fn score(metrics: &mut Metrics) {
    if metrics.total_packages == 0 {
        metrics.health = 0;
        metrics.harmony = 0;
        return;
    }

    let dangling_penalty = (metrics.dangling.len() as i32 * 4).min(20) as u8;
    let orphan_penalty = (metrics.orphans as u8).saturating_mul(5).min(25);
    let foreign_penalty = (metrics.foreign as u8).saturating_mul(2).min(12);
    let heavy_penalty = (metrics.heavy_packages as u8).saturating_mul(3).min(15);
    let meta_penalty = ((metrics.missing_descriptions / 5) as u8)
        .saturating_add((metrics.missing_versions * 2).min(6) as u8)
        .min(8);
    let size_pressure =
        (metrics.total_size as f64 / (1024.0 * 1024.0 * 1024.0 * 8.0)).min(15.0) as u8;

    metrics.health = 100u8
        .saturating_sub(dangling_penalty)
        .saturating_sub(orphan_penalty)
        .saturating_sub(foreign_penalty)
        .saturating_sub(heavy_penalty)
        .saturating_sub(meta_penalty)
        .saturating_sub(size_pressure);

    metrics.dependency_score = 100u8.saturating_sub(dangling_penalty);
    metrics.cleanliness_score = 100u8.saturating_sub(orphan_penalty.saturating_mul(2).min(100));
    metrics.provenance_score = 100u8.saturating_sub(foreign_penalty.saturating_mul(4).min(100));
    metrics.structure_score = if metrics.explicit_ratio >= 0.2 && metrics.explicit_ratio <= 0.82 {
        100u8.saturating_sub(heavy_penalty.min(12))
    } else {
        72u8.saturating_sub(heavy_penalty.min(12))
    };
    metrics.footprint_score = 100u8.saturating_sub(size_pressure.saturating_mul(5).min(100));

    let total = metrics.structure_score as u16
        + metrics.dependency_score as u16
        + metrics.cleanliness_score as u16
        + metrics.provenance_score as u16
        + metrics.footprint_score as u16;
    metrics.harmony = (total / 5) as u8;
}

fn build_graph(packages: &[Package]) -> GraphData {
    let count = packages.len();
    let mut by_name: HashMap<&str, usize> = HashMap::new();
    let mut by_provide: HashMap<String, usize> = HashMap::new();

    for (index, package) in packages.iter().enumerate() {
        by_name.insert(package.name.as_str(), index);
        for provide in &package.provides {
            by_provide
                .entry(normalize_dependency(provide))
                .or_insert(index);
        }
    }

    let mut dependencies: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); count];
    let mut dependents: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); count];
    let mut missing_edges = 0usize;

    for (index, package) in packages.iter().enumerate() {
        for dependency in &package.depends {
            let token = normalize_dependency(dependency);
            let target = by_name
                .get(token.as_str())
                .copied()
                .or_else(|| by_provide.get(&token).copied());

            match target {
                Some(target) if target != index => {
                    dependencies[index].insert(target);
                    dependents[target].insert(index);
                }
                Some(_) => {}
                None => missing_edges += 1,
            }
        }
    }

    let edges = dependencies.iter().map(BTreeSet::len).sum::<usize>();
    let degree = (0..count)
        .map(|index| dependencies[index].len() + dependents[index].len())
        .collect::<Vec<_>>();
    GraphData {
        edges,
        missing_edges,
        degree,
        dependents: dependents
            .into_iter()
            .map(|set| set.into_iter().collect())
            .collect(),
        dependencies: dependencies
            .into_iter()
            .map(|set| set.into_iter().collect())
            .collect(),
    }
}

fn collect_dangling(packages: &[Package]) -> Vec<(String, String)> {
    let mut installed: BTreeSet<&str> = BTreeSet::new();
    let mut provided: BTreeSet<String> = BTreeSet::new();

    for package in packages {
        installed.insert(package.name.as_str());
        for provide in &package.provides {
            provided.insert(normalize_dependency(provide));
        }
    }

    let mut dangling = Vec::new();
    for package in packages {
        for dependency in &package.depends {
            let token = normalize_dependency(dependency);
            if !installed.contains(token.as_str()) && !provided.contains(&token) {
                dangling.push((package.name.clone(), token));
            }
        }
    }

    dangling.sort();
    dangling.dedup();
    dangling
}

fn build_checks(packages: &[Package], metrics: &Metrics, graph: &GraphData) -> Vec<Check> {
    let mut checks = Vec::new();

    checks.push(if metrics.orphans == 0 {
        Check {
            title: "Orphan containment".to_string(),
            status: CheckStatus::Pass,
            summary: "No orphaned packages detected.".to_string(),
            detail: "Every package has a recorded owner or is explicitly installed.".to_string(),
            action: "No orphan sweep is needed right now.".to_string(),
        }
    } else {
        let names = packages
            .iter()
            .filter(|package| package.is_orphan())
            .map(|package| package.name.as_str())
            .take(10)
            .collect::<Vec<_>>()
            .join(", ");
        Check {
            title: "Orphan containment".to_string(),
            status: if metrics.orphans > 4 {
                CheckStatus::Fail
            } else {
                CheckStatus::Warn
            },
            summary: format!(
                "{} orphaned package{} · {} reclaimable.",
                metrics.orphans,
                if metrics.orphans == 1 { "" } else { "s" },
                format_size(
                    packages
                        .iter()
                        .filter(|package| package.is_orphan())
                        .map(|package| package.size)
                        .sum()
                )
            ),
            detail: format!("Detected: {names}"),
            action: "Open Cleanup, select with Space, then preview before confirming.".to_string(),
        }
    });

    checks.push(if metrics.foreign == 0 {
        Check {
            title: "Foreign provenance".to_string(),
            status: CheckStatus::Pass,
            summary: "All packages come from configured repositories.".to_string(),
            detail: "No AUR, local, or untracked packages were reported.".to_string(),
            action: "Nothing to review.".to_string(),
        }
    } else {
        let names = packages
            .iter()
            .filter(|package| package.foreign)
            .map(|package| package.name.as_str())
            .take(10)
            .collect::<Vec<_>>()
            .join(", ");
        Check {
            title: "Foreign provenance".to_string(),
            status: CheckStatus::Info,
            summary: format!(
                "{} foreign package{} present.",
                metrics.foreign,
                if metrics.foreign == 1 { "" } else { "s" }
            ),
            detail: format!("Sources to verify: {names}"),
            action: "Check that AUR and locally built packages are still maintained.".to_string(),
        }
    });

    checks.push(if metrics.dangling.is_empty() {
        Check {
            title: "Dependency integrity".to_string(),
            status: CheckStatus::Pass,
            summary: "Every declared dependency resolves locally.".to_string(),
            detail: format!(
                "{} edges resolved across {} nodes and {} provides.",
                graph.edges,
                metrics.total_packages,
                packages
                    .iter()
                    .filter(|package| !package.provides.is_empty())
                    .count()
            ),
            action: "No missing providers were found.".to_string(),
        }
    } else {
        let examples = metrics
            .dangling
            .iter()
            .take(8)
            .map(|(package, dependency)| format!("{package} → {dependency}"))
            .collect::<Vec<_>>()
            .join(" · ");
        Check {
            title: "Dependency integrity".to_string(),
            status: if metrics.dangling.len() > 4 {
                CheckStatus::Fail
            } else {
                CheckStatus::Warn
            },
            summary: format!(
                "{} unresolved dependency edge{}.",
                metrics.dangling.len(),
                if metrics.dangling.len() == 1 { "" } else { "s" }
            ),
            detail: format!("Examples: {examples}"),
            action: "Review the Graph tab before removing providers or optional chains."
                .to_string(),
        }
    });

    checks.push(if metrics.total_size <= 15 * 1024 * 1024 * 1024 {
        Check {
            title: "Footprint pressure".to_string(),
            status: CheckStatus::Pass,
            summary: format!(
                "{} installed · comfortable footprint.",
                format_size(metrics.total_size)
            ),
            detail: "The package set is within a typical workstation budget.".to_string(),
            action: "Keep an eye on the heaviest packages in Dashboard.".to_string(),
        }
    } else if metrics.total_size <= 40 * 1024 * 1024 * 1024 {
        Check {
            title: "Footprint pressure".to_string(),
            status: CheckStatus::Warn,
            summary: format!(
                "{} installed · growing footprint.",
                format_size(metrics.total_size)
            ),
            detail: format!(
                "{} package(s) exceed 1 GiB and contribute to the total.",
                metrics.heavy_packages
            ),
            action: "Audit the heaviest packages for unused toolchains and caches.".to_string(),
        }
    } else {
        Check {
            title: "Footprint pressure".to_string(),
            status: CheckStatus::Fail,
            summary: format!(
                "{} installed · high pressure.",
                format_size(metrics.total_size)
            ),
            detail: "The installed set is large enough to slow maintenance.".to_string(),
            action: "Review the largest packages and stale dependency chains.".to_string(),
        }
    });

    checks.push(
        if metrics.missing_descriptions + metrics.missing_versions == 0 {
            Check {
                title: "Metadata hygiene".to_string(),
                status: CheckStatus::Pass,
                summary: "Descriptions and versions are present.".to_string(),
                detail: "The interface can explain every package without blanks.".to_string(),
                action: "No action needed.".to_string(),
            }
        } else {
            Check {
                title: "Metadata hygiene".to_string(),
                status: CheckStatus::Info,
                summary: format!(
                    "{} missing description(s), {} missing version(s).",
                    metrics.missing_descriptions, metrics.missing_versions
                ),
                detail: "Empty metadata usually points to locally built or incomplete records."
                    .to_string(),
                action:
                    "Confirm those packages are still needed, then re-check their build source."
                        .to_string(),
            }
        },
    );

    let ratio = (metrics.explicit_ratio * 100.0).round() as u8;
    checks.push(if (20..=82).contains(&ratio) {
        Check {
            title: "Dependency balance".to_string(),
            status: CheckStatus::Pass,
            summary: format!("{ratio}% explicit · healthy balance."),
            detail: format!(
                "{} explicitly installed, {} dependency packages.",
                metrics.explicit, metrics.dependency
            ),
            action: "No balance change is required.".to_string(),
        }
    } else {
        Check {
            title: "Dependency balance".to_string(),
            status: CheckStatus::Info,
            summary: format!("{ratio}% explicit · unusual balance."),
            detail: format!(
                "{} explicitly installed, {} dependency packages.",
                metrics.explicit, metrics.dependency
            ),
            action: "Review whether the package set was installed intentionally.".to_string(),
        }
    });

    checks.push(if metrics.duplicate_families.is_empty() {
        Check {
            title: "Family duplication".to_string(),
            status: CheckStatus::Pass,
            summary: "No sibling package families detected.".to_string(),
            detail: "Stable, git, and binary variants do not overlap in this snapshot.".to_string(),
            action: "No action needed.".to_string(),
        }
    } else {
        Check {
            title: "Family duplication".to_string(),
            status: CheckStatus::Info,
            summary: if metrics.duplicate_families.len() == 1 {
                "1 duplicated package family.".to_string()
            } else {
                format!(
                    "{} duplicated package families.",
                    metrics.duplicate_families.len()
                )
            },
            detail: format!("Review: {}", metrics.duplicate_families.join(", ")),
            action: "Keep one variant per family unless both serve a purpose.".to_string(),
        }
    });

    checks.push(if metrics.cleanup_candidates == 0 {
        Check {
            title: "Cleanup readiness".to_string(),
            status: CheckStatus::Pass,
            summary: "No cleanup candidates are flagged.".to_string(),
            detail: "The cleanup queue is empty for this snapshot.".to_string(),
            action: "Nothing to preview.".to_string(),
        }
    } else {
        Check {
            title: "Cleanup readiness".to_string(),
            status: CheckStatus::Info,
            summary: format!(
                "{} candidate{} · {} reclaimable.",
                metrics.cleanup_candidates,
                if metrics.cleanup_candidates == 1 {
                    ""
                } else {
                    "s"
                },
                format_size(metrics.reclaimable)
            ),
            detail: "Candidates come from the reason field supplied by the scanner.".to_string(),
            action: "Cleanup always requires selection, preview, and typed confirmation."
                .to_string(),
        }
    });

    checks
}

fn duplicate_families(packages: &[Package]) -> Vec<String> {
    let mut families: HashMap<String, Vec<&str>> = HashMap::new();
    for package in packages {
        families
            .entry(package_family(&package.name))
            .or_default()
            .push(package.name.as_str());
    }

    let mut duplicates = families
        .into_iter()
        .filter(|(_, names)| names.len() > 1)
        .map(|(family, mut names)| {
            names.sort();
            format!("{family} ({})", names.join(", "))
        })
        .collect::<Vec<_>>();
    duplicates.sort();
    duplicates
}

fn package_family(name: &str) -> String {
    let mut value = name.to_string();
    if let Some(stripped) = value.strip_prefix("lib32-") {
        value = stripped.to_string();
    }
    for suffix in [
        "-git", "-svn", "-hg", "-cvs", "-bin", "-debug", "-lts", "-common",
    ] {
        if let Some(stripped) = value.strip_suffix(suffix) {
            value = stripped.to_string();
            break;
        }
    }
    value
}

pub fn package_indices(packages: &[Package], filter: &str, sort: PackageSort) -> Vec<usize> {
    let mut fuzzy = FuzzyFilter::new(filter);
    let filtered = fuzzy.is_some();
    let mut matches = Vec::new();

    for (index, package) in packages.iter().enumerate() {
        let score = match score_package(&mut fuzzy, package) {
            Some(score) => score,
            None => continue,
        };
        matches.push((index, score));
    }

    matches.sort_by(|(left, left_score), (right, right_score)| {
        if filtered {
            right_score
                .cmp(left_score)
                .then_with(|| package_order(&packages[*left], &packages[*right], sort))
        } else {
            package_order(&packages[*left], &packages[*right], sort)
        }
    });
    matches.into_iter().map(|(index, _)| index).collect()
}

pub fn cleanup_indices(packages: &[Package], filter: &str) -> Vec<usize> {
    let mut fuzzy = FuzzyFilter::new(filter);
    let filtered = fuzzy.is_some();
    let mut matches = Vec::new();

    for (index, package) in packages.iter().enumerate() {
        if !package.is_cleanup_candidate() {
            continue;
        }
        let score = match score_package(&mut fuzzy, package) {
            Some(score) => score,
            None => continue,
        };
        matches.push((index, score));
    }

    matches.sort_by(|(left, left_score), (right, right_score)| {
        let left_package = &packages[*left];
        let right_package = &packages[*right];
        if filtered {
            right_score.cmp(left_score).then_with(|| {
                right_package
                    .size
                    .cmp(&left_package.size)
                    .then_with(|| left_package.name.cmp(&right_package.name))
            })
        } else {
            right_package
                .size
                .cmp(&left_package.size)
                .then_with(|| left_package.name.cmp(&right_package.name))
        }
    });
    matches.into_iter().map(|(index, _)| index).collect()
}

pub fn graph_indices(packages: &[Package], graph: &GraphData, filter: &str) -> Vec<usize> {
    let mut fuzzy = FuzzyFilter::new(filter);
    let filtered = fuzzy.is_some();
    let mut matches = Vec::new();

    for (index, package) in packages.iter().enumerate() {
        let score = match score_package(&mut fuzzy, package) {
            Some(score) => score,
            None => continue,
        };
        matches.push((index, score));
    }

    matches.sort_by(|(left, left_score), (right, right_score)| {
        if filtered {
            right_score.cmp(left_score).then_with(|| {
                graph.degree[*right]
                    .cmp(&graph.degree[*left])
                    .then_with(|| packages[*left].name.cmp(&packages[*right].name))
            })
        } else {
            graph.degree[*right]
                .cmp(&graph.degree[*left])
                .then_with(|| packages[*left].name.cmp(&packages[*right].name))
        }
    });
    matches.into_iter().map(|(index, _)| index).collect()
}

pub fn check_indices(checks: &[Check], filter: &str) -> Vec<usize> {
    let mut fuzzy = FuzzyFilter::new(filter);
    let mut matches = Vec::new();

    for (index, check) in checks.iter().enumerate() {
        let haystack = format!(
            "{} {} {} {}",
            check.title,
            check.summary,
            check.detail,
            check.status.label()
        );
        let score = match score_filter(&mut fuzzy, &haystack) {
            Some(score) => score,
            None => continue,
        };
        matches.push((index, score));
    }

    matches.sort_by(|(left, left_score), (right, right_score)| {
        right_score.cmp(left_score).then_with(|| left.cmp(right))
    });
    matches.into_iter().map(|(index, _)| index).collect()
}

pub fn timeline_indices(entries: &[TimelineEntry], filter: &str) -> Vec<usize> {
    let mut fuzzy = FuzzyFilter::new(filter);
    let filtered = fuzzy.is_some();
    let mut matches = Vec::new();

    for (index, entry) in entries.iter().enumerate() {
        let score = match score_filter(&mut fuzzy, &entry.searchable()) {
            Some(score) => score,
            None => continue,
        };
        matches.push((index, score));
    }

    if filtered {
        matches.sort_by(|(left, left_score), (right, right_score)| {
            right_score.cmp(left_score).then_with(|| left.cmp(right))
        });
    }
    matches.into_iter().map(|(index, _)| index).collect()
}

fn score_package(filter: &mut Option<FuzzyFilter>, package: &Package) -> Option<u32> {
    let Some(filter) = filter.as_mut() else {
        return Some(0);
    };

    let name_score = filter.score(&package.name);
    let all_score = filter.score(&package.searchable());
    match (name_score, all_score) {
        // Prefer direct name matches over incidental description matches.
        (Some(name), Some(all)) => Some(name.saturating_add(1000).max(all)),
        (Some(name), None) => Some(name.saturating_add(1000)),
        (None, Some(all)) => Some(all),
        (None, None) => None,
    }
}

fn score_filter(filter: &mut Option<FuzzyFilter>, haystack: &str) -> Option<u32> {
    match filter {
        Some(filter) => filter.score(haystack),
        None => Some(0),
    }
}

fn package_order(left: &Package, right: &Package, sort: PackageSort) -> std::cmp::Ordering {
    match sort {
        PackageSort::Size => right
            .size
            .cmp(&left.size)
            .then_with(|| left.name.cmp(&right.name)),
        PackageSort::Name => left.name.cmp(&right.name),
        PackageSort::Reason => left
            .reason_label()
            .cmp(right.reason_label())
            .then_with(|| left.name.cmp(&right.name)),
    }
}

pub fn check_counts(checks: &[Check]) -> (usize, usize, usize, usize) {
    let mut pass = 0;
    let mut info = 0;
    let mut warn = 0;
    let mut fail = 0;
    for check in checks {
        match check.status {
            CheckStatus::Pass => pass += 1,
            CheckStatus::Info => info += 1,
            CheckStatus::Warn => warn += 1,
            CheckStatus::Fail => fail += 1,
        }
    }
    (pass, info, warn, fail)
}

#[cfg(test)]
mod filter_tests {
    use super::*;

    fn package(name: &str, description: &str, size: u64) -> Package {
        Package {
            name: name.to_string(),
            description: description.to_string(),
            size,
            explicit: true,
            reason: "explicit".to_string(),
            ..Package::default()
        }
    }

    #[test]
    fn package_filter_is_fuzzy_not_substring() {
        let packages = vec![
            package("ripgrep", "recursive search", 5),
            package("rustup", "Rust toolchain installer", 900),
            package("hyprland", "Wayland compositor", 20),
        ];
        let indices = package_indices(&packages, "rgrp", PackageSort::Name);
        assert_eq!(indices.len(), 1);
        assert_eq!(packages[indices[0]].name, "ripgrep");
    }

    #[test]
    fn package_filter_is_case_insensitive() {
        let packages = vec![
            package("ripgrep", "recursive search", 5),
            package("firefox", "web browser", 200),
        ];
        let indices = package_indices(&packages, "RIPGREP", PackageSort::Name);
        assert_eq!(indices.len(), 1);
        assert_eq!(packages[indices[0]].name, "ripgrep");
    }

    #[test]
    fn fuzzy_scores_rank_before_sort_order() {
        let packages = vec![
            package("other", "contains rust as part of rustup mention", 900),
            package("rustup", "toolchain", 1),
        ];
        let indices = package_indices(&packages, "rustup", PackageSort::Size);
        assert_eq!(packages[indices[0]].name, "rustup");
    }
}
