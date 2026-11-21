mod support;
use anyhow::{Result, bail};
use clap::{CommandFactory, Parser};
#[cfg(unix)]
use please::process;
use please::{
    adapter::{Adapter, Request},
    cli::Cli,
    config, git_snapshot,
    report::{self, Status},
    terminal,
};
#[cfg(unix)]
use std::time::{Duration, Instant};
use std::{
    cell::Cell,
    fs,
    path::{Path, PathBuf},
};
use tempfile::{TempDir, tempdir};
fn git(root: &Path, args: &[&str]) {
    let out = support::command("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
fn project() -> TempDir {
    let dir = tempdir().unwrap();
    git(dir.path(), &["init", "--quiet"]);
    please::init::init(dir.path()).unwrap();
    support::configure_adapter(dir.path());
    support::commit_all(dir.path());
    dir
}
fn synchronize(root: &Path) {
    please::reconciliation::reconcile(&config::load(root).unwrap(), &mut |_| Ok(true)).unwrap();
}
fn path(root: &Path, relative: &str) -> PathBuf {
    root.join(".please").join(relative)
}
fn replace(root: &Path, file: &str, from: &str, to: &str) {
    let file = path(root, file);
    let text = fs::read_to_string(&file).unwrap();
    assert!(text.contains(from), "fixture replacement missing");
    fs::write(file, text.replace(from, to)).unwrap();
}
fn enable(root: &Path) {
    replace(
        root,
        "reviews/product-consistency/rule.toml",
        "enabled = false",
        "enabled = true",
    );
}
struct Never;
impl Adapter for Never {
    fn review(&self, _: &Request<'_>) -> Result<String> {
        panic!("adapter must not be called")
    }
}
struct Mock {
    calls: Cell<u32>,
    payload: String,
    error: bool,
}
impl Adapter for Mock {
    fn review(&self, request: &Request<'_>) -> Result<String> {
        self.calls.set(self.calls.get() + 1);
        assert!(!request.reviews.is_empty());
        if self.error {
            bail!("mock process failure / timeout");
        }
        Ok(self.payload.clone())
    }
}
fn verdict(id: &str, status: &str) -> serde_json::Value {
    serde_json::json!({"id":id,"status":status,"reason":"Compared explicit expectations with evidence", "findings":[{"message":"Documented expectations inspected", "path":".please/README.md", "line":1}]})
}
fn mock(values: Vec<serde_json::Value>) -> Mock {
    Mock {
        calls: Cell::new(0),
        payload: serde_json::json!({"reviews":values}).to_string(),
        error: false,
    }
}
fn cli(root: &Path, args: &[&str]) -> std::process::Output {
    support::command(env!("CARGO_BIN_EXE_please"))
        .args(args)
        .current_dir(root)
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}
fn version_line() -> String {
    format!("Please Behave {}\n", env!("CARGO_PKG_VERSION"))
}

#[test]
fn execution_points_select_only_their_rules_and_empty_points_establish_no_pass() {
    // REQ-0150: an empty point succeeds without rule results or a claimed rule PASS.
    // REQ-0151: named points select their own rules and unknown points are rejected.
    // REQ-0152: omitting the point selects pre-push.
    let d = project();
    support::check(d.path(), "push-only", &["pre-push"], "exit 0");
    support::check(d.path(), "check-only", &["check"], "exit 0");
    fs::remove_dir_all(path(d.path(), "checks/always-ok")).unwrap();
    for (point, expected) in [
        (None, vec!["push-only"]),
        (Some("check"), vec!["check-only"]),
        (Some("pre-commit"), vec![]),
    ] {
        let report = please::verify_project_at(
            config::load(d.path()).unwrap(),
            &Never,
            please::VerificationMode::AllRules,
            point,
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(report.overall, Status::Pass);
        assert_eq!(report.plan.at, point.unwrap_or("pre-push"));
        assert_eq!(
            report
                .results
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            expected
        );
        if expected.is_empty() {
            let text = terminal::render_outcome(&report, 5, false);
            assert!(text.contains("No rules executed"));
            assert!(!text.contains("1 passed"));
        }
    }
    let error = please::planner::build(&config::load(d.path()).unwrap(), Some("unknown"), false)
        .err()
        .unwrap();
    assert!(error.to_string().contains("unknown execution point"));
}

#[test]
fn run_prints_its_version_first_in_every_mode() {
    // REQ-0184: runs print the binary version before normal output and configuration errors.
    let d = project();
    let version = version_line();
    let shown = String::from_utf8(cli(d.path(), &["--version"]).stdout).unwrap();
    assert!(shown.contains(env!("CARGO_PKG_VERSION")));
    let normal = cli(d.path(), &["run"]);
    assert!(
        String::from_utf8(normal.stdout)
            .unwrap()
            .starts_with(&version)
    );
    let json = cli(d.path(), &["run", "--plan", "--json"]);
    serde_json::from_slice::<serde_json::Value>(&json.stdout).unwrap();
    assert_eq!(String::from_utf8(json.stderr).unwrap(), version);
    fs::write(path(d.path(), "config.toml"), "unknown_setting = 1\n").unwrap();
    for args in [
        &["run"][..],
        &["run", "--plan"],
        &["run", "--plan", "--json"],
    ] {
        let failed = cli(d.path(), args);
        assert_eq!(failed.status.code(), Some(2), "{args:?}");
        assert!(
            String::from_utf8(failed.stderr)
                .unwrap()
                .starts_with(&version),
            "{args:?}"
        );
        if args.contains(&"--json") {
            serde_json::from_slice::<serde_json::Value>(&failed.stdout).unwrap();
        } else {
            assert!(failed.stdout.is_empty(), "{args:?}");
        }
    }
}
#[test]
fn json_plan_errors_use_current_schema_and_blocking_reasons() {
    // REQ-0154: previews leave the saved report, check marker and missing hooks untouched.
    // REQ-0156: JSON previews always carry a schema version, status and blocking reasons.
    // REQ-0157: blocked previews retain known input details and omit unavailable input.
    let d = project();
    support::check(
        d.path(),
        "always-ok",
        &["pre-push"],
        "echo executed > executed",
    );
    let report = path(d.path(), "reports/1.json");
    fs::write(&report, "prior report").unwrap();
    let preview = |args: &[&str], code| {
        let output = cli(d.path(), args);
        assert_eq!(output.status.code(), Some(code));
        let document: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        support::assert_plan_contract(&document);
        let mut compatible = document.clone();
        compatible["plan_schema_version"] = serde_json::json!(1);
        support::assert_plan_contract(&compatible);
        assert_eq!(document["plan_schema_version"], 0);
        assert_eq!(
            document["status"],
            if code == 0 { "planned" } else { "blocked" }
        );
        let reasons = document["blocking_reasons"].as_array().unwrap();
        assert_eq!(reasons.is_empty(), code == 0);
        assert!(
            reasons
                .iter()
                .all(|reason| !reason.as_str().unwrap().trim().is_empty())
        );
        assert!(document.get("errors").is_none());
        if document.get("input").is_some() {
            let plan: please::planner::Plan = serde_json::from_value(document.clone()).unwrap();
            let human_args: Vec<_> = args
                .iter()
                .copied()
                .filter(|arg| *arg != "--json")
                .collect();
            let human = cli(d.path(), &human_args);
            assert_eq!(human.status.code(), Some(code));
            assert!(human.stderr.is_empty());
            let text = String::from_utf8(human.stdout).unwrap();
            let text = text.strip_prefix(&version_line()).unwrap();
            assert_eq!(text, terminal::render_plan(&plan, false));
            assert!(text.starts_with("Please Behave execution plan · "));
            assert!(text.ends_with(
                "Preview: no rules executed, no tokens consumed, no report overwritten.\n"
            ));
        }
        assert_eq!(fs::read_to_string(&report).unwrap(), "prior report");
        assert!(!d.path().join("executed").exists());
        assert!(!d.path().join(".git/hooks/pre-push").exists());
        document
    };
    let args = ["run", "--plan", "--json"];
    let valid = preview(&args, 0);
    assert!(serde_json::from_value::<please::planner::Plan>(valid).is_ok());
    let unknown = preview(&["run", "--plan", "--json", "--at", "missing"], 2);
    assert!(
        unknown["blocking_reasons"][0]
            .as_str()
            .unwrap()
            .contains("unknown execution point")
    );
    assert!(unknown.get("input").is_none());

    let oversized = d.path().join("oversized");
    fs::File::create(&oversized)
        .unwrap()
        .set_len(please::limits::MAX_SNAPSHOT_FILE_BYTES + 1)
        .unwrap();
    let blocked = preview(&args, 2);
    assert_eq!(blocked["input"]["complete"], false);
    assert!(serde_json::from_value::<please::planner::Plan>(blocked).is_ok());
    fs::remove_file(oversized).unwrap();

    for invalid in ["[invalid", "project = ''\n"] {
        fs::write(path(d.path(), "config.toml"), invalid).unwrap();
        let blocked = preview(&args, 2);
        assert!(blocked.get("input").is_none());
    }
}

#[test]
fn plan_contract_covers_review_context_and_rejects_malformed_documents() {
    // REQ-0154: a preview prepares context without an available adapter or a written run context.
    let d = project();
    support::check(d.path(), "prerequisite", &["check"], "exit 0");
    support::write(
        d.path(),
        ".please/adapters/quality.toml",
        "command = 'unavailable-adapter'\ntimeout_seconds = 30\n[parameters]\nmodel = 'synthetic'\n",
    );
    support::write(
        d.path(),
        ".please/reviews/quality/rule.toml",
        r#"
id = "quality"
description = "Inspect changed evidence"
run_at = ["check"]
[review]
adapter = "quality"
prompt = "prompt.md"
include_content = true
include_change_set = true
context = [{path = "evidence.md"}]
"#,
    );
    support::write(
        d.path(),
        ".please/reviews/quality/prompt.md",
        "Inspect the evidence.",
    );
    support::write(d.path(), "evidence.md", "# Evidence\nChanged text.\n");
    let output = support::command(env!("CARGO_BIN_EXE_please"))
        .current_dir(d.path())
        .env("PLEASE_CHANGE_BASE", "HEAD")
        .args(["run", "--at", "check", "--plan", "--json"])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let plan: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    support::assert_plan_contract(&plan);
    assert!(!plan["context"]["sections"].as_array().unwrap().is_empty());
    assert!(!plan["context"]["scopes"].as_array().unwrap().is_empty());
    assert_eq!(plan["scheduled"][1]["adapter"], "quality");
    assert_eq!(plan["planned_model_calls"], 1);
    assert!(!d.path().join(".git/please/context").exists());

    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../knowledge/contracts/plan/plan.schema.json")).unwrap();
    let validator = jsonschema::validator_for(&schema).unwrap();
    for version in [0, 1] {
        let mut compatible = plan.clone();
        compatible["plan_schema_version"] = serde_json::json!(version);
        support::assert_plan_contract(&compatible);
        // The named adapter and disabled metadata remain readable by the existing consumer.
        serde_json::from_value::<please::planner::Plan>(compatible).unwrap();
    }
    for (pointer, value) in [
        ("/plan_schema_version", serde_json::json!(2)),
        ("/plan_schema_version", serde_json::json!("0")),
        ("/status", serde_json::json!("unknown")),
        (
            "/blocking_reasons",
            serde_json::json!(["unexpected blocker"]),
        ),
        ("/input/complete", serde_json::json!(false)),
        ("/context/sections/0/bytes", serde_json::json!(-1)),
        (
            "/scheduled/1/adapter_profile/timeout_seconds",
            serde_json::json!("30"),
        ),
    ] {
        let mut malformed = plan.clone();
        *malformed.pointer_mut(pointer).unwrap() = value;
        assert!(
            !validator.is_valid(&malformed),
            "accepted malformed {pointer}"
        );
    }
    let mut unknown = plan.clone();
    unknown["context"]["unexpected"] = serde_json::json!(true);
    assert!(!validator.is_valid(&unknown));
    let mut missing = plan.clone();
    missing.as_object_mut().unwrap().remove("scheduled");
    assert!(
        !validator.is_valid(&missing),
        "incomplete prepared plan became diagnostic"
    );
    for malformed in [
        serde_json::json!({"plan_schema_version":0,"status":"blocked","blocking_reasons":[]}),
        serde_json::json!({"plan_schema_version":0,"status":"blocked","blocking_reasons":[" "]}),
        serde_json::json!({"plan_schema_version":0,"status":"planned","blocking_reasons":[]}),
    ] {
        assert!(!validator.is_valid(&malformed));
    }
}

#[test]
fn human_plan_uses_english_and_preserves_layout_and_values() {
    // REQ-0155: rendered plans show ordered selection, omissions, required rules, input, adapters and context bounds.
    // Fixed metadata exercises optional display sections without invoking rules or adapters.
    let mut plan: please::planner::Plan = serde_json::from_value(serde_json::json!({
        "plan_schema_version": 1,
        "plan_id": "plan-123",
        "status": "planned",
        "blocking_reasons": [],
        "at": "check",
        "input": {"kind": "worktree", "head": "abc123", "fingerprint": "snapshot-456",
            "complete": true, "limitations": []},
        "checks_only": false,
        "scheduled": [{"id": "architecture", "name": "Architektur", "kind": "review",
            "description": "Prüft das Design", "blocking": true, "selected_by": ["at:check"],
            "adapter": "reviewer", "adapter_profile": {
                "command": "review-adapter", "timeout_seconds": 60}}],
        "omitted": [{"rule": {"id": "unchanged", "kind": "review", "description": "Unused",
            "blocking": true, "selected_by": ["at:check"]}, "reason": "trigger_not_matched"}],
        "required_ids": ["architecture"],
        "adapter": {"command": "fake-adapter", "timeout_seconds": 180},
        "adapter_availability_checked": false,
        "planned_model_calls": 1,
        "change_base": "base-789",
        "context": {"scopes": [{"review": "architecture", "files": ["src/lib.rs"], "content_bytes": 30, "change_set_bytes": 40}],
            "selections": [{"path": "knowledge/requirements/README.md", "headings": []},
                {"path": "DESIGN.md", "headings": ["Architecture", "Boundaries"]}],
            "sections": [{"path": "knowledge/requirements/README.md", "start_line": 2, "end_line": 7, "bytes": 120}],
            "selected_bytes": 120, "byte_limit": 1000, "instruction_bytes": 80,
            "instruction_byte_limit": 500}
    }))
    .unwrap();
    let before = serde_json::to_value(&plan).unwrap();
    let text = terminal::render_plan(&plan, false);
    assert_eq!(
        text,
        concat!(
            "Please Behave execution plan · PLANNED\n",
            "Execution Point: check (execution point) · Execution point · Input: worktree\n",
            "Plan-ID: plan-123\nHEAD: abc123\nSnapshot: snapshot-456 · complete\n",
            "Change base: base-789\n",
            "[1] review Architektur · Prüft das Design\n    Selection: at:check\n",
            "Omitted: review unchanged · trigger_not_matched\nRequired rules: architecture\n",
            "Default adapter: fake-adapter · Timeout: 180 s\n",
            "Adapter availability not checked · Planned model calls: 1\n",
            "Context: 120 / 1000 bytes\n",
            "Review Architektur · Adapter profile: reviewer · Adapter: review-adapter · Timeout: 60 s\n",
            "Scope of architecture: 1 files · Content: 30 bytes · Change set: 40 bytes\n",
            "Review criteria: 80 / 500 bytes\n",
            "Requirements: knowledge/requirements/README.md · entire file\n",
            "Requirements: DESIGN.md · Architecture, Boundaries\n",
            "Context section: knowledge/requirements/README.md:2–7 · 120 bytes\n",
            "Preview: no rules executed, no tokens consumed, no report overwritten.\n",
        )
    );
    assert_eq!(serde_json::to_value(&plan).unwrap(), before);
    let colored = terminal::render_plan(&plan, true);
    assert!(colored.contains("\u{1b}["));
    assert_eq!(
        colored
            .replace("\u{1b}[1m", "")
            .replace("\u{1b}[36m", "")
            .replace("\u{1b}[0m", ""),
        text
    );

    plan.status = "blocked".into();
    plan.input.head = None;
    plan.input.complete = false;
    plan.input.limitations = vec!["File exceeds snapshot limit".into()];
    plan.blocking_reasons = vec!["Input snapshot is incomplete; no verification can start".into()];
    plan.checks_only = true;
    plan.omitted.push(please::planner::Omission {
        rule: plan.scheduled.remove(0),
        reason: "trigger_not_matched".into(),
    });
    let text = terminal::render_plan(&plan, false);
    for expected in [
        "Please Behave execution plan · BLOCKED\n",
        "HEAD: unborn\nSnapshot: snapshot-456 · UNKNOWN\n",
        "Mode: Checks only · no adapter calls\n",
        "No active rules selected; no rule PASS established.\n",
        "Omitted: review architecture · trigger_not_matched\n",
        "Context: 120 / 1000 bytes\n",
        "Blocked: Input snapshot is incomplete; no verification can start\nBlocked: File exceeds snapshot limit\n",
    ] {
        assert!(text.contains(expected), "missing {expected:?} in {text}");
    }
}

#[test]
fn all_command_paths_and_standard_help() {
    // REQ-0276, REQ-0278: supported commands and the standard help/version options parse.
    for words in [
        "init",
        "sync",
        "trust some/repository",
        "trust --remove some/repository",
        "trust --list",
        "run",
        "status",
        "help",
    ] {
        let argv: Vec<_> = std::iter::once("please")
            .chain(words.split_whitespace())
            .collect();
        assert!(Cli::try_parse_from(argv).is_ok(), "{words}");
    }
    for args in [["please", "--help"], ["please", "--version"]] {
        assert_eq!(Cli::try_parse_from(args).unwrap_err().exit_code(), 0);
    }
    for words in [
        "fang",
        "fang an",
        "pass auf",
        "mach was",
        "was ist phase",
        "so hilf mir doch",
        "sync --auto-approve",
        "run --geiz-ist-geil",
        "init extra",
        "was phase",
        "so hilf doch",
        "mach falsch",
    ] {
        let argv: Vec<_> = std::iter::once("please")
            .chain(words.split_whitespace())
            .collect();
        assert!(Cli::try_parse_from(argv).is_err());
    }
}
#[test]
fn init_exact_paths_and_repeated_initialization_rejected() {
    let d = tempdir().unwrap();
    let root = d.path();
    git(root, &["init", "--quiet"]);
    let output = cli(root, &["init"]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(output.stdout, b"Initialized .please/ (9 files created).\n");
    assert!(output.stderr.is_empty());
    for file in [
        "config.toml",
        "README.md",
        "checks/always-ok/rule.toml",
        "checks/deliberately-failing/rule.toml",
        "reviews/product-consistency/rule.toml",
        "reviews/product-consistency/prompt.md",
        "reports/.gitignore",
    ] {
        assert!(path(root, file).is_file(), "{file}");
    }
    // Check the actual directory entry, even on case-insensitive filesystems.
    assert!(
        fs::read_dir(root)
            .unwrap()
            .any(|entry| { entry.unwrap().file_name() == ".please" })
    );
    assert!(!path(root, "reports/1.json").exists());
    fs::write(path(root, "config.toml"), "personal edits").unwrap();
    assert!(!path(root, "adapter.toml").exists());
    let output = cli(root, &["init"]);
    assert_eq!(output.status.code(), Some(2));
    assert!(output.stdout.is_empty());
    assert_eq!(
        output.stderr,
        b"Please Behave: ERROR: Already initialized (.please/ exists).\n"
    );
    assert!(!path(root, "adapter.toml").exists());
    assert_eq!(
        fs::read_to_string(path(root, "config.toml")).unwrap(),
        "personal edits"
    );
}

#[test]
// REQ-0287: an existing directory or file causes ERROR without changing its contents.
fn init_rejects_existing_configuration_entries_without_writes() {
    for kind in ["empty directory", "partial directory", "file"] {
        let d = tempdir().unwrap();
        let root = d.path();
        git(root, &["init", "--quiet"]);
        let directory = root.join(".please");
        if kind == "file" {
            fs::write(&directory, "existing file").unwrap();
        } else {
            fs::create_dir(&directory).unwrap();
            if kind == "partial directory" {
                fs::write(directory.join("config.toml"), "personal edits").unwrap();
            }
        }
        let output = cli(root, &["init"]);
        assert_eq!(output.status.code(), Some(2), "{kind}");
        assert!(output.stdout.is_empty(), "{kind}");
        assert_eq!(
            output.stderr, b"Please Behave: ERROR: Already initialized (.please/ exists).\n",
            "{kind}"
        );
        if kind == "file" {
            assert_eq!(fs::read_to_string(&directory).unwrap(), "existing file");
        } else {
            assert_eq!(
                fs::read_dir(&directory).unwrap().count(),
                usize::from(kind == "partial directory")
            );
            if kind == "partial directory" {
                assert_eq!(
                    fs::read_to_string(directory.join("config.toml")).unwrap(),
                    "personal edits"
                );
            }
        }
    }
}

#[test]
#[cfg(unix)]
// REQ-0287: existing symlinks, including dangling links, are preserved and rejected.
fn init_rejects_configuration_symlinks_without_writes() {
    for target_kind in ["directory", "file", "missing"] {
        let d = tempdir().unwrap();
        let root = d.path();
        git(root, &["init", "--quiet"]);
        let target = root.join("target");
        match target_kind {
            "directory" => fs::create_dir(&target).unwrap(),
            "file" => fs::write(&target, "existing target").unwrap(),
            _ => {}
        }
        let directory = root.join(".please");
        std::os::unix::fs::symlink(&target, &directory).unwrap();
        let output = cli(root, &["init"]);
        assert_eq!(output.status.code(), Some(2), "{target_kind}");
        assert!(output.stdout.is_empty());
        assert_eq!(
            output.stderr,
            b"Please Behave: ERROR: Already initialized (.please/ exists).\n"
        );
        assert_eq!(fs::read_link(&directory).unwrap(), target);
        match target_kind {
            "directory" => assert_eq!(fs::read_dir(&target).unwrap().count(), 0),
            "file" => assert_eq!(fs::read_to_string(&target).unwrap(), "existing target"),
            _ => assert!(!target.exists()),
        }
    }
}

#[test]
fn predecessor_configuration_is_not_loaded_or_modified() {
    let root = tempdir().unwrap();
    git(root.path(), &["init", "--quiet"]);
    let predecessor = root.path().join(".hannes/config.toml");
    support::write(
        root.path(),
        ".hannes/config.toml",
        "published governor configuration",
    );
    assert!(config::load(root.path()).is_err());
    please::init::init(root.path()).unwrap();
    support::configure_adapter(root.path());
    assert!(root.path().join(".please/config.toml").is_file());
    assert!(config::load(root.path()).is_ok());
    assert_eq!(
        fs::read_to_string(predecessor).unwrap(),
        "published governor configuration"
    );
}

#[test]
fn validation_does_not_execute_checks_or_adapter() {
    let d = project();
    enable(d.path());
    replace(
        d.path(),
        "checks/always-ok/rule.toml",
        if cfg!(windows) {
            r#"command = "cmd /C @rule/index.cmd""#
        } else {
            r#"command = "sh @rule/index.sh""#
        },
        r#"command = "nonexistent-command-please""#,
    );
    assert!(config::load(d.path()).is_ok());
    assert_eq!(cli(d.path(), &["sync", "--yes"]).status.code(), Some(0));
}
#[test]
// REQ-0238: an unchanged saved snapshot gives current status.
// REQ-0255: the saved report is readable from the project.
fn default_run_passes_omits_disabled_and_roundtrips_report() {
    let d = project();
    let r = please::verify(d.path(), &Never).unwrap();
    assert_eq!(r.overall, Status::Pass);
    assert_eq!(r.results.len(), 1);
    assert_eq!(r.results[0].id, "always-ok");
    support::assert_report_contract(&serde_json::to_value(&r).unwrap());
    let decoded = report::read(d.path()).unwrap();
    assert_eq!(decoded.snapshot.fingerprint, r.snapshot.fingerprint);
    assert!(!terminal::render(&r, false).contains('\x1b'));
    assert!(!terminal::render(&r, false).contains("deliberately-failing"));
    assert!(terminal::render(&r, true).contains("\x1b[32mPASS"));
    assert_eq!(
        git_snapshot::compare(&r.snapshot, &git_snapshot::capture(d.path()).unwrap(), 5).state,
        "CURRENT"
    );
    assert_eq!(cli(d.path(), &["status"]).status.code(), Some(0));
}
#[test]
fn omitted_review_reports_roundtrip_for_staged_and_explicit_base_points() {
    // REQ-0178, REQ-0259, REQ-0260: both valid omission reasons remain readable.
    for point in ["pre-commit", "check"] {
        for (trigger, reason) in [
            ("", "no_changes"),
            (
                "[review.trigger]\nchanged_files = ['docs/**']\n",
                "trigger_not_matched",
            ),
        ] {
            let d = project();
            support::write(
                d.path(),
                ".please/reviews/unchanged/rule.toml",
                &format!(
                    "id = 'unchanged'\ndescription = 'Review unchanged input'\nrun_at = ['{point}']\n[review]\nprompt = 'prompt.md'\n{trigger}"
                ),
            );
            support::write(
                d.path(),
                ".please/reviews/unchanged/prompt.md",
                "Inspect the input.",
            );
            support::commit_all(d.path());
            assert!(cli(d.path(), &["sync", "--yes"]).status.success());
            let run = support::command(env!("CARGO_BIN_EXE_please"))
                .current_dir(d.path())
                .env("PLEASE_CHANGE_BASE", "HEAD")
                .args(["run", "--at", point])
                .output()
                .unwrap();
            assert_eq!(
                run.status.code(),
                Some(0),
                "{}",
                String::from_utf8_lossy(&run.stderr)
            );
            let document: serde_json::Value =
                serde_json::from_slice(&fs::read(path(d.path(), "reports/1.json")).unwrap())
                    .unwrap();
            support::assert_report_contract(&document);
            assert_eq!(document["plan"]["omitted"][0]["reason"], reason);
            assert!(report::read(d.path()).is_ok(), "{point}: {reason}");
            let status = cli(d.path(), &["status"]);
            assert_eq!(
                status.status.code(),
                Some(0),
                "{}",
                String::from_utf8_lossy(&status.stderr)
            );
            assert!(String::from_utf8_lossy(&status.stdout).contains("CURRENT"));

            let mut invalid = document.clone();
            invalid["plan"]["omitted"][0]["reason"] = serde_json::json!("unknown_reason");
            fs::write(path(d.path(), "reports/1.json"), invalid.to_string()).unwrap();
            assert!(report::read(d.path()).is_err());
            if point == "check" {
                let mut missing_base = document.clone();
                missing_base["plan"]
                    .as_object_mut()
                    .unwrap()
                    .remove("change_base");
                fs::write(path(d.path(), "reports/1.json"), missing_base.to_string()).unwrap();
                assert!(report::read(d.path()).is_err());
            }
        }
    }
}

#[test]
fn report_schema_rejects_malformed_snapshot_results_and_embedded_plan() {
    let d = project();
    let report = please::verify(d.path(), &Never).unwrap();
    let document = serde_json::to_value(&report).unwrap();
    support::assert_report_contract(&document);
    let validator = support::report_contract_validator();
    for (pointer, value) in [
        ("/schema_version", serde_json::json!(2)),
        ("/schema_version", serde_json::json!("0")),
        ("/overall", serde_json::json!("NOT_RUN")),
        ("/snapshot/complete", serde_json::json!("true")),
        ("/snapshot/files", serde_json::json!([])),
        ("/results/0/status", serde_json::json!("unknown")),
        ("/results/0/duration_ms", serde_json::json!(-1)),
        ("/plan/plan_schema_version", serde_json::json!(2)),
    ] {
        let mut invalid = document.clone();
        *invalid.pointer_mut(pointer).unwrap() = value;
        assert!(!validator.is_valid(&invalid), "accepted {pointer}");
    }
    let mut invalid = document.clone();
    invalid["snapshot"]["unexpected"] = serde_json::json!(true);
    assert!(!validator.is_valid(&invalid));
    let mut invalid = document.clone();
    invalid["plan"] = serde_json::json!({"plan_schema_version":0,"status":"blocked","blocking_reasons":["unavailable"]});
    assert!(
        !validator.is_valid(&invalid),
        "diagnostic preview is not a saved prepared plan"
    );
}

#[test]
fn failing_check_runs_remaining_and_blocks_adapter() {
    let d = project();
    enable(d.path());
    replace(
        d.path(),
        "checks/deliberately-failing/rule.toml",
        "enabled = false",
        "enabled = true",
    );
    replace(
        d.path(),
        "checks/deliberately-failing/rule.toml",
        "deliberately-failing",
        "aaa-failure",
    );
    synchronize(d.path());
    let r = please::verify(d.path(), &Never).unwrap();
    support::assert_report_contract(&serde_json::to_value(&r).unwrap());
    assert_eq!(r.overall, Status::Fail);
    assert_eq!(r.overall.exit_code(), 1);
    assert_eq!(r.results[0].status, Status::Fail);
    assert_eq!(r.results[1].status, Status::Pass);
    assert_eq!(r.results[2].status, Status::NotRun);
    assert!(path(d.path(), "reports/1.json").is_file());
    assert_eq!(cli(d.path(), &["run"]).status.code(), Some(1));
}
#[test]
fn spawn_error_writes_blocking_error_report() {
    let d = project();
    replace(
        d.path(),
        "checks/always-ok/rule.toml",
        if cfg!(windows) {
            r#"command = "cmd /C @rule/index.cmd""#
        } else {
            r#"command = "sh @rule/index.sh""#
        },
        r#"command = "nonexistent-command-please""#,
    );
    let r = please::verify(d.path(), &Never).unwrap();
    assert_eq!(r.overall, Status::Error);
    assert_eq!(r.results[0].status, Status::Error);
    assert_eq!(cli(d.path(), &["run"]).status.code(), Some(2));
    assert_eq!(report::read(d.path()).unwrap().overall, Status::Error);
}
#[test]
fn every_review_gets_its_own_call_with_exact_coverage_pass_fail() {
    struct Answering {
        calls: Cell<u32>,
        status: &'static str,
    }
    impl Adapter for Answering {
        fn review(&self, request: &Request<'_>) -> Result<String> {
            self.calls.set(self.calls.get() + 1);
            assert_eq!(request.reviews.len(), 1);
            Ok(
                serde_json::json!({"reviews":[verdict(&request.reviews[0].id, self.status)]})
                    .to_string(),
            )
        }
    }
    for status in ["PASS", "FAIL"] {
        let d = project();
        enable(d.path());
        let original =
            fs::read_to_string(path(d.path(), "reviews/product-consistency/rule.toml")).unwrap();
        fs::write(
            support::review_path(d.path(), "second"),
            original.replace("id = \"product-consistency\"", "id = \"second\""),
        )
        .unwrap();
        let mock = Answering {
            calls: Cell::new(0),
            status,
        };
        let r = please::verify(d.path(), &mock).unwrap();
        assert_eq!(mock.calls.get(), 2);
        assert_eq!(r.results.len(), 3);
        assert_eq!(r.overall.label(), status);
        assert_eq!(r.results[1].id, "product-consistency");
        assert_eq!(r.results[2].id, "second");
    }
}
#[test]
fn malformed_missing_extra_duplicate_adapter_outputs_fail_closed() {
    let mut invalid_line = verdict("product-consistency", "PASS");
    invalid_line["findings"][0]["line"] = serde_json::json!(999999);
    let mut invalid_path = verdict("product-consistency", "PASS");
    invalid_path["findings"][0]["path"] = serde_json::json!("../secrets");
    let mut missing_field = verdict("product-consistency", "PASS");
    missing_field["findings"][0]
        .as_object_mut()
        .unwrap()
        .remove("line");
    let responses = vec![
        "not json".into(), "{}".into(), "{\"reviews\":[]}".into(),
        serde_json::json!({"reviews":[verdict("wrong", "PASS")]}).to_string(),
        serde_json::json!({"reviews":[verdict("product-consistency", "PASS"), verdict("product-consistency", "PASS")]}).to_string(),
        serde_json::json!({"reviews":[verdict("product-consistency", "NOT_RUN")]}).to_string(),
        serde_json::json!({"reviews":[verdict("product-consistency", "PASS")],"extra":true}).to_string(),
        serde_json::json!({"reviews":[invalid_line]}).to_string(),
        serde_json::json!({"reviews":[invalid_path]}).to_string(),
        serde_json::json!({"reviews":[missing_field]}).to_string(),
        format!("{}\nextra text", serde_json::json!({"reviews":[verdict("product-consistency", "PASS")]})),
    ];
    for payload in responses {
        let d = project();
        enable(d.path());
        let mock = Mock {
            calls: Cell::new(0),
            payload,
            error: false,
        };
        let r = please::verify(d.path(), &mock).unwrap();
        assert_eq!(r.overall, Status::Error, "{}", mock.payload);
        assert_eq!(r.results[1].status, Status::Error);
        assert!(path(d.path(), "reports/1.json").is_file());
    }
}
#[test]
fn adapter_error_and_timeout_are_errors() {
    let d = project();
    enable(d.path());
    let mock = Mock {
        calls: Cell::new(0),
        payload: String::new(),
        error: true,
    };
    let r = please::verify(d.path(), &mock).unwrap();
    assert_eq!(r.overall, Status::Error);
    assert_eq!(mock.calls.get(), 1);
}
#[test]
// REQ-0238: modified content makes status stale.
// REQ-0240: changed paths respect the display bound and exact totals omit line counts.
fn status_tracks_staged_modified_untracked_and_ignores_report_changes() {
    let d = project();
    let root = d.path();
    fs::write(root.join("tracked.txt"), "original\n").unwrap();
    git(root, &["add", "tracked.txt"]);
    let r = please::verify(root, &Never).unwrap();
    fs::write(path(root, "reports/extra.json"), "ignored local data").unwrap();
    assert_eq!(
        git_snapshot::compare(&r.snapshot, &git_snapshot::capture(root).unwrap(), 5).state,
        "CURRENT"
    );
    fs::write(root.join("tracked.txt"), "modified same status\n").unwrap();
    for i in 0..7 {
        fs::write(root.join(format!("new-{i}.txt")), "new\n").unwrap();
    }
    git(root, &["add", "new-0.txt"]);
    let changes = git_snapshot::compare(&r.snapshot, &git_snapshot::capture(root).unwrap(), 5);
    assert_eq!(changes.state, "STALE");
    assert_eq!(changes.total_files, Some(8));
    assert_eq!(changes.sample_paths.len(), 5);
    assert!(changes.line_counts.is_none());
    let text = terminal::render_changes(&changes, false);
    assert!(text.contains("changed files since verification: 8"));
    assert!(text.contains("hash-only baseline"));
    assert_eq!(cli(root, &["status"]).status.code(), Some(1));
    let staged = git_snapshot::capture(root).unwrap();
    fs::write(root.join("tracked.txt"), "yet another change\n").unwrap();
    assert_eq!(
        git_snapshot::compare(&staged, &git_snapshot::capture(root).unwrap(), 5).total_files,
        Some(1)
    );
    fs::remove_file(root.join("tracked.txt")).unwrap();
    assert_eq!(
        git_snapshot::compare(&staged, &git_snapshot::capture(root).unwrap(), 5).total_files,
        Some(1)
    );
}
#[test]
fn ignored_configuration_is_still_fingerprinted() {
    let d = project();
    fs::write(d.path().join(".gitignore"), ".please/\n").unwrap();
    let before = git_snapshot::capture(d.path()).unwrap();
    replace(
        d.path(),
        "config.toml",
        "max_display_files = 5",
        "max_display_files = 3",
    );
    let changes = git_snapshot::compare(&before, &git_snapshot::capture(d.path()).unwrap(), 5);
    assert_eq!(changes.state, "STALE");
    assert_eq!(changes.total_files, Some(1));
}

#[test]
fn ignored_artifacts_do_not_change_verification_inputs() {
    let d = project();
    let root = d.path();
    fs::write(root.join(".gitignore"), ".please/\n*.cache\n").unwrap();
    let before = git_snapshot::capture(root).unwrap();
    support::write(root, ".please/bin/__pycache__/adapter.pyc", "bytecode");
    support::write(root, ".please/generated/output.cache", "generated");
    support::write(root, "build.cache", "generated");
    let after = git_snapshot::capture(root).unwrap();
    assert!(before.complete && after.complete);
    assert_eq!(before.fingerprint, after.fingerprint);
    fs::write(root.join("build.cache"), "changed").unwrap();
    fs::remove_dir_all(path(root, "bin/__pycache__")).unwrap();
    assert_eq!(
        before.fingerprint,
        git_snapshot::capture(root).unwrap().fingerprint
    );

    // Tracked contents are inputs even when an ignore pattern matches.
    git(root, &["add", "-f", ".please/generated/output.cache"]);
    let tracked = git_snapshot::capture(root).unwrap();
    assert!(tracked.files.contains_key(".please/generated/output.cache"));
    support::write(root, ".please/generated/output.cache", "changed");
    assert_eq!(
        git_snapshot::compare(&tracked, &git_snapshot::capture(root).unwrap(), 5).total_files,
        Some(1)
    );
}

#[test]
// REQ-0232: ignored governing files and explicit references remain snapshot inputs.
fn ignored_governance_inputs_remain_fingerprinted() {
    let d = project();
    let root = d.path();
    fs::write(root.join(".gitignore"), ".please/\nprivate/\n").unwrap();
    support::check(root, "referenced", &["check"], "exit 0\n");
    support::write(root, ".please/bin/adapter.py", "# adapter\n");
    support::write(
        root,
        ".please/adapter.toml",
        "command = 'python3'\nargs = ['.please/bin/adapter.py']\ntimeout_seconds = 30\n",
    );
    support::write(
        root,
        ".please/adapters/named.toml",
        "command = '.please/bin/named'\ntimeout_seconds = 30\n",
    );
    support::write(root, ".please/bin/named", "# adapter\n");
    support::write(
        root,
        ".please/rule-sets/quality.toml",
        "id = 'quality'\ndescription = 'Quality'\nrules = ['referenced']\nrun_at = ['check']\n",
    );
    support::write(root, "private/parameter.txt", "parameter\n");
    support::write(root, "private/repo.txt", "repository reference\n");
    support::write(root, "private/source.txt", "source reference\n");
    let rule = path(root, "checks/referenced/rule.toml");
    let text = fs::read_to_string(&rule).unwrap();
    let text = text.replace(
        "sh @rule/entrypoint",
        "sh @rule/entrypoint @repo/private/repo.txt @source/private/source.txt",
    );
    fs::write(
        &rule,
        format!("{text}\n[parameters.input]\ntype = 'file'\nvalue = 'private/parameter.txt'\n"),
    )
    .unwrap();
    support::write(root, "private/evidence.md", "# Evidence\n");
    let rule = path(root, "reviews/product-consistency/rule.toml");
    let text = fs::read_to_string(&rule).unwrap();
    fs::write(
        &rule,
        format!("{text}\n[[review.context]]\npath = 'private/evidence.md'\n"),
    )
    .unwrap();

    let before = git_snapshot::capture(root).unwrap();
    assert!(before.complete, "{:?}", before.limitations);
    for input in [
        ".please/config.toml",
        ".please/checks/always-ok/index.sh",
        ".please/checks/referenced/rule.toml",
        ".please/checks/referenced/entrypoint",
        ".please/reviews/product-consistency/prompt.md",
        ".please/rule-sets/quality.toml",
        ".please/adapter.toml",
        ".please/adapters/named.toml",
        ".please/bin/adapter.py",
        ".please/bin/named",
        "private/parameter.txt",
        "private/repo.txt",
        "private/source.txt",
        "private/evidence.md",
    ] {
        assert!(before.files.contains_key(input), "{input}");
        let original = fs::read(root.join(input)).unwrap();
        let mut changed = original.clone();
        changed.extend_from_slice(b"\n# changed\n");
        fs::write(root.join(input), changed).unwrap();
        let after = git_snapshot::capture(root).unwrap();
        assert!(after.complete, "{input}: {:?}", after.limitations);
        let changes = git_snapshot::compare(&before, &after, 5);
        assert_eq!(changes.total_files, Some(1), "{input}");
        assert_eq!(changes.sample_paths, [input], "{input}");
        fs::write(root.join(input), original).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn checks_can_generate_ignored_bytecode_without_invalidating_verification() {
    let d = project();
    fs::write(d.path().join(".gitignore"), ".please/\n").unwrap();
    support::check(
        d.path(),
        "bytecode",
        &["pre-push"],
        "mkdir -p .please/bin/__pycache__\nprintf bytecode > .please/bin/__pycache__/adapter.pyc\n",
    );
    let report = please::verify(d.path(), &Never).unwrap();
    assert_eq!(report.overall, Status::Pass);
    assert!(
        !report
            .snapshot
            .files
            .contains_key(".please/bin/__pycache__/adapter.pyc")
    );
    assert_eq!(
        git_snapshot::compare(
            &report.snapshot,
            &git_snapshot::capture(d.path()).unwrap(),
            5
        )
        .state,
        "CURRENT"
    );
}
#[test]
// REQ-0273: --no-color suppresses ANSI in run output.
// REQ-0275: missing reports exit 3 and corrupt or unsupported reports exit 2.
fn no_report_corrupt_report_schema_and_no_color() {
    let d = project();
    assert_eq!(cli(d.path(), &["status"]).status.code(), Some(3));
    fs::write(path(d.path(), "reports/1.json"), "broken").unwrap();
    assert_eq!(cli(d.path(), &["status"]).status.code(), Some(2));
    let r = please::verify(d.path(), &Never).unwrap();
    let mut value = serde_json::to_value(r).unwrap();
    // REQ-0332: the new report and its embedded plan both use version zero.
    assert_eq!(value["schema_version"], 0);
    assert_eq!(value["plan"]["plan_schema_version"], 0);
    // REQ-0331: both labels use the full report validation, without rewriting stored data.
    for report_version in [0, 1] {
        for plan_version in [0, 1] {
            let mut compatible = value.clone();
            compatible["schema_version"] = serde_json::json!(report_version);
            compatible["plan"]["plan_schema_version"] = serde_json::json!(plan_version);
            fs::write(path(d.path(), "reports/2.json"), compatible.to_string()).unwrap();
            support::assert_report_contract(&compatible);
            let saved = report::read(d.path()).unwrap();
            assert_eq!(saved.schema_version, report_version);
            assert_eq!(saved.plan.plan_schema_version, plan_version);
        }
    }
    for field in ["schema_version", "plan_schema_version"] {
        for version in [
            serde_json::json!(true),
            serde_json::json!(-1),
            serde_json::json!(2),
            serde_json::json!(1.0),
            serde_json::json!("0"),
            serde_json::Value::Null,
        ] {
            let mut invalid = value.clone();
            let target = if field == "schema_version" {
                &mut invalid
            } else {
                &mut invalid["plan"]
            };
            target[field] = version;
            fs::write(path(d.path(), "reports/2.json"), invalid.to_string()).unwrap();
            assert!(report::read(d.path()).is_err());
        }
    }
    value["schema_version"] = serde_json::json!(999);
    fs::write(path(d.path(), "reports/2.json"), value.to_string()).unwrap();
    assert_eq!(cli(d.path(), &["status"]).status.code(), Some(2));
    synchronize(d.path());
    let out = cli(d.path(), &["--no-color", "run"]);
    assert!(out.status.success());
    assert!(!out.stdout.contains(&0x1b));
}
#[test]
// REQ-0234: an oversized input cannot silently produce a complete snapshot.
// REQ-0235: incomplete input blocks execution and produces ERROR.
// REQ-0238: incomplete comparisons report UNKNOWN without exact totals.
fn huge_file_gives_unknown_and_blocks_verification() {
    let d = project();
    let marker = tempdir().unwrap();
    let started = marker.path().join("started");
    support::check(
        d.path(),
        "must-not-start",
        &["pre-push"],
        &format!("touch '{}'\n", started.display()),
    );
    let f = fs::File::create(d.path().join("huge.bin")).unwrap();
    f.set_len(8 * 1024 * 1024 + 1).unwrap();
    let r = please::verify(d.path(), &Never).unwrap();
    assert_eq!(r.overall, Status::Error);
    assert!(!started.exists());
    support::assert_report_contract(&serde_json::to_value(&r).unwrap());
    let changes = git_snapshot::compare(&r.snapshot, &git_snapshot::capture(d.path()).unwrap(), 5);
    assert_eq!(changes.state, "UNKNOWN");
    assert_eq!(changes.total_files, None);
}
#[test]
// REQ-0233: verification leaves HEAD unchanged and does not create an index or branch.
fn no_root_detection_or_git_mutation() {
    let d = tempdir().unwrap();
    git(d.path(), &["init", "--quiet"]);
    please::init::init(d.path()).unwrap();
    support::configure_adapter(d.path());
    let head = fs::read(d.path().join(".git/HEAD")).unwrap();
    please::verify(d.path(), &Never).unwrap();
    assert_eq!(fs::read(d.path().join(".git/HEAD")).unwrap(), head);
    assert!(!d.path().join(".git/index").exists());
    assert!(!d.path().join(".git/refs/heads/master").exists());
    let nested = d.path().join("nested");
    fs::create_dir(&nested).unwrap();
    assert!(please::init::init(&nested).is_err());
    assert!(!nested.join(".please").exists());
}
#[cfg(unix)]
#[test]
fn timeout_output_capture_and_shell_are_explicit() {
    let start = Instant::now();
    let output = process::run(
        support::command("sh").args(["-c", "printf before; while :; do :; done"]),
        Duration::from_millis(80),
    )
    .unwrap();
    assert!(output.timed_out);
    assert!(start.elapsed() < Duration::from_secs(3));
    assert_eq!(output.stdout.text, "before");
    let output = process::run(
        support::command("sh").args(["-c", "yes x | head -c 200000; yes y | head -c 200000 >&2"]),
        Duration::from_secs(10),
    )
    .unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout.text.len(), process::OUTPUT_LIMIT);
    assert_eq!(output.stdout.total_bytes, 200000);
    assert!(output.stdout.truncated);
    assert_eq!(output.stderr.total_bytes, 200000);
    assert!(output.stderr.truncated);
    let d = project();
    fs::write(
        path(d.path(), "checks/always-ok/index.sh"),
        "#!/bin/sh\nwhile :; do :; done\n",
    )
    .unwrap();
    replace(
        d.path(),
        "checks/always-ok/rule.toml",
        "timeout_seconds = 30",
        "timeout_seconds = 1",
    );
    let r = please::verify(d.path(), &Never).unwrap();
    assert_eq!(r.results[0].status, Status::Error);
    assert!(r.results[0].reason.contains("timed out"));
}

#[cfg(unix)]
#[test]
// REQ-0236: a check that changes input blocks the enabled review before its adapter call.
// REQ-0262: tree stability errors remain explicit results even when the check succeeds.
fn changing_files_during_checks_blocks_pass() {
    // REQ-0237: a check that changes project files prevents a successful result.
    let d = project();
    enable(d.path());
    fs::write(
        path(d.path(), "checks/always-ok/index.sh"),
        "#!/bin/sh\nprintf changed > created.txt\n",
    )
    .unwrap();
    let r = please::verify(d.path(), &Never).unwrap();
    assert_eq!(r.overall, Status::Error);
    assert!(r.results.iter().any(|r| r.id == "please-tree-stability"));
}
#[test]
fn terminal_strips_control_sequences_from_untrusted_values() {
    assert_eq!(terminal::clean("evil\x1b[31m\nrow"), "evil [31m row");
}

#[test]
// REQ-0239: staging preserves freshness while a changed HEAD makes it stale.
fn staging_alone_does_not_stale_content_and_head_changes_do() {
    let d = project();
    fs::write(d.path().join("file.txt"), "content\n").unwrap();
    let snapshot = git_snapshot::capture(d.path()).unwrap();
    git(d.path(), &["add", "file.txt"]);
    let after = git_snapshot::capture(d.path()).unwrap();
    assert_eq!(git_snapshot::compare(&snapshot, &after, 5).state, "CURRENT");
    let mut changed_head = after.clone();
    changed_head.head = Some("different-head".into());
    let changes = git_snapshot::compare(&snapshot, &changed_head, 5);
    assert_eq!(changes.state, "STALE");
    assert_eq!(changes.total_files, Some(0));
    assert!(changes.head_changed);
}
#[cfg(unix)]
#[test]
fn unusual_paths_and_binary_files_are_fingerprinted() {
    let d = project();
    for name in [
        "space name.txt",
        "newline\nname.txt",
        "back\\slash.txt",
        ".please/back\\slash.md",
    ] {
        fs::write(d.path().join(name), b"\x00\xffbinary").unwrap();
    }
    let before = git_snapshot::capture(d.path()).unwrap();
    assert!(before.complete);
    for name in [
        "space name.txt",
        "newline\nname.txt",
        "back\\slash.txt",
        ".please/back\\slash.md",
    ] {
        assert!(before.files.contains_key(name), "{name:?}");
        fs::write(d.path().join(name), b"\x01\xffbinary").unwrap();
    }
    let changes = git_snapshot::compare(&before, &git_snapshot::capture(d.path()).unwrap(), 5);
    assert_eq!(changes.total_files, Some(4));
}
#[test]
fn prompt_has_no_safety_instructions() {
    // REQ-0093: adapter instructions impose no read-only restriction on repository access.
    // REQ-0101: review instructions require FAIL when PASS cannot be established.
    let d = project();
    enable(d.path());
    let project = config::load(d.path()).unwrap();
    let snapshot = git_snapshot::capture(d.path()).unwrap();
    let active: Vec<_> = project.reviews.iter().collect();
    let changes = please::change_set::ChangeSet::empty();
    let mut input =
        please::review_input::PreparedReviewInput::prepare(&project, &active, &snapshot, &changes)
            .unwrap();
    input.scopes.get_mut(&active[0].id).unwrap().change_set =
        Some("malicious change set content".into());
    let request = Request {
        execution_point: "pre-push",
        parameters: &Default::default(),
        project: &project,
        reviews: &active,
        snapshot: &snapshot,
        input: &input,
    };
    let text = please::adapter::prompt(&request).unwrap();
    assert!(!text.contains("Truncated"));
    assert!(text.contains("<change-set>"));
    for forbidden in ["untrusted", "read-only", "privileged", "Do not edit"] {
        assert!(!text.contains(forbidden), "{forbidden}");
    }
    assert!(text.contains("FAIL when PASS cannot be established"));
    assert!(text.contains("product-consistency"));
    // A review that is not the owner of the shared change set must not receive it.
    let mut other = project.reviews[0].clone();
    other.id = "other".into();
    let active = [&other];
    let request = Request {
        reviews: &active,
        ..request
    };
    let text = please::adapter::prompt(&request).unwrap();
    assert!(!text.contains("malicious change set content"));
    assert!(!text.contains("<change-set>"));
}

#[test]
fn oversized_change_set_ends_the_review_as_error_without_an_adapter_call() {
    // REQ-0097: an oversized change set produces ERROR before the adapter is invoked.
    let d = project();
    enable(d.path());
    fs::write(d.path().join("big.txt"), "x\n".repeat(20 * 1024)).unwrap();
    git(d.path(), &["add", "big.txt"]);
    let project = config::load(d.path()).unwrap();
    let snapshot = git_snapshot::capture(d.path()).unwrap();
    let changes = please::change_set::collect(d.path(), "pre-commit", None).unwrap();
    let mut with_change_set = project.reviews[0].clone();
    with_change_set.include_change_set = true;
    let active = [&with_change_set];
    let items =
        please::reviews::execute(&project, &active, &snapshot, &changes, &Never, "pre-commit");
    assert_eq!(items.len(), active.len());
    assert!(items.iter().all(|i| i.status == Status::Error));
    assert!(items[0].reason.contains("change set exceeds"));
    let prepared = please::review_input::PreparedReviewInput::prepare(
        &project,
        &[&project.reviews[0]],
        &snapshot,
        &changes,
    )
    .unwrap();
    assert!(prepared.scopes[&project.reviews[0].id].change_set.is_none());
}

#[test]
// REQ-0272: disabled checks and reviews count as defined rules.
fn no_rules_message_requires_both_checks_and_reviews_to_be_undefined() {
    let message = "No rules defined yet.";
    for (keep_check, keep_review) in [(false, false), (true, false), (false, true)] {
        let d = project();
        fs::remove_dir_all(path(d.path(), "checks/always-ok")).unwrap();
        if !keep_check {
            fs::remove_dir_all(path(d.path(), "checks/deliberately-failing")).unwrap();
        }
        if !keep_review {
            fs::remove_dir_all(path(d.path(), "reviews/product-consistency")).unwrap();
        }
        let output = cli(d.path(), &["run"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert_eq!(
            stdout.lines().filter(|line| *line == message).count(),
            usize::from(!keep_check && !keep_review)
        );
        // Defined but disabled rules stay out of the results, but are not absent rules.
        assert!(report::read(d.path()).unwrap().results.is_empty());
    }
}

#[test]
fn help_lists_only_supported_commands_without_redundant_examples() {
    let d = tempdir().unwrap();
    for args in [vec!["--help"], vec!["-h"], vec!["help"]] {
        let output = cli(d.path(), &args);
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.starts_with(
            "Please Behave\n\nEnforce development rules across coding agents and workflows.\n"
        ));
        assert!(!text.contains("Der Harness ohne 'r'"));
        assert!(!text.contains("Examples:"));
        let commands = text
            .split_once("Commands:\n")
            .unwrap()
            .1
            .split_once("\nOptions:")
            .unwrap()
            .0;
        let rows: Vec<_> = commands
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect();
        let phrases = ["init", "sync", "trust", "run", "status", "help"];
        assert_eq!(rows.len(), phrases.len());
        assert_eq!(rows.last(), Some(&"  help    Show help"));
        for (row, phrase) in rows.iter().zip(phrases) {
            let description = row.trim_start().strip_prefix(phrase).unwrap();
            assert!(description.starts_with("  "));
            assert!(!description.trim().is_empty());
        }
        assert!(
            text.contains("--no-color") && text.contains("--help") && text.contains("--version")
        );
    }
    assert!(Cli::try_parse_from(["please", "help"]).is_ok());
}

#[test]
// REQ-0277: unknown command invocations reproduce top-level help.
fn unknown_commands_show_top_level_help_without_execution_or_escalation() {
    let d = tempdir().unwrap();
    let help = cli(d.path(), &["help"]);
    assert!(help.status.success());
    for _ in 0..10 {
        for args in [
            vec!["quatsch"],
            vec!["mach", "falsch"],
            vec!["was", "ist"],
            vec!["--no-color", "quatsch"],
        ] {
            let output = cli(d.path(), &args);
            assert_eq!(output.status.code(), Some(2), "{args:?}");
            assert!(output.stdout.is_empty());
            assert_eq!(output.stderr, help.stdout, "{args:?}");
        }
    }
    assert_eq!(fs::read_dir(d.path()).unwrap().count(), 0);
}

#[test]
fn invalid_arguments_and_options_keep_parser_diagnostics() {
    let d = tempdir().unwrap();
    for args in [
        vec!["init", "extra"],
        vec!["--unknown"],
        vec!["run", "--progress=unknown"],
        vec!["run", "--at"],
        vec!["run", "--json"],
    ] {
        let output = cli(d.path(), &args);
        assert_eq!(output.status.code(), Some(2), "{args:?}");
        assert!(output.stdout.is_empty());
        let stderr = String::from_utf8(output.stderr).unwrap();
        assert!(stderr.starts_with("error:"), "{args:?}: {stderr}");
        assert!(!stderr.contains("Commands:"));
    }
}

#[test]
fn colors_preserve_help_layout_and_use_semantic_status_colors() {
    let help = Cli::command().render_help().to_string();
    assert_eq!(terminal::render_help(&help, false), help);
    let colored = terminal::render_help(&help, true);
    assert!(colored.contains("\x1b[36minit\x1b[0m"));
    assert!(colored.contains("\x1b[1mCommands:\x1b[0m"));
    assert!(colored.contains("\x1b[36m--help\x1b[0m"));
    assert!(colored.contains("\x1b[36m--checks-only\x1b[0m"));
    assert_eq!(
        terminal::paint("PASS", terminal::Tone::Success, true),
        "\x1b[32mPASS\x1b[0m"
    );
    assert_eq!(
        terminal::paint("FAIL", terminal::Tone::Failure, true),
        "\x1b[31mFAIL\x1b[0m"
    );
    let changes = git_snapshot::Changes {
        state: "CURRENT".into(),
        total_files: Some(0),
        sample_paths: vec![],
        head_changed: false,
        line_counts: None,
    };
    assert!(terminal::render_changes(&changes, true).contains("\x1b[32mCURRENT\x1b[0m"));
    assert!(!terminal::render_changes(&changes, false).contains('\x1b'));
}
#[test]
// REQ-0273: redirected stdout and stderr omit ANSI despite forced color.
fn piped_help_and_errors_never_include_ansi_even_if_color_is_forced() {
    let d = tempdir().unwrap();
    for args in [
        vec!["--help"],
        vec!["help"],
        vec!["--version"],
        vec!["quatsch"],
        vec!["--no-color", "quatsch"],
    ] {
        let output = support::command(env!("CARGO_BIN_EXE_please"))
            .args(&args)
            .current_dir(d.path())
            .env_remove("NO_COLOR")
            .env("CLICOLOR_FORCE", "1")
            .output()
            .unwrap();
        assert!(!output.stdout.contains(&0x1b), "stdout: {args:?}");
        assert!(!output.stderr.contains(&0x1b), "stderr: {args:?}");
    }
}

#[test]
fn review_summary_is_visible_for_pass_and_fail_while_details_stay_separate() {
    for status in ["PASS", "FAIL"] {
        let d = project();
        enable(d.path());
        let mut value = verdict("product-consistency", status);
        value["reason"] =
            serde_json::json!("Settings navigation exposes labels and keyboard focus.");
        value["findings"][0]["message"] = serde_json::json!(
            "DETAIL_ONLY: Full explanation of a concrete trigger, consequence, and inspected evidence."
        );
        let adapter = mock(vec![value]);
        let report = please::verify(d.path(), &adapter).unwrap();
        assert_eq!(adapter.calls.get(), 1);
        let short = terminal::render(&report, false);
        assert!(short.contains("SUMMARY"));
        assert!(short.contains("Settings navigation exposes labels and keyboard focus."));
        assert!(!short.contains("DETAIL_ONLY") && !short.contains(".please/README.md:1"));
        assert!(short.contains("please status --report 1 --details"));
        let detailed = terminal::render_details(&report, false);
        assert!(detailed.contains("DETAIL_ONLY") && detailed.contains(".please/README.md:1"));
        assert!(
            report::read(d.path()).unwrap().results[1].findings[0]
                .message
                .contains("DETAIL_ONLY")
        );
    }
}
#[test]
fn adapter_summary_must_be_short_nonempty_and_single_line() {
    let d = project();
    enable(d.path());
    let project = config::load(d.path()).unwrap();
    let snapshot = git_snapshot::capture(d.path()).unwrap();
    let active: Vec<_> = project.reviews.iter().filter(|r| r.enabled).collect();
    for reason in [
        " ".into(),
        "a".repeat(121),
        "first\nsecond".into(),
        "first\tsecond".into(),
        "first\u{2028}second".into(),
    ] {
        let mut value = verdict("product-consistency", "PASS");
        value["reason"] = serde_json::json!(reason);
        let json = serde_json::json!({"reviews":[value]}).to_string();
        assert!(please::reviews::parse(&project, &active, &snapshot, &json).is_err());
    }
    let mut value = verdict("product-consistency", "PASS");
    value["reason"] = serde_json::json!("ä".repeat(120));
    assert!(
        please::reviews::parse(
            &project,
            &active,
            &snapshot,
            &serde_json::json!({"reviews":[value]}).to_string()
        )
        .is_ok()
    );
}
#[cfg(unix)]
#[test]
fn detailed_status_reads_saved_findings_without_another_adapter_call() {
    let d = project();
    enable(d.path());
    let mut value = verdict("product-consistency", "FAIL");
    value["reason"] = serde_json::json!("One keyboard accessibility issue found.");
    value["findings"][0]["message"] =
        serde_json::json!("DETAIL_ONLY: keyboard focus skips a visible control.");
    let adapter = mock(vec![value]);
    please::verify(d.path(), &adapter).unwrap();
    let bytes_before = fs::read(path(d.path(), "reports/1.json")).unwrap();
    let fake = tempdir().unwrap();
    let executable = fake.path().join("fake-adapter");
    write_executable(
        &executable,
        "#!/bin/sh\nprintf 'called' > \"$TEST_ADAPTER_MARKER\"\nexit 90\n",
    )
    .unwrap();
    let marker = fake.path().join("adapter-called");
    let env_path = std::env::join_paths(
        std::iter::once(fake.path().to_path_buf())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    for detailed in [false, true] {
        let mut command = support::command(env!("CARGO_BIN_EXE_please"));
        command.args(["status"]);
        if detailed {
            command.arg("--details");
        }
        let output = command
            .current_dir(d.path())
            .env("PATH", &env_path)
            .env("TEST_ADAPTER_MARKER", &marker)
            .env("NO_COLOR", "1")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("One keyboard accessibility issue found."));
        assert_eq!(text.contains("DETAIL_ONLY"), detailed);
        assert!(text.contains("CURRENT"));
        assert!(!marker.exists());
    }
    assert_eq!(
        fs::read(path(d.path(), "reports/1.json")).unwrap(),
        bytes_before
    );
}
#[test]
fn compact_report_bounds_execution_error_reasons_without_losing_saved_details() {
    let d = project();
    let mut report = please::verify(d.path(), &Never).unwrap();
    report.results[0].status = Status::Fail;
    report.results[0].reason = format!("first line\n{}", "x".repeat(4000));
    let compact = terminal::render(&report, false);
    let row = compact
        .lines()
        .find(|line| line.starts_with("check"))
        .unwrap();
    assert!(row.ends_with('…'));
    assert_eq!(
        compact
            .lines()
            .filter(|line| line.contains("first line"))
            .count(),
        1
    );
    assert!(terminal::render_details(&report, false).contains(&"x".repeat(4000)));
}

#[test]
fn progress_numbers_only_enabled_rules_and_runs_one_review_per_call() {
    use std::cell::RefCell;
    struct Observing<'a> {
        events: &'a RefCell<Vec<String>>,
        calls: Cell<u32>,
    }
    impl Adapter for Observing<'_> {
        fn review(&self, request: &Request<'_>) -> Result<String> {
            self.calls.set(self.calls.get() + 1);
            assert_eq!(request.reviews.len(), 1);
            let events = self.events.borrow();
            assert!(events.iter().any(|line| line.starts_with("[2/4] PASS")));
            let running = if request.reviews[0].id == "second" {
                "[4/4]"
            } else {
                "[3/4]"
            };
            assert!(
                events
                    .iter()
                    .any(|line| line.starts_with(&format!("{running} RUNNING")))
            );
            assert!(
                !events
                    .iter()
                    .any(|line| line.starts_with(&format!("{running} PASS")))
            );
            Ok(serde_json::json!({"reviews":request.reviews.iter().map(|review| verdict(&review.id,if review.id=="second" {"FAIL"} else {"PASS"})).collect::<Vec<_>>()} ).to_string())
        }
    }
    let d = project();
    enable(d.path());
    support::check(d.path(), "second-check", &["pre-push"], "git --version");
    replace(
        d.path(),
        "checks/second-check/rule.toml",
        "Test check",
        "Second deterministic check",
    );
    let original =
        fs::read_to_string(path(d.path(), "reviews/product-consistency/rule.toml")).unwrap();
    fs::write(
        support::review_path(d.path(), "second"),
        original.replace("id = \"product-consistency\"", "id = \"second\""),
    )
    .unwrap();
    let events = RefCell::new(Vec::new());
    let adapter = Observing {
        events: &events,
        calls: Cell::new(0),
    };
    let report = please::verify_project_with_progress(
        config::load(d.path()).unwrap(),
        &adapter,
        &mut |event| {
            events
                .borrow_mut()
                .push(terminal::render_progress(&event, false))
        },
    )
    .unwrap();
    assert_eq!(report.overall, Status::Fail);
    assert_eq!(adapter.calls.get(), 2);
    let output = events.borrow().concat();
    for number in 1..=4 {
        assert!(output.contains(&format!("[{number}/4] RUNNING")));
    }
    assert!(output.contains("[1/4] PASS    · always-ok"));
    assert!(output.contains("[4/4] FAIL    · second"));
    assert!(output.contains("Second deterministic check"));
    assert!(!output.contains("adapter call"));
    assert!(!output.contains("deliberately-failing"));
    assert!(output.contains("Documented expectations inspected"));
}
#[test]
fn failed_checks_report_skipped_reviews_without_claiming_they_started() {
    let d = project();
    enable(d.path());
    replace(
        d.path(),
        "checks/deliberately-failing/rule.toml",
        "enabled = false",
        "enabled = true",
    );
    let mut output = String::new();
    let report = please::verify_project_with_progress(
        config::load(d.path()).unwrap(),
        &Never,
        &mut |event| output.push_str(&terminal::render_progress(&event, false)),
    )
    .unwrap();
    assert_eq!(report.overall, Status::Fail);
    assert!(output.contains("[1/3] PASS    · always-ok"));
    assert!(output.contains("[2/3] FAIL    · deliberately-failing"));
    assert!(output.contains("NOT_RUN · product-consistency"));
    assert!(!output.contains("RUNNING review") && !output.contains("Adapter:"));
    assert!(
        terminal::render_outcome(&report, 2000, false)
            .starts_with("FAIL · 3 rules · 1 failed · 1 not executed · 2.0 s · report 1\n")
    );
    assert!(path(d.path(), "reports/1.json").is_file());
}
#[cfg(unix)]
#[test]
// REQ-0263: stderr exposes a start before completion and the final result after completion.
fn cli_flushes_rule_start_while_the_check_is_still_running_and_keeps_raw_output_private() {
    use std::io::{BufRead, BufReader, Read};
    use std::process::Stdio;
    let d = project();
    let external = tempdir().unwrap();
    support::check(
        d.path(),
        "always-ok",
        &["pre-push"],
        "printf RAW_CHECK_OUTPUT; while [ ! -f \"$TEST_RELEASE\" ]; do sleep 0.02; done",
    );
    replace(
        d.path(),
        "checks/always-ok/rule.toml",
        "Test check",
        "Wait for the test release signal",
    );
    let marker = external.path().join("release");
    synchronize(d.path());
    let mut child = support::command(env!("CARGO_BIN_EXE_please"))
        .args(["run"])
        .current_dir(d.path())
        .env("TEST_RELEASE", &marker)
        .env("NO_COLOR", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut reader = BufReader::new(child.stderr.take().unwrap());
    let mut progress = String::new();
    loop {
        let mut line = String::new();
        assert!(
            reader.read_line(&mut line).unwrap() > 0,
            "missing start event: {progress}"
        );
        progress.push_str(&line);
        if line.starts_with("[1/1] RUNNING") {
            break;
        }
    }
    assert!(
        child.try_wait().unwrap().is_none(),
        "progress arrived only after the process ended"
    );
    assert!(!path(d.path(), "reports/1.json").exists());
    assert!(progress.contains("Wait for the test release signal"));
    fs::write(marker, "continue").unwrap();
    reader.read_to_string(&mut progress).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{progress}");
    assert!(progress.contains("[1/1] PASS"));
    assert!(!progress.contains("RAW_CHECK_OUTPUT"));
    assert!(
        !String::from_utf8(output.stdout)
            .unwrap()
            .contains("RAW_CHECK_OUTPUT")
    );
    assert_eq!(
        report::read(d.path()).unwrap().results[0]
            .stdout
            .as_ref()
            .unwrap()
            .text,
        "RAW_CHECK_OUTPUT"
    );
}
#[test]
// REQ-0274: untrusted rule descriptions cannot inject escapes or extra progress rows.
fn progress_uses_semantic_colors_and_sanitizes_rule_descriptions() {
    let event = please::Progress::RuleStarted {
        number: 1,
        total: 2,
        kind: "check",
        id: "test",
        name: "Tests",
        description: "Tests\nwith\x1bescape",
    };
    let plain = terminal::render_progress(&event, false);
    assert_eq!(plain.lines().count(), 1);
    assert!(!plain.contains('\x1b'));
    assert!(terminal::render_progress(&event, true).contains("\x1b[36mRUNNING"));
    let item = report::Item::simple(
        "test",
        "check",
        Status::Fail,
        "Command exited unsuccessfully".into(),
    );
    let completed = please::Progress::RuleFinished {
        number: 1,
        total: 2,
        item: &item,
        name: "Tests",
        description: "Test check",
    };
    assert!(terminal::render_progress(&completed, true).contains("\x1b[31mFAIL"));
}

#[test]
fn usage_is_summed_across_review_calls() {
    // REQ-0121: usage from two adapter calls is summed and persisted.
    use please::{adapter::Response, report::TokenUsage};
    struct Metered {
        calls: Cell<u32>,
    }
    impl Adapter for Metered {
        fn review(&self, _: &Request<'_>) -> Result<String> {
            panic!("the usage-aware entry point must be used")
        }
        fn review_with_usage(&self, request: &Request<'_>) -> Response {
            self.calls.set(self.calls.get() + 1);
            assert_eq!(request.reviews.len(), 1);
            Response {
                verdict: Ok(serde_json::json!({"reviews": request.reviews.iter().map(|r| verdict(&r.id, "PASS")).collect::<Vec<_>>()} ).to_string()),
                usage: Some(TokenUsage {
                    input_tokens: Some(1000), cached_input_tokens: Some(800),
                    output_tokens: Some(200), reasoning_output_tokens: Some(50),
                    cache_write_input_tokens: None,
                }),
                output: None,
            }
        }
    }
    let d = project();
    enable(d.path());
    let metadata =
        fs::read_to_string(path(d.path(), "reviews/product-consistency/rule.toml")).unwrap();
    fs::write(
        support::review_path(d.path(), "second"),
        metadata.replace("id = \"product-consistency\"", "id = \"second\""),
    )
    .unwrap();
    let adapter = Metered {
        calls: Cell::new(0),
    };
    let r = please::verify(d.path(), &adapter).unwrap();
    assert_eq!(adapter.calls.get(), 2);
    assert_eq!(r.overall, Status::Pass);
    assert_eq!(r.token_usage, Some(2400));
    assert_eq!(
        terminal::render(&r, false)
            .matches("Adapter tokens:")
            .count(),
        1
    );
    assert_eq!(
        report::read(d.path()).unwrap().adapter_usage,
        r.adapter_usage
    );
}

fn copy_executable(source: &Path, executable: &Path) {
    #[cfg(unix)]
    {
        // A concurrent fork can inherit fs::copy's writable descriptor until exec,
        // causing ETXTBSY even after the copying thread closes it. Copy in a child
        // process so no writer for this executable exists in the test process.
        let output = support::command("cp")
            .arg(source)
            .arg(executable)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "cannot copy executable fixture: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    #[cfg(not(unix))]
    fs::copy(source, executable).unwrap();
}

#[cfg(unix)]
fn write_executable(path: &Path, contents: impl AsRef<[u8]>) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    // Rust may hold writable handles to this staging file, but never executes it.
    let source = tempfile::NamedTempFile::new()?;
    fs::write(source.path(), contents)?;
    copy_executable(source.path(), path);
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[test]
fn checks_only_never_calls_adapter_and_counts_only_checks_even_on_failure() {
    use please::{Progress, VerificationMode, verify_project_with_mode};
    for case in ["pass", "fail", "error", "no-checks"] {
        let d = project();
        enable(d.path());
        if case == "fail" {
            replace(
                d.path(),
                "checks/deliberately-failing/rule.toml",
                "enabled = false",
                "enabled = true",
            );
        } else if case == "error" {
            replace(
                d.path(),
                "checks/always-ok/rule.toml",
                if cfg!(windows) {
                    r#"command = "cmd /C @rule/index.cmd""#
                } else {
                    r#"command = "sh @rule/index.sh""#
                },
                r#"command = "please-nonexistent-executable-for-test""#,
            );
        } else if case == "no-checks" {
            replace(
                d.path(),
                "checks/always-ok/rule.toml",
                "enabled = true",
                "enabled = false",
            );
        }
        let mut plan = None;
        let mut review_event = false;
        let report = verify_project_with_mode(
            config::load(d.path()).unwrap(),
            &Never,
            VerificationMode::ChecksOnly,
            &mut |event| match event {
                Progress::Plan { total } => plan = Some(total),
                Progress::RuleStarted { kind: "review", .. } => review_event = true,
                _ => {}
            },
        )
        .unwrap();
        assert!(!review_event);
        assert!(report.checks_only);
        let reviews: Vec<_> = report
            .results
            .iter()
            .filter(|item| item.kind == "review")
            .collect();
        assert_eq!(reviews.len(), 1);
        assert!(reviews.iter().all(|item| item.status == Status::NotRun));
        assert_eq!(
            report.overall,
            match case {
                "fail" => Status::Fail,
                "error" => Status::Error,
                _ => Status::Pass,
            }
        );
        assert_eq!(
            plan,
            Some(match case {
                "fail" => 2,
                "no-checks" => 0,
                _ => 1,
            })
        );
        assert_eq!(report.token_usage, None);
        assert_eq!(report.adapter_usage, None);
        assert!(report::read(d.path()).unwrap().checks_only);
        let text = terminal::render(&report, false);
        assert!(text.contains("Checks only") && text.contains("no adapter calls"));
        if case == "no-checks" {
            assert!(text.contains("No active checks"));
        }
    }
}

#[test]
fn run_and_checks_only_are_scoped_to_verification() {
    use please::cli::Words;
    assert!(Cli::try_parse_from(["please", "mach", "mal"]).is_err());
    for enabled in [false, true] {
        let mut args = vec!["please", "run"];
        if enabled {
            args.push("--checks-only");
        }
        let parsed = Cli::try_parse_from(args).unwrap();
        assert!(matches!(parsed.command, Words::Run { checks_only, .. } if checks_only == enabled));
    }
    assert!(Cli::try_parse_from(["please", "init", "--checks-only"]).is_err());
    assert!(
        Cli::command()
            .render_help()
            .to_string()
            .contains("--checks-only")
    );
}

#[cfg(unix)]
#[test]
fn checks_only_cli_skips_enabled_adapter_reviews_and_status_keeps_the_scope() {
    let d = project();
    enable(d.path());
    let fake = tempdir().unwrap();
    let executable = fake.path().join("fake-adapter");
    let marker = fake.path().join("called");
    write_executable(
        &executable,
        "#!/bin/sh\nprintf called > \"$TEST_ADAPTER_MARKER\"\nexit 23\n",
    )
    .unwrap();
    let search_path = std::env::join_paths(
        std::iter::once(fake.path().to_path_buf())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let run = |args: &[&str]| {
        support::command(env!("CARGO_BIN_EXE_please"))
            .args(args)
            .current_dir(d.path())
            .env("PATH", &search_path)
            .env("TEST_ADAPTER_MARKER", &marker)
            .env("NO_COLOR", "1")
            .output()
            .unwrap()
    };
    synchronize(d.path());
    let output = run(&["run", "--checks-only"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!marker.exists());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("Checks only"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("Adapter:"));
    let saved = fs::read(path(d.path(), "reports/1.json")).unwrap();
    for args in [vec!["status"], vec!["status", "--details"]] {
        let status = run(&args);
        assert!(status.status.success());
        assert!(String::from_utf8_lossy(&status.stdout).contains("Checks only"));
        assert_eq!(fs::read(path(d.path(), "reports/1.json")).unwrap(), saved);
        assert!(!marker.exists());
    }
    // The flag is per invocation; the next ordinary run still invokes enabled reviews.
    let full = run(&["run"]);
    assert_eq!(full.status.code(), Some(2));
    assert!(marker.exists());
    assert!(!report::read(d.path()).unwrap().checks_only);
}

#[test]
fn review_context_sections_keep_source_lines_merge_overlap_and_ignore_code_headings() {
    // REQ-0095: ATX selection includes subsections, ignores fenced headings and rejects missing or ambiguous headings.
    // REQ-0096: selected context retains the original source line range.
    let d = project();
    let document = "# Policy\nIntro\n## Scope\nRequired\n```md\n## Scope\n```\n### Detail\nMore\n## Other\nUnrelated\n";
    fs::write(d.path().join("Policy.md"), document).unwrap();
    let metadata = path(d.path(), "reviews/product-consistency/rule.toml");
    let original = fs::read_to_string(&metadata).unwrap();
    fs::write(&metadata, format!("{original}\n[[review.context]]\npath = \"./Policy.md\"\nheadings = [\"Scope\", \"Detail\"]\n")).unwrap();
    let p = config::load(d.path()).unwrap();
    let excerpts =
        please::review_context::collect(&p.root, &[&p.reviews[0], &p.reviews[0]]).unwrap();
    assert_eq!(excerpts.len(), 1);
    assert_eq!((excerpts[0].start_line, excerpts[0].end_line), (3, 9));
    assert!(excerpts[0].text.contains("Required") && excerpts[0].text.contains("More"));
    assert!(!excerpts[0].text.contains("Unrelated"));
    // Missing and duplicate real headings fail even for disabled reviews.
    fs::write(
        &metadata,
        format!("{original}\n[[review.context]]\npath = \"Policy.md\"\nheadings = [\"Missing\"]\n"),
    )
    .unwrap();
    assert!(config::load(d.path()).is_err());
    fs::write(d.path().join("Policy.md"), "## Scope\nOne\n## Scope\nTwo\n").unwrap();
    fs::write(
        &metadata,
        format!("{original}\n[[review.context]]\npath = \"Policy.md\"\nheadings = [\"Scope\"]\n"),
    )
    .unwrap();
    assert!(config::load(d.path()).is_err());
    // A whole-file entry must not hide an invalid named section in another entry.
    fs::write(&metadata, format!("{original}\n[[review.context]]\npath = \"Policy.md\"\n[[review.context]]\npath = \"Policy.md\"\nheadings = [\"Missing\"]\n")).unwrap();
    assert!(config::load(d.path()).is_err());
}

#[test]
fn review_context_rejects_escaping_missing_and_excessive_sources_without_an_adapter() {
    for extra in [
        "files = [\"../outside.swift\"]",
        "files = [\"missing.swift\"]",
        "files = [\".please\"]",
        "[[review.context]]\npath = \"../outside.md\"",
        "[[review.context]]\npath = \"missing.md\"",
        "[[review.context]]\npath = \"Policy.md\"",
    ] {
        let d = project();
        fs::write(d.path().join("Policy.md"), "x".repeat(65537)).unwrap();
        let file = path(d.path(), "reviews/product-consistency/rule.toml");
        let original = fs::read_to_string(&file).unwrap();
        fs::write(file, format!("{original}\n{extra}\n")).unwrap();
        assert!(config::load(d.path()).is_err(), "{extra}");
    }
}

#[test]
fn triggers_limit_the_scope_of_content_and_change_set() {
    // REQ-0127: initial scope excludes changed files that do not match triggers.
    // REQ-0128: the initial prompt includes the current content of matching files.
    // REQ-0129: the initial change set includes scoped changes and excludes unrelated ones.
    let d = project();
    fs::write(d.path().join("entry.swift"), "old entry\n").unwrap();
    fs::write(d.path().join("unrelated.swift"), "old unrelated\n").unwrap();
    git(d.path(), &["add", "entry.swift", "unrelated.swift"]);
    support::commit_all(d.path());
    fs::write(d.path().join("entry.swift"), "new entry\n").unwrap();
    fs::write(d.path().join("unrelated.swift"), "UNRELATED_SECRET\n").unwrap();
    fs::write(d.path().join("new.swift"), "new staged entry\n").unwrap();
    git(
        d.path(),
        &["add", "entry.swift", "unrelated.swift", "new.swift"],
    );
    fs::write(
        d.path().join("Policy.md"),
        "# Relevant\nRelevant requirement\n# Other\nUNRELATED_POLICY_SECRET\n",
    )
    .unwrap();
    let metadata = path(d.path(), "reviews/product-consistency/rule.toml");
    let original = fs::read_to_string(&metadata).unwrap();
    fs::write(metadata, format!("{}\n[review.trigger]\nchanged_files = [\"entry.swift\", \"new.swift\"]\n[[review.context]]\npath = \"Policy.md\"\nheadings = [\"Relevant\"]\n", original.replace("include_change_set = false", "include_change_set = true"))).unwrap();
    let p = config::load(d.path()).unwrap();
    let snapshot = git_snapshot::capture(d.path()).unwrap();
    let changes = please::change_set::collect(d.path(), "pre-commit", None).unwrap();
    let active = [&p.reviews[0]];
    let input =
        please::review_input::PreparedReviewInput::prepare(&p, &active, &snapshot, &changes)
            .unwrap();
    let request = Request {
        execution_point: "pre-commit",
        parameters: &Default::default(),
        project: &p,
        reviews: &active,
        snapshot: &snapshot,
        input: &input,
    };
    let scope = &input.scopes[&p.reviews[0].id];
    let paths: Vec<_> = scope.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["entry.swift", "new.swift"]);
    let change_set = scope.change_set.as_deref().unwrap();
    assert!(change_set.contains("new entry") && !change_set.contains("UNRELATED_SECRET"));
    let prompt = please::adapter::prompt(&request).unwrap();
    assert!(prompt.contains("new staged entry") && prompt.contains("Relevant requirement"));
    assert!(!prompt.contains("UNRELATED_POLICY_SECRET") && !prompt.contains("unrelated.swift"));
    assert!(prompt.contains("not an exhaustive boundary"));
    assert!(prompt.contains("ONCE"));
}

#[test]
fn yes_is_explicit_and_scoped_to_sync() {
    use please::cli::Words;
    for (args, expected) in [
        (vec!["please", "sync"], false),
        (vec!["please", "sync", "--yes"], true),
    ] {
        assert!(
            matches!(Cli::try_parse_from(args).unwrap().command, Words::Sync { yes } if yes == expected)
        );
    }
    for command in ["run", "init", "status"] {
        let mut args = vec!["please"];
        args.extend(command.split_whitespace());
        args.push("--yes");
        assert!(Cli::try_parse_from(args).is_err());
    }
    let help = Cli::command()
        .find_subcommand_mut("sync")
        .unwrap()
        .render_help()
        .to_string();
    assert!(help.contains("--yes"));
}

#[test]
fn a_counter_missing_from_any_call_stays_unavailable() {
    use please::{adapter::Response, report::TokenUsage};
    struct Partial;
    impl Adapter for Partial {
        fn review(&self, _: &Request<'_>) -> Result<String> {
            panic!("the usage-aware entry point must be used")
        }
        fn review_with_usage(&self, request: &Request<'_>) -> Response {
            Response {
                verdict: Ok(serde_json::json!({"reviews": request.reviews.iter().map(|r| verdict(&r.id, "PASS")).collect::<Vec<_>>()} ).to_string()),
                usage: Some(TokenUsage {
                    input_tokens: Some(10), cached_input_tokens: None,
                    output_tokens: Some(5), reasoning_output_tokens: None,
                    cache_write_input_tokens: None,
                }),
                output: None,
            }
        }
    }
    let d = project();
    enable(d.path());
    let r = please::verify(d.path(), &Partial).unwrap();
    assert_eq!(r.overall, Status::Pass);
    assert_eq!(r.token_usage, Some(15));
    let text = terminal::render(&r, false);
    assert!(
        text.contains("15 total · 10 input (unavailable cached) · 5 output"),
        "{text}"
    );
}

#[cfg(unix)]
#[test]
// REQ-0231: snapshots identify tracked and untracked bytes, sizes, modes and HEAD, excluding Git and reports.
fn snapshot_records_file_identity_and_executable_changes() {
    use std::os::unix::fs::PermissionsExt;
    let d = project();
    let root = d.path();
    support::write(root, "tracked", "abc");
    git(root, &["add", "tracked"]);
    support::write(root, "untracked", "abc");
    fs::set_permissions(root.join("untracked"), fs::Permissions::from_mode(0o644)).unwrap();
    support::write(root, ".please/reports/extra.json", "excluded");
    let before = git_snapshot::capture(root).unwrap();
    assert!(before.complete);
    let head = support::command("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(head.status.success());
    assert_eq!(
        before.head.as_deref(),
        Some(String::from_utf8(head.stdout).unwrap().trim())
    );
    for file in ["tracked", "untracked"] {
        assert_eq!(before.files[file].bytes, 3);
        assert_eq!(
            before.files[file].sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }
    assert!(!before.files["untracked"].executable);
    assert!(
        before
            .files
            .keys()
            .all(|p| !p.starts_with(".git/") && !p.starts_with(".please/reports/"))
    );
    fs::set_permissions(root.join("untracked"), fs::Permissions::from_mode(0o755)).unwrap();
    let after = git_snapshot::capture(root).unwrap();
    assert!(after.files["untracked"].executable);
    let changes = git_snapshot::compare(&before, &after, 5);
    assert_eq!(changes.state, "STALE");
    assert_eq!(changes.sample_paths, ["untracked"]);
}

#[test]
// REQ-0254: invalid configuration preserves the previous report byte for byte.
fn invalid_configuration_does_not_replace_the_saved_report() {
    let d = project();
    please::verify(d.path(), &Never).unwrap();
    let report_path = path(d.path(), "reports/1.json");
    let before = fs::read(&report_path).unwrap();
    support::write(d.path(), ".please/config.toml", "unknown_setting = true\n");
    assert_eq!(cli(d.path(), &["run"]).status.code(), Some(2));
    assert_eq!(fs::read(report_path).unwrap(), before);
}

#[test]
// REQ-0253: every executing run saves its own complete report, including runs that end with FAIL or ERROR.
// REQ-0340: reports are numbered in ascending order, and numbers are never reused.
fn executed_runs_save_numbered_reports_and_leave_no_temporary_files() {
    let d = project();
    for (index, (script, expected)) in [
        ("exit 0\n", Status::Pass),
        ("exit 1\n", Status::Fail),
        ("exit 7\n", Status::Error),
        ("exit 0\n", Status::Pass),
    ]
    .into_iter()
    .enumerate()
    {
        support::check(d.path(), "outcome", &["check"], script);
        synchronize(d.path());
        let out = cli(d.path(), &["run", "--checks-only", "--at", "check"]);
        let number = index + 1;
        assert!(String::from_utf8_lossy(&out.stdout).contains(&format!("· report {number}\n")));
        assert_eq!(report::read(d.path()).unwrap().overall, expected);
        assert_eq!(report::read(d.path()).unwrap().number, number as u64);
        let mut files: Vec<_> = fs::read_dir(path(d.path(), "reports"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .filter(|name| name != ".gitignore")
            .collect();
        files.sort();
        let expected_files: Vec<String> = (1..=number).map(|n| format!("{n}.json")).collect();
        assert_eq!(files, expected_files);
    }
}

#[test]
// REQ-0341: only the newest configured number of reports is kept, and numbering continues after pruning.
fn report_retention_keeps_the_newest_reports_and_continues_numbering() {
    let d = project();
    let config = path(d.path(), "config.toml");
    let text = fs::read_to_string(&config).unwrap();
    assert!(text.contains("[report]\nretain_runs = 10"));
    fs::write(
        &config,
        text.replace("[report]\nretain_runs = 10", "[report]\nretain_runs = 2"),
    )
    .unwrap();
    synchronize(d.path());
    for _ in 0..4 {
        assert!(cli(d.path(), &["run"]).status.success());
    }
    let mut files: Vec<_> = fs::read_dir(path(d.path(), "reports"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name != ".gitignore")
        .collect();
    files.sort();
    assert_eq!(files, ["3.json", "4.json"]);
    assert_eq!(report::saved(d.path()).unwrap(), [3, 4]);
}

#[test]
// REQ-0342: status shows the newest report by default and an earlier retained report on request.
fn status_selects_reports_by_number() {
    let d = project();
    synchronize(d.path());
    support::check(d.path(), "outcome", &["check"], "exit 1\n");
    synchronize(d.path());
    cli(d.path(), &["run", "--checks-only", "--at", "check"]);
    support::check(d.path(), "outcome", &["check"], "exit 0\n");
    synchronize(d.path());
    cli(d.path(), &["run", "--checks-only", "--at", "check"]);
    let text = |args: &[&str]| String::from_utf8_lossy(&cli(d.path(), args).stdout).into_owned();
    assert!(text(&["status"]).contains("reports/2.json"));
    let earlier = cli(d.path(), &["status", "--report", "1"]);
    assert!(String::from_utf8_lossy(&earlier.stdout).contains("Report: .please/reports/1.json"));
    let missing = cli(d.path(), &["status", "--report", "7"]);
    assert_eq!(missing.status.code(), Some(3));
    assert!(missing.stdout.is_empty());
}

#[test]
// REQ-0343: a saved report stays complete and readable after its run context is removed.
fn saved_report_does_not_need_its_run_context() {
    let d = project();
    synchronize(d.path());
    assert!(cli(d.path(), &["run"]).status.success());
    let git_dir = String::from_utf8(
        std::process::Command::new("git")
            .args(["rev-parse", "--absolute-git-dir"])
            .current_dir(d.path())
            .output()
            .unwrap()
            .stdout,
    )
    .unwrap();
    let contexts = std::path::Path::new(git_dir.trim()).join("please/context");
    assert!(contexts.is_dir());
    fs::remove_dir_all(&contexts).unwrap();
    assert!(report::read(d.path()).is_ok());
    assert_eq!(cli(d.path(), &["status"]).status.code(), Some(0));
}

#[test]
// REQ-0256: the saved report identifies the input, repository and UTC run time.
// REQ-0257: the saved report retains results, elapsed durations, findings and configuration-error flags.
// REQ-0258: the saved plan retains required rule IDs and their blocking policy.
fn saved_report_preserves_run_identity_results_and_required_plan() {
    let d = project();
    enable(d.path());
    let adapter = mock(vec![verdict("product-consistency", "FAIL")]);
    let run = please::verify(d.path(), &adapter).unwrap();
    let saved = report::read(d.path()).unwrap();
    assert_eq!(saved.overall, Status::Fail);
    assert_eq!(
        saved.repository,
        d.path().canonicalize().unwrap().to_string_lossy()
    );
    let time = chrono::DateTime::parse_from_rfc3339(&saved.timestamp).unwrap();
    assert_eq!(time.offset().local_minus_utc(), 0);
    assert_eq!(
        saved.snapshot.fingerprint,
        git_snapshot::capture(d.path()).unwrap().fingerprint
    );
    assert_eq!(
        saved.plan.required_ids,
        ["always-ok", "product-consistency"]
    );
    assert_eq!(
        serde_json::to_value(&saved.plan).unwrap(),
        serde_json::to_value(&run.plan).unwrap()
    );
    assert!(saved.plan.scheduled.iter().all(|rule| rule.blocking));
    assert_eq!(saved.results.len(), 2);
    for (actual, expected) in saved.results.iter().zip(&run.results) {
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }
    assert!(saved.results[0].duration_ms > 0);
    let review = &saved.results[1];
    assert_eq!(review.status, Status::Fail);
    assert!(!review.configuration_error);
    assert_eq!(
        review.findings[0].message,
        "Documented expectations inspected"
    );
    assert_eq!(
        review.findings[0].path.as_deref(),
        Some(".please/README.md")
    );
    assert_eq!(review.findings[0].line, Some(1));
}

#[test]
// REQ-0270: overall status, rule counts and elapsed duration occupy one summary line.
fn outcome_summary_keeps_status_counts_and_duration_together() {
    let root = project();
    let mut report = please::verify(root.path(), &Never).unwrap();
    for (status, suffix) in [
        (Status::Pass, ""),
        (Status::Fail, " · 1 failed"),
        (Status::Error, " · 1 errors"),
    ] {
        report.overall = status;
        report.results[0].status = status;
        let text = terminal::render_outcome(&report, 2345, false);
        assert_eq!(
            text.lines().next().unwrap(),
            format!("{} · 1 rules{suffix} · 2.3 s · report 1", status.label())
        );
        assert_eq!(
            text.lines()
                .filter(|line| line.contains(" · 1 rules"))
                .count(),
            1
        );
    }
}

#[test]
// REQ-0280: project commands reject subdirectories and non-repositories; help/version remain available.
fn project_commands_require_the_working_tree_root() {
    let root = project();
    let nested = root.path().join("nested");
    fs::create_dir(&nested).unwrap();
    let outside = tempdir().unwrap();
    for directory in [nested.as_path(), outside.path()] {
        for args in [
            vec!["init"],
            vec!["sync", "--yes"],
            vec!["run", "--at", "check"],
            vec!["run", "--plan"],
            vec!["status"],
        ] {
            let output = cli(directory, &args);
            assert_eq!(
                output.status.code(),
                Some(2),
                "{directory:?}: {args:?}: {output:?}"
            );
        }
        for args in [vec!["help"], vec!["--help"], vec!["--version"]] {
            assert!(cli(directory, &args).status.success(), "{args:?}");
        }
        assert!(!directory.join(".please").exists());
    }
    assert!(!path(root.path(), "reports/1.json").exists());
}

#[test]
// REQ-0281: sync, verification, planning and status preserve HEAD, index, refs, reflogs and Git configuration.
fn project_commands_preserve_git_state() {
    fn state(root: &Path) -> std::collections::BTreeMap<PathBuf, Vec<u8>> {
        fn capture(
            root: &Path,
            path: &Path,
            result: &mut std::collections::BTreeMap<PathBuf, Vec<u8>>,
        ) {
            if path.is_dir() {
                for entry in fs::read_dir(path).unwrap() {
                    capture(root, &entry.unwrap().path(), result);
                }
            } else if path.is_file() {
                result.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    fs::read(path).unwrap(),
                );
            }
        }
        let mut result = std::collections::BTreeMap::new();
        for name in ["HEAD", "index", "config", "packed-refs", "refs", "logs"] {
            capture(root, &root.join(".git").join(name), &mut result);
        }
        result
    }
    let root = project();
    let before = state(root.path());
    for args in [
        vec!["sync", "--yes"],
        vec!["run"],
        vec!["run", "--plan"],
        vec!["status"],
    ] {
        assert!(cli(root.path(), &args).status.success(), "{args:?}");
        assert_eq!(state(root.path()), before, "{args:?}");
    }
}

#[test]
// REQ-0288: initialization changes only .please and leaves the Git hooks directory untouched.
// REQ-0289: the starter enables only its passing check, runs without an adapter and ignores its report.
fn starter_is_confined_and_runs_without_an_adapter() {
    let root = tempdir().unwrap();
    git(root.path(), &["init", "--quiet"]);
    let hooks = root.path().join(".git/hooks");
    let before: std::collections::BTreeMap<_, _> = fs::read_dir(&hooks)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            (
                path.file_name().unwrap().to_owned(),
                fs::read(path).unwrap(),
            )
        })
        .collect();
    assert!(cli(root.path(), &["init"]).status.success());
    let entries: std::collections::BTreeSet<_> = fs::read_dir(root.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(
        entries,
        [
            std::ffi::OsString::from(".git"),
            std::ffi::OsString::from(".please")
        ]
        .into()
    );
    let after: std::collections::BTreeMap<_, _> = fs::read_dir(&hooks)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            (
                path.file_name().unwrap().to_owned(),
                fs::read(path).unwrap(),
            )
        })
        .collect();
    assert_eq!(before, after);
    let config = config::load(root.path()).unwrap();
    assert!(config.adapter.is_none());
    assert_eq!(config.checks.len(), 2);
    assert!(
        config
            .checks
            .iter()
            .any(|c| c.id == "always-ok" && c.enabled)
    );
    assert!(
        config
            .checks
            .iter()
            .any(|c| c.id == "deliberately-failing" && !c.enabled)
    );
    assert_eq!(config.reviews.len(), 1);
    assert!(!config.reviews[0].enabled);
    let report = please::verify(root.path(), &Never).unwrap();
    assert_eq!(report.overall, Status::Pass);
    assert_eq!(report.results.len(), 1);
    assert_eq!(report.results[0].id, "always-ok");
    let ignored = support::command("git")
        .args(["check-ignore", ".please/reports/1.json"])
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(ignored.status.success(), "{ignored:?}");
}
