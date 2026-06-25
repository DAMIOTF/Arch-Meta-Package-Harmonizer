use crate::pacman::{pacman_cache_size_bytes, InstallReason, PackageRecord, SystemSnapshot};
use crate::stats::{format_bytes, PackageSizeEntry, PackageStats};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupItem {
    pub name: String,
    pub version: String,
    pub reason: String,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupAssessment {
    pub orphans: Vec<CleanupItem>,
    pub unnecessary_dependencies: Vec<CleanupItem>,
    pub safe_remove_candidates: Vec<String>,
    pub dry_run_command: String,
    pub recommendations: Vec<String>,

    #[serde(default)]
    pub cache_size_bytes: Option<u64>,
}

impl CleanupAssessment {
    pub fn candidates(&self) -> Vec<&CleanupItem> {
        let mut items: Vec<&CleanupItem> = self
            .orphans
            .iter()
            .chain(self.unnecessary_dependencies.iter())
            .collect();
        items.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
        items
    }

    pub fn reclaimable_bytes(&self) -> u64 {
        self.orphans
            .iter()
            .chain(self.unnecessary_dependencies.iter())
            .map(|item| item.size_bytes)
            .sum()
    }
}

pub fn build_cleanup_assessment(snapshot: &SystemSnapshot, stats: &PackageStats) -> CleanupAssessment {
    let orphans = snapshot
        .packages
        .iter()
        .filter(|package| snapshot.orphan_names.iter().any(|name| name == &package.name))
        .map(to_item)
        .collect::<Vec<_>>();

    let unnecessary_dependencies = snapshot
        .packages
        .iter()
        .filter(|package| matches!(package.install_reason, InstallReason::Dependency) && package.required_by.is_empty())
        .filter(|package| !snapshot.orphan_names.iter().any(|name| name == &package.name))
        .map(to_item)
        .collect::<Vec<_>>();

    let mut safe_remove_candidates = orphans
        .iter()
        .chain(unnecessary_dependencies.iter())
        .map(|item| item.name.clone())
        .collect::<Vec<_>>();
    safe_remove_candidates.sort();
    safe_remove_candidates.dedup();

    let dry_run_command = if safe_remove_candidates.is_empty() {
        String::from("sudo pacman -Rns --print <review-manually>")
    } else {
        format!("sudo pacman -Rns --print {}", safe_remove_candidates.join(" "))
    };

    let mut recommendations = Vec::new();
    if !orphans.is_empty() {
        recommendations.push(format!(
            "Review {} orphaned package(s) for removal.",
            orphans.len()
        ));
    }
    if !unnecessary_dependencies.is_empty() {
        recommendations.push(format!(
            "{} dependency package(s) appear unused and are cleanup candidates.",
            unnecessary_dependencies.len()
        ));
    }
    if stats.heavy_packages.len() >= 5 {
        recommendations.push(String::from(
            "Audit very large packages for optional features and old toolchains."
        ));
    }
    if safe_remove_candidates.is_empty() {
        recommendations.push(String::from(
            "No safe cleanup candidates were detected from the current snapshot."
        ));
    }

    CleanupAssessment {
        orphans,
        unnecessary_dependencies,
        safe_remove_candidates,
        dry_run_command,
        recommendations,
        cache_size_bytes: pacman_cache_size_bytes(),
    }
}

fn to_item(package: &PackageRecord) -> CleanupItem {
    CleanupItem {
        name: package.name.clone(),
        version: package.version.clone(),
        reason: match package.install_reason {
            InstallReason::Explicit => String::from("explicitly installed"),
            InstallReason::Dependency => String::from("installed as dependency"),
            InstallReason::Unknown => String::from("unknown"),
        },
        size_bytes: package.installed_size_bytes,
    }
}

pub fn flatten_candidates(assessment: &CleanupAssessment) -> Vec<PackageSizeEntry> {
    assessment
        .orphans
        .iter()
        .chain(assessment.unnecessary_dependencies.iter())
        .map(|item| PackageSizeEntry {
            name: item.name.clone(),
            version: item.version.clone(),
            size_bytes: item.size_bytes,
            description: Some(item.reason.clone()),
        })
        .collect()
}

pub fn format_cleanup_line(item: &CleanupItem) -> String {
    format!("{} ({}) - {}", item.name, item.version, format_bytes(item.size_bytes))
}
