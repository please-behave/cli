//! Compute ledger-compatible digests from rule source files.
mod canonical;
mod knowledge;

use crate::{
    config::{self, RuleDefinition},
    discovery,
    limits::{MAX_INSTRUCTION_BYTES, MAX_METADATA_BYTES},
    repository_identity,
};
use anyhow::{Context, Result, bail, ensure};
use serde::Serialize;
use serde_json::Map;
use std::{
    io::ErrorKind,
    path::{Component, Path},
};

/// The repository from which the rule files were loaded.
pub enum RuleSource<'a> {
    Local,
    Imported { git: &'a str },
}

#[derive(Serialize)]
struct DigestInput<'a> {
    source: &'a str,
    path: &'a str,
    id: &'a str,
    version: &'a str,
    #[serde(flatten)]
    kind: Kind,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    description: &'a str,
    knowledge: Map<String, serde_json::Value>,
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
enum Kind {
    Check,
    Review { blocking: bool, prompt: String },
}

/// Digest a rule in the project repository or an already resolved import checkout.
/// `directory` is relative to that repository's root. For an import, pass the
/// configured Git selector along with the checkout root.
///
/// Returns `None` when the definition lacks a version or the rule has no knowledge
/// document. An eligible rule with unreadable or invalid digest input returns an error.
pub fn from_directory(
    source: RuleSource<'_>,
    root: &Path,
    directory: &Path,
) -> Result<Option<String>> {
    config::relative(directory)?;
    let definition_path = directory.join("rule.toml");
    let rule: RuleDefinition = discovery::document(root, &definition_path)?;
    let Some(version) = discovery::rule_version(&rule, &definition_path)? else {
        return Ok(None);
    };

    let knowledge_path = directory.join("rule.yaml");
    let full = match config::source_path(root, &knowledge_path) {
        Ok(path) => path,
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == ErrorKind::NotFound) =>
        {
            return Ok(None);
        }
        Err(error) => {
            return Err(error).with_context(|| format!("unsafe {}", knowledge_path.display()));
        }
    };
    let knowledge = config::read_bounded(&full, MAX_METADATA_BYTES)
        .and_then(|text| knowledge::parse(&text))
        .with_context(|| format!("invalid {}", knowledge_path.display()))?;

    let identity = match source {
        RuleSource::Local => repository_identity::read_origin(root),
        RuleSource::Imported { git } => repository_identity::normalize(git),
    }
    .context("cannot determine rule source identity from a network Git remote")?;
    let path = directory
        .components()
        .filter_map(|part| match part {
            Component::Normal(name) => Some(name.to_str().context("rule path must be UTF-8")),
            _ => None,
        })
        .collect::<Result<Vec<_>>>()?
        .join("/");

    let kind = match (&rule.check, &rule.review) {
        (Some(_), None) => {
            ensure!(rule.blocking.is_none(), "a check must not declare blocking");
            Kind::Check
        }
        (None, Some(review)) => {
            let prompt = discovery::review_prompt_path(root, directory, &review.prompt)?;
            Kind::Review {
                blocking: rule.blocking.unwrap_or(true),
                prompt: config::read_bounded(&prompt, MAX_INSTRUCTION_BYTES)
                    .with_context(|| format!("cannot read review prompt {}", prompt.display()))?,
            }
        }
        _ => bail!("expected exactly one rule implementation"),
    };

    canonical::digest(serde_json::to_value(DigestInput {
        source: &identity,
        path: if path.is_empty() { "." } else { &path },
        id: &rule.id,
        version,
        kind,
        name: rule.name.as_deref(),
        description: &rule.description,
        knowledge,
    })?)
    .map(Some)
    .with_context(|| format!("cannot digest rule {:?}", rule.id))
}
