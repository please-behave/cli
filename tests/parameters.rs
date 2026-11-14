#![cfg(unix)]
mod support;
use please::{
    VerificationMode, config, planner,
    provider::{Provider, Request},
    report::{self, Status},
};
use std::{fs, path::Path, sync::Mutex};
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
        support::command("git")
            .args(["init", "--quiet"])
            .current_dir(root.path())
            .status()
            .unwrap()
            .success()
    );
    please::init::init(root.path()).unwrap();
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
    support::check_args(
        root.path(),
        "parameters",
        &["check"],
        "@param/label @param/scopes_file @rule/resource.txt",
        r#"
test "$PWD" = "$(git rev-parse --show-toplevel)" || exit 1
test "$1" = 'arbitrary value $with spaces' || exit 1
test "$2" = "$PWD/conventions/scopes.txt" || exit 1
test "$(cat "$2")" = cli || exit 1
test "$(cat "$3")" = bundled || exit 1
test -z "$(env | grep '^PLEASE_' | grep -v '^PLEASE_ACTIVE_HOOK=' || true)" || exit 1
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
        env["scopes_file"],
        p.root.join(".commit-scopes").to_str().unwrap()
    );
}

#[test]
fn invalid_parameters_are_named_execution_errors() {
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
            "[parameters.'bad-name']\ntype = 'string'",
            "bad-name",
            "ASCII",
        ),
    ] {
        let root = project();
        check(root.path(), "invalid", &["check"], "exit 0");
        parameters(root.path(), "invalid", &format!("\n{declarations}\n"));
        let report = run(root.path());
        assert_eq!(report.overall, Status::Error, "{declarations}");
        let item = &report.results[0];
        assert_eq!(item.status, Status::Error);
        assert!(item.blocking);
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
        let item = please::checks::execute(&p, p.checks.iter().find(|c| c.id == "linked").unwrap());
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
        ("echo details >&2; exit 2", "sh", Status::Error, "status 2"),
        ("exit 7", "sh", Status::Error, "status 7"),
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
        assert!(please::provider::prompt(request)?.contains("rule_parameters"));
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
fn reviews_validate_and_call_by_resolved_environment() {
    let root = project();
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
        3
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
    assert_eq!(calls.len(), 3);
    assert_eq!(calls[0]["label"], "first");
    assert_eq!(calls[1]["label"], "second");
    assert_eq!(calls[2]["label"], "first");
    assert_eq!(
        calls[0]["file"],
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
fn provider_subprocess_receives_only_its_call_parameters_and_clears_inherited_values() {
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
    fs::write(&script, r#"#!/usr/bin/env python3
import json, os, sys
assert not [name for name in os.environ if name.startswith("PLEASE_")]
request = json.load(sys.stdin)
assert request["execution_point"] == "check"
parameters = request["rule_parameters"]
assert "undeclared" not in parameters and "optional" not in parameters
if parameters["label"] == "first":
    identifier = "a"
    assert parameters["file"] == os.getcwd() + "/evidence" and os.path.isfile(parameters["file"])
elif parameters["label"] == "second":
    identifier = "b"
    assert "file" not in parameters
else:
    sys.exit(2)
print(json.dumps({"protocol_version": 1, "reviews": [{"id": identifier, "status": "PASS", "reason": "Parameters delivered", "findings": [{"message": "Evidence read", "path": "evidence", "line": 1}]}], "usage": {"input_tokens": 10, "cached_input_tokens": 3, "output_tokens": 4}}))
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
    let output = support::command(env!("CARGO_BIN_EXE_please"))
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
    assert_eq!(saved.provider_usage.unwrap().input_tokens, Some(20));
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
    let setup = support::command(env!("CARGO_BIN_EXE_please"))
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
        let result = support::command(env!("CARGO_BIN_EXE_please"))
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
                .any(|i| i.id == "required" && i.status == Status::Error && i.blocking)
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
        let result = support::command(root.path().join(".git/hooks").join(event))
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
