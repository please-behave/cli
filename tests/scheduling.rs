//! History changes dispatch, never plan identity or verification results.
mod support;
use please::{
    Progress, config, planner,
    provider::{Provider, Request},
    report::Status,
};
use std::fs;

struct NoReviews;
impl Provider for NoReviews {
    fn review(&self, _: &Request<'_>) -> anyhow::Result<String> {
        panic!("no reviews selected")
    }
}
fn project() -> tempfile::TempDir {
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
    support::commit_all(root.path());
    support::use_head_as_change_base();
    support::check(root.path(), "a", &["check"], "exit 0");
    support::check(root.path(), "b", &["check"], "exit 0");
    let path = root.path().join(".please/config.toml");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(path, text.replace("max_parallel = 4", "max_parallel = 1")).unwrap();
    root
}
fn run(root: &std::path::Path) -> (please::report::Report, Vec<String>) {
    let mut starts = Vec::new();
    let report = please::verify_project_at(
        config::load(root).unwrap(),
        &NoReviews,
        please::VerificationMode::ChecksOnly,
        Some("check"),
        &mut |event| {
            if let Progress::RuleStarted { id, .. } = event {
                starts.push(id.to_owned());
            }
        },
    )
    .unwrap();
    (report, starts)
}
#[test]
fn history_persists_and_changes_dispatch_without_changing_plan() {
    // REQ-0032, REQ-0038, REQ-0044, REQ-0045: estimates reorder dispatch inside the worktree-local cache but not the plan or the result order.
    let root = project();
    let cache = root.path().join(".git/please-scheduling.json");
    let plan = planner::build(&config::load(root.path()).unwrap(), Some("check"), true)
        .unwrap()
        .plan;
    assert!(!cache.exists());
    let (report, starts) = run(root.path());
    assert_eq!(report.overall, Status::Pass);
    assert_eq!(starts, ["a", "b"]);
    let history: serde_json::Value = serde_json::from_slice(&fs::read(&cache).unwrap()).unwrap();
    assert!(history["estimates"]["check"]["a"].as_u64().unwrap() >= 1);
    fs::write(&cache, r#"{"estimates":{"check":{"a":1,"b":10000}}}"#).unwrap();
    let before = fs::read(&cache).unwrap();
    let preview = planner::build(&config::load(root.path()).unwrap(), Some("check"), true)
        .unwrap()
        .plan;
    assert_eq!(preview.plan_id, plan.plan_id);
    assert_eq!(fs::read(&cache).unwrap(), before);
    let (report, starts) = run(root.path());
    assert_eq!(starts, ["b", "a"]);
    assert_eq!(report.overall, Status::Pass);
    assert_eq!(report.plan.plan_id, plan.plan_id);
    assert_eq!(
        report
            .results
            .iter()
            .map(|i| i.id.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    let history: serde_json::Value = serde_json::from_slice(&fs::read(&cache).unwrap()).unwrap();
    let sample = report
        .results
        .iter()
        .find(|i| i.id == "b")
        .unwrap()
        .duration_ms
        .max(1);
    assert_eq!(history["estimates"]["check"]["b"], (10000 + sample) / 2);
}
#[test]
fn corrupt_and_unwritable_cache_never_change_verification() {
    // REQ-0048: a corrupt or unwritable duration cache never changes the outcome.
    let root = project();
    let cache = root.path().join(".git/please-scheduling.json");
    for contents in ["invalid JSON", r#"{"estimates":{"check":{"a":-1}}}"#] {
        fs::write(&cache, contents).unwrap();
        let (report, starts) = run(root.path());
        assert_eq!(report.overall, Status::Pass);
        assert_eq!(starts, ["a", "b"]);
    }
    fs::remove_file(&cache).unwrap();
    fs::create_dir(&cache).unwrap();
    let (report, starts) = run(root.path());
    assert_eq!(report.overall, Status::Pass);
    assert_eq!(starts, ["a", "b"]);
    assert!(cache.is_dir());
}

#[test]
fn review_history_orders_calls_and_preserves_result_order() {
    struct Mock;
    impl Provider for Mock {
        fn review(&self, request: &Request<'_>) -> anyhow::Result<String> {
            assert_eq!(request.reviews.len(), 1);
            Ok(serde_json::json!({"reviews": request.reviews.iter().map(|r| serde_json::json!({"id": r.id, "status": "PASS", "reason": "Inspected", "findings": [{"message": "Evidence", "path": "evidence", "line": 1}]})).collect::<Vec<_>>()}).to_string())
        }
    }
    let root = project();
    support::configure_adapter(root.path());
    support::write(root.path(), "evidence", "Evidence\n");
    for (id, diff) in [("r-a", false), ("r-b", true), ("r-c", true)] {
        fs::write(support::review_path(root.path(), id), format!("id = '{id}'\ndescription = 'Review'\nrun_at = ['check']\n[review]\nprompt = 'prompt.md'\ninclude_change_set = {diff}\n")).unwrap();
    }
    let cache = root.path().join(".git/please-scheduling.json");
    let execute = || {
        let mut starts = Vec::new();
        let mut running = std::collections::BTreeSet::new();
        let mut renderer = please::terminal::ProgressRenderer::new(false, false, 80, 24);
        let mut output = String::new();
        let report = please::verify_project_at(
            config::load(root.path()).unwrap(),
            &Mock,
            please::VerificationMode::AllRules,
            Some("check"),
            &mut |event| {
                output.push_str(&renderer.render(&event));
                match event {
                    Progress::RuleStarted {
                        kind: "review", id, ..
                    } => {
                        assert!(running.is_empty(), "previous call still shown as running");
                        assert!(running.insert(id.to_owned()));
                        starts.push(id.to_owned());
                    }
                    Progress::RuleFinished { item, .. } if item.kind == "review" => {
                        assert!(running.remove(&item.id));
                    }
                    _ => (),
                }
            },
        )
        .unwrap();
        assert_eq!(report.overall, Status::Pass);
        assert_eq!(
            report
                .results
                .iter()
                .filter(|i| i.kind == "review")
                .map(|i| i.id.as_str())
                .collect::<Vec<_>>(),
            ["r-a", "r-b", "r-c"]
        );
        assert!(running.is_empty());
        for (index, id) in starts.iter().enumerate() {
            let number = index + 3;
            assert!(
                output.contains(&format!("[{number}/5] RUNNING · {id}")),
                "{output}"
            );
            assert!(
                output.contains(&format!("[{number}/5] PASS    · {id}")),
                "{output}"
            );
        }
        assert!(output.contains("5/5 completed · 0 running"));
        starts
    };
    assert_eq!(execute(), ["r-a", "r-b", "r-c"]);
    fs::write(
        &cache,
        r#"{"estimates":{"review":{"r-a":1,"r-b":10000,"r-c":1}}}"#,
    )
    .unwrap();
    assert_eq!(execute(), ["r-b", "r-a", "r-c"]);
    let history: serde_json::Value = serde_json::from_slice(&fs::read(&cache).unwrap()).unwrap();
    for id in ["r-a", "r-b", "r-c"] {
        assert!(history["estimates"]["review"][id].as_u64().is_some());
    }
}

#[test]
fn linked_worktree_history_stays_outside_governed_files_and_is_isolated() {
    // REQ-0045: the duration cache lives in the worktree-local Git directory.
    let root = project();
    assert!(
        support::command("git")
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "--quiet",
                "--allow-empty",
                "-m",
                "Initial"
            ])
            .current_dir(root.path())
            .status()
            .unwrap()
            .success()
    );
    let linked = tempfile::tempdir().unwrap();
    assert!(
        support::command("git")
            .args(["worktree", "add", "--quiet", "--detach"])
            .arg(linked.path())
            .current_dir(root.path())
            .status()
            .unwrap()
            .success()
    );
    support::check(linked.path(), "a", &["check"], "exit 0");
    let (report, starts) = run(linked.path());
    assert_eq!(report.overall, Status::Pass);
    assert_eq!(starts, ["a"]);
    assert!(!root.path().join(".git/please-scheduling.json").exists());
    assert!(!linked.path().join("please-scheduling.json").exists());
    let output = support::command("git")
        .args([
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "please-scheduling.json",
        ])
        .current_dir(linked.path())
        .output()
        .unwrap();
    assert!(std::path::Path::new(String::from_utf8(output.stdout).unwrap().trim()).is_file());
}

#[test]
fn remaining_dependency_path_orders_dispatch_without_overriding_eligibility() {
    // REQ-0044, REQ-0046: the longest remaining path decides the order among runnable
    // checks, and an estimate never lets a check start before its prerequisite passed.
    let root = project();
    support::check(root.path(), "c", &["check"], "exit 0");
    support::check(root.path(), "d", &["check"], "exit 0");
    let path = root.path().join(".please/checks/d/rule.toml");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("depends_on = ['c']\n{text}")).unwrap();
    fs::write(
        root.path().join(".git/please-scheduling.json"),
        r#"{"estimates":{"check":{"a":50,"b":10,"c":1,"d":100}}}"#,
    )
    .unwrap();
    let (report, starts) = run(root.path());
    assert_eq!(report.overall, Status::Pass);
    // c leads with a path of 1 + 100; d (100) only becomes runnable after c passed.
    assert_eq!(starts, ["c", "d", "a", "b"]);
    assert_eq!(
        report
            .results
            .iter()
            .map(|i| i.id.as_str())
            .collect::<Vec<_>>(),
        ["a", "b", "c", "d"]
    );
}

#[test]
fn scheduling_priorities_and_estimates_are_not_configurable() {
    // REQ-0049: neither rules nor the project configuration accept scheduling settings.
    for (file, setting) in [
        (".please/checks/a/rule.toml", "priority = 5\n"),
        (".please/checks/a/rule.toml", "estimated_duration_ms = 5\n"),
        (".please/config.toml", "priority = 5\n"),
        (".please/config.toml", "estimated_duration_ms = 5\n"),
    ] {
        let root = project();
        let path = root.path().join(file);
        let text = fs::read_to_string(&path).unwrap();
        let text = if file.ends_with("config.toml") {
            text.replace(
                "max_parallel = 1\n",
                &format!("max_parallel = 1\n{setting}"),
            )
        } else {
            format!("{setting}{text}")
        };
        assert!(text.contains(setting), "{setting} was not added");
        fs::write(&path, text).unwrap();
        let error = config::load(root.path()).unwrap_err();
        assert!(format!("{error:#}").contains(setting.split(' ').next().unwrap()));
    }
}
