//! HDR-001: reviews at commit-msg receive the whole commit message.
#![cfg(unix)]
mod support;

use please::{
    config, planner,
    report::{self, Status},
};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Output,
};

fn cli(root: &Path, args: &[&str]) -> Output {
    support::command(env!("CARGO_BIN_EXE_please"))
        .args(args)
        .current_dir(root)
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

fn git(root: &Path, args: &[&str]) -> Output {
    let path = std::env::join_paths(
        std::iter::once(
            Path::new(env!("CARGO_BIN_EXE_please"))
                .parent()
                .unwrap()
                .to_owned(),
        )
        .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    support::command("git")
        .args(args)
        .current_dir(root)
        .env("PATH", path)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .unwrap()
}

fn ok(output: Output) -> Output {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

struct Fixture {
    _temporary: tempfile::TempDir,
    root: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        ok(git(&root, &["init", "--quiet", "--initial-branch=main"]));
        please::init::init(&root).unwrap();
        fs::remove_dir_all(root.join(".please/checks")).unwrap();
        fs::remove_dir_all(root.join(".please/reviews")).unwrap();
        support::write(&root, ".gitignore", ".please/reports/\n");
        support::write(&root, "private-source", "source text\n");
        support::write(
            &root,
            ".please/reviews/message/rule.toml",
            "id = 'message'\ndescription = 'Review message wording'\nrun_at = ['commit-msg']\n[review]\nprompt = 'prompt.md'\n",
        );
        support::write(
            &root,
            ".please/reviews/message/prompt.md",
            "Reject vague wording; require imperative English.\n",
        );
        support::write(
            &root,
            ".please/provider.toml",
            "command = 'python3'\nargs = ['.git/adapter.py']\ntimeout_seconds = 5\n",
        );
        support::check_args(
            &root,
            "syntax",
            &["commit-msg"],
            "@context",
            "#!/bin/sh\nset -eu\n[ ! -f .git/fail-check ]\nprintf checked > .git/check-marker\nif [ -f .git/change-in-check ]; then file=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))[\"commit_message\"][\"file\"])' \"$1\"); printf 'changed message\\n' > \"$file\"; fi\n",
        );
        support::write(
            &root,
            ".git/adapter.py",
            r#"import json, pathlib, sys
request = json.load(sys.stdin)
root = pathlib.Path('.git')
assert (root / 'check-marker').exists()
with (root / 'calls').open('a') as log:
    log.write(json.dumps(request) + '\n')
assert len(request['reviews']) == 1
assert 'input' not in request['reviews'][0]
assert 'commit_header' not in request['input']
assert 'source' not in json.dumps(request['response_schema'])
lines = request['input']['commit_message']['lines']
mode = (root / 'mode').read_text() if (root / 'mode').exists() else 'PASS'
if mode == 'malformed':
    print('not JSON'); sys.exit(0)
if mode == 'change':
    (root / 'COMMIT_EDITMSG').write_text('changed message\n')
if lines:
    finding = {'path': None, 'line': len(lines) + (1 if mode == 'bad-line' else 0), 'message': 'Message evaluated'}
else:
    finding = {'path': 'private-source', 'line': 1, 'message': 'File evaluated'}
status = mode if mode in ['PASS', 'FAIL', 'ERROR'] else 'PASS'
print(json.dumps({'protocol_version': 2, 'reason': 'Message evaluated', 'findings': [] if status == 'ERROR' else [finding]}))
sys.exit({'PASS': 0, 'FAIL': 1}.get(status, 2))
"#,
        );
        ok(cli(&root, &["sync", "--yes"]));
        Self {
            _temporary: temporary,
            root,
        }
    }
    fn hook(&self) -> Output {
        self.hook_message(".git/COMMIT_EDITMSG")
    }
    fn hook_message(&self, message: &str) -> Output {
        cli(
            &self.root,
            &[
                "run",
                "--at",
                "commit-msg",
                "--git-hook",
                "commit-msg",
                "--git-hook-path",
                ".git/hooks/commit-msg",
                "--",
                message,
            ],
        )
    }
    fn calls(&self) -> Vec<Value> {
        fs::read_to_string(self.root.join(".git/calls"))
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
}

#[test]
fn commit_message_hook_and_adapter_contract() {
    let f = Fixture::new();
    ok(git(&f.root, &["add", "."]));
    ok(git(
        &f.root,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "fix: preserve focus",
            "-m",
            "Body line",
        ],
    ));
    let lines = &f.calls()[0]["input"]["commit_message"]["lines"];
    assert_eq!(
        lines[0],
        serde_json::json!({"number": 1, "text": "fix: preserve focus"})
    );
    assert_eq!(
        lines[2],
        serde_json::json!({"number": 3, "text": "Body line"})
    );
    let saved = report::read(&f.root).unwrap();
    assert_eq!(saved.overall, Status::Pass);
    assert_eq!(
        saved.plan.input.commit_message.as_deref(),
        Some("fix: preserve focus\n\nBody line")
    );
    let details = cli(&f.root, &["status", "--details"]);
    assert_eq!(details.status.code(), Some(1)); // The completed commit changed HEAD.
    assert!(String::from_utf8_lossy(&details.stdout).contains("commit-message:3"));

    for (mode, expected) in [
        ("FAIL", Status::Fail),
        ("ERROR", Status::Error),
        ("malformed", Status::Error),
        ("bad-line", Status::Error),
        ("change", Status::Error),
    ] {
        support::write(&f.root, ".git/mode", mode);
        support::write(
            &f.root,
            ".git/COMMIT_EDITMSG",
            "fix: preserve focus\n\nBody line\n",
        );
        let output = f.hook();
        assert_eq!(
            output.status.code(),
            Some(expected.exit_code()),
            "{mode}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(report::read(&f.root).unwrap().overall, expected, "{mode}");
    }
}

#[test]
fn missing_context_invalid_messages_and_failed_checks_never_call_adapter() {
    let f = Fixture::new();
    for args in [
        vec!["run", "--at", "commit-msg"],
        vec!["run", "--at", "commit-msg", "--plan", "--json"],
    ] {
        assert!(!cli(&f.root, &args).status.success());
        assert!(f.calls().is_empty());
    }
    // Checks-only needs no hook context: the review is NOT_RUN and no adapter is called.
    assert!(
        cli(&f.root, &["run", "--at", "commit-msg", "--checks-only"])
            .status
            .success()
    );
    assert!(f.calls().is_empty());
    for bytes in [
        vec![b'x'; 65537],
        vec![0xff],
        b"fix: bad\0message\n".to_vec(),
    ] {
        fs::write(f.root.join(".git/COMMIT_EDITMSG"), bytes).unwrap();
        assert!(!f.hook().status.success());
        assert!(f.calls().is_empty());
    }
    let path = f.root.join(".git/COMMIT_EDITMSG");
    fs::remove_file(&path).unwrap();
    assert!(!f.hook().status.success());
    support::write(&f.root, ".git/target", "fix: preserve focus\n");
    std::os::unix::fs::symlink("target", &path).unwrap();
    assert!(!f.hook().status.success());
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(!f.hook().status.success());
    fs::remove_dir(&path).unwrap();
    support::write(&f.root, ".git/COMMIT_EDITMSG", "fix: preserve focus\n");
    support::write(&f.root, ".git/fail-check", "");
    assert!(!f.hook().status.success());
    assert!(f.calls().is_empty());
    fs::remove_file(f.root.join(".git/fail-check")).unwrap();
    support::write(&f.root, ".git/change-in-check", "");
    assert!(!f.hook().status.success());
    assert!(f.calls().is_empty());
}

#[test]
fn empty_messages_run_and_the_whole_message_is_bounded_and_part_of_identity() {
    let f = Fixture::new();
    support::write(&f.root, ".git/COMMIT_EDITMSG", "");
    ok(f.hook());
    assert_eq!(
        f.calls().last().unwrap()["input"]["commit_message"]["lines"],
        serde_json::json!([])
    );
    let message = format!("{}\r\n\nbody\ttab\n", "x".repeat(60_000));
    support::write(&f.root, ".git/COMMIT_EDITMSG", &message);
    ok(f.hook());
    let lines = f.calls().last().unwrap()["input"]["commit_message"]["lines"].clone();
    assert_eq!(lines.as_array().unwrap().len(), 3);
    assert_eq!(lines[2]["text"], "body\ttab");
    let before = report::read(&f.root).unwrap().plan.plan_id;
    support::write(&f.root, ".git/COMMIT_EDITMSG", "different message\n");
    ok(f.hook());
    assert_ne!(report::read(&f.root).unwrap().plan.plan_id, before);
    let project = config::load(&f.root).unwrap();
    assert!(
        planner::build(&project, Some("pre-push"), false)
            .unwrap()
            .plan
            .executable()
    );
    // Reviews at commit-msg may select files, context and triggers like any other review.
    let rule = f.root.join(".please/reviews/message/rule.toml");
    let original = fs::read_to_string(&rule).unwrap();
    for extra in [
        "files = ['private-source']",
        "context = [{path = 'private-source'}]",
    ] {
        fs::write(&rule, format!("{original}{extra}\n")).unwrap();
        assert!(config::load(&f.root).is_ok(), "{extra}");
    }
    fs::write(&rule, format!("{original}input = 'commit-header'\n")).unwrap();
    assert!(config::load(&f.root).is_err());
}

#[test]
fn advisory_verdicts_preserve_enforcement_but_message_changes_are_fatal() {
    let f = Fixture::new();
    let rule = f.root.join(".please/reviews/message/rule.toml");
    let original = fs::read_to_string(&rule).unwrap();
    fs::write(
        &rule,
        original.replace("run_at =", "blocking = false\nrun_at ="),
    )
    .unwrap();
    ok(cli(&f.root, &["sync", "--yes"]));
    for mode in ["FAIL", "ERROR"] {
        support::write(&f.root, ".git/COMMIT_EDITMSG", "fix: preserve focus\n");
        support::write(&f.root, ".git/mode", mode);
        ok(f.hook());
        let report = report::read(&f.root).unwrap();
        assert_eq!(report.overall, Status::Pass);
        assert!(
            report
                .results
                .iter()
                .any(|r| r.id == "message" && r.status.label() == mode && !r.blocking)
        );
    }
    support::write(&f.root, ".git/mode", "change");
    assert!(!f.hook().status.success());
    assert_eq!(report::read(&f.root).unwrap().overall, Status::Error);
}

#[test]
fn imported_message_reviews_use_the_same_hook_contract() {
    let f = Fixture::new();
    let source = tempfile::tempdir().unwrap();
    let source = source.path().canonicalize().unwrap();
    ok(git(&source, &["init", "--quiet", "--initial-branch=main"]));
    support::write(
        &source,
        "commit-msg/quality/config.toml",
        "id = 'quality'\ndescription = 'Commit message wording'\nrun_at = ['commit-msg']\nrules = ['rules/reviews/message']\n",
    );
    support::write(
        &source,
        "rules/reviews/message/rule.toml",
        "id = 'message'\ndescription = 'Review message wording'\n[review]\nprompt = 'prompt.md'\n",
    );
    support::write(
        &source,
        "rules/reviews/message/prompt.md",
        "Reject vague wording; require imperative English.\n",
    );
    ok(git(&source, &["add", "."]));
    ok(git(
        &source,
        &[
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-m",
            "test: define external message review",
        ],
    ));
    let revision = String::from_utf8(ok(git(&source, &["rev-parse", "HEAD"])).stdout).unwrap();
    fs::remove_dir_all(f.root.join(".please/reviews")).unwrap();
    support::write(
        &f.root,
        ".please/config.toml",
        &format!(
            "project = 'Import fixture'\n[[imports]]\ngit = {:?}\npath = 'commit-msg/quality'\nref = {:?}\n",
            source.to_str().unwrap(),
            revision.trim()
        ),
    );
    ok(cli(&f.root, &["sync", "--yes"]));
    support::write(&f.root, ".git/COMMIT_EDITMSG", "fix: preserve focus\n");
    ok(f.hook());
    let report = report::read(&f.root).unwrap();
    assert_eq!(report.overall, Status::Pass);
    assert!(
        report
            .results
            .iter()
            .any(|r| r.id.starts_with("git:") && r.kind == "review" && r.status == Status::Pass)
    );
}
