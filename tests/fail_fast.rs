//! Invocation-only fail-fast retains complete results and explicit NOT_RUN entries.
#![cfg(unix)]
mod support;
use please::{
    Progress, VerificationMode, config,
    provider::{Provider, Request},
    report::{self, Status},
};
use std::{fs, path::Path, process::Command, sync::Mutex};
use tempfile::TempDir;

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
    support::write(root.path(), "evidence", "Test evidence\n");
    root
}

fn review(root: &Path, id: &str, provider: &str, blocking: bool) {
    let rule = support::review_path(root, id);
    fs::write(rule, format!("id = {id:?}\ndescription = 'Test review'\nblocking = {blocking}\nrun_at = ['check']\n[review]\nprompt = 'prompt.md'\nprovider = {provider:?}\n")).unwrap();
    support::write(
        root,
        &format!(".please/providers/{provider}.toml"),
        "command = 'dummy-adapter'\ntimeout_seconds = 10\n",
    );
}

struct Dummy {
    status: &'static str,
    calls: Mutex<Vec<Vec<String>>>,
}
impl Provider for Dummy {
    fn review(&self, request: &Request<'_>) -> anyhow::Result<String> {
        self.calls
            .lock()
            .unwrap()
            .push(request.reviews.iter().map(|r| r.id.clone()).collect());
        if self.status == "invalid" {
            return Ok("invalid JSON".into());
        }
        Ok(
            serde_json::json!({"reviews": request.reviews.iter().map(|r| serde_json::json!({
            "id": r.id, "status": self.status, "reason": "Fixture verdict",
            "findings": [{"message": "Read fixture", "path": "evidence", "line": 1}]
        })).collect::<Vec<_>>()} )
            .to_string(),
        )
    }
}
fn run(root: &TempDir, provider: &Dummy, fail_fast: bool) -> (report::Report, Vec<String>) {
    let mut project = config::load(root.path()).unwrap();
    project.config.execution.max_parallel = 1.try_into().unwrap();
    let mut started = Vec::new();
    let report = please::verify_project_with_fail_fast(
        project,
        provider,
        VerificationMode::AllRules,
        Some("check"),
        None,
        fail_fast,
        &mut |event| {
            if let Progress::RuleStarted { id, .. } = event {
                started.push(id.to_owned());
            }
        },
    )
    .unwrap();
    (report, started)
}
fn dummy(status: &'static str) -> Dummy {
    Dummy {
        status,
        calls: Mutex::new(Vec::new()),
    }
}

#[test]
fn cli_defaults_to_complete_execution_and_fail_fast_reports_unstarted_rules() {
    let root = project();
    support::check(root.path(), "a", &["check"], "printf failed-output; exit 1");
    support::check(root.path(), "b", &["check"], "printf completed-output");
    review(root.path(), "later", "one", false);
    for fail_fast in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_please"));
        command.args(["run", "--at", "check", "--max-parallel", "1", "--no-color"]);
        if fail_fast {
            command.arg("--fail-fast");
        }
        let output = command.current_dir(root.path()).output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(1),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let saved = report::read(root.path()).unwrap();
        assert_eq!(saved.overall, Status::Fail);
        assert_eq!(
            saved.results[0].stdout.as_ref().unwrap().text,
            "failed-output"
        );
        assert_eq!(
            saved.results[1].status,
            if fail_fast {
                Status::NotRun
            } else {
                Status::Pass
            }
        );
        assert_eq!(saved.results[2].status, Status::NotRun);
        assert!(!saved.results[2].blocking);
        let progress = String::from_utf8_lossy(&output.stderr);
        if fail_fast {
            assert!(saved.results[1].reason.contains("Fail-fast"));
            assert!(saved.results[2].reason.contains("Fail-fast"));
            assert!(saved.results[1].stdout.is_none());
            assert!(saved.results[1].exit_code.is_none());
            assert!(
                !progress
                    .lines()
                    .any(|line| line.contains("RUNNING") && line.contains("b"))
            );
            assert!(progress.contains("NOT_RUN"));
            assert!(String::from_utf8_lossy(&output.stdout).contains("not executed"));
        }
        let status = Command::new(env!("CARGO_BIN_EXE_please"))
            .args(["status", "--details", "--no-color"])
            .current_dir(root.path())
            .output()
            .unwrap();
        assert_eq!(status.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&status.stdout).contains("failed-output"));
    }
}

#[test]
fn check_errors_stop_even_when_nonblocking_but_advisory_failures_continue() {
    for (script, expected) in [("exit 1", Status::Pass), ("exit 2", Status::Error)] {
        let root = project();
        support::check(root.path(), "a", &["check"], script);
        let path = root.path().join(".please/checks/a/rule.toml");
        fs::write(
            &path,
            fs::read_to_string(&path)
                .unwrap()
                .replace("enabled = true", "enabled = true\nblocking = false"),
        )
        .unwrap();
        support::check(root.path(), "b", &["check"], "exit 0");
        let (report, started) = run(&root, &dummy("PASS"), true);
        assert_eq!(report.overall, expected);
        assert_eq!(
            report.results[1].status,
            if expected == Status::Pass {
                Status::Pass
            } else {
                Status::NotRun
            }
        );
        assert_eq!(started.len(), if expected == Status::Pass { 2 } else { 1 });
        if expected == Status::Error {
            fs::write(
                &path,
                fs::read_to_string(&path)
                    .unwrap()
                    .replace("sh @rule/entrypoint", "please-missing-test-executable"),
            )
            .unwrap();
            let (report, _) = run(&root, &dummy("PASS"), true);
            assert_eq!(report.overall, Status::Error);
            assert_eq!(report.results[0].status, Status::Error);
            assert_eq!(report.results[1].status, Status::NotRun);
        }
    }
}

#[test]
fn review_batches_finish_together_and_only_fatal_results_stop_later_calls() {
    for status in ["FAIL", "ERROR", "invalid"] {
        for blocking in [false, true] {
            for fail_fast in [false, true] {
                let root = project();
                review(root.path(), "a", "first", blocking);
                review(root.path(), "b", "first", blocking);
                review(root.path(), "c", "second", false);
                let provider = dummy(status);
                let (report, started) = run(&root, &provider, fail_fast);
                let stop = fail_fast && blocking;
                assert_eq!(
                    provider.calls.lock().unwrap().len(),
                    if stop { 1 } else { 2 }
                );
                assert_eq!(provider.calls.lock().unwrap()[0], ["a", "b"]);
                assert_eq!(
                    started,
                    if stop {
                        vec!["a", "b"]
                    } else {
                        vec!["a", "b", "c"]
                    }
                );
                assert_ne!(report.results[0].status, Status::NotRun);
                assert_ne!(report.results[1].status, Status::NotRun);
                assert_eq!(report.results[2].status == Status::NotRun, stop);
                assert_eq!(
                    report.overall,
                    if blocking {
                        if status == "FAIL" {
                            Status::Fail
                        } else {
                            Status::Error
                        }
                    } else {
                        Status::Pass
                    }
                );
                assert_eq!(report::read(root.path()).unwrap().overall, report.overall);
            }
        }
    }
}

#[test]
fn review_configuration_errors_stop_calls_even_for_advisory_rules() {
    for missing_prompt in [false, true] {
        let root = project();
        review(root.path(), "a", "first", false);
        review(root.path(), "b", "second", false);
        if missing_prompt {
            fs::remove_file(root.path().join(".please/reviews/a/prompt.md")).unwrap();
        } else {
            let path = root.path().join(".please/reviews/a/rule.toml");
            let text = fs::read_to_string(&path).unwrap();
            fs::write(
                path,
                format!("{text}\n[parameters.required_value]\ntype = 'string'\nrequired = true\n"),
            )
            .unwrap();
        }
        let provider = dummy("PASS");
        let (report, started) = run(&root, &provider, true);
        assert_eq!(report.overall, Status::Error);
        assert!(report.results[0].configuration_error);
        assert_eq!(report.results[1].status, Status::NotRun);
        assert!(provider.calls.lock().unwrap().is_empty());
        assert!(started.is_empty());
    }
}

#[test]
fn fail_fast_has_no_persistent_configuration_or_plan_selection_effect() {
    let root = project();
    support::check(root.path(), "a", &["check"], "exit 1");
    support::check(root.path(), "b", &["check"], "exit 0");
    let output = |extra: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_please"))
            .args(["run", "--at", "check", "--plan", "--json"])
            .args(extra)
            .current_dir(root.path())
            .output()
            .unwrap()
    };
    let normal = output(&[]);
    let fast = output(&["--fail-fast"]);
    assert!(normal.status.success());
    assert!(fast.status.success());
    assert_eq!(normal.stdout, fast.stdout);
    let plan: serde_json::Value = serde_json::from_slice(&fast.stdout).unwrap();
    assert_eq!(plan["scheduled"].as_array().unwrap().len(), 2);
    assert!(!root.path().join(".please/reports/last.json").exists());
    for text in [
        "project = 'Example'\nfail_fast = true\n",
        "project = 'Example'\n[execution]\nfail_fast = true\n",
    ] {
        support::write(root.path(), ".please/config.toml", text);
        assert!(config::load(root.path()).is_err());
    }
}
