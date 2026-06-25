use crate::pacman::{pacman_cache_size_bytes, InstallReason, SystemSnapshot};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissingDependency {
    pub package: String,
    pub missing: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuickScan {
    pub total_packages: usize,
    pub explicit_packages: usize,
    pub dependency_packages: usize,
    pub orphan_count: usize,
    pub missing_dependencies: Vec<MissingDependency>,
    pub cache_size_bytes: Option<u64>,
}

impl QuickScan {
    pub fn has_critical_issues(&self) -> bool {
        !self.missing_dependencies.is_empty()
    }
}

pub fn quick_scan(snapshot: &SystemSnapshot) -> QuickScan {
    let installed: HashSet<&str> = snapshot.packages.iter().map(|p| p.name.as_str()).collect();

    let mut explicit_packages = 0usize;
    let mut dependency_packages = 0usize;
    let mut missing_dependencies = Vec::new();

    for package in &snapshot.packages {
        match package.install_reason {
            InstallReason::Explicit => explicit_packages += 1,
            InstallReason::Dependency => dependency_packages += 1,
            InstallReason::Unknown => {}
        }

        for dependency in &package.depends_on {
            if !installed.contains(dependency.as_str()) {
                missing_dependencies.push(MissingDependency {
                    package: package.name.clone(),
                    missing: dependency.clone(),
                });
            }
        }
    }

    QuickScan {
        total_packages: snapshot.packages.len(),
        explicit_packages,
        dependency_packages,
        orphan_count: snapshot.orphan_names.len(),
        missing_dependencies,
        cache_size_bytes: pacman_cache_size_bytes(),
    }
}
