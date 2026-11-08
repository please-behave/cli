#![cfg(unix)]
mod support;
use please::{
    VerificationMode, config, planner,
    provider::Configured,
    report::{self, Status},
};
use std::{fs, os::unix::fs::PermissionsExt, path::Path, process::Command};
use support::write;

fn fixture() -> tempfile::TempDir {
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
    fs::remove_dir_all(root.path().join(".please/checks")).unwrap();
    fs::remove_dir_all(root.path().join(".please/reviews")).unwrap();
    write(root.path(), "evidence", "Evidence\n");
    root
}
fn review(root: &Path, id: &str, provider: &str, point: &str) {
    write(
        root,
        &format!(".please/reviews/{id}/rule.toml"),
        &format!(
            "id = {id:?}\ndescription = 'Inspect evidence'\nrun_at = [{point:?}]\n[review]\nprovider = {provider:?}\nprompt = 'prompt.md'\n"
        ),
    );
    write(
        root,
        &format!(".please/reviews/{id}/prompt.md"),
        "Unique review instructions\n",
    );
}
fn provider(root: &Path, id: &str, mode: &str, requirements: &str) {
    write(
        root,
        &format!(".please/providers/{id}.toml"),
        &format!(
            "provider = 'cli'\ncommand = './tool'\nargs = [{mode:?}, '$HOME', '$(false)', '|', '']\nmodel = {id:?}\ntimeout_seconds = 1\n{requirements}"
        ),
    );
}
fn tool(root: &Path) {
    write(
        root,
        "tool",
        r#"#!/usr/bin/env python3
import json, os, sys, time
mode = sys.argv[1]
assert sys.argv[2:] == ['$HOME', '$(false)', '|', '']
assert os.environ['PLEASE_EXECUTION_POINT'] == 'check'
if mode == 'unread':
    time.sleep(30)
prompt = sys.stdin.read()
assert 'Unique review instructions' in prompt
assert 'response schema' in prompt
assert 'positive line number' in prompt
assert 'manifest paths' in prompt
line = next(line for line in prompt.splitlines() if line.startswith('Configured reviews (JSON data): '))
reviews = json.loads(line.split(': ', 1)[1])
assert len(reviews) == 1
assert os.environ['PLEASE_MODEL'] in ['first', 'second']
if mode == 'malformed':
    print('{"private-prompt-secret": "never expose"}')
    sys.exit(0)
if mode == 'nonzero':
    print('private-prompt-secret', file=sys.stderr)
    sys.exit(7)
if mode == 'timeout':
    time.sleep(30)
if mode == 'oversized':
    print('x' * 70000)
    sys.exit(0)
status = 'FAIL' if mode == 'fail' else 'ERROR' if mode == 'error' else 'PASS'
findings = [] if mode == 'ungrounded' else [{'message':'Evidence inspected','path':'evidence','line':1}]
print(json.dumps({'reviews':[{'id': reviews[0]['id'], 'status':status, 'reason':'Evidence inspected', 'findings':findings}]}))
"#,
    );
    fs::set_permissions(root.join("tool"), fs::Permissions::from_mode(0o755)).unwrap();
}
fn execute(root: &Path) -> report::Report {
    please::verify_project_at(
        config::load(root).unwrap(),
        &Configured::default(),
        VerificationMode::AllRules,
        Some("check"),
        &mut |_| {},
    )
    .unwrap()
}
#[test]
fn stdin_results_and_provider_isolation() {
    let root = fixture();
    tool(root.path());
    provider(
        root.path(),
        "first",
        "ok",
        "[requirements]\nenv = ['PLEASE_MODEL']\nexecutables = ['./tool', 'python3']\n",
    );
    provider(root.path(), "second", "ok", "");
    for (id, reference) in [("a", "first"), ("b", "first"), ("c", "second")] {
        review(root.path(), id, reference, "check");
    }
    let project = config::load(root.path()).unwrap();
    assert_eq!(
        planner::build(&project, Some("check"), false)
            .unwrap()
            .plan
            .planned_model_calls,
        3
    );
    let result = execute(root.path());
    assert_eq!(result.overall, Status::Pass, "{:?}", result.results);
    assert_eq!(result.results.len(), 3);
    assert!(result.provider_usage.is_none());
    assert_eq!(report::read(root.path()).unwrap().overall, Status::Pass);
}
#[test]
fn process_and_response_errors_never_pass_or_expose_raw_output() {
    for mode in [
        "malformed",
        "nonzero",
        "timeout",
        "unread",
        "oversized",
        "ungrounded",
    ] {
        let root = fixture();
        tool(root.path());
        provider(root.path(), "first", mode, "");
        review(root.path(), "a", "first", "check");
        if mode == "unread" {
            // Larger than the pipe capacity: stdin must remain inside the deadline.
            write(
                root.path(),
                ".please/reviews/a/prompt.md",
                &format!("Unique review instructions\n{}", "\\\n".repeat(14000)),
            );
        }
        let start = std::time::Instant::now();
        let result = execute(root.path());
        assert_eq!(
            result.overall,
            Status::Error,
            "{mode}: {:?}",
            result.results
        );
        assert!(start.elapsed().as_secs() < 10, "{mode}");
        assert!(
            !fs::read_to_string(root.path().join(".please/reports/last.json"))
                .unwrap()
                .contains("private-prompt-secret")
        );
    }
}
#[test]
fn runtime_requirements_are_deferred_until_selected_after_checks() {
    let root = fixture();
    tool(root.path());
    provider(
        root.path(),
        "first",
        "ok",
        "[requirements]\nenv = ['PLEASE_TEST_MISSING_CREDENTIAL_71F52']\n",
    );
    review(root.path(), "a", "first", "pre-push");
    // Unused and inapplicable providers do not require credentials, including plans.
    let project = config::load(root.path()).unwrap();
    assert!(
        planner::build(&project, Some("pre-push"), false)
            .unwrap()
            .plan
            .executable()
    );
    assert_eq!(execute(root.path()).overall, Status::Pass);
    review(root.path(), "a", "first", "check");
    support::check(root.path(), "gate", &["check"], "exit 1\n");
    let gated = execute(root.path());
    assert_eq!(gated.overall, Status::Fail);
    assert!(!format!("{:?}", gated.results).contains("required environment variable"));
    fs::remove_dir_all(root.path().join(".please/checks")).unwrap();
    let result = execute(root.path());
    assert_eq!(result.overall, Status::Error);
    assert!(format!("{:?}", result.results).contains("required environment variable"));
    provider(
        root.path(),
        "first",
        "ok",
        "[requirements]\nexecutables = ['please-missing-test-tool-71f52']\n",
    );
    assert!(format!("{:?}", execute(root.path()).results).contains("required executable"));
    provider(root.path(), "first", "ok", "");
    fs::remove_file(root.path().join("tool")).unwrap();
    assert_eq!(execute(root.path()).overall, Status::Error);
}
#[test]
fn cli_configuration_is_strict_and_legacy_provider_remains_compatible() {
    let root = fixture();
    provider(root.path(), "first", "ok", "");
    review(root.path(), "a", "first", "check");
    let path = root.path().join(".please/providers/first.toml");
    let valid = fs::read_to_string(&path).unwrap();
    for invalid in [
        valid.replace("command = './tool'", "command = ''"),
        valid.replace("provider = 'cli'", "provider = 'http'"),
        format!("{valid}\n[requirements]\nenv = ['BAD=NAME']\n"),
        format!("{valid}\n[review_profile]\nreasoning_effort = 'low'\n"),
        format!("{valid}\napi_key = 'forbidden'\n"),
    ] {
        fs::write(&path, invalid).unwrap();
        assert!(config::load(root.path()).is_err());
    }
    fs::write(
        &path,
        valid.replace("timeout_seconds = 1", "timeout_seconds = 0"),
    )
    .unwrap();
    assert_eq!(
        config::load(root.path())
            .unwrap()
            .review_provider(&config::load(root.path()).unwrap().reviews[0])
            .timeout_seconds,
        0
    );
    fs::write(path, valid).unwrap();
    let project = config::load(root.path()).unwrap();
    assert_eq!(project.provider.provider, "codex-cli");
    assert_eq!(project.review_provider(&project.reviews[0]).provider, "cli");
    // Legacy default configuration can also select generic CLI.
    fs::write(
        root.path().join(".please/provider.toml"),
        "provider = 'cli'\ncommand = 'missing-but-valid'\ntimeout_seconds = 5\n",
    )
    .unwrap();
    assert_eq!(config::load(root.path()).unwrap().provider.provider, "cli");
}
#[test]
fn completed_fail_and_error_respect_review_blocking() {
    let root = fixture();
    tool(root.path());
    provider(root.path(), "first", "fail", "");
    review(root.path(), "a", "first", "check");
    let path = root.path().join(".please/reviews/a/rule.toml");
    fs::write(
        &path,
        fs::read_to_string(&path)
            .unwrap()
            .replace("description =", "blocking = false\ndescription ="),
    )
    .unwrap();
    assert_eq!(execute(root.path()).overall, Status::Pass);
    for mode in ["error", "nonzero", "malformed", "timeout"] {
        provider(root.path(), "first", mode, "");
        let result = execute(root.path());
        assert_eq!(result.overall, Status::Pass);
        assert_eq!(
            result
                .results
                .iter()
                .find(|r| r.kind == "review")
                .unwrap()
                .status,
            Status::Error
        );
    }
    fs::write(
        &path,
        fs::read_to_string(&path)
            .unwrap()
            .replace("blocking = false", "blocking = true"),
    )
    .unwrap();
    assert_eq!(execute(root.path()).overall, Status::Error);
}
