#![cfg(unix)]
mod support;
use please::{config, git_snapshot, planner::Plan, report, review_trigger};
use std::{
    fs,
    path::Path,
    process::{Command, Output},
};
use support::write;

fn git(root: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{:?}: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().into()
}
fn commit(root: &Path) -> String {
    git(root, &["add", "."]);
    git(
        root,
        &[
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
        ],
    );
    git(root, &["rev-parse", "HEAD"])
}
fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "-q"]);
    config::init(root.path()).unwrap();
    support::configure_adapter(root.path());
    fs::remove_dir_all(root.path().join(".please/checks")).unwrap();
    fs::remove_dir_all(root.path().join(".please/reviews")).unwrap();
    write(root.path(), "evidence", "Evidence\n");
    commit(root.path());
    root
}
fn review(root: &Path, trigger: &str, extra: &str) {
    write(
        root,
        ".please/reviews/test/rule.toml",
        &format!(
            "id = 'review'\ndescription = 'Review evidence'\nrun_at = ['check']\n{extra}\n[review]\nprovider = 'fixture'\nprompt = 'prompt.md'\n{trigger}"
        ),
    );
    write(root, ".please/reviews/test/prompt.md", "Inspect evidence\n");
    write(
        root,
        ".please/providers/fixture.toml",
        "command = 'missing-trigger-test-tool'\ntimeout_seconds = 5\n[requirements]\nenv = ['MISSING_TRIGGER_TEST_CREDENTIAL_15821']\n",
    );
}
fn invoke(root: &Path, base: Option<&str>, preview: bool) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_please"));
    cmd.current_dir(root)
        .args(["run", "--at", "check"])
        .env_remove("PLEASE_CHANGE_BASE")
        .env("HANNES_CHANGE_BASE", "HEAD");
    if let Some(base) = base {
        cmd.env("PLEASE_CHANGE_BASE", base);
    }
    if preview {
        cmd.args(["--plan", "--json"]);
    }
    cmd.output().unwrap()
}
fn plan(root: &Path, base: Option<&str>) -> Plan {
    let out = invoke(root, base, true);
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{e}: {:?}", out))
}

#[test]
fn exact_glob_multiple_patterns_and_invalid_configuration() {
    let paths = ["docs/a/b.md", "readme.md", ".hidden", "naïve.md"]
        .into_iter()
        .map(String::from)
        .collect();
    for (pattern, expected) in [
        ("docs/**/b.md", true),
        ("docs/*.md", false),
        ("docs/a/?.md", true),
        ("*.md", true),
        ("README.md", false),
        ("**/.hidden", true),
        ("na?ve.md", true),
    ] {
        let trigger = config::ReviewTrigger {
            changed_files: vec![pattern.into()],
        };
        review_trigger::validate(&trigger).unwrap();
        assert_eq!(
            review_trigger::matches(&trigger, &paths),
            expected,
            "{pattern}"
        );
    }
    let root = fixture();
    for pattern in [
        "/etc/file",
        "../file",
        "a/../b",
        "a//b",
        "a/./b",
        "C:/file",
        "a\\b",
        "[abc",
        "{a,b}",
        "a/**b",
        "",
    ] {
        review(
            root.path(),
            &format!(
                "[review.trigger]\nchanged_files = [{}]\n",
                toml::Value::String(pattern.into())
            ),
            "",
        );
        assert!(config::load(root.path()).is_err(), "{pattern}");
    }
    review(
        root.path(),
        "[review.trigger]\nchanged_files = []\n",
        "enabled = false",
    );
    assert!(config::load(root.path()).is_err());
    let trigger = config::ReviewTrigger {
        changed_files: vec!["absent".into(), "docs/**".into()],
    };
    assert!(review_trigger::matches(&trigger, &paths));
}

#[test]
fn net_comparison_covers_commits_worktree_add_delete_rename_and_unusual_names() {
    let root = fixture();
    for name in ["modified", "deleted", "old", "staged"] {
        write(root.path(), name, "before\n");
    }
    let base = commit(root.path());
    write(root.path(), "earlier", "first commit\n");
    commit(root.path());
    write(root.path(), "later", "last commit\n");
    commit(root.path());
    write(root.path(), "modified", "after\n");
    fs::remove_file(root.path().join("deleted")).unwrap();
    git(root.path(), &["mv", "old", "new"]);
    write(root.path(), "staged", "after\n");
    git(root.path(), &["add", "staged"]);
    write(root.path(), "space and\nnewline", "new\n");
    write(root.path(), ".gitignore", "ignored\n.please/\n");
    write(root.path(), "ignored", "ignored\n");
    write(root.path(), ".please/rubric.md", "ignored config\n");
    write(root.path(), ".please/reports/private", "excluded\n");
    let (resolved, paths) = git_snapshot::changed_paths(root.path(), &base).unwrap();
    assert_eq!(resolved, base);
    for name in [
        "earlier",
        "later",
        "modified",
        "deleted",
        "old",
        "new",
        "staged",
        "space and\nnewline",
        ".please/rubric.md",
    ] {
        assert!(paths.contains(name), "{name}");
    }
    assert!(!paths.contains("ignored"));
    assert!(!paths.contains(".please/reports/private"));
    let (_, head_paths) = git_snapshot::changed_paths(root.path(), "HEAD").unwrap();
    assert!(!head_paths.contains("earlier"));
}

#[test]
fn preview_and_execution_agree_and_nonmatches_need_no_provider_or_context() {
    let root = fixture();
    review(
        root.path(),
        "[review.trigger]\nchanged_files = ['absent']\n",
        "",
    );
    // Section validity is checked only when a triggered review is scheduled.
    let file = root.path().join(".please/reviews/test/rule.toml");
    let text = fs::read_to_string(&file).unwrap().replace(
        "[review.trigger]",
        "[[review.context]]\npath = 'evidence'\nheadings = ['Missing']\n[review.trigger]",
    );
    fs::write(&file, &text).unwrap();
    let preview = plan(root.path(), Some("HEAD"));
    assert!(preview.executable(), "{:?}", preview.blocking_reasons);
    assert_eq!(preview.omitted[0].reason, "trigger_not_matched");
    assert!(preview.scheduled.is_empty());
    assert!(preview.required_ids.is_empty());
    assert_eq!(preview.planned_model_calls, 0);
    assert!(preview.context.sections.is_empty());
    assert!(invoke(root.path(), Some("HEAD"), false).status.success());
    let saved = report::read(root.path()).unwrap();
    assert!(saved.results.is_empty());
    assert_eq!(saved.plan.plan_id, preview.plan_id);
    assert!(please::terminal::render(&saved, false).contains("trigger_not_matched"));
    fs::write(&file, text.replace("['absent']", "['evidence']")).unwrap();
    write(root.path(), "evidence", "Changed\n");
    assert!(!plan(root.path(), Some("HEAD")).executable());
}

#[test]
fn missing_invalid_and_shallow_bases_block_but_disabled_and_other_points_do_not() {
    let root = fixture();
    review(
        root.path(),
        "[review.trigger]\nchanged_files = ['evidence']\n",
        "",
    );
    for base in [None, Some("missing-ref"), Some("--all")] {
        let p = plan(root.path(), base);
        assert!(!p.executable());
        assert!(p.omitted.is_empty());
        assert!(!invoke(root.path(), base, false).status.success());
        assert_eq!(
            report::read(root.path()).unwrap().overall,
            report::Status::Error
        );
    }
    let old = git(root.path(), &["rev-parse", "HEAD"]);
    commit(root.path());
    let shallow = tempfile::tempdir().unwrap();
    git(
        shallow.path(),
        &[
            "clone",
            "--quiet",
            "--depth=1",
            &format!("file://{}", root.path().display()),
            "copy",
        ],
    );
    let copy = shallow.path().join("copy");
    assert!(git_snapshot::changed_paths(&copy, &old).is_err());
    assert!(git_snapshot::changed_paths(&copy, "HEAD").is_ok());
    review(
        root.path(),
        "[review.trigger]\nchanged_files = ['evidence']\n",
        "enabled = false",
    );
    assert!(plan(root.path(), None).executable());
    let file = root.path().join(".please/reviews/test/rule.toml");
    fs::write(
        &file,
        fs::read_to_string(&file)
            .unwrap()
            .replace("enabled = false", "enabled = true")
            .replace("['check']", "['pre-push']"),
    )
    .unwrap();
    assert!(plan(root.path(), None).executable());
}

#[test]
fn matched_reviews_preserve_provider_grouping_and_global_check_gate() {
    let root = fixture();
    review(
        root.path(),
        "[review.trigger]\nchanged_files = ['evidence']\n",
        "",
    );
    write(root.path(), "evidence", "Changed\n");
    let p = plan(root.path(), Some("HEAD"));
    assert!(p.executable());
    assert_eq!(p.planned_model_calls, 1);
    support::check(root.path(), "gate", &["check"], "exit 1\n");
    assert_eq!(
        invoke(root.path(), Some("HEAD"), false).status.code(),
        Some(1)
    );
    let saved = report::read(root.path()).unwrap();
    assert!(
        saved
            .results
            .iter()
            .any(|r| r.kind == "review" && r.status == report::Status::NotRun)
    );
    assert!(!format!("{:?}", saved.results).contains("required environment"));
    // Trigger-free reviews still schedule without a baseline, through an adapter.
    review(root.path(), "", "");
    assert_eq!(plan(root.path(), None).planned_model_calls, 1);
    let file = root.path().join(".please/reviews/test/rule.toml");
    fs::write(
        &file,
        fs::read_to_string(&file)
            .unwrap()
            .replace("provider = 'fixture'\n", ""),
    )
    .unwrap();
    assert_eq!(plan(root.path(), None).planned_model_calls, 1);
}

#[test]
fn truncated_comparisons_and_incomplete_snapshots_never_skip() {
    let root = fixture();
    review(
        root.path(),
        "[review.trigger]\nchanged_files = ['absent']\n",
        "",
    );
    for i in 0..350 {
        write(root.path(), &format!("{i:04}{}", "x".repeat(200)), "new\n");
    }
    let p = plan(root.path(), Some("HEAD"));
    assert!(!p.executable());
    assert!(p.omitted.is_empty());
}

#[test]
fn matched_triggers_execute_through_adapter_boundary() {
    {
        let root = fixture();
        review(
            root.path(),
            "[review.trigger]\nchanged_files = ['evidence']\n",
            "",
        );
        write(
            root.path(),
            ".please/reviews/second/rule.toml",
            &fs::read_to_string(root.path().join(".please/reviews/test/rule.toml"))
                .unwrap()
                .replace("id = 'review'", "id = 'second'"),
        );
        write(
            root.path(),
            ".please/reviews/second/prompt.md",
            "Inspect evidence too\n",
        );
        write(root.path(), "evidence", "Changed\n");
        let log = root.path().join(".please/reports/calls");
        let tool = support::context_provider(root.path());
        let provider = format!(
            "command = {:?}\ntimeout_seconds = 5\n",
            tool.to_str().unwrap()
        );
        write(root.path(), ".please/providers/fixture.toml", &provider);
        let preview = plan(root.path(), Some("HEAD"));
        assert!(preview.executable());
        assert_eq!(preview.planned_model_calls, 1);
        let output = Command::new(env!("CARGO_BIN_EXE_please"))
            .current_dir(root.path())
            .args(["run", "--at", "check"])
            .env("PLEASE_CHANGE_BASE", "HEAD")
            .env(
                "PATH",
                std::env::join_paths(
                    std::iter::once(root.path().to_path_buf())
                        .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
                )
                .unwrap(),
            )
            .env("EXPECTED_EXECUTION_POINT", "check")
            .env("CONTEXT_CALLS", &log)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let saved = report::read(root.path()).unwrap();
        assert_eq!(saved.plan.plan_id, preview.plan_id);
        assert_eq!(saved.results.len(), 2);
        assert!(
            saved
                .results
                .iter()
                .all(|r| r.status == report::Status::Pass)
        );
    }
}

#[test]
fn unreliable_index_flags_block_comparison() {
    let root = fixture();
    git(
        root.path(),
        &["update-index", "--assume-unchanged", "evidence"],
    );
    write(root.path(), "evidence", "Hidden modification\n");
    assert!(git_snapshot::changed_paths(root.path(), "HEAD").is_err());
    git(
        root.path(),
        &["update-index", "--no-assume-unchanged", "evidence"],
    );
    git(
        root.path(),
        &["update-index", "--skip-worktree", "evidence"],
    );
    assert!(git_snapshot::changed_paths(root.path(), "HEAD").is_err());
    git(
        root.path(),
        &["update-index", "--no-skip-worktree", "evidence"],
    );
    assert!(git_snapshot::changed_paths(root.path(), "HEAD").is_ok());
}

// Linux filesystems permit byte filenames that macOS rejects with EILSEQ.
#[cfg(target_os = "linux")]
#[test]
fn non_utf8_names_block_comparison() {
    use std::os::unix::ffi::OsStringExt;
    let root = fixture();
    fs::write(
        root.path()
            .join(std::ffi::OsString::from_vec(vec![b'x', 255])),
        "bad path",
    )
    .unwrap();
    assert!(git_snapshot::changed_paths(root.path(), "HEAD").is_err());
}

#[test]
fn imported_patterns_share_validation_and_consumer_paths() {
    let root = fixture();
    let remote = fixture();
    let cache = tempfile::tempdir().unwrap();
    write(
        remote.path(),
        "rules/config.toml",
        "id = 'imported'\ndescription = 'Imported reviews'\nrules = ['rules/reviews/quality']\nrun_at = ['check']\n",
    );
    write(
        remote.path(),
        "rules/reviews/quality/prompt.md",
        "Inspect consumer evidence\n",
    );
    let rule = "id = 'quality'\ndescription = 'Review consumer'\n[review]\nprompt = 'prompt.md'\n[review.trigger]\nchanged_files = ['../outside']\n";
    write(remote.path(), "rules/reviews/quality/rule.toml", rule);
    let invalid = commit(remote.path());
    let imports = |revision: &str| {
        format!(
            "project = 'My_App'\n[[imports]]\ngit = {:?}\npath = 'rules'\nref = {revision:?}\n",
            remote.path().to_str().unwrap()
        )
    };
    write(root.path(), ".please/config.toml", &imports(&invalid));
    let error = please::external::load(root.path(), true, Some(cache.path())).unwrap_err();
    assert!(format!("{error:#}").contains("invalid changed_files pattern"));
    write(
        remote.path(),
        "rules/reviews/quality/rule.toml",
        &rule.replace("../outside", "evidence"),
    );
    let valid = commit(remote.path());
    write(root.path(), ".please/config.toml", &imports(&valid));
    let project = please::external::load(root.path(), true, Some(cache.path())).unwrap();
    let review = project
        .reviews
        .iter()
        .find(|r| r.id.ends_with(":quality"))
        .unwrap();
    let trigger = review.trigger.as_ref().unwrap();
    assert!(!review_trigger::matches(
        trigger,
        &git_snapshot::changed_paths(root.path(), "HEAD").unwrap().1
    ));
    write(root.path(), "evidence", "Consumer changed\n");
    assert!(review_trigger::matches(
        trigger,
        &git_snapshot::changed_paths(root.path(), "HEAD").unwrap().1
    ));
}
