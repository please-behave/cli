//! Deliver release and deployment facts independently of verification evidence.
use crate::{config, gateway, git_snapshot, process, repository_identity};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::{io::Read, path::Path, time::Duration};
use ureq::http::Response;

const MAX_RECEIPT_BYTES: u64 = 1024;

#[derive(Serialize)]
struct Release<'a> {
    repository: &'a str,
    version: &'a str,
    commit: &'a str,
}

#[derive(Serialize)]
struct Deployment<'a> {
    target: &'a str,
    repository: &'a str,
    #[serde(flatten)]
    revision: Revision<'a>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Revision<'a> {
    Version { version: &'a str },
    Commit { commit: &'a str },
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

    let invalid_receipt = "release fact gateway returned an invalid receipt";
    match (status, read_outcome(response, "release fact")?) {
        (201, Outcome::Created) => Ok("Release recorded."),
        (200, Outcome::Unchanged) => Ok("Release already known."),
        (409, Outcome::Conflict) => bail!("Release version conflicts with a recorded commit."),
        _ => bail!("{invalid_receipt}"),
    }
}

pub fn deployment(
    root: &Path,
    gateway_url: &str,
    target: &str,
    repository: &str,
    version: Option<&str>,
    commit: Option<&str>,
) -> Result<&'static str> {
    let endpoint = gateway::endpoint(gateway_url, "/v0/facts/deployments")?;
    ensure!(
        config::valid_target_name(target),
        "--target must start with an ASCII letter or digit and contain 1 to 64 ASCII letters, digits, dots, underscores or hyphens"
    );
    let invalid_repository = "--repository must identify a deployed repository by its host and path (1 to 256 characters after normalization)";
    let repository = repository_identity::from_identity_or_remote(repository)
        .filter(|identity| identity.chars().count() <= 256)
        .context(invalid_repository)?;
    let revision = match (version, commit) {
        (Some(version), None) => {
            ensure!(
                (1..=256).contains(&version.chars().count()) && !version.contains('\0'),
                "deployment version must contain 1 to 256 characters and no NUL"
            );
            Revision::Version { version }
        }
        (None, Some(commit)) => {
            ensure!(
                is_full_object_id(commit),
                "--commit must be a full Git object ID (40 or 64 hexadecimal characters)"
            );
            Revision::Commit { commit }
        }
        _ => bail!("specify exactly one of --version and --commit"),
    };
    git_snapshot::require_root(root)?;
    let settings = gateway::settings(root)?;
    let body = serde_json::to_vec(&Deployment {
        target,
        repository: &repository,
        revision,
    })?;
    let authorization = gateway::authorization(&settings, &endpoint)?;
    let response = gateway::post(endpoint, authorization, &body, "deployment fact")?;
    let status = response.status().as_u16();
    match status {
        200 | 201 => match (status, read_outcome(response, "deployment fact")?) {
            (201, Outcome::Created) => Ok("Deployment recorded."),
            (200, Outcome::Unchanged) => Ok("Deployment unchanged."),
            _ => bail!("deployment fact gateway returned an invalid receipt"),
        },
        400 => {
            if read_receipt(response, "deployment fact")?.is_empty() {
                bail!("Deployment target is unknown to the gateway for the authenticated sender.");
            }
            bail!("deployment fact gateway rejected the deployment document");
        }
        401 | 403 => bail!("deployment fact gateway rejected the credentials or authorization"),
        413 => bail!("deployment fact gateway rejected the deployment document"),
        429 => bail!("deployment fact gateway rate limit exceeded"),
        _ => bail!("deployment fact gateway error (HTTP {status})"),
    }
}

fn read_outcome(response: Response<ureq::Body>, fact: &str) -> Result<Outcome> {
    let body = read_receipt(response, fact)?;
    let receipt: Receipt = serde_json::from_slice(&body)
        .map_err(|_| anyhow::anyhow!("{fact} gateway returned an invalid receipt"))?;
    Ok(receipt.outcome)
}

fn read_receipt(response: Response<ureq::Body>, fact: &str) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    response
        .into_body()
        .into_reader()
        .take(MAX_RECEIPT_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|_| anyhow::anyhow!("could not receive the {fact} gateway response"))?;
    ensure!(
        body.len() as u64 <= MAX_RECEIPT_BYTES,
        "{fact} gateway returned an invalid receipt"
    );
    Ok(body)
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
