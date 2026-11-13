//! Invocation-only fail-fast retains complete results and explicit NOT_RUN entries.
#![cfg(unix)]
mod support;
use please::{
    Progress, VerificationMode, config,
    provider::{Provider, Request},
    report::{self, Status},
};
use std::{fs, path::Path, sync::Mutex};
use tempfile::TempDir;

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
        let mut command = support::command(env!("CARGO_BIN_EXE_please"));
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
        let status = support::command(env!("CARGO_BIN_EXE_please"))
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
        support::command(env!("CARGO_BIN_EXE_please"))
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

#[test]
fn termination_configuration_is_optional_strict_and_does_not_enable_fail_fast() {
    let root = project();
    let defaults = config::load(root.path()).unwrap().config.execution;
    assert!(!defaults.fail_fast_terminate_running_instances);
    assert_eq!(defaults.fail_fast_termination_grace_seconds, 5);
    for value in ["0", "3601", "-1", "1.5", "'5'", "true"] {
        support::write(
            root.path(),
            ".please/config.toml",
            &format!(
                "project = 'Example'\n[execution]\nfail_fast_termination_grace_seconds = {value}\n"
            ),
        );
        let error = config::load(root.path()).unwrap_err();
        assert!(
            format!("{error:#}").contains(".please/config.toml"),
            "{value}: {error:#}"
        );
    }
    for value in ["'true'", "1"] {
        support::write(
            root.path(),
            ".please/config.toml",
            &format!(
                "project = 'Example'\n[execution]\nfail_fast_terminate_running_instances = {value}\n"
            ),
        );
        assert!(config::load(root.path()).is_err());
    }
    support::check(root.path(), "a", &["check"], "exit 1");
    support::check(root.path(), "b", &["check"], "printf completed-output");
    for grace in [1, 3600] {
        support::write(
            root.path(),
            ".please/config.toml",
            &format!(
                "project = 'Example'\n[execution]\nmax_parallel = 1\nfail_fast_terminate_running_instances = true\nfail_fast_termination_grace_seconds = {grace}\n"
            ),
        );
        let provider = dummy("PASS");
        let (saved, started) = run(&root, &provider, false);
        assert_eq!(saved.overall, Status::Fail);
        assert_eq!(saved.results[1].status, Status::Pass);
        assert_eq!(started, ["a", "b"]);
        let mut project = config::load(root.path()).unwrap();
        let plan = please::planner::build(&project, Some("check"), false)
            .unwrap()
            .plan;
        project
            .config
            .execution
            .fail_fast_terminate_running_instances = false;
        project.config.execution.fail_fast_termination_grace_seconds = 5;
        assert_eq!(
            plan.plan_id,
            please::planner::build(&project, Some("check"), false)
                .unwrap()
                .plan
                .plan_id
        );
    }
}

// State files live under .git so coordination does not change the checked snapshot.
fn termination_fixture(
    root: &TempDir,
    stubborn_child: bool,
    stubborn_parent: bool,
    failure_exit: i32,
) {
    support::write(
        root.path(),
        ".please/config.toml",
        "project = 'Example'\n[execution]\nmax_parallel = 2\nfail_fast_terminate_running_instances = true\nfail_fast_termination_grace_seconds = 1\n",
    );
    support::check(
        root.path(),
        "a",
        &["check"],
        &format!(
            "i=0\nwhile [ ! -f .git/parent-ready ] || [ ! -f .git/child-ready ]; do\n i=$((i+1)); [ $i -lt 500 ] || exit 2; sleep 0.01\ndone\nprintf triggering-output\nexit {failure_exit}\n"
        ),
    );
    support::check(
        root.path(),
        "b",
        &["check"],
        "exec python3 .please/checks/b/process.py",
    );
    support::write(
        root.path(),
        ".please/checks/b/process.py",
        &format!(
            r#"
import os, signal, sys
from pathlib import Path
stubborn = {stubborn}
stubborn_parent = {stubborn_parent}
def child_term(signum, frame):
    Path('.git/child-terminated').write_text('SIGTERM')
    print('child terminated gracefully', flush=True)
    sys.exit(0)
child = os.fork()
if child == 0:
    signal.signal(signal.SIGTERM, signal.SIG_IGN if stubborn else child_term)
    Path('.git/child-pid').write_text(str(os.getpid()))
    Path('.git/child-ready').touch()
    while True:
        signal.pause()
def parent_term(signum, frame):
    print('parent received SIGTERM', flush=True)
    print('partial stderr', file=sys.stderr, flush=True)
    if stubborn_parent:
        return
    if not stubborn:
        os.waitpid(child, 0)
    sys.exit(0)
signal.signal(signal.SIGTERM, parent_term)
print('partial stdout', flush=True)
Path('.git/parent-pid').write_text(str(os.getpid()))
Path('.git/parent-ready').touch()
while True:
    signal.pause()
"#,
            stubborn = if stubborn_child { "True" } else { "False" },
            stubborn_parent = if stubborn_parent { "True" } else { "False" }
        ),
    );
    support::check(root.path(), "c", &["check"], "printf should-not-start");
    for id in ["b", "c"] {
        let path = root.path().join(format!(".please/checks/{id}/rule.toml"));
        let text = fs::read_to_string(&path)
            .unwrap()
            .replace("[check]", "[check]\nresource = 'shared'");
        fs::write(path, text).unwrap();
    }
}

#[test]
fn fail_fast_terminates_groups_gracefully_and_escalates_after_the_leader_exits() {
    for (stubborn, stubborn_parent) in [(false, false), (true, false), (true, true)] {
        for failure_exit in [1, 2] {
            let root = project();
            termination_fixture(&root, stubborn, stubborn_parent, failure_exit);
            let output = support::command(env!("CARGO_BIN_EXE_please"))
                .args(["run", "--at", "check", "--fail-fast", "--no-color"])
                .current_dir(root.path())
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(failure_exit),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            let saved = report::read(root.path()).unwrap();
            assert_eq!(
                saved
                    .results
                    .iter()
                    .map(|i| i.id.as_str())
                    .collect::<Vec<_>>(),
                ["a", "b", "c"]
            );
            let cancelled = &saved.results[1];
            assert_eq!(cancelled.status, Status::Cancelled);
            assert!(cancelled.blocking);
            assert_eq!(
                cancelled.exit_code,
                if stubborn_parent { None } else { Some(0) }
            ); // Even a graceful exit is cancellation.
            assert!(cancelled.reason.contains("Fail-fast"));
            let stdout = &cancelled.stdout.as_ref().unwrap().text;
            assert!(stdout.contains("partial stdout"));
            assert!(stdout.contains("parent received SIGTERM"));
            assert!(
                cancelled
                    .stderr
                    .as_ref()
                    .unwrap()
                    .text
                    .contains("partial stderr")
            );
            assert_eq!(saved.results[2].status, Status::NotRun);
            assert!(saved.results[2].stdout.is_none());
            assert_eq!(
                saved.overall,
                if failure_exit == 1 {
                    Status::Fail
                } else {
                    Status::Error
                }
            );
            if stubborn {
                assert!(cancelled.duration_ms >= 1000, "grace period was skipped");
                assert!(
                    cancelled.duration_ms < 9000,
                    "check timeout used instead of cancellation"
                );
            } else {
                assert!(root.path().join(".git/child-terminated").exists());
                assert!(stdout.contains("child terminated gracefully"));
            }
            // A killed orphan may briefly remain a zombie until the OS reaps it.
            // It must never remain a live descendant after Please exits.
            for file in [".git/child-pid", ".git/parent-pid"] {
                let pid = fs::read_to_string(root.path().join(file)).unwrap();
                let state = support::command("ps")
                    .args(["-o", "stat=", "-p", pid.trim()])
                    .output()
                    .unwrap();
                let state = String::from_utf8_lossy(&state.stdout);
                assert!(
                    state.trim().is_empty() || state.trim().starts_with('Z'),
                    "descendant still running: {state}"
                );
            }
            let status = support::command(env!("CARGO_BIN_EXE_please"))
                .args(["status", "--details", "--no-color"])
                .current_dir(root.path())
                .output()
                .unwrap();
            assert_eq!(status.status.code(), Some(failure_exit));
            let text = String::from_utf8_lossy(&status.stdout);
            assert!(text.contains("CANCELLED"));
            assert!(text.contains("partial stdout"));
            assert!(String::from_utf8_lossy(&output.stdout).contains("1 cancelled"));
            assert!(String::from_utf8_lossy(&output.stderr).contains("CANCELLED"));
        }
    }
}

#[test]
fn cancellation_cannot_pass_or_override_the_triggering_failure() {
    use please::report::Item;
    let cancelled = Item::simple("cancelled", "check", Status::Cancelled, "Fail-fast".into());
    assert_eq!(report::overall(&[cancelled]), Status::Error);
    for trigger in [Status::Fail, Status::Error] {
        let mut cancelled =
            Item::simple("cancelled", "check", Status::Cancelled, "Fail-fast".into());
        cancelled.blocking = false;
        assert_eq!(
            report::overall(&[
                Item::simple("trigger", "check", trigger, "triggered".into()),
                cancelled
            ]),
            trigger
        );
    }
}

#[test]
fn running_checks_finish_by_default_and_without_invocation_fail_fast() {
    for (setting, fail_fast) in [(None, true), (Some(false), true), (Some(true), false)] {
        let root = project();
        let mut text = "project = 'Example'\n[execution]\nmax_parallel = 2\n".to_string();
        if let Some(setting) = setting {
            text.push_str(&format!(
                "fail_fast_terminate_running_instances = {setting}\n"
            ));
        }
        support::write(root.path(), ".please/config.toml", &text);
        support::check(
            root.path(),
            "a",
            &["check"],
            "i=0\nwhile [ ! -f .git/running ]; do i=$((i+1)); [ $i -lt 500 ] || exit 2; sleep 0.01; done\nexit 1",
        );
        support::check(
            root.path(),
            "b",
            &["check"],
            "touch .git/running\ni=0\nwhile [ ! -f .git/release ]; do i=$((i+1)); [ $i -lt 500 ] || exit 2; sleep 0.01; done\nprintf finished-normally",
        );
        let provider = dummy("PASS");
        let result = please::verify_project_with_fail_fast(
            config::load(root.path()).unwrap(),
            &provider,
            VerificationMode::AllRules,
            Some("check"),
            None,
            fail_fast,
            &mut |event| {
                if let Progress::RuleFinished { item, .. } = event {
                    if item.id == "a" {
                        fs::write(
                            root.path().join(".git/release"),
                            "release after known failure",
                        )
                        .unwrap();
                    }
                }
            },
        )
        .unwrap();
        assert_eq!(result.overall, Status::Fail);
        assert_eq!(result.results[1].status, Status::Pass);
        assert_eq!(
            result.results[1].stdout.as_ref().unwrap().text,
            "finished-normally"
        );
    }
}
