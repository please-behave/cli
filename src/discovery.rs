//! Local discovery and explicit external Rule references share one Rule loader.
use crate::{
    config::source_path,
    config::{self, Check, Config, Review, RuleDefinition, RuleSet},
    limits::*,
};
use anyhow::{Context, Result, ensure};
use std::{
    fs,
    path::{Path, PathBuf},
};

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
    set.rules = loaded
        .checks
        .iter()
        .map(|(_, rule)| rule.id.clone())
        .chain(loaded.reviews.iter().map(|(_, rule)| rule.id.clone()))
        .collect();
    loaded.providers = providers(root, &selected.join("providers"))?;
    loaded.rule_sets.push((central, set));
    Ok(loaded)
}
fn providers(
    root: &Path,
    directory: &Path,
) -> Result<Vec<(PathBuf, String, config::ProviderConfig)>> {
    entries(root, directory)?
        .into_iter()
        .map(|path| {
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
            let provider =
                document(root, &path).with_context(|| format!("invalid provider {id:?}"))?;
            Ok((path, id, provider))
        })
        .collect()
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

fn check_entrypoint(root: &Path, directory: &Path, entrypoint: &str) -> Result<PathBuf> {
    if !entrypoint.contains('/') {
        return resource(root, directory, entrypoint);
    }
    ensure!(
        !entrypoint.contains(['\\', ':', '\0']),
        "check entrypoint must be a relative source path using /: {entrypoint:?}"
    );
    config::relative(Path::new(entrypoint))?;
    resolve_resource(root, Path::new(entrypoint))
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
                    check.entrypoint.is_some() != check.command.is_some(),
                    "{}: check requires exactly one of entrypoint or command",
                    path.display()
                );
                ensure!(
                    check.command.is_none() || check.interpreter.is_none(),
                    "{}: interpreter requires entrypoint and cannot be used with command",
                    path.display()
                );
                if let Some(command) = &check.command {
                    ensure!(
                        command
                            .first()
                            .is_some_and(|program| !program.trim().is_empty())
                            && command.iter().all(|argument| !argument.contains('\0')),
                        "{}: command requires a nonblank executable and arguments without NUL",
                        path.display()
                    );
                }
                let entrypoint = check
                    .entrypoint
                    .as_deref()
                    .map(|entrypoint| check_entrypoint(root, &dir, entrypoint))
                    .transpose()?;
                if let Some(interpreter) = &check.interpreter {
                    ensure!(
                        !interpreter.trim().is_empty() && !interpreter.contains('\0'),
                        "interpreter must name one executable"
                    );
                }
                loaded.checks.push((
                    path.clone(),
                    Check {
                        resource: check.resource,
                        id: rule.id.clone(),
                        name,
                        description: rule.description,
                        enabled: rule.enabled,
                        blocking: rule.blocking,
                        entrypoint,
                        command: check.command,
                        source_root: root.to_path_buf(),
                        rule_directory: full,
                        interpreter: check.interpreter,
                        timeout_seconds: check.timeout_seconds,
                        run_at: rule.run_at.clone(),
                        parameters: crate::parameters::Binding::local(rule.parameters),
                    },
                ));
            }
            ("reviews", None, Some(review)) => {
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
                        include_diff: review.include_diff,
                        trigger: review.trigger,
                        provider: review.provider,
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

/// Initialize configuration only when the configuration directory is absent.
pub(crate) fn init(root: &Path) -> Result<usize> {
    use std::{fs::OpenOptions, io::Write};
    let root = root.canonicalize()?;
    match root.join(config::path("")).symlink_metadata() {
        Ok(_) => anyhow::bail!("Already initialized (.please/ exists)."),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    crate::git_snapshot::require_root(&root)?;
    for relative in [
        "",
        "checks",
        "checks/always-ok",
        "checks/deliberately-failing",
        "reviews",
        "reviews/product-consistency",
        "reports",
    ] {
        let relative = config::path(relative);
        let full = root.join(&relative);
        if full
            .symlink_metadata()
            .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
        {
            fs::create_dir(&full)?;
        }
        ensure!(
            source_path(&root, &relative)?.is_dir(),
            "expected configuration directory"
        );
    }
    let project = root.file_name().unwrap_or_default().to_string_lossy();
    let config = Config {
        project: project.into_owned(),
        max_display_files: MAX_DISPLAY_FILES,
        execution: Default::default(),
        execution_points: Default::default(),
        imports: vec![],
    };
    let (entrypoint, pass, fail) = if cfg!(windows) {
        ("index.cmd", "@git --version\r\n", "@exit /b 1\r\n")
    } else {
        (
            "index.sh",
            "#!/bin/sh\nexec git --version\n",
            "#!/bin/sh\nexit 1\n",
        )
    };
    let check = |id: &str, description: &str, enabled: bool| -> Result<String> {
        Ok(toml::to_string(&RuleDefinition {
            id: id.into(),
            name: None,
            description: description.into(),
            enabled,
            blocking: true,
            run_at: vec!["pre-push".into()],
            parameters: Default::default(),
            check: Some(crate::config::CheckDefinition {
                resource: None,
                entrypoint: Some(entrypoint.into()),
                command: None,
                interpreter: None,
                timeout_seconds: 30,
            }),
            review: None,
        })?)
    };
    let review = RuleDefinition {
        id: "product-consistency".into(),
        name: None,
        description: "Evaluate behavior against documented expectations".into(),
        enabled: false,
        blocking: true,
        run_at: vec!["pre-push".into()],
        parameters: Default::default(),
        check: None,
        review: Some(crate::config::ReviewDefinition {
            include_diff: true,
            trigger: None,
            provider: None,
            prompt: "prompt.md".into(),
            files: vec![],
            context: vec![],
        }),
    };
    let files = [
        ("config.toml".into(), toml::to_string(&config)?),
        ("provider.toml".into(), toml::to_string(&crate::config::ProviderConfig::scaffold())?),
        ("checks/always-ok/rule.toml".into(), check("always-ok", "Example deterministic pass", true)?),
        (format!("checks/always-ok/{entrypoint}"), pass.into()),
        ("checks/deliberately-failing/rule.toml".into(), check("deliberately-failing", "Disabled example demonstrating failure", false)?),
        (format!("checks/deliberately-failing/{entrypoint}"), fail.into()),
        ("reviews/product-consistency/rule.toml".into(), toml::to_string(&review)?),
        ("reviews/product-consistency/prompt.md".into(), "Evaluate the current working files against documented expectations. Cite concrete file paths and lines. Report uncertainty as ERROR, grounded contradictions as FAIL, and PASS only with evidence. Customize before enabling.\n".into()),
        ("reports/.gitignore".into(), "*\n!.gitignore\n".into()),
        ("README.md".into(), include_str!("../docs/configuration.md").into()),
    ];
    let mut created = 0;
    for (relative, text) in files {
        let full = root.join(config::path(&relative));
        match OpenOptions::new().write(true).create_new(true).open(&full) {
            Ok(mut file) => {
                file.write_all(text.as_bytes())?;
                #[cfg(unix)]
                if relative.ends_with(".sh") {
                    use std::os::unix::fs::PermissionsExt;
                    file.set_permissions(fs::Permissions::from_mode(0o755))?;
                }
                created += 1;
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing = source_path(&root, &config::path(&relative))?;
                ensure!(
                    existing.is_file(),
                    "configuration path must be a regular file"
                );
            }
            Err(e) => return Err(e.into()),
        }
    }
    Ok(created)
}
