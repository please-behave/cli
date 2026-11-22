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
            digest_definition(
                root,
                directory,
                rule,
                || Some(source.to_owned()),
                &path,
                version,
            )
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
                    self.source.is_some()
                        && self.version.is_some()
                        && digest.strip_prefix("v1:sha256:").is_some_and(|hex| {
                            hex.len() == 64
                                && hex
                                    .bytes()
                                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
                        })
                }),
            "invalid report rule metadata"
        );
        Ok(())
    }
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
    digest_definition(
        root,
        directory,
        &rule,
        source,
        &portable_path(directory)?,
        version,
    )
}

fn portable_path(directory: &Path) -> Result<String> {
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
    path: &str,
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
    let knowledge = config::read_bounded(&full, MAX_METADATA_BYTES)
        .and_then(|text| knowledge::parse(&text))
        .with_context(|| format!("invalid {}", knowledge_path.display()))?;

    let identity =
        source().context("cannot determine rule source identity from a network Git remote")?;

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
        path,
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
