//! Projects saved verification evidence onto the gateway ingest contract.
use crate::{
    config::{self, Config},
    discovery, git_snapshot,
    report::{self, Item, Report, Status},
    repository_identity,
    rule_digest::RuleMetadata,
};
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use std::{path::Path, time::Duration};
use ureq::http::{HeaderValue, Uri};

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

fn gateway_endpoint(gateway_url: &str) -> Result<Uri> {
    let invalid_url = "--gateway-url must be an HTTP or HTTPS origin without credentials, a path, query or fragment";
    let base: Uri = gateway_url
        .parse()
        .map_err(|_| anyhow::anyhow!(invalid_url))?;
    let scheme = base.scheme().context(invalid_url)?;
    let authority = base.authority().context(invalid_url)?;
    let host = authority.host();
    ensure!(
        matches!(scheme.as_str(), "http" | "https")
            && !gateway_url.contains(['?', '#', '@'])
            && base.path() == "/"
            && !host.is_empty()
            && (authority.as_str() == host || authority.port_u16().is_some()),
        "{invalid_url}"
    );
    Uri::builder()
        .scheme(scheme.clone())
        .authority(authority.clone())
        .path_and_query("/v0/attestations")
        .build()
        .map_err(|_| anyhow::anyhow!(invalid_url))
}

pub fn send(root: &Path, gateway_url: &str) -> Result<()> {
    let endpoint = gateway_endpoint(gateway_url)?;
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

    let config = discovery::document::<Config>(root, &config::path("config.toml")).map_err(|_| {
        anyhow::anyhow!("cannot read .please/config.toml; check the project configuration")
    });
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

    let settings = config?
        .publish
        .context("configure [publish] in .please/config.toml")?;
    settings.validate()?;
    let repository = settings.repository.or_else(|| repository_identity::read_origin(root))
        .context("cannot determine repository identity; set a network Git remote named origin or publish.repository")?;

    let key = std::env::var(&settings.key_env)
        .ok()
        .filter(|key| !key.trim().is_empty())
        .context("publish API key environment variable is unset, empty or invalid; check publish.key_env")?;
    ensure!(
        key.bytes().all(|byte| byte.is_ascii_graphic()),
        "publish API key is invalid"
    );
    let mut authorization = HeaderValue::from_str(&format!("Bearer {key}"))
        .map_err(|_| anyhow::anyhow!("publish API key is invalid"))?;
    authorization.set_sensitive(true);
    let body = serde_json::to_vec(&attestation(&report, &repository, commit))?;

    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(30)))
        .max_redirects(0)
        .http_status_as_error(false)
        .build()
        .new_agent();
    let response = agent
        .post(endpoint)
        .header("Authorization", authorization)
        .header("Content-Type", "application/json")
        .send(body.as_slice())
        .map_err(|_| {
            anyhow::anyhow!("could not reach the attestation gateway or receive its response")
        })?;
    match response.status().as_u16() {
        201 => Ok(()),
        401 | 403 => bail!("attestation gateway rejected the API credentials"),
        429 => bail!("attestation gateway rate limit exceeded"),
        400 | 413 => bail!("attestation gateway rejected the attestation document"),
        status => bail!("attestation gateway error (HTTP {status})"),
    }
}
