use super::{committed_text, post};
use crate::{config, discovery, limits::MAX_METADATA_BYTES, rule_digest, rules};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};
use ureq::http::{HeaderValue, Uri};

const LOCAL_SETS: &str = ".please/rule-sets";
const MAX_BATCH_SETS: usize = 100;
const MAX_RECEIPT_BYTES: u64 = 2 * 1024 * 1024;
const INVALID_RECEIPT: &str = "registration gateway returned an invalid receipt";

#[derive(Serialize)]
pub(super) struct Batch<'a> {
    pub commit: &'a str,
    pub sets: &'a [Set<'a>],
}

#[derive(Serialize)]
pub(super) struct Set<'a> {
    source: &'a str,
    path: String,
    id: String,
    version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    description: String,
    knowledge: Map<String, Value>,
    members: Vec<&'a Member>,
}

#[derive(Serialize)]
pub(super) struct Member {
    pub source: String,
    pub path: String,
    pub id: String,
    pub version: String,
    pub digest: String,
    pub enabled: bool,
}

impl Member {
    pub(super) fn label(&self) -> String {
        format!("{} {} ({})", self.id, self.version, self.path)
    }
}

pub(super) fn definition_paths<'a>(
    paths: impl Iterator<Item = &'a str>,
) -> Result<BTreeSet<PathBuf>> {
    paths
        .filter_map(|name| {
            let path = Path::new(name);
            (is_local(path) || path.file_name().is_some_and(|name| name == "ruleset.toml"))
                .then_some((name, path))
        })
        .map(|(name, path)| {
            ensure!(
                !name.contains('\u{fffd}'),
                "unsupported non-UTF-8 Rule Set path"
            );
            config::relative(path)?;
            Ok(path.to_path_buf())
        })
        .collect()
}

fn is_local(path: &Path) -> bool {
    path.parent() == Some(Path::new(LOCAL_SETS))
        && path
            .extension()
            .is_some_and(|extension| extension == "toml")
}

pub(super) fn prepare<'a>(
    root: &Path,
    source: &'a str,
    tree: &BTreeMap<&str, (&str, &str)>,
    paths: &BTreeSet<PathBuf>,
    rules: &'a BTreeMap<PathBuf, Member>,
) -> Result<Vec<Set<'a>>> {
    ensure!(
        paths.len() <= MAX_BATCH_SETS,
        "registration exceeds the gateway limit of {MAX_BATCH_SETS} Rule Sets per batch"
    );
    let sets = paths
        .iter()
        .map(|path| {
            prepare_set(root, source, tree, path, rules)
                .with_context(|| format!("cannot register Rule Set definition {}", path.display()))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut identities = BTreeSet::new();
    for set in &sets {
        ensure!(
            identities.insert((&set.path, &set.id)),
            "duplicate Rule Set {} ({})",
            set.id,
            set.path
        );
    }
    Ok(sets)
}

fn prepare_set<'a>(
    root: &Path,
    source: &'a str,
    tree: &BTreeMap<&str, (&str, &str)>,
    path: &Path,
    rules: &'a BTreeMap<PathBuf, Member>,
) -> Result<Set<'a>> {
    let definition = committed_text(root, path, MAX_METADATA_BYTES, tree)?;
    let set: config::RuleSet = discovery::parse_document(&definition, path)?;
    rules::name(&set.id)?;
    discovery::validate_rule_set_name(&set, path)?;
    let version = discovery::rule_set_version(&set, path)?
        .with_context(|| format!("Rule Set {} is missing version", set.id))?
        .to_owned();
    config::nonempty(&set.description, "Rule Set description")?;
    let knowledge_path = path.with_file_name("ruleset.yaml");
    let knowledge = committed_text(root, &knowledge_path, MAX_METADATA_BYTES, tree)
        .with_context(|| format!("Rule Set {} requires a committed ruleset.yaml", set.id))?;
    let knowledge = rule_digest::knowledge::parse(&knowledge)
        .with_context(|| format!("invalid {}", knowledge_path.display()))?;
    let mut seen = BTreeSet::new();
    let mut members = Vec::new();
    for reference in &set.rules {
        let member = resolve_member(rules, reference.id(), is_local(path)).with_context(|| {
            format!(
                "cannot register Rule Set {} member {:?}",
                set.id,
                reference.id()
            )
        })?;
        // Repeated references may add execution points to one member.
        if seen.insert(&member.path) {
            members.push(member);
        }
    }
    Ok(Set {
        source,
        path: rule_digest::portable_path(path.parent().context("missing Rule Set directory")?)?,
        id: set.id,
        version,
        name: set.name,
        description: set.description,
        knowledge,
        members,
    })
}

fn resolve_member<'a>(
    rules: &'a BTreeMap<PathBuf, Member>,
    reference: &str,
    local: bool,
) -> Result<&'a Member> {
    if local {
        let mut candidates = rules.iter().filter(|(path, rule)| {
            rule.id == reference
                && path.parent().is_some_and(|parent| {
                    parent == Path::new(".please/checks") || parent == Path::new(".please/reviews")
                })
        });
        let (_, member) = candidates.next().context("unknown rule ID")?;
        ensure!(candidates.next().is_none(), "ambiguous rule ID");
        return Ok(member);
    }
    let path = Path::new(reference);
    config::relative(path)?;
    // Git paths omit redundant components that a Rule directory reference can use.
    let path = rule_digest::portable_path(path)?;
    let directory = Path::new(&path);
    ensure!(
        matches!(
            directory
                .parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str()),
            Some("checks" | "reviews")
        ),
        "rule reference must name a directory directly under checks/ or reviews/"
    );
    rules.get(directory).context("unknown rule directory")
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    outcomes: Vec<Outcome>,
}

#[derive(Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case", deny_unknown_fields)]
enum Outcome {
    Created { digest: String },
    Unchanged { digest: String },
    Conflict { digest: String },
    InvalidMembers { failures: Vec<Failure> },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Failure {
    member_index: usize,
    reason: Reason,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Reason {
    Unregistered,
    DigestMismatch,
}

pub(super) fn send(
    endpoint: Uri,
    authorization: HeaderValue,
    body: &[u8],
    sets: &[Set<'_>],
    show: &mut impl FnMut(&str),
) -> Result<bool> {
    let receipt: Receipt = post(endpoint, authorization, body, MAX_RECEIPT_BYTES)?;
    ensure!(receipt.outcomes.len() == sets.len(), "{INVALID_RECEIPT}");
    for (set, outcome) in sets.iter().zip(&receipt.outcomes) {
        match outcome {
            Outcome::Created { digest }
            | Outcome::Unchanged { digest }
            | Outcome::Conflict { digest } => {
                ensure!(rule_digest::valid_digest(digest), "{INVALID_RECEIPT}");
            }
            Outcome::InvalidMembers { failures } => {
                let mut indices = BTreeSet::new();
                ensure!(
                    !failures.is_empty()
                        && failures.iter().all(|failure| {
                            failure.member_index < set.members.len()
                                && indices.insert(failure.member_index)
                        }),
                    "{INVALID_RECEIPT}"
                );
            }
        }
    }
    let mut failed = false;
    for (set, outcome) in sets.iter().zip(receipt.outcomes) {
        let label = format!("Rule Set {} {} ({})", set.id, set.version, set.path);
        match outcome {
            Outcome::Created { .. } => show(&format!("created {label}")),
            Outcome::Unchanged { .. } => show(&format!("unchanged {label}")),
            Outcome::Conflict { .. } => {
                failed = true;
                show(&format!(
                    "conflict {label} (content changed without a new version)"
                ));
            }
            Outcome::InvalidMembers { failures } => {
                failed = true;
                show(&format!("invalid_members {label}"));
                for failure in failures {
                    let member = set.members[failure.member_index].label();
                    let reason = match failure.reason {
                        Reason::Unregistered => "rule version is unregistered",
                        Reason::DigestMismatch => "rule digest does not match (digest_mismatch)",
                    };
                    show(&format!("  member {member}: {reason}"));
                }
            }
        }
    }
    Ok(failed)
}
