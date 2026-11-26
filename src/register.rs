//! Submit rule, Rule Set and deployment target registrations in order.
mod sets;
mod targets;

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
    let config = gateway::project_config(root)?;
    config.validate_targets().context(".please/config.toml")?;
    let settings = gateway::settings_from(&config)?;
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
    let files = tree.keys().copied().chain(files.split_terminator('\0'));
    let directories = rule_directories(files.clone())?;
    let set_paths = sets::definition_paths(files)?;
    if directories.is_empty() && set_paths.is_empty() && config.targets.is_empty() {
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
    let mut members = BTreeMap::new();
    for directory in &directories {
        let (rule, verification) = prepare_rule(root, directory, &source, &tree)
            .with_context(|| format!("cannot register rule directory {}", directory.display()))?;
        batch.rules.push(rule);
        members.insert(directory.clone(), verification);
    }
    let sets = sets::prepare(root, &source, &tree, &set_paths, &members)?;
    let rules_body = batch_body(&batch)?;
    let sets_body = batch_body(&sets::Batch {
        commit: &head,
        sets: &sets,
    })
    .context("cannot prepare Rule Set registration batch")?;
    let targets_body = targets::prepare(&config.targets)
        .context("cannot prepare deployment target registration batch")?;
    require_clean_inputs(root, &directories, &set_paths, &head)?;
    let authorization = match api_key {
        Some(header) => header,
        None => authorize(&settings, &endpoint)?,
    };
    let mut rules_failed = false;
    if !members.is_empty() {
        let receipt = post(
            endpoint,
            authorization.clone(),
            &rules_body,
            MAX_RECEIPT_BYTES,
        )?;
        rules_failed = report_rules(&members, receipt, &mut show)?;
    }
    let mut sets_failed = false;
    if !sets.is_empty() {
        let endpoint = gateway::endpoint(gateway_url, "/v0/rule-set-registrations")?;
        sets_failed = sets::send(
            endpoint,
            authorization.clone(),
            &sets_body,
            &sets,
            &mut show,
        )
        .context("cannot register Rule Sets")?;
    }
    if let Some(body) = targets_body {
        let endpoint = gateway::endpoint(gateway_url, "/v0/target-registrations")?;
        targets::send(endpoint, authorization, &body, &config.targets, &mut show)
            .context("cannot register deployment targets")?;
    }
    match (rules_failed, sets_failed) {
        (true, true) => bail!("one or more rule and Rule Set registrations failed"),
        (true, false) => bail!("one or more rule registrations failed"),
        (false, true) => bail!("one or more Rule Set registrations failed"),
        (false, false) => Ok(()),
    }
}

fn batch_body(batch: &impl Serialize) -> Result<Vec<u8>> {
    let body = serde_json::to_vec(batch)?;
    ensure!(
        body.len() <= MAX_BATCH_BYTES,
        "registration exceeds the gateway request size limit"
    );
    Ok(body)
}

fn report_rules(
    members: &BTreeMap<PathBuf, sets::Member>,
    receipt: Receipt,
    show: &mut impl FnMut(&str),
) -> Result<bool> {
    ensure!(
        receipt.outcomes.len() == members.len(),
        "registration gateway returned an invalid receipt"
    );
    let mut failed = false;
    for (rule, outcome) in members.values().zip(receipt.outcomes) {
        let label = rule.label();
        let status = match outcome.outcome {
            State::Created => "created",
            State::Unchanged => "unchanged",
            State::Conflict => {
                failed = true;
                "conflict (content changed without a new version)"
            }
        };
        if outcome.digest == rule.digest {
            show(&format!("{status} {label}"));
        } else {
            failed = true;
            show(&format!(
                "{status} {label} (returned digest does not match)"
            ));
        }
    }
    Ok(failed)
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
) -> Result<(Value, sets::Member)> {
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
        sets::Member {
            source: source.to_owned(),
            path: rule_digest::portable_path(directory)?,
            id: rule.id.clone(),
            version: version.to_owned(),
            digest,
            enabled: rule.enabled,
        },
    ))
}

fn require_clean_inputs(
    root: &Path,
    directories: &BTreeSet<PathBuf>,
    set_paths: &BTreeSet<PathBuf>,
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
    .context("cannot check registration files against the current commit")?;
    let set_knowledge: BTreeSet<_> = set_paths
        .iter()
        .map(|path| path.with_file_name("ruleset.yaml"))
        .collect();
    for change in status.split_terminator('\0') {
        let path = Path::new(change.get(3..).context("invalid Git status entry")?);
        ensure!(
            !directories.contains(Path::new("."))
                && !path.ancestors().any(|part| directories.contains(part))
                && !set_paths.contains(path)
                && !set_knowledge.contains(path),
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

fn post<T: serde::de::DeserializeOwned>(
    endpoint: Uri,
    authorization: HeaderValue,
    body: &[u8],
    receipt_limit: u64,
) -> Result<T> {
    let response = gateway::post(endpoint, authorization, body, "registration")?;
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
        .take(receipt_limit + 1)
        .read_to_end(&mut body)
        .map_err(|_| anyhow::anyhow!("could not receive the registration gateway response"))?;
    ensure!(
        body.len() as u64 <= receipt_limit,
        "registration gateway returned an invalid receipt"
    );
    serde_json::from_slice(&body)
        .map_err(|_| anyhow::anyhow!("registration gateway returned an invalid receipt"))
}
