use crate::pacman::{InstallReason, PackageRecord, RecentPackageAction, SystemSnapshot};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageSizeEntry {
    pub name: String,
    pub version: String,
    pub size_bytes: u64,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SizeBucket {
    pub label: String,
    pub count: usize,
    pub total_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageStats {
    pub total_packages: usize,
    pub total_size_bytes: u64,
    pub average_size_bytes: u64,
    pub median_size_bytes: u64,
    pub largest_packages: Vec<PackageSizeEntry>,
    pub heavy_packages: Vec<PackageSizeEntry>,
    pub size_buckets: Vec<SizeBucket>,
    pub recent_packages: Vec<RecentPackageAction>,
    pub orphan_count: usize,
    #[serde(default)]
    pub explicit_packages: usize,
    #[serde(default)]
    pub dependency_packages: usize,
}

impl PackageStats {

    pub fn explicit_to_dependency_ratio(&self) -> Option<f64> {
        if self.dependency_packages == 0 {
            None
        } else {
            Some(self.explicit_packages as f64 / self.dependency_packages as f64)
        }
    }
}

pub fn compute_package_stats(snapshot: &SystemSnapshot) -> PackageStats {
    let mut entries: Vec<PackageSizeEntry> = snapshot
        .packages
        .iter()
        .map(|package| PackageSizeEntry {
            name: package.name.clone(),
            version: package.version.clone(),
            size_bytes: package.installed_size_bytes,
            description: package.description.clone(),
        })
        .collect();

    entries.sort_by(|left, right| {
        right
            .size_bytes
            .cmp(&left.size_bytes)
            .then(left.name.cmp(&right.name))
    });

    let total_packages = entries.len();
    let total_size_bytes = entries.iter().map(|entry| entry.size_bytes).sum::<u64>();
    let average_size_bytes = if total_packages == 0 {
        0
    } else {
        total_size_bytes / total_packages as u64
    };

    let mut sorted_sizes: Vec<u64> = entries.iter().map(|entry| entry.size_bytes).collect();
    sorted_sizes.sort_unstable();
    let median_size_bytes = median(&sorted_sizes);

    let largest_packages = entries.iter().take(20).cloned().collect();
    let mut heavy_packages: Vec<PackageSizeEntry> = entries
        .iter()
        .filter(|entry| entry.size_bytes >= 200 * 1024 * 1024)
        .take(20)
        .cloned()
        .collect();

    if heavy_packages.is_empty() {
        heavy_packages = entries.iter().take(5).cloned().collect();
    }

    let mut explicit_packages = 0usize;
    let mut dependency_packages = 0usize;
    for package in &snapshot.packages {
        match package.install_reason {
            InstallReason::Explicit => explicit_packages += 1,
            InstallReason::Dependency => dependency_packages += 1,
            InstallReason::Unknown => {}
        }
    }

    PackageStats {
        total_packages,
        total_size_bytes,
        average_size_bytes,
        median_size_bytes,
        largest_packages,
        heavy_packages,
        size_buckets: build_size_buckets(&entries),
        recent_packages: snapshot.recent_actions.clone(),
        orphan_count: snapshot.orphan_names.len(),
        explicit_packages,
        dependency_packages,
    }
}

pub fn format_bytes(bytes: u64) -> String {
    let units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut index = 0usize;

    while value >= 1024.0 && index + 1 < units.len() {
        value /= 1024.0;
        index += 1;
    }

    if index == 0 {
        format!("{} {}", bytes, units[index])
    } else {
        format!("{value:.1} {}", units[index])
    }
}

pub fn format_signed_bytes(bytes: i128) -> String {
    if bytes < 0 {
        format!("-{}", format_bytes(bytes.unsigned_abs() as u64))
    } else {
        format_bytes(bytes as u64)
    }
}

fn build_size_buckets(packages: &[PackageSizeEntry]) -> Vec<SizeBucket> {
    let mut tiny_count = 0usize;
    let mut small_count = 0usize;
    let mut medium_count = 0usize;
    let mut large_count = 0usize;
    let mut huge_count = 0usize;
    let mut tiny_total = 0u64;
    let mut small_total = 0u64;
    let mut medium_total = 0u64;
    let mut large_total = 0u64;
    let mut huge_total = 0u64;

    for package in packages {
        match package.size_bytes {
            0..=1_048_575 => {
                tiny_count += 1;
                tiny_total += package.size_bytes;
            }
            1_048_576..=24_999_999 => {
                small_count += 1;
                small_total += package.size_bytes;
            }
            25_000_000..=99_999_999 => {
                medium_count += 1;
                medium_total += package.size_bytes;
            }
            100_000_000..=499_999_999 => {
                large_count += 1;
                large_total += package.size_bytes;
            }
            _ => {
                huge_count += 1;
                huge_total += package.size_bytes;
            }
        }
    }

    vec![
        SizeBucket {
            label: String::from("Tiny (<1 MiB)"),
            count: tiny_count,
            total_bytes: tiny_total,
        },
        SizeBucket {
            label: String::from("Small (1-25 MiB)"),
            count: small_count,
            total_bytes: small_total,
        },
        SizeBucket {
            label: String::from("Medium (25-100 MiB)"),
            count: medium_count,
            total_bytes: medium_total,
        },
        SizeBucket {
            label: String::from("Large (100-500 MiB)"),
            count: large_count,
            total_bytes: large_total,
        },
        SizeBucket {
            label: String::from("Huge (>=500 MiB)"),
            count: huge_count,
            total_bytes: huge_total,
        },
    ]
}

fn median(values: &[u64]) -> u64 {
    if values.is_empty() {
        return 0;
    }

    let middle = values.len() / 2;
    if values.len() % 2 == 1 {
        values[middle]
    } else {
        (values[middle - 1] + values[middle]) / 2
    }
}

pub fn summarize_packages(packages: &[PackageRecord]) -> PackageStats {
    let snapshot = SystemSnapshot {
        generated_at: String::new(),
        packages: packages.to_vec(),
        orphan_names: Vec::new(),
        recent_actions: Vec::new(),
    };

    compute_package_stats(&snapshot)
}
