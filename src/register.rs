//! Submit committed rule metadata as one registration batch.
use crate::{
    config::{self, PublishAuth, PublishSettings, RuleDefinition},
    discovery, gateway,
    git_snapshot::{self, git},
    limits::{MAX_INSTRUCTION_BYTES, MAX_METADATA_BYTES},
    process, rule_digest,
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};
use ureq::http::{HeaderValue, Uri, uri::Authority};

const GIT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_BATCH_RULES: usize = 100;
const MAX_BATCH_BYTES: usize = 2 * 1024 * 1024;
const MAX_RECEIPT_BYTES: u64 = 16 * 1024;

#[derive(Serialize)]
struct Batch<'a> {
    commit: &'a str,
    rules: Vec<Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    outcomes: Vec<Outcome>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Outcome {
    outcome: State,
    digest: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum State {
    Created,
    Unchanged,
    Conflict,
}

struct PreparedRule {
    label: String,
    digest: String,
}

pub fn send(root: &Path, gateway_url: &str, show: impl FnMut(&str)) -> Result<()> {
    send_with_authorization(root, gateway_url, show, gateway::authorization)
}

pub(crate) fn send_with_authorization(
    root: &Path,
    gateway_url: &str,
    mut show: impl FnMut(&str),
    authorize: impl FnOnce(&PublishSettings, &Uri) -> Result<HeaderValue>,
) -> Result<()> {
    let endpoint = gateway::endpoint(gateway_url, "/v0/rule-registrations")?;
    git_snapshot::require_root(root)?;
    let settings = gateway::settings(root)?;
    let source = gateway::repository(root, &settings)?;
    validate_source(&source)?;
    let api_key = match settings.auth {
        PublishAuth::ApiKey => Some(gateway::api_key_authorization(&settings)?),
        PublishAuth::GithubOidc => None,
    };
    let head = head(root)?;
    let tree = git_text(root, &["ls-tree", "-rz", "--full-tree", &head])
        .context("cannot enumerate files in the registration commit")?;
    let tree = committed_files(&tree)?;
    let files = git_text(
        root,
        &[
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ],
    )
    .context("cannot enumerate local rule files")?;
    let directories = rule_directories(tree.keys().copied().chain(files.split_terminator('\0')))?;
    if directories.is_empty() {
        show("No rule directories to register.");
        return Ok(());
    }
    ensure!(
        directories.len() <= MAX_BATCH_RULES,
        "registration exceeds the gateway limit of {MAX_BATCH_RULES} rules per batch"
    );
    let mut batch = Batch {
        commit: &head,
        rules: Vec::with_capacity(directories.len()),
    };
    let mut prepared = Vec::with_capacity(directories.len());
    for directory in &directories {
        let (rule, verification) = prepare_rule(root, directory, &source, &tree)
            .with_context(|| format!("cannot register rule directory {}", directory.display()))?;
        batch.rules.push(rule);
        prepared.push(verification);
    }
    let body = serde_json::to_vec(&batch)?;
    ensure!(
        body.len() <= MAX_BATCH_BYTES,
        "registration exceeds the gateway request size limit"
    );
    require_clean_directories(root, &directories, &head)?;
    let authorization = match api_key {
        Some(header) => header,
        None => authorize(&settings, &endpoint)?,
    };
    let response = gateway::post(endpoint, authorization, &body, "registration")?;
    let receipt = read_receipt(response)?;
    ensure!(
        receipt.outcomes.len() == prepared.len(),
        "registration gateway returned an invalid receipt"
    );
    let mut failed = false;
    for (rule, outcome) in prepared.into_iter().zip(receipt.outcomes) {
        let status = match outcome.outcome {
            State::Created => "created",
            State::Unchanged => "unchanged",
            State::Conflict => {
                failed = true;
                "conflict (content changed without a new version)"
            }
        };
        if outcome.digest == rule.digest {
            show(&format!("{status} {}", rule.label));
        } else {
            failed = true;
            show(&format!(
                "{status} {} (returned digest does not match)",
                rule.label
            ));
        }
    }
    ensure!(!failed, "one or more rule registrations failed");
    Ok(())
}

fn validate_source(source: &str) -> Result<()> {
    ensure!(
        source.chars().count() <= 256
            && !source.chars().any(char::is_control)
            && !source.ends_with(".git")
            && !source.ends_with('/')
            && source.split_once('/').is_some_and(|(host, path)| {
                !host.contains('@')
                    && !path.is_empty()
                    && !path.starts_with(['/', '\\'])
                    && host
                        .parse::<Authority>()
                        .is_ok_and(|authority| authority.host() == host)
            }),
        "registration requires a normalized Git host and path in origin or publish.repository"
    );
    Ok(())
}

fn head(root: &Path) -> Result<String> {
    let value = git_text(root, &["rev-parse", "--verify", "HEAD^{commit}"])
        .context("registration requires a current Git commit")?;
    let commit = value.trim_end();
    ensure!(
        matches!(commit.len(), 40 | 64) && commit.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "registration requires a current Git commit"
    );
    Ok(commit.into())
}

fn git_text(root: &Path, args: &[&str]) -> Result<String> {
    let output = git_snapshot::path_output(
        git(root).env("GIT_NO_REPLACE_OBJECTS", "1").args(args),
        GIT_TIMEOUT,
    )?;
    ensure!(
        output.status.success() && !output.timed_out && !output.stdout.truncated,
        "Git could not inspect the registration inputs"
    );
    Ok(output.stdout.text)
}

fn committed_files(tree: &str) -> Result<BTreeMap<&str, (&str, &str)>> {
    tree.split_terminator('\0')
        .map(|entry| {
            let (metadata, path) = entry.split_once('\t').context("invalid Git tree entry")?;
            let (mode, object) = metadata.split_once(' ').context("invalid Git tree entry")?;
            let (_, oid) = object.split_once(' ').context("invalid Git tree entry")?;
            Ok((path, (mode, oid)))
        })
        .collect()
}

fn rule_directories<'a>(paths: impl Iterator<Item = &'a str>) -> Result<BTreeSet<PathBuf>> {
    paths
        .filter_map(|name| {
            let path = Path::new(name);
            (path.file_name().is_some_and(|name| name == "rule.toml")).then_some((name, path))
        })
        .map(|(name, path)| {
            ensure!(
                !name.contains('\u{fffd}'),
                "unsupported non-UTF-8 rule path"
            );
            config::relative(path)?;
            Ok(path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(Path::new("."))
                .to_path_buf())
        })
        .collect()
}

fn committed_text(
    root: &Path,
    path: &Path,
    limit: u64,
    tree: &BTreeMap<&str, (&str, &str)>,
) -> Result<String> {
    let path_key = rule_digest::portable_path(path)?;
    let changed = || format!("{} differs from the current commit", path.display());
    let (mode, oid) = tree.get(path_key.as_str()).with_context(changed)?;
    ensure!(matches!(*mode, "100644" | "100755"), "{}", changed());
    let full = config::source_path(root, path).with_context(changed)?;
    let text = config::read_bounded(&full, limit)?;
    let hash = process::run_with_input(
        git(root).args(["hash-object", "--stdin"]),
        GIT_TIMEOUT,
        text.as_bytes().to_vec(),
        |_| {},
    )?;
    ensure!(
        hash.status.success() && !hash.timed_out && hash.stdout.text.trim_end() == *oid,
        "{}",
        changed()
    );
    Ok(text)
}

fn prepare_rule(
    root: &Path,
    directory: &Path,
    source: &str,
    tree: &BTreeMap<&str, (&str, &str)>,
) -> Result<(Value, PreparedRule)> {
    let definition_path = directory.join("rule.toml");
    let definition = committed_text(root, &definition_path, MAX_METADATA_BYTES, tree)?;
    let rule: RuleDefinition = discovery::parse_document(&definition, &definition_path)?;
    config::identity(&rule.id, &rule.description, &mut BTreeSet::new())?;
    let version = discovery::rule_version(&rule, &definition_path)?
        .with_context(|| format!("rule {} is missing version", rule.id))?;
    let knowledge = committed_text(root, &directory.join("rule.yaml"), MAX_METADATA_BYTES, tree)
        .with_context(|| format!("rule {} requires a committed rule.yaml", rule.id))?;
    let document = rule_digest::rule_document(
        root,
        directory,
        &rule,
        source,
        version,
        &knowledge,
        |prompt| {
            committed_text(
                root,
                prompt.strip_prefix(root)?,
                MAX_INSTRUCTION_BYTES,
                tree,
            )
        },
    )?;
    let digest = rule_digest::digest(document.clone())?;
    Ok((
        document,
        PreparedRule {
            label: format!("{} {} ({})", rule.id, version, directory.display()),
            digest,
        },
    ))
}

fn require_clean_directories(
    root: &Path,
    directories: &BTreeSet<PathBuf>,
    commit: &str,
) -> Result<()> {
    let status = git_text(
        root,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--no-renames",
            "--ignore-submodules=none",
            "--untracked-files=all",
        ],
    )
    .context("cannot check rule files against the current commit")?;
    for change in status.split_terminator('\0') {
        let path = Path::new(change.get(3..).context("invalid Git status entry")?);
        ensure!(
            !directories.contains(Path::new("."))
                && !path.ancestors().any(|part| directories.contains(part)),
            "{} differs from the current commit",
            path.display()
        );
    }
    ensure!(
        head(root)? == commit,
        "Git HEAD changed during rule registration"
    );
    Ok(())
}

fn read_receipt(response: ureq::http::Response<ureq::Body>) -> Result<Receipt> {
    match response.status().as_u16() {
        200 => {}
        401 | 403 => bail!("registration gateway rejected the API credentials"),
        429 => bail!("registration gateway rate limit exceeded"),
        400 | 413 => bail!("registration gateway rejected the registration document"),
        status => bail!("registration gateway error (HTTP {status})"),
    }
    let mut body = Vec::new();
    response
        .into_body()
        .into_reader()
        .take(MAX_RECEIPT_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|_| anyhow::anyhow!("could not receive the registration gateway response"))?;
    ensure!(
        body.len() as u64 <= MAX_RECEIPT_BYTES,
        "registration gateway returned an invalid receipt"
    );
    serde_json::from_slice(&body)
        .map_err(|_| anyhow::anyhow!("registration gateway returned an invalid receipt"))
}
