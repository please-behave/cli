use crate::{git_snapshot::Snapshot, process::Capture};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{io::Write, path::Path};

pub use crate::limits::MAX_SUMMARY_CHARS;

/// Adapter-reported counters summed across review invocations. A counter is
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
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finding {
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
    /// Named adapter entry whose adapter call produced this review result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
    pub duration_ms: u64,
    pub reason: String,
    /// Optional longer explanation of an adapter verdict.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
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
            adapter: None,
            duration_ms: 0,
            reason,
            explanation: None,
            findings: vec![],
            exit_code: None,
            stdout: None,
            stderr: None,
        }
    }
}
pub const REPORT_SCHEMA_VERSION: u32 = 0;
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Report {
    pub schema_version: u32,
    pub timestamp: String,
    pub repository: String,
    pub project: String,
    pub overall: Status,
    pub snapshot: Snapshot,
    pub results: Vec<Item>,
    pub git_summary: String,
    pub max_display_files: usize,
    pub token_usage: Option<u64>,
    #[serde(default)]
    pub adapter_usage: Option<TokenUsage>,
    #[serde(default)]
    pub checks_only: bool,
    pub plan: crate::planner::Plan,
    /// Position in the report history; the file name carries it, the document does not.
    #[serde(skip)]
    pub number: u64,
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
/// Save the report under the next history number, prune beyond `retain` and return the number.
pub fn write(root: &Path, report: &Report, retain: usize) -> Result<u64> {
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
    let mut number = numbers(&directory)?.last().map_or(1, |latest| latest + 1);
    loop {
        match temporary.persist_noclobber(directory.join(file_name(number))) {
            Ok(_) => break,
            // A concurrent run took this number first.
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                temporary = error.file;
                number += 1;
            }
            Err(error) => return Err(error.error.into()),
        }
    }
    #[cfg(unix)]
    std::fs::File::open(&directory)?.sync_all()?;
    prune(&directory, retain);
    Ok(number)
}

fn file_name(number: u64) -> String {
    format!("{number}.json")
}

/// Ascending report numbers present in the directory.
fn numbers(directory: &Path) -> Result<Vec<u64>> {
    let mut found: Vec<u64> = std::fs::read_dir(directory)?
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter_map(|name| {
            let digits = name.strip_suffix(".json")?;
            let number: u64 = digits.parse().ok()?;
            (number > 0 && file_name(number) == name).then_some(number)
        })
        .collect();
    found.sort_unstable();
    Ok(found)
}

fn prune(directory: &Path, retain: usize) {
    let Ok(found) = numbers(directory) else {
        return;
    };
    let surplus = found.len().saturating_sub(retain);
    for number in found.into_iter().take(surplus) {
        let _ = std::fs::remove_file(directory.join(file_name(number)));
    }
}

/// Ascending numbers of the saved reports.
pub fn saved(root: &Path) -> Result<Vec<u64>> {
    let directory = root.join(crate::config::path("reports"));
    if !directory.try_exists()? {
        return Ok(vec![]);
    }
    numbers(&crate::config::inside(
        root,
        &crate::config::path("reports"),
    )?)
}

/// Read the newest saved report.
pub fn read(root: &Path) -> Result<Report> {
    let number = saved(root)?.last().copied().context("no saved report")?;
    read_number(root, number)
}

/// Read the saved report with the given history number.
pub fn read_number(root: &Path, number: u64) -> Result<Report> {
    let path = crate::config::inside(
        root,
        &crate::config::path(format!("reports/{}", file_name(number))),
    )
    .with_context(|| format!("no saved report {number}"))?;
    let mut report: Report = serde_json::from_str(&crate::config::read_bounded(
        &path,
        crate::limits::MAX_REPORT_BYTES,
    )?)?;
    report.number = number;
    ensure!(
        matches!(report.schema_version, 0 | 1),
        "unsupported report schema; rerun please run"
    );
    let plan = &report.plan;
    ensure!(
        matches!(plan.plan_schema_version, 0 | 1)
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
            rule.adapter.is_some() == rule.adapter_profile.is_some()
                && (rule.adapter.is_none() || rule.kind == "review"),
            "inconsistent report adapter metadata"
        );
        ensure!(
            rule.adapter_origin.is_none() || (rule.kind == "review" && rule.adapter.is_some()),
            "inconsistent external adapter provenance"
        );
        if let Some(config) = &rule.adapter_profile {
            crate::config::validate_adapter(config).context("invalid report adapter profile")?;
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
        if let Some(adapter) = &item.adapter {
            ensure!(
                item.kind == "review"
                    && plan
                        .scheduled
                        .iter()
                        .any(|r| r.id == item.id && r.adapter.as_ref() == Some(adapter)),
                "inconsistent review adapter"
            );
        }
        for finding in &item.findings {
            if finding.path.is_none() && finding.line.is_some() {
                ensure!(
                    plan.input.commit_message.is_some(),
                    "inconsistent commit message evidence"
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
            && plan.omitted.iter().all(|o| matches!(
                o.reason.as_str(),
                "trigger_not_matched" | "no_changes"
            ) && o.rule.kind == "review"
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
            && (plan.omitted.is_empty()
                || matches!(plan.at.as_str(), "pre-commit" | "commit-msg" | "pre-push")
                || plan.change_base.is_some())
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
            || (report.adapter_usage.is_none()
                && report.token_usage.is_none()
                && report
                    .results
                    .iter()
                    .all(|item| item.kind != "review" || item.status == Status::NotRun)),
        "inconsistent checks-only report scope"
    );
    Ok(report)
}
