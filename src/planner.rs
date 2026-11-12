//! Offline rule selection and bounded preparation, shared by preview and execution.
use crate::{
    config::Project,
    git_snapshot::{self, Snapshot},
    review_context,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const PLAN_SCHEMA_VERSION: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_origin: Option<crate::external::Origin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review_input: Option<crate::config::ReviewInput>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_config: Option<crate::config::ProviderConfig>,
    pub kind: String,
    pub description: String,
    pub blocking: bool,
    pub selected_by: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<crate::rules::EffectiveRule>,
}
impl Rule {
    pub fn display_name(&self) -> &str {
        self.name
            .as_deref()
            .unwrap_or_else(|| self.id.rsplit(':').next().unwrap_or(&self.id))
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Omission {
    pub rule: Rule,
    pub reason: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit_header: Option<String>,
    pub kind: String,
    pub head: Option<String>,
    pub fingerprint: String,
    pub complete: bool,
    pub limitations: Vec<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextSection {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub bytes: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextPlan {
    pub entry_files: Vec<String>,
    pub selections: Vec<crate::config::ReviewContext>,
    pub sections: Vec<ContextSection>,
    pub selected_bytes: usize,
    pub byte_limit: usize,
    pub instruction_bytes: usize,
    pub instruction_byte_limit: usize,
    pub manifest_hint_limit: usize,
    pub orientation_diff_bytes: usize,
    pub orientation_diff_truncated: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub change_base: Option<String>,
    pub plan_schema_version: u32,
    pub plan_id: String,
    pub status: String,
    pub blocking_reasons: Vec<String>,
    pub at: String,
    pub input: Input,
    pub checks_only: bool,
    pub scheduled: Vec<Rule>,
    pub omitted: Vec<Omission>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disabled_rules: Vec<Rule>,
    pub required_ids: Vec<String>,
    pub provider: Option<crate::config::ProviderConfig>,
    pub provider_availability_checked: bool,
    pub planned_model_calls: usize,
    pub context: ContextPlan,
}
impl Plan {
    /// Human labels are independent of the technical execution identities.
    pub fn display_names(&self) -> std::collections::BTreeMap<String, String> {
        let mut names = std::collections::BTreeMap::new();
        let mut used = BTreeSet::new();
        for rule in &self.scheduled {
            let base = rule.display_name();
            let duplicate = self
                .scheduled
                .iter()
                .filter(|r| r.display_name() == base)
                .count()
                > 1;
            let origin = rule
                .origin
                .as_ref()
                .and_then(|o| o.external.as_ref())
                .map(|o| o.path.rsplit('/').next().unwrap_or(&o.path))
                .unwrap_or("local");
            let label = if duplicate {
                format!("{base} [{origin}]")
            } else {
                base.into()
            };
            let mut unique = label.clone();
            let mut occurrence = 2;
            while !used.insert(unique.clone()) {
                unique = format!("{label} #{occurrence}");
                occurrence += 1;
            }
            names.insert(rule.id.clone(), unique);
        }
        names
    }

    pub fn executable(&self) -> bool {
        self.status == "planned"
    }
    pub fn schedules(&self, id: &str) -> bool {
        self.scheduled.iter().any(|r| r.id == id)
    }
}
pub struct PreparedPlan {
    pub plan: Plan,
    pub snapshot: Snapshot,
    pub review_input: Option<crate::review_input::PreparedReviewInput>,
}

fn metadata_path(path: &str) -> bool {
    let parts: Vec<_> = path.split('/').collect();
    match parts.as_slice() {
        [".please", "config.toml" | "provider.toml"] => true,
        [".please", "checks" | "reviews", _, "rule.toml"] => true,
        [".please", "rule-sets" | "providers", filename] => filename.ends_with(".toml"),
        _ => false,
    }
}

pub fn build(
    project: &Project,
    requested: Option<&str>,
    checks_only: bool,
) -> Result<PreparedPlan> {
    build_with_context(project, requested, checks_only, None)
}

pub fn build_with_context(
    project: &Project,
    requested: Option<&str>,
    checks_only: bool,
    hook: Option<&crate::reconciliation::HookContext>,
) -> Result<PreparedPlan> {
    let name = requested.unwrap_or("pre-push");
    anyhow::ensure!(
        project.effective.points.contains(name),
        "unknown execution point: {name}"
    );
    let selects = |id: &str| {
        project
            .effective
            .rules
            .get(id)
            .is_some_and(|r| r.execution_points.contains(name))
    };
    let snapshot = git_snapshot::capture(&project.root).unwrap_or_else(|error| Snapshot {
        head: None,
        fingerprint: String::new(),
        files: Default::default(),
        complete: false,
        limitations: vec![format!("{error:#}")],
    });
    let needs_changes = !checks_only
        && project
            .reviews
            .iter()
            .any(|r| r.enabled && selects(&r.id) && r.trigger.is_some());
    let changes = needs_changes.then(|| -> Result<_> {
        let base = std::env::var("PLEASE_CHANGE_BASE").map_err(|_| {
            anyhow::anyhow!(
                "PLEASE_CHANGE_BASE must explicitly name a baseline commit for triggered reviews"
            )
        })?;
        git_snapshot::changed_paths(&project.root, &base)
    });
    let mut scheduled = Vec::new();
    let mut omitted = Vec::new();
    let mut disabled_rules = Vec::new();
    let mut required_ids = Vec::new();
    let mut blocking_reasons = Vec::new();
    if let Some(Err(error)) = &changes {
        blocking_reasons.push(format!("Cannot determine review changes: {error:#}"));
    }
    for (id, display_name, kind, description, enabled, blocking) in project
        .checks
        .iter()
        .map(|r| {
            (
                &r.id,
                &r.name,
                "check",
                &r.description,
                r.enabled,
                r.blocking,
            )
        })
        .chain(project.reviews.iter().map(|r| {
            (
                &r.id,
                &r.name,
                "review",
                &r.description,
                r.enabled,
                r.blocking,
            )
        }))
    {
        if !selects(id) && enabled {
            continue;
        }
        let origin = project.effective.rules.get(id).cloned();
        let selected_by = if enabled {
            vec![format!("at:{name}")]
        } else {
            vec![]
        };
        let review = project.reviews.iter().find(|r| r.id == *id);
        let provider = review.and_then(|r| r.provider.clone());
        let provider_config = provider.as_ref().map(|id| project.providers[id].clone());
        let rule = Rule {
            provider_origin: provider
                .as_ref()
                .and_then(|id| project.external_providers.get(id))
                .map(|source| source.origin.clone()),
            review_input: review
                .filter(|r| r.input == crate::config::ReviewInput::CommitHeader)
                .map(|r| r.input),
            depends_on: project
                .checks
                .iter()
                .find(|r| r.id == *id)
                .map(|r| r.depends_on.clone())
                .unwrap_or_default(),
            provider,
            provider_config,
            id: id.clone(),
            name: Some(display_name.clone()),
            kind: kind.into(),
            description: description.clone(),
            blocking,
            selected_by,
            origin,
        };
        if !enabled {
            disabled_rules.push(rule);
            continue;
        }
        if let (Some(trigger), Some(Ok((_, paths)))) =
            (review.and_then(|r| r.trigger.as_ref()), &changes)
        {
            if !crate::review_trigger::matches(trigger, paths) {
                omitted.push(Omission {
                    rule,
                    reason: "trigger_not_matched".into(),
                });
                continue;
            }
        }
        required_ids.push(id.clone());
        scheduled.push(rule);
    }
    validate_dependencies(project, &scheduled)?;
    if needs_changes && changes.as_ref().is_some_and(|r| r.is_ok()) {
        match git_snapshot::capture(&project.root) {
            Ok(after) if git_snapshot::compare(&snapshot, &after, 0).state == "CURRENT" => {}
            _ => blocking_reasons.push(
                "Working files changed during change comparison or coverage is incomplete".into(),
            ),
        }
    }
    if !snapshot.complete {
        blocking_reasons.push("Input snapshot is incomplete; no verification can start".into());
    }
    // Checks-only runs report reviews as NOT_RUN: no input is prepared and no adapter is needed.
    let active: Vec<_> = project
        .reviews
        .iter()
        .filter(|r| !checks_only && scheduled.iter().any(|p| p.id == r.id))
        .collect();
    let mut context = ContextPlan {
        entry_files: active
            .iter()
            .flat_map(|r| r.files.iter().cloned())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        selections: active
            .iter()
            .flat_map(|r| r.context.iter().cloned())
            .collect(),
        sections: vec![],
        selected_bytes: 0,
        byte_limit: review_context::MAX_CONTEXT_BYTES,
        instruction_bytes: active.iter().map(|r| r.text.len()).sum(),
        instruction_byte_limit: crate::limits::MAX_TOTAL_INSTRUCTION_BYTES,
        manifest_hint_limit: crate::limits::MAX_MANIFEST_HINTS,
        orientation_diff_bytes: 0,
        orientation_diff_truncated: false,
    };
    for section in &mut context.selections {
        section.headings.sort();
        section.headings.dedup();
    }
    context
        .selections
        .sort_by(|a, b| (&a.path, &a.headings).cmp(&(&b.path, &b.headings)));
    context
        .selections
        .dedup_by(|a, b| a.path == b.path && a.headings == b.headings);
    let mut review_input = None;
    if !active.is_empty() && blocking_reasons.is_empty() {
        match crate::review_input::PreparedReviewInput::prepare_with_context(
            project, &active, &snapshot, hook,
        ) {
            Ok(input) => {
                context.selected_bytes = input.excerpts.iter().map(|e| e.text.len()).sum();
                context.sections = input
                    .excerpts
                    .iter()
                    .map(|e| ContextSection {
                        path: e.path.clone(),
                        start_line: e.start_line,
                        end_line: e.end_line,
                        bytes: e.text.len(),
                    })
                    .collect();
                context.orientation_diff_bytes = input.diff.len();
                context.orientation_diff_truncated = input.diff_truncated;
                review_input = Some(input);
            }
            Err(error) => blocking_reasons.push(format!("Cannot prepare review input: {error:#}")),
        }
    }
    for review in &active {
        if project.review_provider(review).is_none() {
            blocking_reasons.push(format!("Review {}: no adapter configured; configure .please/provider.toml or select a named provider", review.id));
        }
    }
    let provider = project.provider.clone();
    let mut plan = Plan {
        change_base: changes
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .map(|(base, _)| base.clone()),
        plan_schema_version: PLAN_SCHEMA_VERSION,
        plan_id: String::new(),
        status: if blocking_reasons.is_empty() {
            "planned"
        } else {
            "blocked"
        }
        .into(),
        blocking_reasons,
        at: name.into(),
        input: Input {
            commit_header: review_input
                .as_ref()
                .and_then(|input| input.commit_header.clone()),
            kind: "worktree".into(),
            head: snapshot.head.clone(),
            fingerprint: snapshot.fingerprint.clone(),
            complete: snapshot.complete,
            limitations: snapshot.limitations.clone(),
        },
        checks_only,
        scheduled,
        omitted,
        disabled_rules,
        required_ids,
        provider,
        provider_availability_checked: false,
        planned_model_calls: active
            .iter()
            .filter(|review| review.execution_error.is_none())
            .filter_map(|review| {
                review
                    .parameters
                    .resolve(&project.root, &review.id)
                    .ok()
                    .map(|environment| {
                        (
                            review.provider.clone(),
                            environment,
                            review.include_diff,
                            review.input,
                        )
                    })
            })
            .collect::<BTreeSet<_>>()
            .len(),
        context,
    };
    // Normalize metadata separately from raw file hashes so TOML formatting and
    // selector order do not change semantic identity. The report still
    // records the exact raw snapshot for stability/freshness comparisons.
    let mut checks: Vec<_> = project
        .checks
        .iter()
        .filter(|r| selects(&r.id))
        .cloned()
        .collect();
    for check in &mut checks {
        check.run_at.sort();
    }
    let mut reviews: Vec<_> = project
        .reviews
        .iter()
        .filter(|r| selects(&r.id))
        .cloned()
        .collect();
    for review in &mut reviews {
        if let Some(trigger) = &mut review.trigger {
            trigger.changed_files.sort();
            trigger.changed_files.dedup();
        }
        review.files.sort();
        review.run_at.sort();
        for context in &mut review.context {
            context.headings.sort();
            context.headings.dedup();
        }
        review
            .context
            .sort_by(|a, b| (&a.path, &a.headings).cmp(&(&b.path, &b.headings)));
    }
    let files: std::collections::BTreeMap<_, _> = snapshot
        .files
        .iter()
        .filter(|(p, _)| !metadata_path(p))
        .collect();
    let excerpts = review_input
        .as_ref()
        .map_or(&[][..], |input| input.excerpts.as_slice());
    let named_providers = project.providers.clone();
    let identity = serde_json::json!({
        "plan_schema_version": PLAN_SCHEMA_VERSION, "verifier_version": env!("CARGO_PKG_VERSION"),
        "project": project.config.project,
        "at": name, "checks_only": checks_only, "checks": checks, "reviews": reviews,
        "provider": plan.provider, "named_providers": named_providers, "head": snapshot.head, "files": files, "context": excerpts,
        "commit_header": plan.input.commit_header,
        "provider_origins": project.external_providers.iter().map(|(id, source)| (id, &source.origin)).collect::<std::collections::BTreeMap<_,_>>(),
        "complete": snapshot.complete, "change_base": plan.change_base, "scheduled": plan.scheduled, "omitted": plan.omitted,
    });
    plan.plan_id = format!("{:x}", Sha256::digest(serde_json::to_vec(&identity)?));
    Ok(PreparedPlan {
        plan,
        snapshot,
        review_input,
    })
}

pub fn apply(project: &mut Project, plan: &Plan) {
    project.checks.retain(|r| plan.schedules(&r.id));
    project.reviews.retain(|r| plan.schedules(&r.id));
}

/// Resolve only exact identities in the effective selection; never expand it.
fn validate_dependencies(project: &Project, scheduled: &[Rule]) -> Result<()> {
    use std::collections::BTreeMap;
    let checks: BTreeMap<_, _> = scheduled
        .iter()
        .filter(|r| r.kind == "check")
        .map(|r| (r.id.as_str(), r))
        .collect();
    for rule in checks.values() {
        for dependency in &rule.depends_on {
            anyhow::ensure!(
                !project.reviews.iter().any(|r| r.id == *dependency),
                "Check {}: depends_on {dependency:?} refers to an AI review; only check dependencies are supported",
                rule.id
            );
            if !checks.contains_key(dependency.as_str()) {
                let candidates: Vec<_> = project
                    .checks
                    .iter()
                    .filter(|c| c.id.rsplit(':').next() == Some(dependency.as_str()))
                    .map(|c| c.id.as_str())
                    .collect();
                anyhow::ensure!(
                    candidates.len() <= 1,
                    "Check {}: ambiguous depends_on identity {dependency:?}; use an exact namespaced check ID: {}",
                    rule.id,
                    candidates.join(", ")
                );
                anyhow::bail!(
                    "Check {}: depends_on {dependency:?} is not selected at this execution point; explicitly select the enabled prerequisite using its exact check ID",
                    rule.id
                );
            }
            anyhow::ensure!(
                *dependency != rule.id,
                "Check {}: depends_on self-dependency",
                rule.id
            );
        }
    }
    let mut remaining: BTreeSet<_> = checks.keys().copied().collect();
    while !remaining.is_empty() {
        let ready: Vec<_> = remaining
            .iter()
            .copied()
            .filter(|id| {
                checks[id]
                    .depends_on
                    .iter()
                    .all(|d| !remaining.contains(d.as_str()))
            })
            .collect();
        anyhow::ensure!(
            !ready.is_empty(),
            "Check dependency cycle involving: {}",
            remaining.iter().copied().collect::<Vec<_>>().join(", ")
        );
        for id in ready {
            remaining.remove(id);
        }
    }
    Ok(())
}
