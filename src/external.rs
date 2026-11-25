//! Git distribution and isolated loading; semantics remain in config and rules.
mod cache;
pub mod trust;

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

/// A named adapter package selected independently of Rule Sets.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterImport {
    pub id: String,
    pub git: String,
    pub path: String,
    #[serde(rename = "ref")]
    pub reference: String,
}
impl AdapterImport {
    fn selector(&self) -> Import {
        Import {
            git: self.git.clone(),
            path: self.path.clone(),
            reference: self.reference.clone(),
            execution_points: BTreeMap::new(),
            rule_parameters: BTreeMap::new(),
        }
    }
}

/// Runtime cache location stays private; public provenance contains no cache paths.
#[derive(Debug)]
pub struct AdapterSource {
    pub origin: Origin,
    root: PathBuf,
}
impl AdapterSource {
    pub(crate) fn resolve(
        &self,
        adapter: &config::AdapterProfile,
        repo: &Path,
    ) -> Result<config::AdapterProfile> {
        cache::validate_revision(
            self.root
                .parent()
                .context("missing adapter cache revision")?,
            &self.origin.commit,
        )
        .context("external adapter profile cache unavailable or corrupt; run please sync")?;
        let mut adapter = adapter.clone();
        for token in std::iter::once(&mut adapter.command)
            .chain(&mut adapter.args)
            .chain(&mut adapter.requirements.executables)
        {
            if let Some(literal) = token.strip_prefix("@@") {
                *token = format!("@{literal}");
                continue;
            }
            if !token.starts_with('@') {
                continue;
            }
            let (prefix, suffix) = token.split_once('/').context(
                "external adapter profile references require @adapter/, @source/ or @repo/",
            )?;
            config::relative(Path::new(suffix))
                .context("unsafe external adapter profile resource path")?;
            let (root, relative): (&Path, PathBuf) = match prefix {
                "@adapter" => (&self.root, Path::new(&self.origin.path).join(suffix)),
                "@source" => (&self.root, PathBuf::from(suffix)),
                "@repo" => (repo, PathBuf::from(suffix)),
                _ => anyhow::bail!(
                    "unknown external adapter path reference; escape a literal @ with @@"
                ),
            };
            *token = config::source_path(root, &relative)?
                .to_str()
                .context("adapter resource path must be UTF-8")?
                .to_owned();
        }
        Ok(adapter)
    }
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
        config::relative(Path::new(&self.path)).context("invalid external import path")?;
        ensure!(
            !self.path.chars().any(char::is_control),
            "invalid external import path"
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

/// Validates a Git location as an import would and returns it as the import resolves it.
pub fn repository_selector(git: &str, root: &Path) -> Result<String> {
    let mut import = Import {
        git: git.to_owned(),
        path: ".".into(),
        reference: "HEAD".into(),
        execution_points: BTreeMap::new(),
        rule_parameters: BTreeMap::new(),
    };
    import.validate(root)?;
    Ok(import.git)
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
    let mut imports: Vec<_> = loaded
        .config
        .imports
        .iter()
        .cloned()
        .map(|import| (import, None))
        .collect();
    let mut adapter_ids = std::collections::BTreeSet::new();
    for adapter in &loaded.config.adapter_imports {
        config::identity(&adapter.id, "external adapter profile", &mut adapter_ids)?;
        imports.push((adapter.selector(), Some(adapter.id.clone())));
    }
    for (import, _) in &mut imports {
        import.validate(&root)?;
    }
    trust::require(imports.iter().map(|(import, _)| import.git.as_str()))?;
    imports.sort_by_key(|(import, adapter)| {
        (
            import.key(),
            adapter.clone(),
            serde_json::to_string(&import.rule_parameters).unwrap(),
        )
    });
    ensure!(
        imports.len() <= MAX_RULES,
        "at most {MAX_RULES} external Rule Set and adapter imports"
    );
    // Validate scope-local semantics first; adapter references resolve after merging imports.
    let mut project = config::validate(root.clone(), loaded)?;
    for (import, _) in &imports {
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
        project.resolve_adapters()?;
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
        project.resolve_adapters()?;
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
        refresh || matches!(previous.version, 0 | 1),
        "unsupported external resolution state; run please sync"
    );
    let mut next = State {
        version: 0,
        resolutions: BTreeMap::new(),
    };
    let mut resolved: BTreeMap<(String, String), String> = BTreeMap::new();
    let mut checkouts: BTreeMap<(String, String), PathBuf> = BTreeMap::new();
    let mut bindings: BTreeMap<String, usize> = BTreeMap::new();
    let mut adapter_definitions: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    for (import, adapter_id) in imports {
        let repository = import.repository();
        let key = match &adapter_id {
            Some(id) => hash(serde_json::to_vec(&("adapter", id, import.key()))?),
            None => import.key(),
        };
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
                    "missing or unsafe external import directory {:?}",
                    import.path
                )
            })?;
        ensure!(boundary.is_dir(), "external import path is not a directory");
        if let Some(id) = adapter_id {
            ensure!(
                !project.adapters.contains_key(&id),
                "duplicate adapter profile identifier {id:?}; adapter profile overrides are unsupported"
            );
            ensure!(
                project.adapters.len() < MAX_RULES,
                "at most {MAX_RULES} named adapter profiles including imports"
            );
            let adapter: config::AdapterProfile = crate::discovery::document(
                &checkout,
                &Path::new(&import.path).join("adapter.toml"),
            )
            .with_context(|| format!("invalid external adapter profile {id:?}"))?;
            config::validate_adapter(&adapter)
                .with_context(|| format!("invalid external adapter profile {id:?}"))?;
            let source = AdapterSource {
                origin: origin.clone(),
                root: checkout,
            };
            source.resolve(&adapter, &root).with_context(|| {
                format!("invalid external adapter profile resources for {id:?}")
            })?;
            project.adapters.insert(id.clone(), adapter);
            project.external_adapters.insert(id, source);
            next.resolutions.insert(key, origin);
            continue;
        }
        let mut definitions = crate::discovery::load_external(
            &checkout,
            Path::new(&import.path),
            project.config.clone(),
            &import.git,
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
            .flat_map(|(_, set)| {
                set.run_at.iter().chain(
                    set.rules
                        .iter()
                        .filter_map(|reference| reference.run_at())
                        .flatten(),
                )
            })
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
        let ids: Vec<_> = definitions
            .adapters
            .iter()
            .map(|(_, id, _)| id.clone())
            .collect();
        let mut external = config::validate(root.clone(), definitions)
            .with_context(|| format!("invalid external rule set {:?}", import.path))?;
        for id in ids {
            let adapter = external.adapters.remove(&id).unwrap();
            let definition = serde_json::to_value(&adapter)?;
            if adapter_definitions.get(&id) == Some(&definition) {
                continue;
            }
            ensure!(
                !project.adapters.contains_key(&id),
                "duplicate adapter profile identifier {id:?} in import {:?}; adapter profile overrides are unsupported",
                import.path
            );
            ensure!(
                project.adapters.len() < MAX_RULES,
                "at most {MAX_RULES} named adapter profiles including imports"
            );
            project.adapters.insert(id.clone(), adapter);
            adapter_definitions.insert(id, definition);
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
        let mapped_point =
            |point: &String| import.execution_points.get(point).unwrap_or(point).clone();
        for effective in external.effective.rules.values_mut() {
            effective.execution_points = effective
                .execution_points
                .iter()
                .map(mapped_point)
                .collect();
        }
        for (set, points) in external.versioned_rule_sets {
            project
                .versioned_rule_sets
                .entry(set)
                .or_default()
                .extend(points.iter().map(mapped_point));
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
            for dependency in &mut check.depends_on {
                if !dependency.contains(':') {
                    *dependency = format!("{prefix}{dependency}");
                }
            }
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
            for reference in &mut set.rules {
                let id = reference.id_mut();
                *id = format!("{prefix}{id}");
            }
        }
        for (id, effective) in external.effective.rules {
            let run_id = format!("{prefix}{id}");
            if let Some(metadata) = external.rule_metadata.remove(&id) {
                project.rule_metadata.insert(run_id.clone(), metadata);
            }
            let source = PathBuf::from(format!(
                "git/{repository}/{}@{}/{}",
                import.path,
                origin.commit,
                effective.source.display()
            ));
            project.effective.rules.insert(
                run_id,
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
    project.resolve_adapters()?;
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
