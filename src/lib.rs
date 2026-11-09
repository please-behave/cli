mod check_command;
pub mod checks;
pub mod cli;
pub mod config;
pub mod discovery;
pub mod external;
pub mod git_snapshot;
pub mod limits;
pub mod parameters;
pub mod planner;
pub mod process;
pub mod provider;
pub mod reconciliation;
pub mod report;
pub mod review_context;
pub mod review_contract;
pub mod review_input;
pub mod review_trigger;
pub mod reviews;
pub mod rules;
pub mod terminal;

use anyhow::Result;
use report::{Item, Report, Status};
use std::path::Path;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum VerificationMode {
    AllRules,
    ChecksOnly,
}

pub enum Progress<'a> {
    Plan {
        total: usize,
    },
    Phase(&'static str),
    RuleQueued {
        number: usize,
        total: usize,
        name: &'a str,
        description: &'a str,
        resource: Option<&'a str>,
    },
    Tick,
    RuleStarted {
        number: usize,
        total: usize,
        kind: &'static str,
        id: &'a str,
        name: &'a str,
        description: &'a str,
    },
    RuleFinished {
        number: usize,
        total: usize,
        item: &'a Item,
        name: &'a str,
        description: &'a str,
    },
    ReviewBatch {
        count: usize,
        calls: usize,
        timeout_seconds: u64,
    },
}

pub fn verify(root: &Path, provider: &dyn provider::Provider) -> Result<Report> {
    verify_project(config::load(root)?, provider)
}

/// Verify an already validated project without loading configuration twice.
pub fn verify_project(
    project: config::Project,
    provider: &dyn provider::Provider,
) -> Result<Report> {
    verify_project_with_progress(project, provider, &mut |_| {})
}

/// Report execution events immediately, without coupling verification to terminal I/O.
pub fn verify_project_with_progress(
    project: config::Project,
    provider: &dyn provider::Provider,
    progress: &mut dyn FnMut(Progress<'_>),
) -> Result<Report> {
    verify_project_with_mode(project, provider, VerificationMode::AllRules, progress)
}

pub fn verify_project_with_mode(
    project: config::Project,
    provider: &dyn provider::Provider,
    mode: VerificationMode,
    progress: &mut dyn FnMut(Progress<'_>),
) -> Result<Report> {
    verify_project_at(project, provider, mode, None, progress)
}

pub fn verify_project_at(
    project: config::Project,
    provider: &dyn provider::Provider,
    mode: VerificationMode,
    at: Option<&str>,
    progress: &mut dyn FnMut(Progress<'_>),
) -> Result<Report> {
    verify_project_with_context(project, provider, mode, at, None, progress)
}

/// Execute rules after the command lifecycle has validated and reconciled the project.
pub fn verify_project_with_context(
    project: config::Project,
    provider: &dyn provider::Provider,
    mode: VerificationMode,
    at: Option<&str>,
    hook: Option<&reconciliation::HookContext>,
    progress: &mut dyn FnMut(Progress<'_>),
) -> Result<Report> {
    verify_project_with_fail_fast(project, provider, mode, at, hook, false, progress)
}

/// Execute with invocation-only fail-fast scheduling; existing entry points run all rules.
pub fn verify_project_with_fail_fast(
    mut project: config::Project,
    provider: &dyn provider::Provider,
    mode: VerificationMode,
    at: Option<&str>,
    hook: Option<&reconciliation::HookContext>,
    fail_fast: bool,
    progress: &mut dyn FnMut(Progress<'_>),
) -> Result<Report> {
    let checks_only = mode == VerificationMode::ChecksOnly;
    progress(Progress::Phase(
        "Preparing execution plan and working files …",
    ));
    let planner::PreparedPlan {
        plan,
        snapshot,
        review_input,
    } = planner::build(&project, at, checks_only)?;
    anyhow::ensure!(
        plan.selection_complete,
        "Execution point requires reviews; --checks-only cannot omit required rules"
    );
    let names = plan.display_names();
    planner::apply(&mut project, &plan);
    let check_count = project.checks.iter().filter(|c| c.enabled).count();
    let active: Vec<_> = project
        .reviews
        .iter()
        .filter(|r| r.enabled && !checks_only)
        .collect();
    let total = check_count + active.len();
    progress(Progress::Plan { total });
    if checks_only {
        progress(Progress::Phase(
            "Checks only: reviews omitted; no provider call.",
        ));
        if check_count == 0 {
            progress(Progress::Phase("No active checks configured."));
        }
    }
    progress(Progress::Phase("Capturing working files …"));
    let mut items = Vec::new();
    let mut provider_usage = None;
    if !plan.executable() {
        items.push(Item::simple(
            "please-plan",
            "integrity",
            Status::Error,
            plan.blocking_reasons.join("; "),
        ));
    }
    let checks: Vec<_> = project.checks.iter().filter(|c| c.enabled).collect();
    if plan.executable() {
        items.extend(checks::schedule(
            &checks,
            project.config.execution.max_parallel,
            fail_fast,
            |check| checks::execute_with_context(&project, check, &plan.at, hook),
            &mut |event| {
                let index = match event {
                    checks::Event::Queued(index, _)
                    | checks::Event::Started(index)
                    | checks::Event::Finished(index, _) => index,
                    checks::Event::Tick => {
                        progress(Progress::Tick);
                        return;
                    }
                };
                let check = checks[index];
                let number = index + 1;
                let name = &names[&check.id];
                let description = &check.description;
                progress(match event {
                    checks::Event::Queued(_, resource) => Progress::RuleQueued {
                        number,
                        total,
                        name,
                        description,
                        resource,
                    },
                    checks::Event::Started(_) => Progress::RuleStarted {
                        number,
                        total,
                        kind: "check",
                        id: &check.id,
                        name,
                        description,
                    },
                    checks::Event::Finished(_, item) => Progress::RuleFinished {
                        number,
                        total,
                        item,
                        name,
                        description,
                    },
                    checks::Event::Tick => unreachable!(),
                });
            },
        ));
    } else {
        for (offset, check) in checks.iter().enumerate() {
            let item = Item::simple(
                &check.id,
                "check",
                Status::NotRun,
                "Execution plan blocked; no command started".into(),
            );
            progress(Progress::RuleFinished {
                number: offset + 1,
                total,
                item: &item,
                name: &names[&check.id],
                description: &check.description,
            });
            items.push(item);
        }
    }
    if !snapshot.complete && !items.iter().any(|x| x.kind == "integrity") {
        items.push(Item::simple(
            "please-snapshot",
            "integrity",
            Status::Error,
            "Snapshot incomplete; verification cannot establish coverage".into(),
        ));
    }
    if !active.is_empty() {
        if report::overall(&items) == Status::Pass {
            progress(Progress::Phase(
                "Comparing working files before the provider call …",
            ));
            if let Some(error) = stability_error(&project.root, &snapshot) {
                items.push(error);
            }
        }
        if report::overall(&items) == Status::Pass {
            let (results, usage) = reviews::execute_prepared_observed(
                &project,
                &active,
                &snapshot,
                review_input.as_ref().ok_or_else(|| {
                    anyhow::anyhow!("executable review plan has no prepared input")
                })?,
                provider,
                &plan.at,
                &mut |batch, completed| {
                    if fail_fast && completed.iter().any(report::prevents_success) {
                        return false;
                    }
                    progress(Progress::ReviewBatch {
                        count: batch.len(),
                        calls: 1,
                        timeout_seconds: batch
                            .iter()
                            .filter_map(|r| project.review_provider(r).map(|p| p.timeout_seconds))
                            .max()
                            .unwrap_or(0),
                    });
                    for review in batch {
                        let offset = active.iter().position(|r| r.id == review.id).unwrap();
                        progress(Progress::RuleStarted {
                            number: check_count + offset + 1,
                            total,
                            kind: "review",
                            id: &review.id,
                            name: &names[&review.id],
                            description: &review.description,
                        });
                    }
                    true
                },
            );
            provider_usage = usage;
            for (offset, item) in results.iter().enumerate() {
                progress(Progress::RuleFinished {
                    number: check_count + offset + 1,
                    total,
                    item,
                    name: &names[&item.id],
                    description: &plan
                        .scheduled
                        .iter()
                        .find(|r| r.id == item.id)
                        .unwrap()
                        .description,
                });
            }
            items.extend(results);
        } else {
            for (offset, review) in active.iter().enumerate() {
                let item = Item::simple(
                    &review.id,
                    "review",
                    Status::NotRun,
                    if fail_fast && items.iter().any(report::prevents_success) {
                        "Fail-fast triggered by deterministic checks or snapshot; no provider call made".into()
                    } else {
                        "Deterministic checks or snapshot failed; no provider call made".into()
                    },
                );
                progress(Progress::RuleFinished {
                    number: check_count + offset + 1,
                    total,
                    item: &item,
                    name: &names[&review.id],
                    description: &review.description,
                });
                items.push(item);
            }
        }
    }
    if snapshot.complete && !items.iter().any(|i| i.id == "please-tree-stability") {
        progress(Progress::Phase("Checking final working file state …"));
        if let Some(error) = stability_error(&project.root, &snapshot) {
            items.push(error);
        }
    }
    for item in &mut items {
        if let Some(rule) = plan.scheduled.iter().find(|r| r.id == item.id) {
            item.blocking = rule.blocking;
        }
    }
    let report = Report {
        schema_version: 4,
        timestamp: chrono::Utc::now().to_rfc3339(),
        repository: project.root.to_string_lossy().into_owned(),
        project: project.config.project,
        provider: "adapter".into(),
        overall: report::overall(&items),
        git_summary: format!(
            "{} working files fingerprinted; HEAD {}; {}",
            snapshot.files.len(),
            snapshot.head.as_deref().unwrap_or("unborn"),
            git_snapshot::working_summary(&project.root)
                .unwrap_or_else(|e| format!("Git status summary unavailable: {e}"))
        ),
        snapshot,
        results: items,
        max_display_files: project.config.max_display_files,
        token_usage: provider_usage.and_then(report::TokenUsage::total),
        provider_usage,
        checks_only,
        plan,
    };
    progress(Progress::Phase("Saving report …"));
    report::write(&project.root, &report)?;
    Ok(report)
}

fn stability_error(root: &Path, snapshot: &git_snapshot::Snapshot) -> Option<Item> {
    let reason = match git_snapshot::capture(root) {
        Ok(end) if git_snapshot::compare(snapshot, &end, 5).state == "CURRENT" => return None,
        Ok(_) => "Working files changed during verification or snapshot is incomplete; rerun after changes stop".into(),
        Err(error) => format!("Cannot confirm final snapshot: {error:#}"),
    };
    Some(Item::simple(
        "please-tree-stability",
        "integrity",
        Status::Error,
        reason,
    ))
}
