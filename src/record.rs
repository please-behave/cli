//! Deliver release facts independently of verification evidence.
use crate::{gateway, git_snapshot, process};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{io::Read, path::Path, time::Duration};

const MAX_RECEIPT_BYTES: u64 = 1024;

#[derive(Serialize)]
struct Release<'a> {
    repository: &'a str,
    version: &'a str,
    commit: &'a str,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    outcome: Outcome,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Outcome {
    Created,
    Unchanged,
    Conflict,
}

pub fn release(
    root: &Path,
    gateway_url: &str,
    version: &str,
    commit: Option<&str>,
) -> Result<&'static str> {
    let endpoint = gateway::endpoint(gateway_url, "/v0/facts/releases")?;
    ensure!(
        (1..=256).contains(&version.chars().count()) && !version.contains('\0'),
        "release version must contain 1 to 256 characters and no NUL"
    );
    git_snapshot::require_root(root)?;
    let head;
    let commit = match commit {
        Some(commit) => {
            ensure!(
                is_full_object_id(commit),
                "--commit must be a full Git object ID (40 or 64 hexadecimal characters)"
            );
            commit
        }
        None => {
            head = head_commit(root)?;
            &head
        }
    };
    let settings = gateway::settings(root)?;
    let repository = gateway::repository(root, &settings)?;
    let body = serde_json::to_vec(&Release {
        repository: &repository,
        version,
        commit,
    })?;
    let authorization = gateway::authorization(&settings, &endpoint)?;
    let response = gateway::post(endpoint, authorization, &body, "release fact")?;
    let status = response.status().as_u16();
    match status {
        200 | 201 | 409 => {}
        401 | 403 => {
            bail!("release fact gateway rejected the credentials or repository authorization")
        }
        429 => bail!("release fact gateway rate limit exceeded"),
        400 | 413 => bail!("release fact gateway rejected the release document"),
        _ => bail!("release fact gateway error (HTTP {status})"),
    }

    let mut body = Vec::new();
    response
        .into_body()
        .into_reader()
        .take(MAX_RECEIPT_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|_| anyhow::anyhow!("could not receive the release fact gateway response"))?;
    let invalid_receipt = "release fact gateway returned an invalid receipt";
    ensure!(body.len() as u64 <= MAX_RECEIPT_BYTES, "{invalid_receipt}");
    let receipt: Receipt =
        serde_json::from_slice(&body).map_err(|_| anyhow::anyhow!(invalid_receipt))?;
    match (status, receipt.outcome) {
        (201, Outcome::Created) => Ok("Release recorded."),
        (200, Outcome::Unchanged) => Ok("Release already known."),
        (409, Outcome::Conflict) => bail!("Release version conflicts with a recorded commit."),
        _ => bail!("{invalid_receipt}"),
    }
}

fn is_full_object_id(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn head_commit(root: &Path) -> Result<String> {
    let missing_head = "recording a release without --commit requires a current Git commit";
    let output = process::run(
        git_snapshot::git(root)
            .env("GIT_NO_REPLACE_OBJECTS", "1")
            .args(["rev-parse", "--verify", "HEAD^{commit}"]),
        Duration::from_secs(10),
    )
    .context(missing_head)?;
    let head = output.stdout.text.trim_end_matches(['\r', '\n']);
    ensure!(
        output.status.success()
            && !output.timed_out
            && !output.stdout.truncated
            && is_full_object_id(head),
        "{missing_head}"
    );
    Ok(head.to_owned())
}
