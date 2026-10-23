//! Transparent, weighted health score.
//!
//! Check results are grouped by category before weighting.  That prevents the
//! same underlying problem (for example two integrity signals) from being
//! counted twice.  Every component exposes its weight, score and the exact
//! contribution used by the final formula.

use std::collections::BTreeMap;

use crate::analysis::checks::{CheckStatus, ChecksReport};

/// One weighted contribution to the health score.
#[derive(Debug, Clone, PartialEq)]
pub struct HealthComponent {
    pub id: String,
    pub category: String,
    pub title: String,
    pub weight: f32,
    pub score: f32,
    pub contributed: f32,
    pub status: CheckStatus,
    pub check_ids: Vec<String>,
    pub explanation: String,
}

/// Final health report.
#[derive(Debug, Clone, PartialEq)]
pub struct HealthReport {
    pub score: f32,
    pub grade: String,
    pub components: Vec<HealthComponent>,
    pub formula: String,
    pub notes: Vec<String>,
}

impl HealthReport {
    /// Human-readable, multi-line explanation of every contribution.
    pub fn explanation(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("Health: {:.1}/100 ({})\n", self.score, self.grade));
        out.push_str(&format!("Formula: {}\n", self.formula));
        for component in &self.components {
            out.push_str(&format!(
                "- {} [{}] weight {:.2} × score {:.1} = {:.2}: {}\n",
                component.title,
                component.status.as_str(),
                component.weight,
                component.score,
                component.contributed,
                component.explanation
            ));
        }
        for note in &self.notes {
            out.push_str(&format!("Note: {note}\n"));
        }
        out
    }

    pub fn is_healthy(&self) -> bool {
        self.score >= 80.0
    }
}

/// Compute the weighted health score.
///
/// `warn` and `critical` are the lower bounds for the "good"/"fair" grades.
pub fn compute_health(report: &ChecksReport, warn: f32, critical: f32) -> HealthReport {
    let mut groups: BTreeMap<String, Group> = BTreeMap::new();
    let mut notes = Vec::new();

    for result in &report.results {
        if result.weight <= 0.0 {
            continue;
        }
        if result.status == CheckStatus::Unknown {
            notes.push(format!(
                "{} was unknown and treated as neutral (not a penalty)",
                result.title
            ));
        }
        let group = groups
            .entry(result.category.clone())
            .or_insert_with(|| Group {
                title: result.category.clone(),
                weight: 0.0,
                weighted_score: 0.0,
                minimum_status: result.status,
                ids: Vec::new(),
                details: Vec::new(),
            });
        group.weight += result.weight;
        group.weighted_score += result.weight * result.score;
        group.ids.push(result.id.clone());
        group.details.push(result.summary.clone());
        group.minimum_status = worse(group.minimum_status, result.status);
    }

    let mut components = Vec::new();
    let mut total_weight = 0.0f32;
    let mut total_contribution = 0.0f32;

    for (category, group) in groups {
        if group.weight <= 0.0 {
            continue;
        }
        let score = (group.weighted_score / group.weight).clamp(0.0, 100.0);
        let contributed = group.weight * score;
        total_weight += group.weight;
        total_contribution += contributed;

        let explanation = if group.ids.len() > 1 {
            format!(
                "{} checks combined in category '{category}' (no double counting): {}",
                group.ids.len(),
                group.details.join("; ")
            )
        } else {
            group.details.first().cloned().unwrap_or_default()
        };

        components.push(HealthComponent {
            id: category.clone(),
            category,
            title: group.title,
            weight: group.weight,
            score,
            contributed,
            status: group.minimum_status,
            check_ids: group.ids,
            explanation,
        });
    }

    components.sort_by(|a, b| b.weight.total_cmp(&a.weight).then_with(|| a.id.cmp(&b.id)));

    let score = if total_weight > 0.0 {
        (total_contribution / total_weight).clamp(0.0, 100.0)
    } else {
        100.0
    };

    let grade = grade_for(score, warn, critical);
    let formula = if total_weight > 0.0 {
        let terms: Vec<String> = components
            .iter()
            .map(|component| format!("({:.2} × {:.1})", component.weight, component.score))
            .collect();
        format!("{} / {:.2} = {:.1}", terms.join(" + "), total_weight, score)
    } else {
        "no weighted checks; neutral score 100".to_string()
    };

    HealthReport {
        score,
        grade,
        components,
        formula,
        notes,
    }
}

fn worse(left: CheckStatus, right: CheckStatus) -> CheckStatus {
    fn rank(status: CheckStatus) -> u8 {
        match status {
            CheckStatus::Critical => 3,
            CheckStatus::Warning => 2,
            CheckStatus::Unknown => 1,
            CheckStatus::Ok => 0,
        }
    }
    if rank(right) > rank(left) {
        right
    } else {
        left
    }
}

fn grade_for(score: f32, warn: f32, critical: f32) -> String {
    if score >= 90.0 {
        "excellent".to_string()
    } else if score >= warn {
        "good".to_string()
    } else if score >= critical {
        "fair".to_string()
    } else {
        "critical".to_string()
    }
}

struct Group {
    title: String,
    weight: f32,
    weighted_score: f32,
    minimum_status: CheckStatus,
    ids: Vec<String>,
    details: Vec<String>,
}

/// Build a report directly from components, useful for tests and alternate
/// signal sources.
pub fn from_components(mut components: Vec<HealthComponent>) -> HealthReport {
    components.sort_by(|a, b| b.weight.total_cmp(&a.weight).then_with(|| a.id.cmp(&b.id)));
    let total_weight: f32 = components.iter().map(|component| component.weight).sum();
    let total_contribution: f32 = components
        .iter()
        .map(|component| component.contributed)
        .sum();
    let score = if total_weight > 0.0 {
        (total_contribution / total_weight).clamp(0.0, 100.0)
    } else {
        100.0
    };
    let formula = if total_weight > 0.0 {
        format!(
            "{} / {:.2} = {:.1}",
            components
                .iter()
                .map(|component| format!("({:.2} × {:.1})", component.weight, component.score))
                .collect::<Vec<_>>()
                .join(" + "),
            total_weight,
            score
        )
    } else {
        "no components; neutral score 100".to_string()
    };
    HealthReport {
        score,
        grade: grade_for(score, 80.0, 60.0),
        components,
        formula,
        notes: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::checks::{CheckResult, CheckStatus, ChecksReport};

    #[test]
    fn merges_same_category_without_double_counting() {
        let results = vec![
            CheckResult::new(
                "pacnew",
                "Pacnew",
                "configuration",
                CheckStatus::Warning,
                1.0,
                "2 files",
            ),
            CheckResult::new(
                "config-owner",
                "Config owner",
                "configuration",
                CheckStatus::Critical,
                1.0,
                "1 owner mismatch",
            ),
            CheckResult::new("cycles", "Cycles", "packages", CheckStatus::Ok, 0.5, "none"),
        ];
        let report = compute_health(&ChecksReport::new(results), 80.0, 60.0);

        assert_eq!(report.components.len(), 2, "categories must be merged");
        let configuration = report
            .components
            .iter()
            .find(|component| component.category == "configuration")
            .expect("configuration component");
        assert_eq!(configuration.weight, 2.0);
        assert_eq!(configuration.score, 40.0);
        assert_eq!(configuration.check_ids.len(), 2);
        assert!(configuration.explanation.contains("no double counting"));
        assert_eq!(report.score, (2.0 * 40.0 + 0.5 * 100.0) / 2.5);
        assert!((report.score - 52.0).abs() < 0.01);
        assert_eq!(report.grade, "critical");
        assert!(report.formula.contains("2.00 × 40.0"));
    }

    #[test]
    fn empty_report_is_neutral() {
        let report = compute_health(&ChecksReport::new(Vec::new()), 80.0, 60.0);
        assert_eq!(report.score, 100.0);
        assert_eq!(report.grade, "excellent");
        assert!(report.components.is_empty());
    }

    #[test]
    fn unknown_signals_are_neutral_and_explained() {
        let results = vec![CheckResult::new(
            "integrity",
            "Integrity",
            "integrity",
            CheckStatus::Unknown,
            1.0,
            "pacman unavailable",
        )];
        let report = compute_health(&ChecksReport::new(results), 80.0, 60.0);
        assert_eq!(report.score, 100.0);
        assert!(report.notes.iter().any(|note| note.contains("unknown")));
    }
}
