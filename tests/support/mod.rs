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
assert request['protocol_version'] == 0
assert request['execution_point'] == os.environ["EXPECTED_EXECUTION_POINT"]
assert os.path.isfile(request['context_file'])
print(json.dumps({'protocol_version': request['protocol_version'], 'reason': 'Context delivered', 'findings': [{'message': 'Evidence inspected', 'path': 'evidence', 'line': 1}]}))
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

/// Trust a repository for imports in an isolated, process-wide user trust store.
///
/// The store location is the same for every test of a binary, so the process-wide variable
/// cannot make tests disagree; subprocesses inherit it.
pub fn trust(repository: &Path) {
    trust_location(repository.to_str().unwrap());
}

/// Like [`trust`] for a Git location that is not a local path.
pub fn trust_location(location: &str) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        std::env::set_var("XDG_CONFIG_HOME", tempfile::tempdir().unwrap().keep());
    });
    please::external::trust::add(location).unwrap();
}

/// Commit everything so that the unpushed history is a change set for reviews at pre-push.
pub fn commit_all(root: &Path) {
    for args in [
        vec!["add", "-A"],
        vec![
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
            "commit",
            "-qm",
            "fixture",
            "--allow-empty",
        ],
    ] {
        let status = command("git")
            .current_dir(root)
            .args(args)
            .status()
            .unwrap();
        assert!(status.success());
    }
}

/// Stage everything so that the staged changes are a change set at pre-commit and commit-msg.
pub fn stage_all(root: &Path) {
    let status = command("git")
        .current_dir(root)
        .args(["add", "-A"])
        .status()
        .unwrap();
    assert!(status.success());
}

/// Make in-process runs at check compare against HEAD. The value is the same for every
/// test of a binary, so the process-wide variable cannot make tests disagree.
pub fn use_head_as_change_base() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe { std::env::set_var("PLEASE_CHANGE_BASE", "HEAD") });
}

/// Validate real plan output against the authoritative API schema.
pub fn assert_plan_contract(value: &serde_json::Value) {
    static VALIDATOR: std::sync::OnceLock<jsonschema::Validator> = std::sync::OnceLock::new();
    let validator = VALIDATOR.get_or_init(|| {
        let schema: serde_json::Value = serde_json::from_str(include_str!(
            "../../knowledge/contracts/plan/plan.schema.json"
        ))
        .unwrap();
        jsonschema::validator_for(&schema).unwrap()
    });
    let errors: Vec<_> = validator
        .iter_errors(value)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:#?}\n{value:#}");
}

/// Resolve the embedded Plan schema from bundled source; validation stays offline.
pub fn report_contract_validator() -> &'static jsonschema::Validator {
    static VALIDATOR: std::sync::OnceLock<jsonschema::Validator> = std::sync::OnceLock::new();
    VALIDATOR.get_or_init(|| {
        let plan: serde_json::Value = serde_json::from_str(include_str!(
            "../../knowledge/contracts/plan/plan.schema.json"
        ))
        .unwrap();
        let schema: serde_json::Value = serde_json::from_str(include_str!(
            "../../knowledge/contracts/report/report.schema.json"
        ))
        .unwrap();
        let registry = jsonschema::Registry::new()
            .add(plan["$id"].as_str().unwrap(), plan.clone())
            .unwrap()
            .prepare()
            .unwrap();
        jsonschema::options()
            .with_registry(&registry)
            .build(&schema)
            .unwrap()
    })
}

pub fn assert_report_contract(value: &serde_json::Value) {
    let errors: Vec<_> = report_contract_validator()
        .iter_errors(value)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:#?}\n{value:#}");
}

/// Validate raw Rule TOML against its configuration schema without resolving inputs.
pub fn rule_contract_validator() -> &'static jsonschema::Validator {
    static VALIDATOR: std::sync::OnceLock<jsonschema::Validator> = std::sync::OnceLock::new();
    VALIDATOR.get_or_init(|| {
        let schema: serde_json::Value = serde_json::from_str(include_str!(
            "../../knowledge/contracts/configuration/rule.schema.json"
        ))
        .unwrap();
        let parameters: serde_json::Value = serde_json::from_str(include_str!(
            "../../knowledge/contracts/configuration/parameters.schema.json"
        ))
        .unwrap();
        let registry = jsonschema::Registry::new()
            .add(parameters["$id"].as_str().unwrap(), parameters.clone())
            .unwrap()
            .prepare()
            .unwrap();
        jsonschema::options()
            .with_registry(&registry)
            .build(&schema)
            .unwrap()
    })
}

pub fn assert_rule_contract(text: &str) {
    let parsed: toml::Value = toml::from_str(text).unwrap();
    let value = serde_json::to_value(parsed).unwrap();
    let errors: Vec<_> = rule_contract_validator()
        .iter_errors(&value)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:#?}\n{text}");
}
