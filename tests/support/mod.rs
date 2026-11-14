#![allow(dead_code)]
use std::{fs, path::Path, process::Command};

pub fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

pub fn check(root: &Path, id: &str, at: &[&str], script: &str) {
    check_args(root, id, at, "", script);
}

/// A check whose command passes `arguments` (references such as `@context`) to its script.
pub fn check_args(root: &Path, id: &str, at: &[&str], arguments: &str, script: &str) {
    let command = format!(
        "{} @rule/entrypoint {arguments}",
        if cfg!(windows) { "cmd /C" } else { "sh" }
    );
    let command = command.trim_end();
    write(
        root,
        &format!(".please/checks/{id}/rule.toml"),
        &format!(
            "id = {id:?}\ndescription = 'Test check'\nenabled = true\nrun_at = {at:?}\n[check]\ncommand = {command:?}\ntimeout_seconds = 10\n"
        ),
    );
    write(root, &format!(".please/checks/{id}/entrypoint"), script);
}

pub fn review_path(root: &Path, id: &str) -> std::path::PathBuf {
    let directory = root.join(".please/reviews").join(id);
    fs::create_dir_all(&directory).unwrap();
    fs::copy(
        root.join(".please/reviews/product-consistency/prompt.md"),
        directory.join("prompt.md"),
    )
    .unwrap();
    directory.join("rule.toml")
}

/// Explicit test configuration; no executable is launched by in-memory providers.
pub fn configure_adapter(root: &Path) {
    write(
        root,
        ".please/provider.toml",
        "command = 'fake-adapter'\ntimeout_seconds = 180\n",
    );
}

/// Provider-neutral process fixture validating execution context.
#[cfg(unix)]
pub fn context_provider(directory: &Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let script = directory.join("fake-adapter");
    fs::write(&script, r#"#!/usr/bin/env python3
import json, os, sys
assert not [name for name in os.environ if name.startswith("PLEASE_")]
with open(os.environ["CONTEXT_CALLS"], "a") as log:
    log.write("review\n")
request = json.load(sys.stdin)
assert request['protocol_version'] == 2
assert request['execution_point'] == os.environ["EXPECTED_EXECUTION_POINT"]
assert os.path.isfile(request['context_file'])
print(json.dumps({'protocol_version': 2, 'reason': 'Context delivered', 'findings': [{'message': 'Evidence inspected', 'path': 'evidence', 'line': 1}]}))
"#).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    script
}

/// The only way tests start processes: the harness environment must never leak into them.
///
/// Removes every `PLEASE_*` and `GIT_*` variable plus `CI` and `GITHUB_ACTIONS`, which the
/// governor, Git hooks and CI export while running this very test suite.
pub fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    let mut command = Command::new(program);
    for (name, _) in std::env::vars_os() {
        let text = name.to_string_lossy();
        if text.starts_with("PLEASE_")
            || text.starts_with("GIT_")
            || matches!(text.as_ref(), "CI" | "GITHUB_ACTIONS")
        {
            command.env_remove(name);
        }
    }
    command
}
