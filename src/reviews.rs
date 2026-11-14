use crate::review_contract::{ReviewResponse, VERDICTS, valid_summary};
use crate::{
    config::{Project, Review},
    git_snapshot::Snapshot,
    provider::{Provider, Request},
    report::{Item, MAX_SUMMARY_CHARS, Status, TokenUsage},
};
use anyhow::{Result, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    time::Instant,
};
pub fn parse(
    project: &Project,
    active: &[&Review],
    snapshot: &Snapshot,
    json: &str,
) -> Result<Vec<Item>> {
    parse_with_message(project, active, snapshot, json, None)
}

fn parse_with_message(
    project: &Project,
    active: &[&Review],
    snapshot: &Snapshot,
    json: &str,
    message_lines: Option<usize>,
) -> Result<Vec<Item>> {
    ensure!(
        json.len() as u64 <= crate::limits::MAX_PROVIDER_RESPONSE_BYTES,
        "provider JSON exceeds {} bytes",
        crate::limits::MAX_PROVIDER_RESPONSE_BYTES
    );
    // Options are nullable, but must still be present as required by the output schema.
    let value: serde_json::Value = serde_json::from_str(json)?;
    if let Some(reviews) = value.get("reviews").and_then(|x| x.as_array()) {
        for review in reviews {
            if let Some(findings) = review.get("findings").and_then(|x| x.as_array()) {
                for finding in findings {
                    ensure!(
                        finding.get("path").is_some() && finding.get("line").is_some(),
                        "finding must include path and line (nullable)"
                    );
                }
            }
        }
    }
    let response: ReviewResponse = serde_json::from_str(json)?;
    let expected: BTreeSet<_> = active.iter().map(|r| r.id.as_str()).collect();
    let mut mapped = BTreeMap::new();
    for verdict in response.reviews {
        ensure!(
            expected.contains(verdict.id.as_str()),
            "unexpected review ID: {}",
            verdict.id
        );
        ensure!(VERDICTS.contains(&verdict.status), "invalid review status");
        ensure!(
            valid_summary(&verdict.reason),
            "review reason must be one nonempty line of at most {MAX_SUMMARY_CHARS} characters"
        );
        ensure!(
            verdict.explanation.as_ref().is_none_or(|text| {
                !text.trim().is_empty() && text.len() <= crate::limits::MAX_EXPLANATION_BYTES
            }),
            "review explanation must be nonblank and at most {} bytes",
            crate::limits::MAX_EXPLANATION_BYTES
        );
        ensure!(
            verdict.findings.len() <= crate::limits::MAX_FINDINGS,
            "too many findings"
        );
        for finding in &verdict.findings {
            ensure!(
                !finding.message.trim().is_empty()
                    && finding.message.len() <= crate::limits::MAX_FINDING_MESSAGE_BYTES,
                "invalid finding message"
            );
            ensure!(finding.line != Some(0), "evidence line must be positive");
            if let Some(path) = &finding.path {
                ensure!(
                    snapshot.files.contains_key(path),
                    "evidence path not in verified manifest: {path}"
                );
                let full = crate::config::inside(&project.root, Path::new(path))?;
                ensure!(full.is_file(), "evidence path must be a file");
                if let Some(line) = finding.line {
                    let text =
                        crate::config::read_bounded(&full, crate::limits::MAX_SNAPSHOT_FILE_BYTES)?;
                    ensure!(
                        (line as usize) <= text.lines().count(),
                        "evidence line beyond file: {path}:{line}"
                    );
                }
            } else if let Some(line) = finding.line {
                // Without a path, the line cites the commit message.
                ensure!(
                    message_lines.is_some_and(|count| (line as usize) <= count),
                    "line requires evidence path or a commit message line"
                );
            }
        }
        let mut item = Item::simple(&verdict.id, "review", verdict.status, verdict.reason);
        item.blocking = active.iter().find(|r| r.id == verdict.id).unwrap().blocking;
        item.explanation = verdict.explanation;
        item.findings = verdict.findings;
        ensure!(
            mapped.insert(verdict.id.clone(), item).is_none(),
            "duplicate review ID: {}",
            verdict.id
        );
    }
    ensure!(
        mapped.len() == expected.len(),
        "incomplete review ID coverage"
    );
    Ok(mapped.into_values().collect())
}
pub fn execute(
    project: &Project,
    active: &[&Review],
    snapshot: &Snapshot,
    provider: &dyn Provider,
    execution_point: &str,
) -> Vec<Item> {
    execute_with_usage(project, active, snapshot, provider, execution_point).0
}
pub fn execute_with_usage(
    project: &Project,
    active: &[&Review],
    snapshot: &Snapshot,
    provider: &dyn Provider,
    execution_point: &str,
) -> (Vec<Item>, Option<TokenUsage>) {
    match crate::review_input::PreparedReviewInput::prepare(project, active, snapshot) {
        Ok(input) => execute_prepared(project, active, snapshot, &input, provider, execution_point),
        Err(error) => (
            active
                .iter()
                .map(|r| Item::simple(&r.id, "review", Status::Error, format!("{error:#}")))
                .collect(),
            None,
        ),
    }
}
pub fn execute_prepared(
    project: &Project,
    active: &[&Review],
    snapshot: &Snapshot,
    input: &crate::review_input::PreparedReviewInput,
    provider: &dyn Provider,
    execution_point: &str,
) -> (Vec<Item>, Option<TokenUsage>) {
    execute_prepared_observed(
        project,
        active,
        snapshot,
        input,
        provider,
        execution_point,
        &mut |_, _| true,
    )
}

/// Observe each not-yet-started adapter call and optionally stop scheduling it.
pub(crate) fn execute_prepared_observed(
    project: &Project,
    active: &[&Review],
    snapshot: &Snapshot,
    input: &crate::review_input::PreparedReviewInput,
    provider: &dyn Provider,
    execution_point: &str,
    before_call: &mut dyn FnMut(&Review, &[Item]) -> bool,
) -> (Vec<Item>, Option<TokenUsage>) {
    let mut calls = Vec::new();
    let mut items = Vec::new();
    for review in active {
        let resolved = review
            .parameters
            .resolve(&project.root, &review.id)
            .and_then(|environment| {
                anyhow::ensure!(
                    review.execution_error.is_none(),
                    "Rule {}: {}",
                    review.id,
                    review.execution_error.as_deref().unwrap_or_default()
                );
                Ok(environment)
            });
        match resolved {
            Ok(environment) => calls.push((*review, environment)),
            Err(error) => {
                let mut item =
                    Item::simple(&review.id, "review", Status::Error, format!("{error:#}"));
                item.blocking = review.blocking;
                item.configuration_error = true;
                items.push(item);
            }
        }
    }
    let history = crate::scheduling::History::load(&project.root);
    // Stable sorting preserves the configured order on ties.
    calls.sort_by_cached_key(|(review, _)| {
        std::cmp::Reverse(history.estimate("review", &review.id))
    });
    let mut usages = Vec::new();
    for (review, parameters) in calls {
        if !before_call(review, &items) {
            let mut item = Item::simple(
                &review.id,
                "review",
                Status::NotRun,
                "Fail-fast triggered; no provider call made".into(),
            );
            item.blocking = review.blocking;
            items.push(item);
            continue;
        }
        // REQ-0125: one review per adapter call.
        let reviews = [review];
        // A snapshot check is required before every provider invocation. Never
        // reuse prepared evidence after edits.
        let stable = input.ensure_stable().is_ok()
            && crate::git_snapshot::capture(&project.root).is_ok_and(|current| {
                crate::git_snapshot::compare(snapshot, &current, 0).state == "CURRENT"
            });
        let provider_integrity = || -> Result<()> {
            if let Some((source, config)) = review.provider.as_ref().and_then(|id| {
                project
                    .external_providers
                    .get(id)
                    .zip(project.providers.get(id))
            }) {
                source.resolve(config, &project.root)?;
            }
            Ok(())
        };
        let invalid_provider = provider_integrity().err();
        let start = Instant::now();
        let response = if stable && invalid_provider.is_none() {
            provider.review_with_usage(&Request {
                project,
                reviews: &reviews,
                snapshot,
                input,
                parameters: &parameters,
                execution_point,
            })
        } else {
            crate::provider::Response {
                verdict: Err(anyhow::anyhow!(
                    "Working files or commit message changed, or input is incomplete; no provider call made"
                )),
                usage: None,
                output: None,
            }
        };
        usages.push(response.usage);
        let output = response.output;
        let invalid_provider = invalid_provider.or_else(|| provider_integrity().err());
        let result = if let Some(error) = &invalid_provider {
            Err(anyhow::anyhow!(
                "Invalid external provider resources: {error:#}"
            ))
        } else {
            response
            .verdict
            .and_then(|json| {
                input.ensure_stable()?;
                parse_with_message(project, &reviews, snapshot, &json, input.commit_message.as_ref().map(|m| m.lines.len())).map_err(|_| anyhow::anyhow!("Invalid adapter review response; structured verdict and evidence requirements were not met; inspect captured output with please status --details"))
            })
        };
        let mut results = match result {
            Ok(items) => items,
            Err(error) => vec![Item::simple(
                &review.id,
                "review",
                Status::Error,
                format!("{error:#}"),
            )],
        };
        for item in &mut results {
            item.duration_ms = start.elapsed().as_millis() as u64;
            item.blocking = review.blocking;
            item.configuration_error = invalid_provider.is_some();
            // REQ-0131: an adapter ERROR keeps the call's output for diagnosis.
            if let (Status::Error, Some(output)) = (item.status, &output) {
                item.exit_code = output.exit_code;
                item.stdout = Some(output.stdout.clone());
                item.stderr = Some(output.stderr.clone());
            }
        }
        items.extend(results);
    }
    items.sort_by_key(|item| active.iter().position(|r| r.id == item.id).unwrap());
    (items, combined_usage(&usages))
}

fn combined_usage(usages: &[Option<TokenUsage>]) -> Option<TokenUsage> {
    let sum = |counter: fn(&TokenUsage) -> Option<u64>| {
        usages.iter().try_fold(0u64, |sum, usage| {
            sum.checked_add(counter(usage.as_ref()?)?)
        })
    };
    let total = TokenUsage {
        input_tokens: sum(|u| u.input_tokens),
        cached_input_tokens: sum(|u| u.cached_input_tokens),
        output_tokens: sum(|u| u.output_tokens),
        reasoning_output_tokens: sum(|u| u.reasoning_output_tokens),
        cache_write_input_tokens: sum(|u| u.cache_write_input_tokens),
    };
    (!usages.is_empty() && !total.is_unavailable()).then_some(total)
}
