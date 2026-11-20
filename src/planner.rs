//! Offline rule selection and bounded preparation, shared by preview and execution.
use crate::{
    change_set,
    config::Project,
    git_snapshot::{self, Snapshot},
    review_context,
};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub const PLAN_SCHEMA_VERSION: u32 = 0;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_origin: Option<crate::external::Origin>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_profile: Option<crate::config::AdapterProfile>,
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
    pub commit_message: Option<String>,
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
pub struct ScopePlan {
    pub review: String,
    pub files: Vec<String>,
    pub content_bytes: usize,
    pub change_set_bytes: usize,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContextPlan {
    pub scopes: Vec<ScopePlan>,
    pub selections: Vec<crate::config::ReviewContext>,
    pub sections: Vec<ContextSection>,
    pub selected_bytes: usize,
    pub byte_limit: usize,
    pub instruction_bytes: usize,
    pub instruction_byte_limit: usize,
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
    pub adapter: Option<crate::config::AdapterProfile>,
    pub adapter_availability_checked: bool,
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
        [".please", "config.toml" | "adapter.toml"] => true,
        [".please", "checks" | "reviews", _, "rule.toml"] => true,
        [".please", "rule-sets" | "adapters", filename] => filename.ends_with(".toml"),
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
    let needs_changes = !checks_only && project.reviews.iter().any(|r| r.enabled && selects(&r.id));
    let changes = needs_changes.then(|| change_set::collect(&project.root, name, hook));
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
        .map(|r| (&r.id, &r.name, "check", &r.description, r.enabled, true))
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
        let adapter = review.and_then(|r| r.adapter.clone());
        let adapter_profile = adapter.as_ref().map(|id| project.adapters[id].clone());
        let rule = Rule {
            adapter_origin: adapter
                .as_ref()
                .and_then(|id| project.external_adapters.get(id))
                .map(|source| source.origin.clone()),
            depends_on: project
                .checks
                .iter()
                .find(|r| r.id == *id)
                .map(|r| r.depends_on.clone())
                .unwrap_or_default(),
            adapter,
            adapter_profile,
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
        if let (Some(review), Some(Ok(changed))) = (review, &changes) {
            if crate::review_input::scope_changes(review, changed).is_empty() {
                omitted.push(Omission {
                    rule,
                    reason: if review.trigger.is_some() {
                        "trigger_not_matched"
                    } else {
                        "no_changes"
                    }
                    .into(),
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
        scopes: vec![],
        selections: active
            .iter()
            .flat_map(|r| r.context.iter().cloned())
            .collect(),
        sections: vec![],
        selected_bytes: 0,
        byte_limit: review_context::MAX_CONTEXT_BYTES,
        instruction_bytes: active.iter().map(|r| r.text.len()).sum(),
        instruction_byte_limit: crate::limits::MAX_TOTAL_INSTRUCTION_BYTES,
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
        // At commit-msg the whole message is declared input, so its file is required.
        let message_file = if name == "commit-msg" {
            hook.ok_or_else(|| {
                anyhow::anyhow!("a review at commit-msg requires the managed commit-msg hook")
            })
            .and_then(|hook| hook.commit_message_path(&project.root))
            .map(Some)
        } else {
            Ok(None)
        };
        let prepared = message_file.and_then(|file| {
            let changed = changes
                .as_ref()
                .and_then(|result| result.as_ref().ok())
                .ok_or_else(|| anyhow::anyhow!("changed files are unavailable"))?;
            crate::review_input::PreparedReviewInput::prepare_with_message(
                project, &active, &snapshot, file, changed,
            )
        });
        match prepared {
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
                context.scopes = active
                    .iter()
                    .filter_map(|r| input.scopes.get(&r.id).map(|scope| (r, scope)))
                    .map(|(review, scope)| ScopePlan {
                        review: review.id.clone(),
                        files: scope.files.iter().map(|f| f.path.clone()).collect(),
                        content_bytes: scope
                            .files
                            .iter()
                            .map(|f| f.content.as_ref().map_or(0, String::len))
                            .sum(),
                        change_set_bytes: scope.change_set.as_ref().map_or(0, String::len),
                    })
                    .collect();
                review_input = Some(input);
            }
            Err(error) => blocking_reasons.push(format!("Cannot prepare review input: {error:#}")),
        }
    }
    for review in &active {
        if project.review_adapter(review).is_none() {
            blocking_reasons.push(format!("Review {}: no adapter profile configured; configure .please/adapter.toml or select a named adapter profile", review.id));
        }
    }
    let adapter = project.adapter.clone();
    let mut plan = Plan {
        change_base: changes
            .as_ref()
            .and_then(|r| r.as_ref().ok())
            .and_then(|changed| changed.base().map(str::to_owned)),
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
            commit_message: review_input
                .as_ref()
                .and_then(|input| input.commit_message.as_ref().map(|m| m.text())),
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
        adapter,
        adapter_availability_checked: false,
        planned_model_calls: active
            .iter()
            .filter(|review| review.execution_error.is_none())
            .filter(|review| review.parameters.resolve(&project.root, &review.id).is_ok())
            .count(),
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
    // Governance metadata is normalized above, so its formatting stays out of the scopes too.
    let scopes: Option<std::collections::BTreeMap<_, _>> = review_input.as_ref().map(|input| {
        input
            .scopes
            .iter()
            .map(|(id, scope)| {
                let files: Vec<_> = scope
                    .files
                    .iter()
                    .filter(|file| !metadata_path(&file.path))
                    .collect();
                (id, (files, &scope.change_set))
            })
            .collect()
    });
    let named_adapters = project.adapters.clone();
    let identity = serde_json::json!({
        "plan_schema_version": PLAN_SCHEMA_VERSION, "verifier_version": env!("CARGO_PKG_VERSION"),
        "project": project.config.project,
        "at": name, "checks_only": checks_only, "checks": checks, "reviews": reviews,
        "adapter": plan.adapter, "named_adapters": named_adapters, "head": snapshot.head, "files": files, "context": excerpts, "scopes": scopes,
        "commit_message": plan.input.commit_message,
        "adapter_origins": project.external_adapters.iter().map(|(id, source)| (id, &source.origin)).collect::<std::collections::BTreeMap<_,_>>(),
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
