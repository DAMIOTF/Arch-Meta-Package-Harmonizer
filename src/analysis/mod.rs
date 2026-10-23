//! Analysis layer: checks, health, Harmony suggestions, timelines, blueprints.
//!
//! [`analyze`] is the cohesive entry point used by the CLI/TUI.  Lower-level
//! modules remain public so reports can select just the data they need.

pub mod blueprint;
pub mod checks;
pub mod harmony;
pub mod health;
pub mod timeline;

pub use blueprint::{
    Blueprint, BlueprintDiff, BlueprintPackage, apply, apply_blueprint, blueprint_to_toml, diff,
    diff_blueprint, export, export_blueprint, parse, parse_blueprint, plan,
};
pub use checks::{
    CheckOptions, CheckResult, CheckStatus, ChecksReport, check_cache_old_versions, check_cycles,
    check_foreign_packages, check_kernel_modules, check_pacman_integrity, check_pacnew_and_pacsave,
    check_sync_db_age, run_checks,
};
pub use harmony::{
    HarmonyCategory, HarmonyConfig, Suggestion, SuggestionRule, default_harmony,
    suggest as harmony_suggest,
};
pub use health::{HealthComponent, HealthReport, compute_health};
pub use timeline::{
    LogPackageAction, PackageAction, PacmanTransaction, default_log_path, load_timeline,
    parse_pacman_log,
};

use crate::config::Config;
use crate::graph::{Graph, RemovalImpact};
use crate::model::Snapshot;

/// Options for a complete analysis run.
#[derive(Debug, Clone)]
pub struct AnalysisOptions {
    pub checks: CheckOptions,
    pub protected: Vec<String>,
    pub harmony: HarmonyConfig,
    pub removal_targets: Vec<String>,
    pub health_warn: f32,
    pub health_critical: f32,
    pub include_default_harmony: bool,
}

impl Default for AnalysisOptions {
    fn default() -> Self {
        Self {
            checks: CheckOptions::default(),
            protected: Vec::new(),
            harmony: default_harmony(),
            removal_targets: Vec::new(),
            health_warn: 80.0,
            health_critical: 60.0,
            include_default_harmony: true,
        }
    }
}

impl AnalysisOptions {
    /// Derive checks/health/protection settings from [`Config`].
    pub fn from_config(config: &Config) -> Self {
        let mut options = Self::default();
        options.checks.sync_max_age_days = config.thresholds.sync_db_max_age_days;
        options.checks.cache_old_days = config.thresholds.cache_old_days;
        options.protected = config.protected_packages.clone();
        options.health_warn = config.thresholds.health_warn;
        options.health_critical = config.thresholds.health_critical;
        options
    }

    /// Enable or disable the built-in Harmony catalog.
    pub fn with_default_harmony(mut self, enabled: bool) -> Self {
        self.include_default_harmony = enabled;
        self
    }
}

/// Full analysis result.
#[derive(Debug, Clone)]
pub struct Analysis {
    pub snapshot: Snapshot,
    pub graph: Graph,
    pub checks: ChecksReport,
    pub health: HealthReport,
    pub suggestions: Vec<Suggestion>,
    pub removal: Option<RemovalImpact>,
    /// Requested removal targets skipped because they are protected.
    pub protected_skipped: Vec<String>,
    pub warnings: Vec<String>,
}

/// Run the full analysis pipeline.
pub fn analyze(snapshot: &Snapshot, options: &AnalysisOptions) -> Analysis {
    let graph = Graph::build(&snapshot.packages);
    let checks = run_checks(snapshot, &graph, &options.checks);
    let health = compute_health(&checks, options.health_warn, options.health_critical);

    let harmony_config = if options.include_default_harmony {
        let mut config = default_harmony();
        merge_harmony(&mut config, &options.harmony);
        config
    } else {
        options.harmony.clone()
    };
    let suggestions = harmony_config.suggest(&snapshot.packages);

    let mut protected_skipped = Vec::new();
    let target_indices: Vec<usize> = options
        .removal_targets
        .iter()
        .filter_map(|name| {
            if options.protected.iter().any(|protected| protected == name) {
                protected_skipped.push(name.clone());
                None
            } else {
                snapshot.find_index(name)
            }
        })
        .collect();
    let removal = if target_indices.is_empty() {
        None
    } else {
        Some(graph.removal_impact(&target_indices))
    };

    Analysis {
        snapshot: snapshot.clone(),
        graph,
        checks,
        health,
        suggestions,
        removal,
        protected_skipped,
        warnings: Vec::new(),
    }
}

fn merge_harmony(base: &mut HarmonyConfig, additions: &HarmonyConfig) {
    for category in &additions.categories {
        match base
            .categories
            .iter_mut()
            .find(|existing| existing.id == category.id)
        {
            Some(existing) => *existing = category.clone(),
            None => base.categories.push(category.clone()),
        }
    }
    for rule in &additions.rules {
        match base
            .rules
            .iter_mut()
            .find(|existing| existing.id == rule.id)
        {
            Some(existing) => *existing = rule.clone(),
            None => base.rules.push(rule.clone()),
        }
    }
    for (key, value) in &additions.extra {
        base.extra.insert(key.clone(), value.clone());
    }
    base.categories
        .sort_by(|a, b| b.priority.cmp(&a.priority).then_with(|| a.id.cmp(&b.id)));
    base.rules.sort_by(|a, b| a.id.cmp(&b.id));
}

/// Resolve removal names to indices without applying protection rules.
pub fn removal_indices(graph: &Graph, names: &[String]) -> Vec<usize> {
    names
        .iter()
        .filter_map(|name| graph.index_of(name))
        .collect()
}

/// Compute removal impact for names, skipping protected packages.
pub fn removal_impact_for(
    graph: &Graph,
    names: &[String],
    protected: &[String],
) -> (Option<RemovalImpact>, Vec<String>) {
    let mut skipped = Vec::new();
    let indices: Vec<usize> = names
        .iter()
        .filter_map(|name| {
            if protected.iter().any(|entry| entry == name) {
                skipped.push(name.clone());
                None
            } else {
                graph.index_of(name)
            }
        })
        .collect();

    if indices.is_empty() {
        (None, skipped)
    } else {
        (Some(graph.removal_impact(&indices)), skipped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Package;

    fn snapshot() -> Snapshot {
        Snapshot::new(vec![
            Package {
                name: "app".into(),
                version: "1.0-1".into(),
                explicit: true,
                reason: Some("1".into()),
                size: 100,
                depends: vec!["libfoo".into()],
                ..Package::default()
            },
            Package {
                name: "libfoo".into(),
                version: "1.0-1".into(),
                explicit: false,
                reason: Some("0".into()),
                size: 50,
                ..Package::default()
            },
            Package {
                name: "docker".into(),
                version: "1.0-1".into(),
                explicit: true,
                reason: Some("1".into()),
                ..Package::default()
            },
        ])
    }

    #[test]
    fn pipeline_computes_health_suggestions_and_removal() {
        let mut options = AnalysisOptions::default();
        options.checks.run_integrity = false;
        options.checks.root_prefix = std::env::temp_dir().join("amph-definitely-missing-root");
        options.checks.pacman_root = options.checks.root_prefix.join("var/lib/pacman");
        options.checks.cache_dir = options.checks.root_prefix.join("var/cache/pacman/pkg");
        options.removal_targets = vec!["app".into(), "linux".into()];
        options.protected = vec!["linux".into()];

        let analysis = analyze(&snapshot(), &options);
        assert!(analysis.removal.is_some());
        assert_eq!(analysis.protected_skipped, vec!["linux"]);
        assert!(
            analysis
                .suggestions
                .iter()
                .any(|suggestion| suggestion.package == "docker-buildx")
        );
        assert!(analysis.health.score > 0.0);
        assert_eq!(analysis.graph.len(), 3);
    }
}
