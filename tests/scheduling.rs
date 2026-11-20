//! History changes dispatch, never plan identity or verification results.
mod support;
use please::{
    Progress,
    adapter::{Adapter, Request},
    config, planner,
    report::Status,
};
use std::fs;

struct NoReviews;
impl Adapter for NoReviews {
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
    impl Adapter for Mock {
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

/// Hold workers until coordinator events release them; estimates never act as sleeps.
fn gated_run(
    rules: &[(&str, u64, Option<&str>, &[&str])],
    limit: usize,
    fail_fast: bool,
    releases: &[&[&str]],
    with_history: bool,
) -> Vec<String> {
    let root = project();
    let mut estimates = serde_json::Map::new();
    for &(id, estimate, resource, dependencies) in rules {
        support::check(
            root.path(),
            id,
            &["check"],
            &format!("while [ ! -f .git/release-{id} ]; do sleep 0.01; done"),
        );
        let path = root.path().join(format!(".please/checks/{id}/rule.toml"));
        let text = fs::read_to_string(&path).unwrap();
        let text = format!("depends_on = {dependencies:?}\n{text}");
        let text = match resource {
            Some(resource) => text.replace("[check]", &format!("[check]\nresource = {resource:?}")),
            None => text,
        };
        fs::write(path, text).unwrap();
        estimates.insert(id.into(), estimate.into());
    }
    if with_history {
        fs::write(
            root.path().join(".git/please-scheduling.json"),
            serde_json::json!({"estimates": {"check": estimates}}).to_string(),
        )
        .unwrap();
    }
    let mut project = config::load(root.path()).unwrap();
    project.config.execution.max_parallel = limit.try_into().unwrap();
    let mut starts = Vec::new();
    let mut running = std::collections::BTreeSet::new();
    let mut passed = std::collections::BTreeSet::new();
    let report = please::verify_project_with_fail_fast(
        project,
        &NoReviews,
        please::VerificationMode::ChecksOnly,
        Some("check"),
        None,
        fail_fast,
        &mut |event| match event {
            Progress::RuleStarted { id, .. } => {
                let rule = rules.iter().find(|r| r.0 == id).unwrap();
                assert!(rule.3.iter().all(|d| passed.contains(*d)));
                if let Some(resource) = rule.2 {
                    assert!(running.iter().all(|active: &String| {
                        rules.iter().find(|r| r.0 == active).unwrap().2 != Some(resource)
                    }));
                }
                assert!(running.insert(id.to_owned()));
                assert!(running.len() <= limit);
                starts.push(id.to_owned());
                for release in releases.get(starts.len() - 1).copied().unwrap_or_default() {
                    fs::write(root.path().join(format!(".git/release-{release}")), "").unwrap();
                }
            }
            Progress::RuleFinished { item, .. } => {
                assert!(running.remove(&item.id));
                assert_eq!(item.status, Status::Pass, "{}: {}", item.id, item.reason);
                passed.insert(item.id.clone());
            }
            _ => (),
        },
    )
    .unwrap();
    assert_eq!(report.overall, Status::Pass);
    assert!(running.is_empty());
    assert_eq!(passed.len(), rules.len());
    assert_eq!(
        report
            .results
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        rules.iter().map(|r| r.0).collect::<Vec<_>>()
    );
    starts
}

#[test]
fn fail_fast_fast_lane_refills_independently_of_long_path_slots() {
    // REQ-0333, REQ-0044, REQ-0030, REQ-0046: one slot stays shortest-first,
    // while the other stays longest-path-first, whichever worker finishes first.
    let rules = [
        ("a", 1000, None, &[][..]),
        ("b", 900, None, &[][..]),
        ("c", 1, None, &[][..]),
        ("d", 2, None, &[][..]),
        ("e", 800, None, &[][..]),
        ("f", 3, None, &[][..]),
    ];
    assert_eq!(
        gated_run(
            &rules,
            2,
            true,
            &[&[], &["a"], &["c"], &["b"], &["d"], &["e", "f"]],
            true
        ),
        ["c", "a", "b", "d", "e", "f"]
    );
    assert_eq!(
        gated_run(
            &rules,
            3,
            true,
            &[&[], &[], &["a"], &["c"], &["b"], &["d", "e", "f"]],
            true
        ),
        ["c", "a", "b", "e", "d", "f"]
    );
}

#[test]
fn fast_lane_preserves_complete_serial_and_equal_estimate_order() {
    // REQ-0333, REQ-0044: complete and serial runs retain the existing order;
    // equal estimates use canonical IDs, including without local history.
    let rules = [
        ("a", 1000, None, &[][..]),
        ("b", 10, None, &[][..]),
        ("c", 1, None, &[][..]),
    ];
    assert_eq!(
        &gated_run(&rules, 2, false, &[&[], &["a", "b", "c"]], true)[..2],
        ["a", "b"]
    );
    for fail_fast in [false, true] {
        assert_eq!(
            gated_run(&rules, 1, fail_fast, &[&["a", "b", "c"]], true),
            ["a", "b", "c"]
        );
    }
    for with_history in [false, true] {
        let equal = [
            ("a", 1, None, &[][..]),
            ("b", 1, None, &[][..]),
            ("c", 1, None, &[][..]),
        ];
        assert_eq!(
            &gated_run(&equal, 2, true, &[&[], &["a", "b", "c"]], with_history)[..2],
            ["a", "b"]
        );
    }
}

#[test]
fn fast_lane_skips_resource_and_dependency_waiters_without_reserving_capacity() {
    // REQ-0333, REQ-0046, REQ-0030: even very small estimates cannot override
    // prerequisites or locks, and long checks may occupy the fast lane.
    let rules = [
        ("a", 1000, Some("shared"), &[][..]),
        ("b", 1, Some("shared"), &[][..]),
        ("c", 2, None, &["b"][..]),
        ("d", 3, None, &[][..]),
        ("e", 900, None, &[][..]),
    ];
    assert_eq!(
        gated_run(
            &rules,
            2,
            true,
            &[&[], &["e"], &["b"], &["c", "d"], &["a"]],
            true
        ),
        ["b", "e", "d", "c", "a"]
    );
    // A free fast lane scans past a shorter resource or dependency waiter.
    for (resource, dependencies) in [(Some("shared"), &[][..]), (None, &["a"][..])] {
        let blocked = [
            ("a", 1000, Some("shared"), &[][..]),
            ("b", 1, None, &[][..]),
            ("c", 2, resource, dependencies),
            ("d", 3, None, &[][..]),
        ];
        assert_eq!(
            gated_run(&blocked, 2, true, &[&[], &["b"], &["a"], &["c", "d"]], true),
            ["b", "a", "d", "c"]
        );
    }
    // Only one check is eligible initially, and it must use the fast lane.
    let chain = [("a", 10000, None, &[][..]), ("b", 1, None, &["a"][..])];
    assert_eq!(
        gated_run(&chain, 2, true, &[&["a"], &["b"]], true),
        ["a", "b"]
    );
}

#[test]
fn short_fast_lane_failure_stops_pending_checks_and_drains_running_work() {
    // REQ-0333, REQ-0068, REQ-0073, REQ-0086: an early short failure
    // stops dispatch, retains pending results, and lets the long worker finish.
    let root = project();
    support::check(
        root.path(),
        "a",
        &["check"],
        "while [ ! -f .git/short-finished ]; do sleep 0.01; done",
    );
    support::check(root.path(), "b", &["check"], "exit 0");
    support::check(root.path(), "c", &["check"], "exit 1");
    fs::write(
        root.path().join(".git/please-scheduling.json"),
        r#"{"estimates":{"check":{"a":1000,"b":900,"c":1}}}"#,
    )
    .unwrap();
    let mut project = config::load(root.path()).unwrap();
    project.config.execution.max_parallel = 2.try_into().unwrap();
    let mut starts = Vec::new();
    let report = please::verify_project_with_fail_fast(
        project,
        &NoReviews,
        please::VerificationMode::ChecksOnly,
        Some("check"),
        None,
        true,
        &mut |event| match event {
            Progress::RuleStarted { id, .. } => starts.push(id.to_owned()),
            Progress::RuleFinished { item, .. } if item.id == "c" => {
                fs::write(root.path().join(".git/short-finished"), "").unwrap();
            }
            _ => (),
        },
    )
    .unwrap();
    assert_eq!(starts, ["c", "a"]);
    assert_eq!(report.overall, Status::Fail);
    assert_eq!(
        report.results.iter().map(|r| r.status).collect::<Vec<_>>(),
        [Status::Pass, Status::NotRun, Status::Fail]
    );
    assert!(report.results[1].reason.contains("Fail-fast"));
    assert!(report.results[1].stdout.is_none());
}
