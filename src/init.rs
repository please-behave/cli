//! One-time project initialization: scaffolds `.please/` and never touches an existing one.
use crate::{
    config::{self, Config, RuleDefinition, source_path},
    limits::*,
};
use anyhow::{Result, ensure};
use std::{fs, path::Path};

/// Initialize configuration only when the configuration directory is absent.
pub fn init(root: &Path) -> Result<usize> {
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
        context: Default::default(),
        execution_points: Default::default(),
        imports: vec![],
        provider_imports: vec![],
    };
    let (runner, entrypoint, pass, fail) = if cfg!(windows) {
        (
            "cmd /C",
            "index.cmd",
            "@git --version\r\n",
            "@exit /b 1\r\n",
        )
    } else {
        (
            "sh",
            "index.sh",
            "#!/bin/sh\nexec git --version\n",
            "#!/bin/sh\nexit 1\n",
        )
    };
    let check = |id: &str, description: &str, enabled: bool| -> Result<String> {
        Ok(toml::to_string(&RuleDefinition {
            id: id.into(),
            depends_on: None,
            name: None,
            description: description.into(),
            enabled,
            blocking: true,
            run_at: vec!["pre-push".into()],
            parameters: Default::default(),
            check: Some(crate::config::CheckDefinition {
                resource: None,
                command: crate::check_command::Command::new(&format!(
                    "{runner} @rule/{entrypoint}"
                ))?,
                timeout_seconds: 30,
            }),
            review: None,
        })?)
    };
    let review = RuleDefinition {
        id: "product-consistency".into(),
        depends_on: None,
        name: None,
        description: "Evaluate behavior against documented expectations".into(),
        enabled: false,
        blocking: true,
        run_at: vec!["pre-push".into()],
        parameters: Default::default(),
        check: None,
        review: Some(crate::config::ReviewDefinition {
            input: Default::default(),
            parameters: Default::default(),
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
