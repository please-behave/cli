use crate::{git_snapshot::Snapshot, process::Capture};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{io::Write, path::Path};

pub use crate::limits::MAX_SUMMARY_CHARS;

/// Provider-reported counters summed across review invocations. A counter is
/// `None` when any invocation did not report it as a positive number.
/// Cache and reasoning counters are subsets, never added to the total again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenUsage {
    pub input_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub reasoning_output_tokens: Option<u64>,
    pub cache_write_input_tokens: Option<u64>,
}
impl TokenUsage {
    pub fn total(self) -> Option<u64> {
        self.input_tokens?.checked_add(self.output_tokens?)
    }
    pub fn is_unavailable(self) -> bool {
        self.input_tokens.is_none()
            && self.cached_input_tokens.is_none()
            && self.output_tokens.is_none()
            && self.reasoning_output_tokens.is_none()
            && self.cache_write_input_tokens.is_none()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Status {
    Pass,
    Fail,
    Error,
    NotRun,
    Cancelled,
}
impl Status {
    pub fn label(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::Error => "ERROR",
            Self::NotRun => "NOT_RUN",
            Self::Cancelled => "CANCELLED",
        }
    }
    pub fn exit_code(self) -> i32 {
        match self {
            Self::Pass => 0,
            Self::Fail => 1,
            _ => 2,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceSource {
    CommitHeader,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<EvidenceSource>,
    pub message: String,
    pub path: Option<String>,
    pub line: Option<u32>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub id: String,
    pub kind: String,
    pub status: Status,
    pub blocking: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub configuration_error: bool,
    /// Immediate failed prerequisite; its originating outcome determines enforcement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked_by: Option<String>,
    pub duration_ms: u64,
    pub reason: String,
    pub findings: Vec<Finding>,
    pub exit_code: Option<i32>,
    pub stdout: Option<Capture>,
    pub stderr: Option<Capture>,
}
fn is_false(value: &bool) -> bool {
    !value
}
impl Item {
    pub fn simple(id: &str, kind: &str, status: Status, reason: String) -> Self {
        Self {
            id: id.into(),
            kind: kind.into(),
            status,
            blocking: true,
            configuration_error: false,
            blocked_by: None,
            duration_ms: 0,
            reason,
            findings: vec![],
            exit_code: None,
            stdout: None,
            stderr: None,
        }
    }
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema_version: u32,
    pub timestamp: String,
    pub repository: String,
    pub project: String,
    pub provider: String,
    pub overall: Status,
    pub snapshot: Snapshot,
    pub results: Vec<Item>,
    pub git_summary: String,
    pub max_display_files: usize,
    pub token_usage: Option<u64>,
    #[serde(default)]
    pub provider_usage: Option<TokenUsage>,
    #[serde(default)]
    pub checks_only: bool,
    pub plan: crate::planner::Plan,
}
/// Whether a completed result prevents successful governance execution.
pub(crate) fn prevents_success(item: &Item) -> bool {
    match item.status {
        Status::Error => item.kind != "review" || item.blocking || item.configuration_error,
        Status::Fail => item.blocking,
        Status::NotRun => item.blocked_by.is_none(),
        Status::Cancelled => true,
        Status::Pass => false,
    }
}

pub fn overall(items: &[Item]) -> Status {
    overall_of(items, false)
}

/// Overall status; a checks-only run is decided by its checks, as its reviews are `NOT_RUN` by request.
pub fn overall_of(items: &[Item], checks_only: bool) -> Status {
    let items: Vec<&Item> = items
        .iter()
        .filter(|x| !(checks_only && x.kind == "review" && x.status == Status::NotRun))
        .collect();
    if items
        .iter()
        .any(|x| x.status == Status::Error && prevents_success(x))
    {
        Status::Error
    } else if items.iter().any(|x| x.status == Status::Fail && x.blocking) {
        Status::Fail
    } else if items
        .iter()
        .any(|x| matches!(x.status, Status::NotRun | Status::Cancelled) && prevents_success(x))
    {
        Status::Error
    } else {
        Status::Pass
    }
}
pub fn write(root: &Path, report: &Report) -> Result<()> {
    let target = root.join(crate::config::path("reports"));
    if target
        .symlink_metadata()
        .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
    {
        crate::config::source_path(root, &crate::config::path(""))?;
        std::fs::create_dir(&target)?;
    }
    let directory = crate::config::inside(root, &crate::config::path("reports"))?;
    // Reports must be local regular files, never an output symlink into another directory.
    ensure!(
        directory == root.join(crate::config::path("reports")),
        "reports directory cannot be a symlink"
    );
    let mut temporary = tempfile::NamedTempFile::new_in(&directory)?;
    serde_json::to_writer_pretty(&mut temporary, report)?;
    temporary.write_all(b"\n")?;
    temporary.as_file().sync_all()?;
    temporary.persist(directory.join("last.json"))?;
    #[cfg(unix)]
    std::fs::File::open(&directory)?.sync_all()?;
    Ok(())
}
pub fn read(root: &Path) -> Result<Report> {
    let path = crate::config::inside(root, &crate::config::path("reports/last.json"))?;
    let report: Report = serde_json::from_str(&crate::config::read_bounded(
        &path,
        crate::limits::MAX_REPORT_BYTES,
    )?)?;
    ensure!(
        report.schema_version == 4,
        "unsupported report schema; rerun please run"
    );
    let plan = &report.plan;
    ensure!(
        plan.plan_schema_version == crate::planner::PLAN_SCHEMA_VERSION
            && !plan.at.is_empty()
            && matches!(plan.status.as_str(), "planned" | "blocked")
            && plan.executable() == plan.blocking_reasons.is_empty()
            && plan.input.kind == "worktree"
            && plan.plan_id.len() == 64
            && plan.plan_id.bytes().all(|b| b.is_ascii_hexdigit())
            && plan.checks_only == report.checks_only
            && plan.input.fingerprint == report.snapshot.fingerprint
            && plan.input.head == report.snapshot.head
            && plan.input.complete == report.snapshot.complete,
        "inconsistent report plan/input"
    );
    for rule in plan
        .scheduled
        .iter()
        .chain(&plan.disabled_rules)
        .chain(plan.omitted.iter().map(|o| &o.rule))
    {
        ensure!(
            rule.provider.is_some() == rule.provider_config.is_some()
                && (rule.provider.is_none() || rule.kind == "review"),
            "inconsistent report provider metadata"
        );
        ensure!(
            rule.provider_origin.is_none() || (rule.kind == "review" && rule.provider.is_some()),
            "inconsistent external provider provenance"
        );
        if let Some(config) = &rule.provider_config {
            crate::config::validate_provider(config)
                .context("invalid report provider configuration")?;
        }
    }
    let expected: std::collections::BTreeSet<_> =
        plan.scheduled.iter().map(|r| (&r.id, &r.kind)).collect();
    let actual: std::collections::BTreeSet<_> = report
        .results
        .iter()
        .filter(|r| r.kind != "integrity")
        .map(|r| (&r.id, &r.kind))
        .collect();
    ensure!(
        expected == actual
            && expected.len() == plan.scheduled.len()
            && plan
                .scheduled
                .iter()
                .all(|r| matches!(r.kind.as_str(), "check" | "review"))
            && report
                .results
                .iter()
                .filter(|r| r.kind != "integrity")
                .count()
                == actual.len(),
        "inconsistent report rule coverage"
    );
    ensure!(
        plan.scheduled.iter().all(|rule| rule
            .origin
            .as_ref()
            .is_some_and(|o| o.execution_points.contains(&plan.at))
            && report
                .results
                .iter()
                .any(|item| item.id == rule.id && item.blocking == rule.blocking)),
        "inconsistent report rule metadata"
    );
    ensure!(
        report.results.iter().all(|item| !item.configuration_error
            || (item.kind == "review" && item.status == Status::Error)),
        "inconsistent review configuration error"
    );
    for item in &report.results {
        for finding in &item.findings {
            if finding.source == Some(EvidenceSource::CommitHeader) {
                ensure!(
                    finding.path.is_none()
                        && finding.line == Some(1)
                        && plan.input.commit_header.is_some()
                        && plan.scheduled.iter().any(|r| r.id == item.id
                            && r.review_input == Some(crate::config::ReviewInput::CommitHeader)),
                    "inconsistent commit-header evidence"
                );
            }
        }
        if let Some(dependency) = &item.blocked_by {
            ensure!(
                item.kind == "check"
                    && item.status == Status::NotRun
                    && item.duration_ms == 0
                    && item.exit_code.is_none()
                    && item.stdout.is_none()
                    && item.stderr.is_none()
                    && item.findings.is_empty()
                    && plan
                        .scheduled
                        .iter()
                        .any(|r| r.id == item.id && r.depends_on.contains(dependency))
                    && report.results.iter().any(|r| r.id == *dependency
                        && r.kind == "check"
                        && r.status != Status::Pass),
                "inconsistent dependency skip metadata"
            );
            let mut seen = std::collections::BTreeSet::new();
            let mut current = item;
            while let Some(id) = &current.blocked_by {
                ensure!(seen.insert(&current.id), "cyclic dependency skip metadata");
                current = report
                    .results
                    .iter()
                    .find(|r| r.id == *id)
                    .ok_or_else(|| anyhow::anyhow!("missing dependency skip result"))?;
            }
        }
    }
    let required: std::collections::BTreeSet<_> = plan.required_ids.iter().collect();
    ensure!(
        required.len() == plan.required_ids.len()
            && required == plan.scheduled.iter().map(|r| &r.id).collect()
            && plan
                .omitted
                .iter()
                .all(|o| o.reason == "trigger_not_matched"
                    && o.rule.kind == "review"
                    && !required.contains(&o.rule.id)
                    && o.rule
                        .origin
                        .as_ref()
                        .is_some_and(|r| r.execution_points.contains(&plan.at)))
            && plan
                .omitted
                .iter()
                .map(|o| &o.rule.id)
                .collect::<std::collections::BTreeSet<_>>()
                .len()
                == plan.omitted.len()
            && (plan.omitted.is_empty() || plan.change_base.is_some())
            && plan
                .change_base
                .as_ref()
                .is_none_or(|base| matches!(base.len(), 40 | 64)
                    && base.bytes().all(|b| b.is_ascii_hexdigit())),
        "inconsistent required rules"
    );
    ensure!(
        report.overall != Status::Pass || (plan.executable() && plan.input.complete),
        "blocked/incomplete plan cannot pass"
    );
    ensure!(
        report.repository == root.canonicalize()?.to_string_lossy(),
        "report belongs to a different repository; rerun please run"
    );
    ensure!(
        report.overall == overall_of(&report.results, report.checks_only),
        "inconsistent report overall status"
    );
    ensure!(
        !report.checks_only
            || (report.provider_usage.is_none()
                && report.token_usage.is_none()
                && report
                    .results
                    .iter()
                    .all(|item| item.kind != "review" || item.status == Status::NotRun)),
        "inconsistent checks-only report scope"
    );
    Ok(report)
}
