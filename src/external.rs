//! Git distribution and isolated loading; semantics remain in config and rules.
mod cache;

use crate::{
    config::{self, Project},
    limits::*,
    rules::EffectiveRule,
};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Import {
    pub git: String,
    pub path: String,
    #[serde(rename = "ref")]
    pub reference: String,
    #[serde(default)]
    pub execution_points: BTreeMap<String, String>,
    #[serde(default)]
    pub rule_parameters: BTreeMap<String, BTreeMap<String, toml::Value>>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Origin {
    /// SHA-256 of the repository selector, never a credential-bearing URL.
    pub repository: String,
    pub path: String,
    pub reference: String,
    pub commit: String,
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u32,
    resolutions: BTreeMap<String, Origin>,
}

fn hash(value: impl AsRef<[u8]>) -> String {
    format!("{:x}", Sha256::digest(value.as_ref()))
}

impl Import {
    fn validate(&mut self, root: &Path) -> Result<()> {
        ensure!(
            !self.git.is_empty()
                && !self.git.starts_with("-")
                && !self.git.chars().any(char::is_control),
            "invalid Git repository selector"
        );
        if let Some((scheme, address)) = self.git.split_once("://") {
            ensure!(
                ["https", "http", "ssh", "git", "file"].contains(&scheme),
                "unsupported Git URL scheme; use HTTPS, SSH, Git, file or a local repository"
            );
            let authority = address.split('/').next().unwrap_or_default();
            ensure!(
                !authority.is_empty() || scheme == "file",
                "invalid Git repository URL"
            );
            ensure!(
                !authority.contains('@') || scheme == "ssh",
                "credential-bearing Git URLs are unsupported; configure host Git credentials"
            );
            ensure!(
                !(scheme == "ssh"
                    && authority
                        .split_once('@')
                        .is_some_and(|(userinfo, _)| userinfo.contains(':'))),
                "passwords in Git URLs are unsupported; configure SSH authentication"
            );
            ensure!(
                !self.git.contains(['?', '#']),
                "Git URL queries and fragments are unsupported"
            );
        } else if Path::new(&self.git).is_absolute()
            || self.git.starts_with("./")
            || self.git.starts_with("../")
            || !self.git.contains(':')
        {
            let path = root.join(&self.git);
            self.git = path
                .to_str()
                .context("Git repository path must be UTF-8")?
                .to_owned();
        } else {
            ensure!(
                !self.git.split_once("::").is_some_and(|(helper, _)| helper
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"+-_".contains(&c))),
                "unsupported Git remote-helper selector"
            );
            ensure!(
                self.git
                    .split_once(':')
                    .is_some_and(|(host, path)| !host.is_empty()
                        && !path.is_empty()
                        && !host.contains('/')),
                "invalid SSH repository selector"
            );
        }
        ensure!(
            !self.reference.is_empty()
                && self.reference.len() <= MAX_EXTERNAL_REFERENCE_BYTES
                && !self.reference.starts_with('-')
                && !self.reference.chars().any(char::is_control),
            "invalid Git reference"
        );
        config::relative(Path::new(&self.path)).context("invalid external rule-set path")?;
        ensure!(
            !self.path.chars().any(char::is_control),
            "invalid external rule-set path"
        );
        self.path = Path::new(&self.path)
            .components()
            .filter_map(|c| match c {
                Component::Normal(s) => Some(s.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("/");
        if self.path.is_empty() {
            self.path = ".".into();
        }
        Ok(())
    }
    fn repository(&self) -> String {
        hash(&self.git)
    }
    fn key(&self) -> String {
        hash(serde_json::to_vec(&(&self.repository(), &self.reference, &self.path)).unwrap())
    }
}

/// Explicit cache selection supports embedders and offline integration tests.
pub fn load(root: &Path, refresh: bool, cache_directory: Option<&Path>) -> Result<Project> {
    load_with_changes(root, refresh, cache_directory, &mut false)
}

pub(crate) fn load_with_changes(
    root: &Path,
    refresh: bool,
    cache_directory: Option<&Path>,
    changed: &mut bool,
) -> Result<Project> {
    let root = root.canonicalize()?;
    crate::git_snapshot::require_root(&root)?;
    let loaded = crate::discovery::load(&root)?;
    let mut imports = loaded.config.imports.clone();
    for import in &mut imports {
        import.validate(&root)?;
    }
    imports.sort_by_key(|import| {
        (
            import.key(),
            serde_json::to_string(&import.rule_parameters).unwrap(),
        )
    });
    ensure!(
        imports.len() <= MAX_RULES,
        "at most {MAX_RULES} external rule sets"
    );
    // Validate scope-local semantics first; provider references resolve after merging imports.
    let mut project = config::validate(root.clone(), loaded)?;
    for import in &imports {
        for (source, target) in &import.execution_points {
            let entity = format!(
                "import {:?} @ {:?}: execution_points {source:?} -> {target:?}",
                import.path, import.reference
            );
            crate::rules::name(source).with_context(|| entity.clone())?;
            crate::rules::name(target).with_context(|| entity.clone())?;
            ensure!(
                !crate::rules::BUILTIN_POINTS.contains(&source.as_str()),
                "{entity}: built-in source is reserved"
            );
            ensure!(
                !crate::rules::BUILTIN_POINTS.contains(&target.as_str()),
                "{entity}: built-in target is reserved"
            );
            ensure!(
                project.config.execution_points.custom.contains(target),
                "{entity}: unknown target; declare it in execution_points.custom"
            );
        }
    }
    if imports.is_empty() && !refresh {
        project.resolve_providers()?;
        return Ok(project);
    }
    let state_directory = PathBuf::from(cache::git(
        &root,
        None,
        &["rev-parse", "--absolute-git-dir"],
        false,
    )?)
    .join("please");
    let state_path = state_directory.join("external-rule-sets.json");
    if imports.is_empty() {
        project.resolve_providers()?;
        if state_path.try_exists()? {
            let _lock = cache::Lock::acquire(&state_directory.join("resolution.lock"))?;
            fs::remove_file(&state_path)?;
            fs::File::open(&state_directory)?.sync_all()?;
            *changed = true;
        }
        return Ok(project);
    }
    let cache_directory = cache_directory
        .map(Path::to_path_buf)
        .map(Ok)
        .unwrap_or_else(cache::directory)?;
    // The user-selected cache root may use platform aliases such as /var on macOS.
    // Managed children are checked separately and must never traverse symlinks.
    if refresh {
        fs::create_dir_all(&cache_directory).context("cannot create external cache root")?;
    }
    let cache_directory = cache_directory
        .canonicalize()
        .context("external cache unavailable; run please sync")?;
    let _state_lock = if refresh {
        cache::create_directory(&state_directory)
            .context("cannot create local Git resolution directory")?;
        cache::Lock::acquire(&state_directory.join("resolution.lock"))?
    } else {
        config::source_path(state_directory.parent().unwrap(), Path::new("please"))
            .context("external resolution state unavailable or unsafe; run please sync")?;
        cache::Lock::read(&state_directory.join("resolution.lock"))
            .context("external resolution state unavailable; run please sync")?
    };
    if !refresh {
        config::source_path(&state_directory, Path::new("external-rule-sets.json"))
            .context("external resolution state unavailable or unsafe; run please sync")?;
    }
    let previous: State = if refresh {
        State::default()
    } else {
        serde_json::from_str(
            &config::read_bounded(&state_path, MAX_METADATA_BYTES)
                .context("external resolution state unavailable; run please sync")?,
        )
        .context("invalid external resolution state; run please sync")?
    };
    ensure!(
        refresh || previous.version == 1,
        "unsupported external resolution state; run please sync"
    );
    let mut next = State {
        version: 1,
        resolutions: BTreeMap::new(),
    };
    let mut resolved: BTreeMap<(String, String), String> = BTreeMap::new();
    let mut checkouts: BTreeMap<(String, String), PathBuf> = BTreeMap::new();
    let mut bindings: BTreeMap<String, usize> = BTreeMap::new();
    let mut provider_sources = BTreeMap::new();
    for import in imports {
        let repository = import.repository();
        let key = import.key();
        let origin = if refresh {
            let ref_key = (repository.clone(), import.reference.clone());
            let commit = if let Some(commit) = resolved.get(&ref_key) {
                commit.clone()
            } else {
                let commit = cache::resolve(
                    &root,
                    &cache_directory,
                    &repository,
                    &import.git,
                    &import.reference,
                    changed,
                )?;
                resolved.insert(ref_key, commit.clone());
                commit
            };
            Origin {
                repository: repository.clone(),
                path: import.path.clone(),
                reference: import.reference.clone(),
                commit,
            }
        } else {
            let origin = previous
                .resolutions
                .get(&key)
                .context("external import has no validated revision; run please sync")?
                .clone();
            ensure!(
                origin.repository == repository
                    && origin.path == import.path
                    && origin.reference == import.reference,
                "external resolution state does not match configuration; run please sync"
            );
            origin
        };
        let revision_key = (repository.clone(), origin.commit.clone());
        let checkout = if let Some(checkout) = checkouts.get(&revision_key) {
            checkout.clone()
        } else {
            let checkout = cache::checkout(
                &root,
                &cache_directory,
                &repository,
                &origin.commit,
                refresh,
                changed,
            )
            .context("external cache unavailable or corrupt; run please sync")?;
            checkouts.insert(revision_key, checkout.clone());
            checkout
        };
        let boundary =
            config::source_path(&checkout, Path::new(&import.path)).with_context(|| {
                format!(
                    "missing or unsafe external rule-set directory {:?}",
                    import.path
                )
            })?;
        ensure!(
            boundary.is_dir(),
            "external rule-set path is not a directory"
        );
        let mut definitions = crate::discovery::load_external(
            &checkout,
            Path::new(&import.path),
            project.config.clone(),
        )
        .context("invalid external rule-set definitions")?;
        ensure!(
            definitions.rule_sets.len() == 1,
            "invalid external rule set: selected directory must define exactly one rule set"
        );
        // External roots have no project declarations. Validate all source names
        // in their own scope before translating effective assignments.
        let used: std::collections::BTreeSet<String> = definitions
            .rule_sets
            .iter()
            .flat_map(|(_, set)| &set.run_at)
            .chain(definitions.checks.iter().flat_map(|(_, rule)| &rule.run_at))
            .chain(
                definitions
                    .reviews
                    .iter()
                    .flat_map(|(_, rule)| &rule.run_at),
            )
            .cloned()
            .collect();
        for (source, target) in &import.execution_points {
            ensure!(
                used.contains(source),
                "import {:?} @ {:?}: execution_points {source:?} -> {target:?}: unknown source; source must be used by this imported Rule Set",
                import.path,
                import.reference
            );
        }
        definitions.config.execution_points.custom = used
            .iter()
            .filter(|point| !crate::rules::BUILTIN_POINTS.contains(&point.as_str()))
            .cloned()
            .collect();
        let sources: Vec<_> = definitions
            .providers
            .iter()
            .map(|(path, id, _)| {
                (
                    id.clone(),
                    (repository.clone(), origin.commit.clone(), path.clone()),
                )
            })
            .collect();
        let mut external = config::validate(root.clone(), definitions)
            .with_context(|| format!("invalid external rule set {:?}", import.path))?;
        for (id, source) in sources {
            if provider_sources.get(&id) == Some(&source) {
                continue;
            }
            ensure!(
                !project.providers.contains_key(&id),
                "duplicate provider identifier {id:?} in import {:?}; provider overrides are unsupported",
                import.path
            );
            ensure!(
                project.providers.len() < MAX_RULES,
                "at most {MAX_RULES} named providers including imports"
            );
            project
                .providers
                .insert(id.clone(), external.providers.remove(&id).unwrap());
            provider_sources.insert(id, source);
        }
        ensure!(
            project.checks.len()
                + project.reviews.len()
                + external.checks.len()
                + external.reviews.len()
                <= MAX_RULES
                && project.rule_sets.len() + external.rule_sets.len() <= MAX_RULES,
            "at most {MAX_RULES} rules and {MAX_RULES} rule sets including imports"
        );
        ensure!(
            project
                .reviews
                .iter()
                .chain(&external.reviews)
                .map(|r| r.text.len())
                .sum::<usize>()
                <= MAX_TOTAL_INSTRUCTION_BYTES,
            "total review instructions exceed {MAX_TOTAL_INSTRUCTION_BYTES} bytes"
        );
        for effective in external.effective.rules.values_mut() {
            effective.execution_points = effective
                .execution_points
                .iter()
                .map(|point| import.execution_points.get(point).unwrap_or(point).clone())
                .collect();
        }
        for source in &used {
            let target = import.execution_points.get(source).unwrap_or(source);
            ensure!(
                project.effective.points.contains(target),
                "import {:?} @ {:?}: execution_points {source:?} -> {target:?}: unknown resulting execution point; declare the custom target in execution_points.custom",
                import.path,
                import.reference
            );
        }
        // Resolution identifies immutable definitions. Execution identifies the
        // import binding, independently of custom execution-point mappings.
        let binding = hash(serde_json::to_vec(&(&key, &import.rule_parameters)).unwrap());
        let occurrence = bindings.entry(binding.clone()).or_default();
        let prefix = format!("git:{binding}:{}:{}:", import.path, *occurrence + 1);
        *occurrence += 1;
        for id in import.rule_parameters.keys() {
            ensure!(
                external.effective.rules.contains_key(id),
                "import {:?}: rule {id:?}: rule_parameters names an unknown rule",
                import.path
            );
        }
        for check in &mut external.checks {
            check.parameters.bind_import(
                import
                    .rule_parameters
                    .get(&check.id)
                    .cloned()
                    .unwrap_or_default(),
            );
            check.id = format!("{prefix}{}", check.id);
        }
        for review in &mut external.reviews {
            review.parameters.bind_import(
                import
                    .rule_parameters
                    .get(&review.id)
                    .cloned()
                    .unwrap_or_default(),
            );
            review.id = format!("{prefix}{}", review.id);
        }
        for set in &mut external.rule_sets {
            set.id = format!("{prefix}{}", set.id);
            for id in &mut set.rules {
                *id = format!("{prefix}{id}");
            }
        }
        for (id, effective) in external.effective.rules {
            let source = PathBuf::from(format!(
                "git/{repository}/{}@{}/{}",
                import.path,
                origin.commit,
                effective.source.display()
            ));
            project.effective.rules.insert(
                format!("{prefix}{id}"),
                EffectiveRule {
                    source,
                    execution_points: effective.execution_points,
                    rule_sets: effective
                        .rule_sets
                        .into_iter()
                        .map(|id| format!("{prefix}{id}"))
                        .collect(),
                    external: Some(origin.clone()),
                },
            );
        }
        project.effective.diagnostics.extend(
            external
                .effective
                .diagnostics
                .into_iter()
                .map(|message| format!("external {}: {message}", import.path)),
        );
        project.checks.extend(external.checks);
        project.reviews.extend(external.reviews);
        project.rule_sets.extend(external.rule_sets);
        next.resolutions.insert(key, origin);
    }
    project.resolve_providers()?;
    crate::review_context::collect(&root, &project.reviews.iter().collect::<Vec<_>>())?;
    project.checks.sort_by(|a, b| a.id.cmp(&b.id));
    project.reviews.sort_by(|a, b| a.id.cmp(&b.id));
    project.rule_sets.sort_by(|a, b| a.id.cmp(&b.id));
    if refresh {
        let bytes = serde_json::to_vec_pretty(&next)?;
        ensure!(
            bytes.len() as u64 <= MAX_METADATA_BYTES,
            "external resolution state exceeds {MAX_METADATA_BYTES} bytes"
        );
        // Publication still runs as before; only the displayed outcome compares contents.
        let previous = config::read_bounded(&state_path, MAX_METADATA_BYTES).ok();
        *changed |= previous.as_ref().map(|text| text.as_bytes()) != Some(bytes.as_slice());
        let mut temporary = tempfile::NamedTempFile::new_in(&state_directory)?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        temporary
            .persist(&state_path)
            .context("cannot publish external resolution state")?;
        fs::File::open(&state_directory)?.sync_all()?;
    }
    Ok(project)
}
