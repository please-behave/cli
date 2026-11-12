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
    config::init(root.path()).unwrap();
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
fn review_history_orders_whole_batches_and_preserves_result_order() {
    struct Mock;
    impl Provider for Mock {
        fn review(&self, request: &Request<'_>) -> anyhow::Result<String> {
            Ok(serde_json::json!({"reviews": request.reviews.iter().map(|r| serde_json::json!({"id": r.id, "status": "PASS", "reason": "Inspected", "findings": [{"message": "Evidence", "path": "evidence", "line": 1}]})).collect::<Vec<_>>()}).to_string())
        }
    }
    let root = project();
    support::configure_adapter(root.path());
    support::write(root.path(), "evidence", "Evidence\n");
    for (id, diff) in [("r-a", false), ("r-b", true), ("r-c", true)] {
        fs::write(support::review_path(root.path(), id), format!("id = '{id}'\ndescription = 'Review'\nrun_at = ['check']\n[review]\nprompt = 'prompt.md'\ninclude_diff = {diff}\n")).unwrap();
    }
    let cache = root.path().join(".git/please-scheduling.json");
    let execute = || {
        let mut starts = Vec::new();
        let mut batches = Vec::new();
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
                        assert!(running.insert(id.to_owned()));
                        starts.push(id.to_owned());
                    }
                    Progress::RuleFinished { item, .. } if item.kind == "review" => {
                        assert!(running.remove(&item.id));
                    }
                    Progress::ReviewBatch { count, .. } => {
                        assert!(running.is_empty(), "previous batch still shown as running");
                        batches.push(count);
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
        (starts, batches)
    };
    assert_eq!(
        execute(),
        (vec!["r-a".into(), "r-b".into(), "r-c".into()], vec![1, 2])
    );
    fs::write(
        &cache,
        r#"{"estimates":{"review":{"r-a":1,"r-b":10000,"r-c":1}}}"#,
    )
    .unwrap();
    assert_eq!(
        execute(),
        (vec!["r-b".into(), "r-c".into(), "r-a".into()], vec![2, 1])
    );
    let history: serde_json::Value = serde_json::from_slice(&fs::read(&cache).unwrap()).unwrap();
    for id in ["r-a", "r-b", "r-c"] {
        assert!(history["estimates"]["review"][id].as_u64().is_some());
    }
}

#[test]
fn linked_worktree_history_stays_outside_governed_files_and_is_isolated() {
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
    config::init(linked.path()).unwrap();
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
