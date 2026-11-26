use crate::config::DeploymentTarget;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use ureq::http::{HeaderValue, Uri};

#[derive(Serialize)]
struct Batch<'a> {
    targets: &'a [DeploymentTarget],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    outcomes: Vec<Outcome>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Outcome {
    name: String,
    outcome: State,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum State {
    Created,
    Unchanged,
}

pub(super) fn prepare(targets: &[DeploymentTarget]) -> Result<Option<Vec<u8>>> {
    (!targets.is_empty())
        .then(|| super::batch_body(&Batch { targets }))
        .transpose()
}

pub(super) fn send(
    endpoint: Uri,
    authorization: HeaderValue,
    body: &[u8],
    targets: &[DeploymentTarget],
    show: &mut impl FnMut(&str),
) -> Result<()> {
    let receipt: Receipt = super::post(endpoint, authorization, body, super::MAX_RECEIPT_BYTES)?;
    ensure!(
        receipt.outcomes.len() == targets.len()
            && receipt
                .outcomes
                .iter()
                .zip(targets)
                .all(|(outcome, target)| outcome.name == target.name),
        "registration gateway returned an invalid receipt"
    );
    for outcome in receipt.outcomes {
        let state = match outcome.outcome {
            State::Created => "created",
            State::Unchanged => "unchanged",
        };
        show(&format!("{state} {}", outcome.name));
    }
    Ok(())
}
