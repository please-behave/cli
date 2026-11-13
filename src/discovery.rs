//! Local discovery and explicit external Rule references share one Rule loader.
use crate::{
    config::source_path,
    config::{self, Check, Config, Review, RuleDefinition, RuleSet},
    limits::*,
};
use anyhow::{Context, Result, ensure};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

/// Enumerate declared local inputs without resolving imports or executing tools.
pub(crate) fn snapshot_inputs(root: &Path) -> Result<BTreeSet<PathBuf>> {
    fn argument(root: &Path, value: &Path, paths: &mut BTreeSet<PathBuf>) {
        let full = root.join(value);
        if let Ok(relative) = full.strip_prefix(root) {
            if config::relative(relative).is_ok() && full.is_file() {
                paths.insert(relative.to_path_buf());
            }
        }
    }
    fn parameters(
        root: &Path,
        values: impl Iterator<Item = toml::Value>,
        paths: &mut BTreeSet<PathBuf>,
    ) {
        for value in values {
            if let Some(value) = value.as_str() {
                argument(root, Path::new(value), paths);
            }
        }
    }
    fn provider(root: &Path, path: &Path, paths: &mut BTreeSet<PathBuf>) -> Result<()> {
        let provider: config::ProviderConfig = document(root, path)?;
        paths.insert(path.to_owned());
        for value in std::iter::once(&provider.command).chain(&provider.args) {
            argument(root, Path::new(value), paths);
        }
        Ok(())
    }

    let directory = config::path("");
    let central = directory.join("config.toml");
    let mut paths = BTreeSet::new();
    if !root.join(&central).try_exists()? {
        return Ok(paths);
    }
    let config: Config = document(root, &central)?;
    paths.insert(central);
    for import in config.imports {
        parameters(
            root,
            import
                .rule_parameters
                .into_values()
                .flat_map(|v| v.into_values()),
            &mut paths,
        );
    }
    for path in entries(root, &directory.join("rule-sets"))? {
        paths.insert(path);
    }
    let legacy = directory.join("provider.toml");
    if root.join(&legacy).try_exists()? {
        provider(root, &legacy, &mut paths)?;
    }
    for path in entries(root, &directory.join("providers"))? {
        if !root.join(&path).is_dir() {
            provider(root, &path, &mut paths)?;
        }
    }
    for kind in ["checks", "reviews"] {
        for dir in entries(root, &directory.join(kind))? {
            let path = dir.join("rule.toml");
            let rule: RuleDefinition = document(root, &path)?;
            paths.insert(path);
            parameters(
                root,
                rule.parameters.into_values().filter_map(|p| p.value),
                &mut paths,
            );
            if let Some(check) = rule.check {
                for path in check.command.input_paths(root, &root.join(&dir))? {
                    argument(root, &path, &mut paths);
                }
            }
            if let Some(review) = rule.review {
                let prompt = resource(root, &dir, &review.prompt)?;
                paths.insert(prompt.strip_prefix(root)?.to_path_buf());
                for path in review
                    .files
                    .iter()
                    .chain(review.context.iter().map(|c| &c.path))
                {
                    argument(root, Path::new(path), &mut paths);
                }
            }
        }
    }
    Ok(paths)
}

pub(crate) struct Loaded {
    pub config: Config,
    pub providers: Vec<(PathBuf, String, config::ProviderConfig)>,
    pub checks: Vec<(PathBuf, Check)>,
    pub reviews: Vec<(PathBuf, Review)>,
    pub rule_sets: Vec<(PathBuf, RuleSet)>,
}
pub(crate) fn load(root: &Path) -> Result<Loaded> {
    let directory = config::path("");
    let config = document(root, &directory.join("config.toml"))?;
    let mut sets = Vec::new();
    for path in entries(root, &directory.join("rule-sets"))? {
        let full = source_path(root, &path)?;
        ensure!(
            full.is_file() && path.extension().is_some_and(|e| e == "toml"),
            "{}: expected a rule-set TOML file",
            path.display()
        );
        let set: RuleSet = document(root, &path)?;
        sets.push((path, set));
    }
    let mut directories = Vec::new();
    for kind in ["checks", "reviews"] {
        for dir in entries(root, &directory.join(kind))? {
            directories.push((kind.to_owned(), dir));
        }
    }
    let mut loaded = load_rules(root, config, sets, directories)?;
    loaded.providers = providers(root, &directory.join("providers"))?;
    Ok(loaded)
}
pub(crate) fn load_external(root: &Path, selected: &Path, consumer: Config) -> Result<Loaded> {
    let central = selected.join("config.toml");
    let mut set: RuleSet = document(root, &central)?;
    let mut directories = std::collections::BTreeMap::new();
    for reference in &set.rules {
        let reference = reference.id();
        let directory = source_path(root, Path::new(reference))
            .with_context(|| format!("missing or unsafe rule reference {reference:?}"))?;
        ensure!(
            directory.is_dir(),
            "rule reference must name a directory: {reference:?}"
        );
        let kind = directory
            .parent()
            .and_then(Path::file_name)
            .and_then(|s| s.to_str());
        ensure!(
            matches!(kind, Some("checks" | "reviews")),
            "rule reference must name a directory directly under checks/ or reviews/: {reference:?}"
        );
        directories.insert(
            directory.strip_prefix(root)?.to_path_buf(),
            kind.unwrap().to_owned(),
        );
        ensure!(directories.len() <= MAX_RULES, "at most {MAX_RULES} rules");
    }
    let mut loaded = load_rules(
        root,
        consumer,
        vec![],
        directories
            .into_iter()
            .map(|(path, kind)| (kind, path))
            .collect(),
    )?;
    // External paths select definitions; the shared resolver still operates on IDs.
    let ids: std::collections::BTreeMap<_, _> = loaded
        .checks
        .iter()
        .map(|(source, rule)| (source.parent().unwrap().to_path_buf(), rule.id.clone()))
        .chain(
            loaded
                .reviews
                .iter()
                .map(|(source, rule)| (source.parent().unwrap().to_path_buf(), rule.id.clone())),
        )
        .collect();
    for reference in &mut set.rules {
        let directory = source_path(root, Path::new(reference.id()))?;
        *reference.id_mut() = ids[directory.strip_prefix(root)?].clone();
    }
    loaded.providers = providers(root, &selected.join("providers"))?;
    loaded.rule_sets.push((central, set));
    Ok(loaded)
}
fn providers(
    root: &Path,
    directory: &Path,
) -> Result<Vec<(PathBuf, String, config::ProviderConfig)>> {
    let mut providers = Vec::new();
    for path in entries(root, directory)? {
        // Supporting adapter directories are data, never recursively discovered.
        if source_path(root, &path)?.is_dir() {
            continue;
        }
        ensure!(
            path.extension().is_some_and(|e| e == "toml"),
            "{}: expected a provider TOML file",
            path.display()
        );
        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .context("provider ID must be UTF-8")?
            .to_owned();
        let provider = document(root, &path).with_context(|| format!("invalid provider {id:?}"))?;
        providers.push((path, id, provider));
    }
    Ok(providers)
}

pub(crate) fn document<T: serde::de::DeserializeOwned>(root: &Path, path: &Path) -> Result<T> {
    let full =
        source_path(root, path).with_context(|| format!("missing or unsafe {}", path.display()))?;
    let text = config::read_bounded(&full, MAX_METADATA_BYTES)?;
    toml::from_str(&text).map_err(|e: toml::de::Error| {
        anyhow::anyhow!("invalid {}: {}", path.display(), e.message())
    })
}

fn resource(root: &Path, directory: &Path, filename: &str) -> Result<PathBuf> {
    ensure!(
        !filename.is_empty()
            && filename != "."
            && filename != ".."
            && !filename.contains(['/', '\\', ':', '\0'])
            && !Path::new(filename).is_absolute(),
        "resource must be a filename directly inside its rule directory: {filename:?}"
    );
    resolve_resource(root, &directory.join(filename))
}

fn resolve_resource(root: &Path, relative: &Path) -> Result<PathBuf> {
    let path = match source_path(root, relative) {
        Ok(path) => path,
        // Missing implementations become per-rule execution errors, including
        // when an interpreter would otherwise turn a missing script into FAIL.
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
        {
            return Ok(root.join(relative));
        }
        Err(error) => return Err(error),
    };
    ensure!(
        path.is_file(),
        "resource must be a regular file: {}",
        path.display()
    );
    Ok(path)
}

fn entries(root: &Path, directory: &Path) -> Result<Vec<PathBuf>> {
    match root.join(directory).symlink_metadata() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.into()),
        Ok(_) => {}
    }
    let full = source_path(root, directory)?;
    ensure!(full.is_dir(), "{} must be a directory", directory.display());
    let mut entries = fs::read_dir(full)?
        .map(|e| Ok(directory.join(e?.file_name())))
        .collect::<Result<Vec<_>>>()?;
    entries.sort();
    ensure!(
        entries.len() <= MAX_RULES,
        "at most {MAX_RULES} entries in {}",
        directory.display()
    );
    Ok(entries)
}

fn load_rules(
    root: &Path,
    config: Config,
    rule_sets: Vec<(PathBuf, RuleSet)>,
    directories: Vec<(String, PathBuf)>,
) -> Result<Loaded> {
    let mut loaded = Loaded {
        config,
        providers: vec![],
        rule_sets,
        checks: vec![],
        reviews: vec![],
    };
    for (kind, dir) in directories {
        let full = source_path(root, &dir)?;
        ensure!(
            full.is_dir(),
            "{}: a rule must be a directory",
            dir.display()
        );
        let path = dir.join("rule.toml");
        let rule: RuleDefinition = document(root, &path)?;
        let name = rule.name.unwrap_or_else(|| rule.id.clone());
        ensure!(
            !name.is_empty()
                && name.chars().count() <= MAX_RULE_NAME_CHARS
                && name.trim() == name
                && !name.chars().any(char::is_control),
            "{}: name must contain 1..={MAX_RULE_NAME_CHARS} characters without control characters or surrounding whitespace",
            path.display()
        );
        match (kind.as_str(), rule.check, rule.review) {
            ("checks", Some(check), None) => {
                ensure!(
                    check
                        .command
                        .first()
                        .is_some_and(|program| !program.trim().is_empty())
                        && check
                            .command
                            .iter()
                            .all(|argument| !argument.contains('\0')),
                    "{}: command requires a nonblank executable and arguments without NUL",
                    path.display()
                );
                for name in check.command.parameter_names() {
                    ensure!(
                        rule.parameters.contains_key(name),
                        "{}: command refers to undeclared parameter {name:?}",
                        path.display()
                    );
                }
                loaded.checks.push((
                    path.clone(),
                    Check {
                        depends_on: rule
                            .depends_on
                            .unwrap_or_default()
                            .into_iter()
                            .collect::<std::collections::BTreeSet<_>>()
                            .into_iter()
                            .collect(),
                        resource: check.resource,
                        id: rule.id.clone(),
                        name,
                        description: rule.description,
                        enabled: rule.enabled,
                        blocking: rule.blocking,
                        command: check.command,
                        source_root: root.to_path_buf(),
                        rule_directory: full,
                        timeout_seconds: check.timeout_seconds,
                        run_at: rule.run_at.clone(),
                        parameters: crate::parameters::Binding::local(rule.parameters),
                    },
                ));
            }
            ("reviews", None, Some(review)) => {
                ensure!(
                    rule.depends_on.is_none(),
                    "{}: depends_on is supported only for deterministic checks; reviews cannot declare dependencies",
                    path.display()
                );
                let prompt = resource(root, &dir, &review.prompt)?;
                ensure!(
                    Path::new(&review.prompt)
                        .extension()
                        .is_some_and(|e| e == "md"),
                    "review prompt must be Markdown (.md)"
                );
                let (text, execution_error) =
                    match config::read_bounded(&prompt, MAX_INSTRUCTION_BYTES) {
                        Ok(text) => (text, None),
                        Err(error)
                            if error
                                .downcast_ref::<std::io::Error>()
                                .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
                        {
                            (
                                String::new(),
                                Some(format!("Missing review prompt {}", prompt.display())),
                            )
                        }
                        Err(error) => return Err(error),
                    };
                loaded.reviews.push((
                    path.clone(),
                    Review {
                        input: review.input,
                        include_diff: review.include_diff
                            && review.input == config::ReviewInput::Worktree,
                        trigger: review.trigger,
                        provider: review.provider,
                        adapter_parameters: review.parameters,
                        id: rule.id.clone(),
                        name,
                        description: rule.description,
                        enabled: rule.enabled,
                        blocking: rule.blocking,
                        files: review.files,
                        context: review.context,
                        run_at: rule.run_at.clone(),
                        text,
                        execution_error,
                        parameters: crate::parameters::Binding::local(rule.parameters),
                    },
                ));
            }
            _ => anyhow::bail!(
                "{}: expected exactly one [{}] implementation",
                path.display(),
                if kind == "checks" { "check" } else { "review" }
            ),
        };
        ensure!(
            loaded.checks.len() + loaded.reviews.len() <= MAX_RULES,
            "at most {MAX_RULES} rules"
        );
    }
    Ok(loaded)
}
