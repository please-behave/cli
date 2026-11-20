//! DEC-0016/DEC-0018: dependencies constrain checks; aggregate checks gate reviews.
mod support;
use please::{
    adapter::{Adapter, Request},
    config, planner,
    report::{self, Status},
};
use std::{
    fs,
    sync::atomic::{AtomicUsize, Ordering},
};
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
    support::commit_all(root.path());
    support::use_head_as_change_base();
    support::write(root.path(), "evidence", "Evidence\n");
    root
}
fn check(root: &TempDir, id: &str, deps: &[&str], script: &str) {
    support::check(root.path(), id, &["check"], script);
    let path = root.path().join(format!(".please/checks/{id}/rule.toml"));
    let text = fs::read_to_string(&path).unwrap();
    fs::write(path, format!("depends_on = {deps:?}\n{text}")).unwrap();
}
fn review(root: &TempDir) {
    fs::write(support::review_path(root.path(), "review"), "id = 'review'\ndescription = 'Review'\nrun_at = ['check']\n[review]\nprompt = 'prompt.md'\n").unwrap();
}
struct Mock(AtomicUsize);
impl Adapter for Mock {
    fn review(&self, request: &Request<'_>) -> anyhow::Result<String> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(serde_json::json!({"reviews": request.reviews.iter().map(|r| serde_json::json!({"id": r.id, "status": "PASS", "reason": "Inspected", "findings": [{"message": "Evidence", "path": "evidence", "line": 1}]})).collect::<Vec<_>>()}).to_string())
    }
}
fn verify(root: &TempDir, adapter: &Mock, fail_fast: bool) -> report::Report {
    please::verify_project_with_fail_fast(
        config::load(root.path()).unwrap(),
        adapter,
        please::VerificationMode::AllRules,
        Some("check"),
        None,
        fail_fast,
        &mut |_| {},
    )
    .unwrap()
}

#[test]
fn invalid_dependencies_block_planning_before_execution() {
    // REQ-0054, REQ-0055: missing, unselected, self and cyclic dependencies are rejected before any command runs.
    for (deps, expected) in [
        (vec!["missing"], "not selected"),
        (vec!["a"], "self-dependency"),
        (vec!["b"], "cycle"),
        (vec!["review"], "review"),
        (vec!["always-ok"], "not selected"),
    ] {
        let root = project();
        check(&root, "a", &deps, "exit 0");
        check(&root, "b", &["a"], "exit 0");
        review(&root);
        let error = planner::build(&config::load(root.path()).unwrap(), Some("check"), false)
            .err()
            .expect("invalid plan");
        assert!(format!("{error:#}").contains(expected), "{error:#}");
        let output = support::command(env!("CARGO_BIN_EXE_please"))
            .args(["run", "--at", "check", "--plan", "--json"])
            .current_dir(root.path())
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2));
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["status"], "blocked");
        assert!(!root.path().join(".please/reports/last.json").exists());
    }
}

#[test]
fn reviews_cannot_declare_dependencies_even_when_disabled_or_empty() {
    for enabled in [true, false] {
        let root = project();
        review(&root);
        let path = support::review_path(root.path(), "review");
        let text = fs::read_to_string(&path).unwrap();
        fs::write(
            path,
            format!("depends_on = []\nenabled = {enabled}\n{text}"),
        )
        .unwrap();
        let error = config::load(root.path()).unwrap_err();
        assert!(format!("{error:#}").contains("depends_on"));
    }
}

#[cfg(unix)]
#[test]
fn chains_branching_and_resources_obey_pass_dependencies() {
    // REQ-0032, REQ-0034, REQ-0038, REQ-0050: exact-ID dependencies and a shared resource constrain dispatch; plan identity and result order stay canonical.
    let root = project();
    let signals = tempfile::tempdir().unwrap();
    let dir = signals.path().display();
    // Reverse ID order forces readiness, rather than alphabetical dispatch.
    check(&root, "z-build", &[], &format!("touch '{dir}/build'"));
    for id in ["b-test", "c-test"] {
        check(
            &root,
            id,
            &["z-build"],
            &format!(
                "test -f '{dir}/build' || exit 1\nmkdir '{dir}/lock' || exit 1\ntouch '{dir}/{id}'\nrmdir '{dir}/lock'"
            ),
        );
        let path = root.path().join(format!(".please/checks/{id}/rule.toml"));
        let text = fs::read_to_string(&path).unwrap();
        fs::write(path, format!("{text}resource = 'shared'\n")).unwrap();
    }
    check(
        &root,
        "a-smoke",
        &["c-test", "b-test"],
        &format!("test -f '{dir}/b-test' && test -f '{dir}/c-test'"),
    );
    let adapter = Mock(AtomicUsize::new(0));
    let report = verify(&root, &adapter, false);
    assert_eq!(report.overall, Status::Pass);
    assert!(report.results.iter().all(|r| r.status == Status::Pass));
    assert_eq!(report.results[0].id, "a-smoke");
    assert_eq!(report::read(root.path()).unwrap().overall, Status::Pass);
    let before = report.plan.plan_id;
    check(
        &root,
        "a-smoke",
        &["b-test", "c-test", "b-test"],
        &format!("test -f '{dir}/b-test' && test -f '{dir}/c-test'"),
    );
    assert_eq!(
        planner::build(&config::load(root.path()).unwrap(), Some("check"), false)
            .unwrap()
            .plan
            .plan_id,
        before
    );
}

#[cfg(unix)]
#[test]
fn originating_outcome_gates_reviews_and_skips_propagate_without_execution() {
    // REQ-0062: prerequisite skips preserve the originating FAIL or ERROR overall outcome.
    // REQ-0059, REQ-0060, REQ-0061: only PASS satisfies a dependency; dependents become NOT_RUN transitively naming their prerequisite.
    for (exit, fast, expected) in [
        (1, false, Status::Fail),
        (1, true, Status::Fail),
        (2, false, Status::Error),
        (2, true, Status::Error),
    ] {
        let root = project();
        check(&root, "a", &[], &format!("exit {exit}"));
        check(&root, "b", &["a"], "exit 99");
        check(&root, "c", &["b"], "exit 99");
        check(&root, "d-independent", &[], "exit 0");
        review(&root);
        let adapter = Mock(AtomicUsize::new(0));
        let report = verify(&root, &adapter, fast);
        assert_eq!(report.overall, expected, "{exit} {fast}");
        assert_eq!(adapter.0.load(Ordering::SeqCst), 0);
        for (id, dependency) in [("b", "a"), ("c", "b")] {
            let item = report.results.iter().find(|r| r.id == id).unwrap();
            assert_eq!(item.status, Status::NotRun);
            assert_eq!(item.blocked_by.as_deref(), Some(dependency));
            assert!(item.stdout.is_none());
            assert!(item.reason.contains(dependency));
        }
        assert_eq!(report::read(root.path()).unwrap().overall, expected);
    }
}

fn disable(root: &TempDir, id: &str) {
    let path = root.path().join(format!(".please/checks/{id}/rule.toml"));
    let text = fs::read_to_string(&path).unwrap();
    fs::write(path, text.replace("enabled = true", "enabled = false")).unwrap();
}

#[test]
fn plans_record_the_canonical_dependencies_of_each_check() {
    // REQ-0058: scheduled checks carry their sorted, deduplicated dependency IDs.
    let root = project();
    check(&root, "b", &[], "exit 0");
    check(&root, "c", &[], "exit 0");
    check(&root, "a", &["c", "b", "c"], "exit 0");
    let plan = planner::build(&config::load(root.path()).unwrap(), Some("check"), false)
        .unwrap()
        .plan;
    let scheduled = |id: &str| plan.scheduled.iter().find(|r| r.id == id).unwrap();
    assert_eq!(scheduled("a").depends_on, ["b", "c"]);
    assert!(scheduled("b").depends_on.is_empty());
}

#[test]
fn disabled_checks_keep_dependency_syntax_validation_without_resolution() {
    // REQ-0056: a disabled check may name an unresolvable target but not a malformed one.
    let root = project();
    check(&root, "enabled", &[], "exit 0");
    check(&root, "off", &["nowhere"], "exit 0");
    disable(&root, "off");
    let plan = planner::build(&config::load(root.path()).unwrap(), Some("check"), false)
        .unwrap()
        .plan;
    assert_eq!(plan.scheduled.len(), 1);
    for malformed in ["", " padded", "line\nbreak"] {
        check(&root, "off", &[malformed], "exit 0");
        disable(&root, "off");
        assert!(config::load(root.path()).is_err(), "accepted {malformed:?}");
    }
}

#[cfg(unix)]
#[test]
fn waiting_checks_leave_concurrency_slots_to_runnable_ones() {
    // REQ-0036: a check waiting for a resource or a prerequisite occupies no slot.
    let root = project();
    support::write(
        root.path(),
        ".please/config.toml",
        "project = 'Example'\n[execution]\nmax_parallel = 2\n",
    );
    let signals = tempfile::tempdir().unwrap();
    let dir = signals.path().display();
    // The holder only finishes once the runnable check ran, which needs a free slot
    // while the holder runs, the resource waiter and the dependent are still waiting.
    check(
        &root,
        "a-holder",
        &[],
        &format!(
            "i=0\nwhile [ ! -f '{dir}/done' ]; do i=$((i+1)); [ $i -lt 500 ] || exit 2; sleep 0.01; done"
        ),
    );
    check(&root, "b-resource", &[], "exit 0");
    check(&root, "c-dependent", &["a-holder"], "exit 0");
    check(&root, "d-runnable", &[], &format!("touch '{dir}/done'"));
    for id in ["a-holder", "b-resource"] {
        let path = root.path().join(format!(".please/checks/{id}/rule.toml"));
        let text = fs::read_to_string(&path).unwrap();
        fs::write(path, format!("{text}resource = 'shared'\n")).unwrap();
    }
    let report = verify(&root, &Mock(AtomicUsize::new(0)), false);
    assert_eq!(report.overall, Status::Pass, "{:?}", report.results);
    assert!(report.results.iter().all(|r| r.status == Status::Pass));
}
