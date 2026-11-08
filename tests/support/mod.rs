#![allow(dead_code)]
use std::{fs, path::Path};

pub fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

pub fn check(root: &Path, id: &str, at: &[&str], script: &str) {
    let command = if cfg!(windows) {
        "cmd /C @rule/entrypoint"
    } else {
        "sh @rule/entrypoint"
    };
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
assert os.environ["PLEASE_EXECUTION_POINT"] == os.environ["EXPECTED_EXECUTION_POINT"]
with open(os.environ["CONTEXT_CALLS"], "a") as log:
    log.write("review\n")
request = json.load(sys.stdin)
assert request['protocol_version'] == 1
print(json.dumps({'protocol_version': 1, 'reviews': [{'id': r['id'], 'status': 'PASS', 'reason': 'Context delivered', 'findings': [{'message': 'Evidence inspected', 'path': 'evidence', 'line': 1}]} for r in request['reviews']]}))
"#).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    script
}
