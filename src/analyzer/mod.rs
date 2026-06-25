use crate::cleanup::{build_cleanup_assessment, CleanupAssessment};
use crate::deps::{inspect_dependencies, DependencyInspection};
use crate::pacman::{pacman_cache_size_bytes, SystemSnapshot};
use crate::stats::{compute_package_stats, format_bytes, PackageStats};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemAnalysis {
    pub snapshot: SystemSnapshot,
    pub stats: PackageStats,
    pub cleanup: CleanupAssessment,
    pub dependencies: DependencyInspection,
    pub health_score: u8,
    #[serde(default)]
    pub bloat_score: u8,
    #[serde(default)]
    pub cache_warning: Option<String>,
    #[serde(default)]
    pub unused_dependency_confidence: String,
    #[serde(default)]
    pub recommendations: Vec<String>,
    #[serde(default)]
    pub penalty_breakdown: Vec<String>,
}

pub fn analyze(snapshot: &SystemSnapshot) -> SystemAnalysis {
    let stats = compute_package_stats(snapshot);
    let cleanup = build_cleanup_assessment(snapshot, &stats);
    let dependencies = inspect_dependencies(snapshot);
    let cache_warning = build_cache_warning();
    let (bloat_score, penalty_breakdown, unused_dependency_confidence) =
        compute_bloat_score(&stats, &cleanup, &dependencies);
    let health_score = if stats.total_packages == 0 {
        0
    } else {
        100u8.saturating_sub(bloat_score)
    };
    let recommendations = build_recommendations(
        &stats,
        &cleanup,
        &dependencies,
        health_score,
        bloat_score,
        cache_warning.as_deref(),
        &unused_dependency_confidence,
    );

    SystemAnalysis {
        snapshot: snapshot.clone(),
        stats,
        cleanup,
        dependencies,
        health_score,
        bloat_score,
        cache_warning,
        unused_dependency_confidence,
        recommendations,
        penalty_breakdown,
    }
}

fn compute_bloat_score(
    stats: &PackageStats,
    cleanup: &CleanupAssessment,
    dependencies: &DependencyInspection,
) -> (u8, Vec<String>, String) {
    let mut score: i32 = 0;
    let mut penalties = Vec::new();

    let orphan_penalty = (cleanup.orphans.len() as i32 * 2).min(20);
    if orphan_penalty > 0 {
        score += orphan_penalty;
        penalties.push(format!("orphans: +{}", orphan_penalty));
    }

    let unused_dependency_penalty = (cleanup.unnecessary_dependencies.len() as i32 * 4).min(10);
    if unused_dependency_penalty > 0 {
        score += unused_dependency_penalty;
        penalties.push(format!("unused dependencies: +{}", unused_dependency_penalty));
    }

    let heavy_penalty = ((stats.heavy_packages.len() as i32) / 2).min(12);
    if heavy_penalty > 0 {
        score += heavy_penalty;
        penalties.push(format!("heavy packages: +{}", heavy_penalty));
    }

    let redundant_penalty = (dependencies.redundant_packages.len() as i32 * 2).min(8);
    if redundant_penalty > 0 {
        score += redundant_penalty;
        penalties.push(format!("redundant families: +{}", redundant_penalty));
    }

    let size_pressure = if stats.total_packages == 0 {
        0
    } else {
        let size_gib = stats.total_size_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
        let package_pressure = (size_gib / 1.5).round() as i32;
        let inventory_pressure = (stats.total_packages as f64 / 150.0).round() as i32;
        package_pressure + inventory_pressure
    }
    .min(25);

    if size_pressure > 0 {
        score += size_pressure;
        penalties.push(format!("installed size pressure: +{}", size_pressure));
    }

    let cache_penalty = if let Some(bytes) = pacman_cache_size_bytes() {
        if bytes >= 20 * 1024 * 1024 * 1024 {
            8
        } else if bytes >= 10 * 1024 * 1024 * 1024 {
            5
        } else if bytes >= 5 * 1024 * 1024 * 1024 {
            3
        } else {
            0
        }
    } else {
        0
    };

    if cache_penalty > 0 {
        score += cache_penalty;
        penalties.push(format!("pacman cache pressure: +{}", cache_penalty));
    }

    if stats.total_packages == 0 {
        score = 0;
    }

    let confidence = if cleanup.unnecessary_dependencies.is_empty() {
        String::from("low")
    } else if cleanup.unnecessary_dependencies.len() < 5 {
        String::from("medium")
    } else {
        String::from("high")
    };

    (score.clamp(0, 100) as u8, penalties, confidence)
}

fn build_cache_warning() -> Option<String> {
    let bytes = pacman_cache_size_bytes()?;

    if bytes >= 20 * 1024 * 1024 * 1024 {
        Some(format!("Pacman cache is very large at {}; consider `paccache -ruk0` after review.", format_bytes(bytes)))
    } else if bytes >= 10 * 1024 * 1024 * 1024 {
        Some(format!("Pacman cache is large at {}; consider pruning old package archives.", format_bytes(bytes)))
    } else if bytes >= 5 * 1024 * 1024 * 1024 {
        Some(format!("Pacman cache is growing at {}; it may be worth trimming stale packages.", format_bytes(bytes)))
    } else {
        None
    }
}

fn build_recommendations(
    stats: &PackageStats,
    cleanup: &CleanupAssessment,
    dependencies: &DependencyInspection,
    health_score: u8,
    bloat_score: u8,
    cache_warning: Option<&str>,
    unused_dependency_confidence: &str,
) -> Vec<String> {
    let mut recommendations = Vec::new();

    if !cleanup.orphans.is_empty() {
        recommendations.push(format!(
            "Review and remove {} orphaned package(s) after verifying they are not needed.",
            cleanup.orphans.len()
        ));
    }

    if !cleanup.unnecessary_dependencies.is_empty() {
        recommendations.push(format!(
            "{} dependency package(s) appear unused and are strong cleanup candidates.",
            cleanup.unnecessary_dependencies.len()
        ));
    }

    if let Some(heavy) = stats.heavy_packages.first() {
        recommendations.push(format!(
            "Largest package detected: {} using {}.",
            heavy.name,
            format_bytes(heavy.size_bytes)
        ));
    }

    if !dependencies.suspicious_chains.is_empty() {
        recommendations.push(format!(
            "Inspect {} dependency warning(s) for broad or redundant package chains.",
            dependencies.suspicious_chains.len()
        ));
    }

    if let Some(cache_warning) = cache_warning {
        recommendations.push(cache_warning.to_string());
    }

    recommendations.push(format!(
        "Unused dependency confidence is {}.",
        unused_dependency_confidence
    ));

    if health_score >= 90 {
        recommendations.push(String::from("System package state looks healthy overall."));
    } else if health_score >= 70 {
        recommendations.push(String::from("System is generally healthy, but a targeted cleanup would help."));
    } else {
        recommendations.push(String::from("Health score is low enough to justify a careful cleanup and dependency review."));
    }

    if bloat_score >= 70 {
        recommendations.push(String::from("The package set looks bloated; review large packages and stale dependencies first."));
    }

    recommendations
}

pub fn package_overview_line(analysis: &SystemAnalysis) -> String {
    format!(
        "{} packages | {} orphans | {} heavy packages | {} score",
        analysis.stats.total_packages,
        analysis.cleanup.orphans.len(),
        analysis.stats.heavy_packages.len(),
        analysis.health_score
    )
}
