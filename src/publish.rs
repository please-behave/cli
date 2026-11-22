//! Projects saved verification evidence onto the gateway ingest contract.
use crate::{
    gateway, git_snapshot,
    report::{self, Item, Report, Status},
    rule_digest::RuleMetadata,
};
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use std::path::Path;

#[derive(Serialize)]
struct Attestation<'a> {
    schema_version: u32,
    repository: &'a str,
    commit: &'a str,
    execution_points: [&'a str; 1],
    overall: Status,
    results: Vec<RuleResult<'a>>,
}

#[derive(Serialize)]
struct RuleResult<'a> {
    id: &'a str,
    kind: &'a str,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    rule: Option<PublishedRule<'a>>,
}

#[derive(Serialize)]
struct PublishedRule<'a> {
    source: &'a str,
    path: &'a str,
    id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    version: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    digest: Option<&'a str>,
}

impl<'a> PublishedRule<'a> {
    fn from_metadata(metadata: &'a RuleMetadata) -> Option<Self> {
        let source = metadata.source.as_deref()?;
        if source.chars().count() > 256
            || metadata.path.chars().count() > 256
            || metadata.id.chars().count() > 128
        {
            return None;
        }
        let pair = metadata.version.as_deref().zip(metadata.digest.as_deref());
        Some(Self {
            source,
            path: &metadata.path,
            id: &metadata.id,
            version: pair.map(|(version, _)| version),
            digest: pair.map(|(_, digest)| digest),
        })
    }
}

impl<'a> From<&'a Item> for RuleResult<'a> {
    fn from(item: &'a Item) -> Self {
        let status = match item.status {
            Status::Cancelled => "NOT_RUN",
            Status::Fail | Status::Error if item.kind == "review" && !item.blocking => "WARN",
            status => status.label(),
        };
        Self {
            id: &item.id,
            kind: &item.kind,
            status,
            rule: item
                .rule_metadata
                .as_ref()
                .and_then(PublishedRule::from_metadata),
        }
    }
}

fn attestation<'a>(report: &'a Report, repository: &'a str, commit: &'a str) -> Attestation<'a> {
    Attestation {
        schema_version: report.schema_version,
        repository,
        commit,
        execution_points: [&report.plan.at],
        overall: report.overall,
        results: report.results.iter().map(RuleResult::from).collect(),
    }
}

pub fn send(root: &Path, gateway_url: &str) -> Result<()> {
    let endpoint = gateway::endpoint(gateway_url, "/v0/attestations")?;
    git_snapshot::require_root(root)?;
    let number = report::saved(root)?
        .last()
        .copied()
        .context("no saved report; run please run before publishing")?;
    let report = report::read_number(root, number)
        .map_err(|_| anyhow::anyhow!("cannot read saved report {number}; run please run again"))?;
    let commit = report
        .snapshot
        .head
        .as_deref()
        .filter(|head| !head.is_empty())
        .context("saved report has no commit; create a commit and run please run again")?;

    let settings = gateway::settings(root);
    let (changes, _) = git_snapshot::compare_current(
        root,
        &report.snapshot,
        report.max_display_files,
    )
    .map_err(|_| {
        anyhow::anyhow!("saved report freshness is UNKNOWN; run please run again before publishing")
    })?;
    ensure!(
        changes.state == "CURRENT",
        "saved report is {}; run please run again before publishing",
        changes.state
    );

    let settings = settings?;
    let repository = gateway::repository(root, &settings)?;
    let authorization = gateway::authorization(&settings.key_env)?;
    let body = serde_json::to_vec(&attestation(&report, &repository, commit))?;

    let response = gateway::post(endpoint, authorization, &body, "attestation")?;
    match response.status().as_u16() {
        201 => Ok(()),
        401 | 403 => bail!("attestation gateway rejected the API credentials"),
        429 => bail!("attestation gateway rate limit exceeded"),
        400 | 413 => bail!("attestation gateway rejected the attestation document"),
        status => bail!("attestation gateway error (HTTP {status})"),
    }
}
