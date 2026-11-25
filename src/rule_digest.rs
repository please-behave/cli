//! Compute ledger-compatible digests from rule source files.
mod canonical;
pub(crate) mod knowledge;

use crate::{
    config::{self, RuleDefinition},
    discovery,
    limits::{MAX_INSTRUCTION_BYTES, MAX_METADATA_BYTES},
    repository_identity,
};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
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

/// Portable metadata captured from a rule directory before import qualification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuleMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub path: String,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub digest: Option<String>,
}

impl RuleMetadata {
    pub(crate) fn from_definition(
        source: Option<&str>,
        root: &Path,
        directory: &Path,
        rule: &RuleDefinition,
        version: Option<&str>,
    ) -> Option<Self> {
        if source.is_none() && version.is_none() {
            return None;
        }
        let path = portable_path(directory).ok()?;
        // Missing or unusable digest inputs leave the rule's identity and
        // declared version available without changing its execution outcome.
        let digest = source.zip(version).and_then(|(source, version)| {
            digest_definition(root, directory, rule, || Some(source.to_owned()), version)
                .ok()
                .flatten()
        });
        Some(Self {
            source: source.map(str::to_owned),
            path,
            id: rule.id.clone(),
            version: version.map(str::to_owned),
            digest,
        })
    }

    pub(crate) fn validate(&self) -> Result<()> {
        ensure!(
            [&self.path, &self.id]
                .into_iter()
                .all(|value| !value.is_empty() && !value.contains('\0'))
                && self.source.as_deref().is_none_or(|source| {
                    !source.is_empty() && !source.contains('\0') && source.chars().count() <= 2048
                })
                && (self.source.is_some() || self.version.is_some())
                && self
                    .version
                    .as_deref()
                    .is_none_or(discovery::valid_rule_version)
                && self.digest.as_deref().is_none_or(|digest| {
                    self.source.is_some() && self.version.is_some() && valid_digest(digest)
                }),
            "invalid report rule metadata"
        );
        Ok(())
    }
}

pub(crate) fn valid_digest(digest: &str) -> bool {
    digest.strip_prefix("v1:sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
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
    let source = || match source {
        RuleSource::Local => repository_identity::read_origin(root),
        RuleSource::Imported { git } => repository_identity::normalize(git),
    };
    digest_definition(root, directory, &rule, source, version)
}

pub(crate) fn portable_path(directory: &Path) -> Result<String> {
    let path = directory
        .components()
        .filter_map(|part| match part {
            Component::Normal(name) => Some(name.to_str().context("rule path must be UTF-8")),
            _ => None,
        })
        .collect::<Result<Vec<_>>>()?
        .join("/");
    Ok(if path.is_empty() { ".".into() } else { path })
}

fn digest_definition(
    root: &Path,
    directory: &Path,
    rule: &RuleDefinition,
    source: impl FnOnce() -> Option<String>,
    version: &str,
) -> Result<Option<String>> {
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
    let knowledge = config::read_bounded(&full, MAX_METADATA_BYTES)?;
    let identity =
        source().context("cannot determine rule source identity from a network Git remote")?;
    let document = rule_document(
        root,
        directory,
        rule,
        &identity,
        version,
        &knowledge,
        |prompt| {
            config::read_bounded(prompt, MAX_INSTRUCTION_BYTES)
                .with_context(|| format!("cannot read review prompt {}", prompt.display()))
        },
    )?;
    digest(document)
        .map(Some)
        .with_context(|| format!("cannot digest rule {:?}", rule.id))
}

pub(crate) fn rule_document(
    root: &Path,
    directory: &Path,
    rule: &RuleDefinition,
    source: &str,
    version: &str,
    knowledge: &str,
    read_prompt: impl FnOnce(&Path) -> Result<String>,
) -> Result<serde_json::Value> {
    let path = portable_path(directory)?;
    let knowledge = knowledge::parse(knowledge)
        .with_context(|| format!("invalid {}", directory.join("rule.yaml").display()))?;
    let kind = match (&rule.check, &rule.review) {
        (Some(_), None) => {
            ensure!(rule.blocking.is_none(), "a check must not declare blocking");
            Kind::Check
        }
        (None, Some(review)) => {
            let prompt = discovery::review_prompt_path(root, directory, &review.prompt)?;
            Kind::Review {
                blocking: rule.blocking.unwrap_or(true),
                prompt: read_prompt(&prompt)?,
            }
        }
        _ => bail!("expected exactly one rule implementation"),
    };

    Ok(serde_json::to_value(DigestInput {
        source,
        path: &path,
        id: &rule.id,
        version,
        kind,
        name: rule.name.as_deref(),
        description: &rule.description,
        knowledge,
    })?)
}

pub(crate) fn digest(document: serde_json::Value) -> Result<String> {
    canonical::digest(document)
}
