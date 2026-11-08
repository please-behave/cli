//! Domain validation and effective assignments, independent of configuration loading.
use crate::config::{ExecutionPoints, RuleSet};
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
};

pub const BUILTIN_POINTS: [&str; 4] = ["pre-commit", "commit-msg", "pre-push", "check"];

#[derive(Debug, Clone)]
pub struct Assignment {
    pub id: String,
    pub enabled: bool,
    pub run_at: Vec<String>,
    pub source: PathBuf,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveRule {
    pub source: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<crate::external::Origin>,
    pub execution_points: BTreeSet<String>,
    pub rule_sets: BTreeSet<String>,
}
#[derive(Debug, Clone)]
pub struct EffectiveConfig {
    pub points: BTreeSet<String>,
    pub rules: BTreeMap<String, EffectiveRule>,
    pub diagnostics: Vec<String>,
}
pub(crate) fn name(value: &str) -> Result<()> {
    ensure!(
        !value.is_empty()
            && value.len() <= crate::limits::MAX_ID_BYTES
            && value.as_bytes()[0].is_ascii_alphanumeric()
            && value
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || b"-_".contains(&c)),
        "invalid execution-point or rule-set name {value:?}: use lowercase ASCII letters, digits, - or _, starting with a letter or digit"
    );
    Ok(())
}
fn references(
    values: &[String],
    points: &BTreeSet<String>,
    entity: &str,
) -> Result<BTreeSet<String>> {
    let mut result = BTreeSet::new();
    for value in values {
        ensure!(
            points.contains(value),
            "{entity}: run_at: unknown execution point {value:?}"
        );
        ensure!(
            result.insert(value.clone()),
            "{entity}: run_at: duplicate execution point {value:?}"
        );
    }
    Ok(result)
}
pub fn resolve(
    declarations: &ExecutionPoints,
    rules: &[Assignment],
    sets: &[(PathBuf, RuleSet)],
) -> Result<EffectiveConfig> {
    let mut points: BTreeSet<String> = BUILTIN_POINTS.into_iter().map(String::from).collect();
    for point in &declarations.custom {
        name(point)
            .map_err(|e| anyhow::anyhow!(".please/config.toml: execution_points.custom: {e}"))?;
        ensure!(
            points.insert(point.clone()),
            ".please/config.toml: execution_points.custom: duplicate or reserved execution point {point:?}"
        );
    }
    let mut ids = BTreeMap::new();
    for rule in rules {
        ensure!(
            ids.insert(rule.id.clone(), rule).is_none(),
            "{}: rule {}: duplicate id",
            rule.source.display(),
            rule.id
        );
    }
    let mut memberships: BTreeMap<String, (BTreeSet<String>, BTreeSet<String>)> = BTreeMap::new();
    let mut set_ids = BTreeSet::new();
    for (source, set) in sets {
        let entity = format!("{}: rule set {}", source.display(), set.id);
        name(&set.id).map_err(|e| anyhow::anyhow!("{entity}: id: {e}"))?;
        ensure!(
            set_ids.insert(&set.id),
            "{entity}: id: duplicate rule-set ID"
        );
        ensure!(
            !set.description.trim().is_empty() && !set.description.contains('\0'),
            "{entity}: description must be nonempty and contain no NUL"
        );
        let at = references(&set.run_at, &points, &entity)?;
        ensure!(
            !at.is_empty(),
            "{entity}: run_at: rule set needs at least one execution point"
        );
        for id in &set.rules {
            ensure!(
                ids.contains_key(id),
                "{entity}: rules: unknown rule {id:?}; rule sets may reference rules only, never other rule sets"
            );
            {
                let (effective, sets) = memberships.entry(id.clone()).or_default();
                effective.extend(at.iter().cloned());
                sets.insert(set.id.clone());
            }
        }
    }
    let mut effective = BTreeMap::new();
    let mut diagnostics = Vec::new();
    for (id, rule) in ids {
        let entity = format!("{}: rule {id}", rule.source.display());
        let own = references(&rule.run_at, &points, &entity)?;
        let (at, sets) = memberships
            .remove(&id)
            .unwrap_or((own.clone(), BTreeSet::new()));
        if !sets.is_empty() && !own.is_empty() {
            diagnostics.push(format!(
                "{entity}: own run_at is overridden by rule sets: {}",
                sets.iter().cloned().collect::<Vec<_>>().join(", ")
            ));
        }
        ensure!(
            !rule.enabled || !at.is_empty(),
            "{entity}: run_at: active rule needs an effective execution point (own run_at or an rule set)"
        );
        effective.insert(
            id,
            EffectiveRule {
                source: rule.source.clone(),
                external: None,
                execution_points: if rule.enabled { at } else { BTreeSet::new() },
                rule_sets: sets,
            },
        );
    }
    Ok(EffectiveConfig {
        points,
        rules: effective,
        diagnostics,
    })
}
