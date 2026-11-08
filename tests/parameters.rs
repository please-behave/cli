#![cfg(unix)]
mod support;
use please::{
    VerificationMode, config, planner,
    provider::{Provider, Request},
    report::{self, Status},
};
use std::{fs, path::Path, process::Command, sync::Mutex};
use support::{check, write};
use tempfile::TempDir;

struct Never;
impl Provider for Never {
    fn review(&self, _: &Request<'_>) -> anyhow::Result<String> {
        panic!("unexpected provider call")
    }
}
fn project() -> TempDir {
    let root = tempfile::tempdir().unwrap();
    assert!(
        Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(root.path())
            .status()
            .unwrap()
            .success()
    );
    config::init(root.path()).unwrap();
    support::configure_adapter(root.path());
    root
}
fn parameters(root: &Path, id: &str, text: &str) {
    let path = root.join(format!(".please/checks/{id}/rule.toml"));
    let mut rule = fs::read_to_string(&path).unwrap();
    rule.push_str(text);
    fs::write(path, rule).unwrap();
}
fn run(root: &Path) -> report::Report {
    please::verify_project_at(
        config::load(root).unwrap(),
        &Never,
        VerificationMode::AllRules,
        Some("check"),
        &mut |_| {},
    )
    .unwrap()
}

#[test]
fn local_parameters_deliver_strings_and_consumer_files_without_changing_rule_resources() {
    let root = project();
    write(root.path(), "conventions/scopes.txt", "cli\n");
    check(
        root.path(),
        "parameters",
        &["check"],
        r#"
test "$PWD" = "$(git rev-parse --show-toplevel)" || exit 1
test "$PLEASE_PARAM_LABEL" = 'arbitrary value $with spaces' || exit 1
test "$PLEASE_PARAM_SCOPES_FILE" = "$PWD/conventions/scopes.txt" || exit 1
test "$(cat "$PLEASE_PARAM_SCOPES_FILE")" = cli || exit 1
test "$(cat "$PLEASE_RULE_DIRECTORY/resource.txt")" = bundled || exit 1
test "${PLEASE_PARAM_UNUSED+x}" != x || exit 1
"#,
    );
    write(
        root.path(),
        ".please/checks/parameters/resource.txt",
        "bundled\n",
    );
    parameters(
        root.path(),
        "parameters",
        r#"
[parameters.label]
type = 'string'
required = true
value = 'arbitrary value $with spaces'
[parameters.scopes_file]
type = 'file'
required = true
value = 'conventions/scopes.txt'
[parameters.unused]
type = 'file'
"#,
    );
    assert_eq!(run(root.path()).overall, Status::Pass);
    // Root-level parameter files work too.
    let path = root.path().join(".please/checks/parameters/rule.toml");
    write(root.path(), ".commit-scopes", "cli\n");
    fs::write(
        &path,
        fs::read_to_string(&path)
            .unwrap()
            .replace("conventions/scopes.txt", ".commit-scopes"),
    )
    .unwrap();
    let p = config::load(root.path()).unwrap();
    let env = p
        .checks
        .iter()
        .find(|c| c.id == "parameters")
        .unwrap()
        .parameters
        .resolve(&p.root, "parameters")
        .unwrap();
    assert_eq!(
        env["PLEASE_PARAM_SCOPES_FILE"],
        p.root.join(".commit-scopes").to_str().unwrap()
    );
}

#[test]
fn invalid_parameters_are_named_execution_errors_and_block_nonblocking_rules() {
    for (declarations, parameter, reason) in [
        (
            "[parameters.scope]\ntype = 'string'\nrequired = true",
            "scope",
            "missing",
        ),
        (
            "[parameters.scope]\ntype = 'number'",
            "scope",
            "unsupported",
        ),
        (
            "[parameters.scope]\ntype = 'file'\nvalue = 'absent'",
            "scope",
            "exist",
        ),
        (
            "[parameters.scope]\ntype = 'file'\nvalue = '.'",
            "scope",
            "regular file",
        ),
        (
            "[parameters.scope]\ntype = 'file'\nvalue = '/etc/passwd'",
            "scope",
            "relative",
        ),
        (
            "[parameters.scope]\ntype = 'file'\nvalue = '../outside'",
            "scope",
            "parents",
        ),
        (
            "[parameters.scope]\ntype = 'string'\nvalue = 3",
            "scope",
            "string",
        ),
        (
            "[parameters.scope]\ntype = 'string'\nvalue = \"\\u0000\"",
            "scope",
            "NUL",
        ),
        (
            "[parameters.scope]\ntype = 'string'\n[parameters.SCOPE]\ntype = 'string'",
            "scope",
            "collides",
        ),
        (
            "[parameters.'bad-name']\ntype = 'string'",
            "bad-name",
            "ASCII",
        ),
    ] {
        let root = project();
        check(root.path(), "invalid", &["check"], "exit 0");
        parameters(root.path(), "invalid", &format!("\n{declarations}\n"));
        let path = root.path().join(".please/checks/invalid/rule.toml");
        fs::write(
            &path,
            fs::read_to_string(&path)
                .unwrap()
                .replace("enabled = true", "enabled = true\nblocking = false"),
        )
        .unwrap();
        let report = run(root.path());
        assert_eq!(report.overall, Status::Error, "{declarations}");
        let item = &report.results[0];
        assert_eq!(item.status, Status::Error);
        assert!(!item.blocking);
        assert!(
            item.reason.contains("invalid")
                && item.reason.contains(parameter)
                && item.reason.contains(reason),
            "{}",
            item.reason
        );
        assert_eq!(report::read(root.path()).unwrap().overall, Status::Error);
    }
}

#[test]
fn file_parameters_reject_symlink_traversal_even_when_the_target_is_inside() {
    use std::os::unix::fs::symlink;
    for external in [false, true] {
        let root = project();
        let outside = tempfile::tempdir().unwrap();
        write(root.path(), "conventions/scopes", "cli");
        write(outside.path(), "scopes", "cli");
        symlink(
            if external {
                outside.path().to_path_buf()
            } else {
                root.path().join("conventions")
            },
            root.path().join("link"),
        )
        .unwrap();
        check(root.path(), "linked", &["check"], "exit 0");
        parameters(
            root.path(),
            "linked",
            "\n[parameters.scope]\ntype = 'file'\nvalue = 'link/scopes'\n",
        );
        let p = config::load(root.path()).unwrap();
        let item = please::checks::execute(
            &p,
            p.checks.iter().find(|c| c.id == "linked").unwrap(),
            "check",
        );
        assert_eq!(item.status, Status::Error);
        assert!(item.reason.contains("scope") && item.reason.contains("symlink"));
    }
}

#[test]
fn disabled_rules_do_not_resolve_parameters_or_missing_implementations() {
    let root = project();
    check(root.path(), "disabled", &["check"], "exit 0");
    parameters(
        root.path(),
        "disabled",
        "\n[parameters.scope]\ntype = 'file'\nrequired = true\n",
    );
    let path = root.path().join(".please/checks/disabled/rule.toml");
    fs::write(
        &path,
        fs::read_to_string(&path)
            .unwrap()
            .replace("enabled = true", "enabled = false"),
    )
    .unwrap();
    fs::remove_file(root.path().join(".please/checks/disabled/entrypoint")).unwrap();
    let report = run(root.path());
    assert!(report.results.is_empty());
    assert_eq!(report.overall, Status::Pass);
}

#[test]
fn outcomes_distinguish_violations_from_missing_processes_scripts_and_signals() {
    for (script, interpreter, status, reason) in [
        ("exit 0", "sh", Status::Pass, ""),
        ("exit 1", "sh", Status::Fail, "violation"),
        ("echo details >&2; exit 2", "sh", Status::Error, "exit 2"),
        ("exit 7", "sh", Status::Fail, "Unexpected"),
        ("kill -TERM $$", "sh", Status::Error, "signal"),
        ("exit 0", "please-no-such-interpreter", Status::Error, ""),
        ("exit 0", ".", Status::Error, ""),
    ] {
        let root = project();
        check(root.path(), "outcome", &["check"], script);
        let path = root.path().join(".please/checks/outcome/rule.toml");
        fs::write(
            &path,
            fs::read_to_string(&path).unwrap().replace(
                "command = \"sh @rule/entrypoint\"",
                &format!("command = \"{interpreter} @rule/entrypoint\""),
            ),
        )
        .unwrap();
        let report = run(root.path());
        assert_eq!(report.overall, status, "{script} {interpreter}");
        assert!(
            report.results[0].reason.contains(reason),
            "{}",
            report.results[0].reason
        );
        if status == Status::Pass {
            assert!(report.results[0].reason.is_empty());
            assert_eq!(report.results[0].exit_code, Some(0));
        }
        if script.contains("details") {
            assert!(
                report.results[0]
                    .stderr
                    .as_ref()
                    .unwrap()
                    .text
                    .contains("details")
            );
        }
    }
    let root = project();
    check(root.path(), "missing", &["check"], "exit 0");
    fs::remove_file(root.path().join(".please/checks/missing/entrypoint")).unwrap();
    let report = run(root.path());
    assert_eq!(report.results[0].status, Status::Error);
    assert!(report.results[0].reason.contains("entrypoint"));
}

struct Reviews {
    calls: Mutex<Vec<please::parameters::Environment>>,
}
impl Provider for Reviews {
    fn review(&self, request: &Request<'_>) -> anyhow::Result<String> {
        self.calls.lock().unwrap().push(request.parameters.clone());
        assert!(please::provider::prompt(request)?.contains("parameter_environment"));
        Ok(serde_json::json!({"reviews": request.reviews.iter().map(|r| serde_json::json!({
            "id":r.id, "status":"PASS", "reason":"Satisfied", "findings":[{"message":"Verified", "path":"evidence", "line":1}]
        })).collect::<Vec<_>>()}).to_string())
    }
}
fn review(root: &Path, id: &str, parameter: &str) {
    write(root, "evidence", "Evidence\n");
    write(
        root,
        &format!(".please/reviews/{id}/rule.toml"),
        &format!(
            "id = {id:?}\ndescription = 'Review parameters'\nblocking = false\nrun_at = ['check']\n[review]\nprompt = 'prompt.md'\n{parameter}"
        ),
    );
    write(
        root,
        &format!(".please/reviews/{id}/prompt.md"),
        "Review the configured evidence.",
    );
}
#[test]
fn reviews_validate_and_batch_by_resolved_environment_and_nonblocking_fail_continues() {
    let root = project();
    check(root.path(), "advisory", &["check"], "exit 1");
    let path = root.path().join(".please/checks/advisory/rule.toml");
    fs::write(
        &path,
        fs::read_to_string(&path)
            .unwrap()
            .replace("enabled = true", "enabled = true\nblocking = false"),
    )
    .unwrap();
    for (id, value) in [("a", "first"), ("b", "second"), ("c", "first")] {
        review(
            root.path(),
            id,
            &format!(
                "[parameters.label]\ntype = 'string'\nvalue = {value:?}\n[parameters.file]\ntype = 'file'\nvalue = 'evidence'\n"
            ),
        );
    }
    let project = config::load(root.path()).unwrap();
    assert_eq!(
        planner::build(&project, Some("check"), false)
            .unwrap()
            .plan
            .planned_model_calls,
        2
    );
    let provider = Reviews {
        calls: Mutex::new(vec![]),
    };
    let report = please::verify_project_at(
        project,
        &provider,
        VerificationMode::AllRules,
        Some("check"),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.overall, Status::Pass);
    let calls = provider.calls.lock().unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["PLEASE_PARAM_LABEL"], "first");
    assert_eq!(calls[1]["PLEASE_PARAM_LABEL"], "second");
    assert_eq!(
        calls[0]["PLEASE_PARAM_FILE"],
        root.path()
            .canonicalize()
            .unwrap()
            .join("evidence")
            .to_str()
            .unwrap()
    );
    drop(calls);
    review(
        root.path(),
        "invalid",
        "[parameters.required]\ntype = 'file'\nrequired = true\n",
    );
    let report = please::verify_project_at(
        config::load(root.path()).unwrap(),
        &provider,
        VerificationMode::AllRules,
        Some("check"),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.overall, Status::Error);
    let invalid = report.results.iter().find(|i| i.id == "invalid").unwrap();
    assert_eq!(invalid.status, Status::Error);
    assert!(invalid.reason.contains("required"));
    assert!(!invalid.blocking);
    assert!(invalid.configuration_error);
    report::read(root.path()).unwrap();
}

#[test]
fn provider_subprocess_receives_only_its_batch_parameters_and_clears_inherited_values() {
    use std::os::unix::fs::PermissionsExt;
    let root = project();
    review(
        root.path(),
        "a",
        "[parameters.label]\ntype = 'string'\nvalue = 'first'\n[parameters.file]\ntype = 'file'\nvalue = 'evidence'\n[parameters.optional]\ntype = 'string'\n",
    );
    review(
        root.path(),
        "b",
        "[parameters.label]\ntype = 'string'\nvalue = 'second'\n",
    );
    let fake = tempfile::tempdir().unwrap();
    let script = fake.path().join("fake-adapter");
    fs::write(&script, r#"#!/bin/sh
set -eu
test "$PLEASE_EXECUTION_POINT" = check
test "${PLEASE_PARAM_UNDECLARED+x}" != x
test "${PLEASE_PARAM_UNDECLARED+x}" != x
test "${PLEASE_PARAM_OPTIONAL+x}" != x
case "$PLEASE_PARAM_LABEL" in
 first) id=a; test "$PLEASE_PARAM_FILE" = "$PWD/evidence"; test -f "$PLEASE_PARAM_FILE";;
 second) id=b; test "${PLEASE_PARAM_FILE+x}" != x;;
 *) exit 2;;
esac
cat >/dev/null
printf '{"protocol_version":1,"reviews":[{"id":"%s","status":"PASS","reason":"Parameters delivered","findings":[{"message":"Evidence read","path":"evidence","line":1}]}],"usage":{"input_tokens":10,"cached_input_tokens":3,"output_tokens":4}}' "$id"

"#).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    // Use an isolated child rather than mutating this test process's environment.
    let harness = fake.path().join("bin");
    fs::create_dir(&harness).unwrap();
    fs::copy(&script, harness.join("fake-adapter")).unwrap();
    let path = std::env::join_paths(
        std::iter::once(harness).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_please"))
        .args(["run", "--at", "check"])
        .current_dir(root.path())
        .env("PATH", path)
        .env("PLEASE_EXECUTION_POINT", "spoofed")
        .env("PLEASE_PARAM_UNDECLARED", "secret")
        .env("PLEASE_PARAM_OPTIONAL", "secret")
        .env("PLEASE_PARAM_FILE", "wrong")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let saved = report::read(root.path()).unwrap();
    assert_eq!(saved.results.len(), 2);
    assert_eq!(saved.plan.planned_model_calls, 2);
    assert_eq!(saved.provider_usage.unwrap().input_tokens, 20);
    assert_eq!(saved.token_usage, Some(28));
}

#[test]
fn execution_errors_fail_closed_for_explicit_points_ci_and_managed_hooks() {
    let root = project();
    check(
        root.path(),
        "required",
        &["pre-commit", "commit-msg", "pre-push", "check", "ci"],
        "exit 0",
    );
    write(
        root.path(),
        ".please/config.toml",
        "project = 'My_App'\n[execution_points]\ncustom = ['ci']\n",
    );
    parameters(
        root.path(),
        "required",
        "\n[parameters.scopes_file]\ntype = 'file'\nrequired = true\n",
    );
    let path = root.path().join(".please/checks/required/rule.toml");
    fs::write(
        &path,
        fs::read_to_string(&path)
            .unwrap()
            .replace("enabled = true", "enabled = true\nblocking = false"),
    )
    .unwrap();
    let setup = Command::new(env!("CARGO_BIN_EXE_please"))
        .args(["sync", "--yes"])
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(
        setup.status.success(),
        "{}",
        String::from_utf8_lossy(&setup.stderr)
    );
    for point in ["check", "ci", "pre-commit", "commit-msg", "pre-push"] {
        let result = Command::new(env!("CARGO_BIN_EXE_please"))
            .args(["run", "--at", point])
            .current_dir(root.path())
            .output()
            .unwrap();
        assert_eq!(
            result.status.code(),
            Some(2),
            "{point}: {}",
            String::from_utf8_lossy(&result.stderr)
        );
        let report = report::read(root.path()).unwrap();
        assert!(
            report
                .results
                .iter()
                .any(|i| i.id == "required" && i.status == Status::Error && !i.blocking)
        );
        assert!(String::from_utf8_lossy(&result.stdout).contains("ERROR"));
    }
    let paths = std::env::join_paths(
        std::iter::once(
            Path::new(env!("CARGO_BIN_EXE_please"))
                .parent()
                .unwrap()
                .to_path_buf(),
        )
        .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    for event in ["pre-commit", "commit-msg", "pre-push"] {
        let result = Command::new(root.path().join(".git/hooks").join(event))
            .current_dir(root.path())
            .env("PATH", &paths)
            .output()
            .unwrap();
        assert!(!result.status.success(), "{event}");
        let saved = report::read(root.path()).unwrap();
        assert_eq!(saved.overall, Status::Error);
        assert!(
            saved
                .results
                .iter()
                .any(|i| i.id == "required" && i.reason.contains("scopes_file"))
        );
    }
}

#[test]
fn missing_and_disabled_review_prompts_produce_execution_errors_only_when_selected() {
    let root = project();
    review(root.path(), "missing", "");
    fs::remove_file(root.path().join(".please/reviews/missing/prompt.md")).unwrap();
    let report = run(root.path());
    assert_eq!(report.overall, Status::Error);
    assert_eq!(report.results[0].id, "missing");
    assert!(report.results[0].reason.contains("prompt"));
    assert!(report.results[0].configuration_error);
    let path = root.path().join(".please/reviews/missing/rule.toml");
    fs::write(
        &path,
        fs::read_to_string(&path)
            .unwrap()
            .replace("blocking = false", "enabled = false\nblocking = false"),
    )
    .unwrap();
    assert!(run(root.path()).results.is_empty());
}
