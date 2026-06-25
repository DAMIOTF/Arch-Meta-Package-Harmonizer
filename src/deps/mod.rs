use crate::pacman::{normalize_dependency_token, InstallReason, SystemSnapshot};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyNode {
    pub name: String,
    pub depends_on: Vec<String>,
    pub required_by: Vec<String>,
    pub optional_for: Vec<String>,
    pub is_explicit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyInspection {
    pub nodes: Vec<DependencyNode>,
    pub leaf_packages: Vec<String>,
    pub root_packages: Vec<String>,
    pub redundant_packages: Vec<String>,
    pub suspicious_chains: Vec<String>,
}

pub fn inspect_dependencies(snapshot: &SystemSnapshot) -> DependencyInspection {
    let nodes = snapshot
        .packages
        .iter()
        .map(|package| DependencyNode {
            name: package.name.clone(),
            depends_on: package.depends_on.clone(),
            required_by: package.required_by.clone(),
            optional_for: package.optional_for.clone(),
            is_explicit: matches!(package.install_reason, InstallReason::Explicit),
        })
        .collect::<Vec<_>>();

    let leaf_packages = nodes
        .iter()
        .filter(|node| node.depends_on.is_empty())
        .map(|node| node.name.clone())
        .collect::<Vec<_>>();

    let root_packages = nodes
        .iter()
        .filter(|node| node.required_by.is_empty())
        .map(|node| node.name.clone())
        .collect::<Vec<_>>();

    let redundant_packages = nodes
        .iter()
        .filter(|node| !node.is_explicit && node.required_by.is_empty())
        .map(|node| node.name.clone())
        .collect::<Vec<_>>();

    let mut suspicious_chains = Vec::new();
    for node in &nodes {
        if node.depends_on.iter().any(|dependency| dependency == &node.name) {
            suspicious_chains.push(format!("{} depends on itself", node.name));
        }
        if node.depends_on.len() >= 25 {
            suspicious_chains.push(format!(
                "{} has an unusually broad dependency fan-out ({} dependencies)",
                node.name,
                node.depends_on.len()
            ));
        }
        if !node.is_explicit && node.required_by.is_empty() {
            suspicious_chains.push(format!(
                "{} is installed as a dependency but nothing currently requires it",
                node.name
            ));
        }
    }

    suspicious_chains.sort();
    suspicious_chains.dedup();

    DependencyInspection {
        nodes,
        leaf_packages,
        root_packages,
        redundant_packages,
        suspicious_chains,
    }
}

pub fn dependency_chain_summary(inspection: &DependencyInspection) -> Vec<String> {
    inspection.suspicious_chains.clone()
}

pub fn dependency_family_counts(snapshot: &SystemSnapshot) -> Vec<(String, usize)> {
    let mut families: BTreeMap<String, usize> = BTreeMap::new();

    for package in &snapshot.packages {
        let family = package_family(&package.name);
        *families.entry(family).or_insert(0) += 1;
    }

    families.into_iter().filter(|(_, count)| *count > 1).collect()
}

fn package_family(name: &str) -> String {
    let mut value = name.to_string();

    if let Some(stripped) = value.strip_prefix("lib32-") {
        value = stripped.to_string();
    }

    for suffix in ["-git", "-svn", "-hg", "-cvs", "-bin", "-debug", "-lts"] {
        if let Some(stripped) = value.strip_suffix(suffix) {
            value = stripped.to_string();
            break;
        }
    }

    if let Some(stripped) = value.strip_suffix("-common") {
        value = stripped.to_string();
    }

    value
}

pub fn normalize_dependency(value: &str) -> String {
    normalize_dependency_token(value)
}

pub fn suspicious_chain_for_package(package: &str, snapshot: &SystemSnapshot) -> Option<Vec<String>> {
    let nodes: BTreeSet<&str> = snapshot.packages.iter().map(|pkg| pkg.name.as_str()).collect();
    if !nodes.contains(package) {
        return None;
    }

    let mut chain = Vec::new();
    let mut current = package.to_string();
    let mut seen = BTreeSet::new();

    for _ in 0..12 {
        if !seen.insert(current.clone()) {
            chain.push(current.clone());
            break;
        }
        chain.push(current.clone());
        let next = snapshot
            .packages
            .iter()
            .find(|pkg| pkg.name == current)
            .and_then(|pkg| pkg.depends_on.first())
            .map(|dep| normalize_dependency(dep));

        match next {
            Some(next_name) if nodes.contains(next_name.as_str()) => current = next_name,
            _ => break,
        }
    }

    if chain.len() >= 4 {
        Some(chain)
    } else {
        None
    }
}
