//! REC-001: isolated configuration, reconciliation and real Git hook execution.
#![cfg(unix)]
mod support;
use please::{
    config,
    reconciliation::{self, HookInvocation},
};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Command, Output},
};
use tempfile::TempDir;

fn git(root: &Path, args: &[&str]) -> Output {
    Command::new("git")
        .args(args)
        .current_dir(root)
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
fn project(points: &[&str]) -> TempDir {
    let root = tempfile::tempdir().unwrap();
    ok(git(root.path(), &["init", "--quiet"]));
    config::init(root.path()).unwrap();
    support::configure_adapter(root.path());
    fs::write(
        root.path().join(".please/config.toml"),
        "project = 'My_App'\n[execution_points]\ncustom = ['first', 'second']\n",
    )
    .unwrap();
    assignments_write(root.path(), points);
    root
}
fn assignments_write(root: &Path, points: &[&str]) {
    let path = root.join(".please/checks/always-ok/rule.toml");
    let mut rule: toml::Value = toml::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    rule["run_at"] = toml::Value::Array(
        points
            .iter()
            .map(|point| toml::Value::String((*point).into()))
            .collect(),
    );
    fs::write(path, toml::to_string(&rule).unwrap()).unwrap();
}
fn synchronize(root: &Path) {
    reconciliation::reconcile(&config::load(root).unwrap(), &mut |_| Ok(true)).unwrap();
}
fn hook(root: &Path, event: &str) -> std::path::PathBuf {
    root.join(".git/hooks").join(event)
}
fn executable(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}
fn cli(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_please"))
        .args(args)
        .current_dir(root)
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}
fn hooked_git(root: &Path, args: &[&str]) -> Output {
    let path = std::env::join_paths(
        std::iter::once(
            Path::new(env!("CARGO_BIN_EXE_please"))
                .parent()
                .unwrap()
                .to_path_buf(),
        )
        .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    Command::new("git")
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
fn checker(root: &Path, script: &str) {
    fs::write(root.join("checker.sh"), script).unwrap();
    let path = root.join(".please/checks/always-ok/rule.toml");
    let rule: toml::Value = toml::from_str(&fs::read_to_string(path).unwrap()).unwrap();
    let points: Vec<_> = rule["run_at"]
        .as_array()
        .unwrap()
        .iter()
        .map(|point| point.as_str().unwrap())
        .collect();
    support::check(root, "always-ok", &points, "exec sh checker.sh");
}

#[test]
fn obsolete_hooks_are_rejected_and_provider_validation_is_preserved() {
    let root = project(&["pre-commit"]);
    let path = root.path().join(".please/config.toml");
    let original = fs::read_to_string(&path).unwrap();
    for obsolete in [
        "[hooks]",
        "[hooks.git]",
        "[hooks.git]\npre-commit = 'first'",
        "[hooks.git]\npre-commit = 'pre-commit'",
    ] {
        fs::write(&path, format!("{original}\n{obsolete}\n")).unwrap();
        let diagnostic = format!("{:#}", config::load(root.path()).unwrap_err());
        assert!(
            diagnostic.contains(".please/config.toml")
                && diagnostic.contains("unknown field `hooks`")
                && diagnostic.contains("expected"),
            "{diagnostic}"
        );
        assert_eq!(cli(root.path(), &["sync", "--yes"]).status.code(), Some(2));
        assert!(!hook(root.path(), "pre-commit").exists());
    }
    fs::write(path, original).unwrap();
    let provider = root.path().join(".please/provider.toml");
    let original = fs::read_to_string(&provider).unwrap();
    for bad in [
        format!("{original}unsupported = true\n"),
        original.replace("180", "3601"),
        original.replace("fake-adapter", ""),
    ] {
        fs::write(&provider, bad).unwrap();
        assert_eq!(cli(root.path(), &["sync"]).status.code(), Some(2));
    }
}

#[test]
fn approval_updates_removals_and_idempotency() {
    let root = project(&["pre-commit"]);
    let p = config::load(root.path()).unwrap();
    let mut prompted = 0;
    assert!(
        reconciliation::reconcile(&p, &mut |plan| {
            prompted += 1;
            assert!(plan.display().contains("Create hook"));
            Ok(false)
        })
        .is_err()
    );
    assert_eq!(prompted, 1);
    assert!(!hook(root.path(), "pre-commit").exists());
    synchronize(root.path());
    let path = hook(root.path(), "pre-commit");
    let metadata = fs::metadata(&path).unwrap();
    reconciliation::reconcile(&p, &mut |_| panic!("synchronized state must not prompt")).unwrap();
    assert_eq!(
        metadata.modified().unwrap(),
        fs::metadata(&path).unwrap().modified().unwrap()
    );
    assert!(metadata.permissions().mode() & 0o111 != 0);
    let output = ok(cli(root.path(), &["sync"]));
    assert_eq!(output.stdout, b"Already synchronized.\n");
    assert!(output.stderr.is_empty());
    // Outdated managed content is restored to the fixed execution point.
    fs::write(
        &path,
        fs::read_to_string(&path)
            .unwrap()
            .replace("--at 'pre-commit'", "--at 'first'"),
    )
    .unwrap();
    assignments_write(root.path(), &["pre-commit", "pre-push"]);
    let updated = ok(cli(root.path(), &["sync", "--yes"]));
    assert_eq!(updated.stdout, b"Synchronization completed successfully.\n");
    assert!(
        String::from_utf8_lossy(&updated.stderr)
            .contains("  Update hook managed section/permissions (preserve unmanaged content): ")
    );
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .contains("--at 'pre-commit'")
    );
    assert!(hook(root.path(), "pre-push").exists());
    assignments_write(root.path(), &["check"]);
    synchronize(root.path());
    assert!(!path.exists());
    assert!(!hook(root.path(), "pre-push").exists());
}

#[test]
fn mixed_ownership_preserves_bytes_permissions_and_custom_exit_behavior() {
    let root = project(&["pre-commit"]);
    let path = hook(root.path(), "pre-commit");
    let original = "#!/bin/sh\n# custom behavior\nprintf 'custom ran\\n' >&2\nexit 17\n";
    executable(&path, original);
    fs::set_permissions(&path, fs::Permissions::from_mode(0o750)).unwrap();
    synchronize(root.path());
    assert!(
        fs::read_to_string(&path)
            .unwrap()
            .ends_with(&original[10..])
    );
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o750
    );
    let output = hooked_git(
        root.path(),
        &["commit", "--allow-empty", "-m", "test: example"],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("custom ran"));
    assignments_write(root.path(), &["check"]);
    synchronize(root.path());
    assert_eq!(fs::read_to_string(&path).unwrap(), original);
}

#[test]
fn conflicts_preserve_all_hooks_before_confirmation() {
    let root = project(&["pre-commit", "pre-push"]);
    let path = hook(root.path(), "pre-push");
    for original in [
        "#!/usr/bin/env python3\nprint('custom')\n",
        "#!/bin/sh\n# BEGIN PLEASE MANAGED\n",
        "#!/bin/sh\n# END PLEASE MANAGED\n",
        "#!/bin/sh\n# Please Behave hook wrapper\n",
        "#!/bin/sh\n# BEGIN PLEASE MANAGED\n# Please Behave repository: foreign\n# END PLEASE MANAGED\n",
    ] {
        executable(&path, original);
        assert!(
            reconciliation::reconcile(&config::load(root.path()).unwrap(), &mut |_| panic!(
                "conflict must precede approval"
            ))
            .is_err()
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), original);
        assert!(!hook(root.path(), "pre-commit").exists());
    }
    fs::remove_file(&path).unwrap();
    synchronize(root.path());
    let valid = fs::read_to_string(&path).unwrap();
    for invalid in [
        format!("{valid}{valid}"),
        valid.replace("# END PLEASE MANAGED\n", ""),
        valid.replace("# BEGIN PLEASE MANAGED", "# BEGIN PLEASE MANAGED broken"),
    ] {
        fs::write(&path, &invalid).unwrap();
        assert!(reconciliation::inspect(&config::load(root.path()).unwrap()).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), invalid);
    }
}

#[test]
fn unsafe_paths_symlinks_and_foreign_ownership_fail_safely() {
    let root = project(&["pre-commit"]);
    let outside = tempfile::tempdir().unwrap();
    ok(git(
        root.path(),
        &["config", "core.hooksPath", outside.path().to_str().unwrap()],
    ));
    assert!(reconciliation::inspect(&config::load(root.path()).unwrap()).is_err());
    assert_eq!(fs::read_dir(outside.path()).unwrap().count(), 0);
    ok(git(root.path(), &["config", "--unset", "core.hooksPath"]));
    let path = hook(root.path(), "pre-commit");
    std::os::unix::fs::symlink(outside.path().join("missing"), &path).unwrap();
    assert!(reconciliation::inspect(&config::load(root.path()).unwrap()).is_err());
    fs::remove_file(&path).unwrap();
    fs::create_dir(&path).unwrap();
    assert!(reconciliation::inspect(&config::load(root.path()).unwrap()).is_err());
    fs::remove_dir(&path).unwrap();
    synchronize(root.path());
    let other = project(&["pre-commit"]);
    fs::copy(&path, hook(other.path(), "pre-commit")).unwrap();
    assert!(reconciliation::inspect(&config::load(other.path()).unwrap()).is_err());
    ok(git(root.path(), &["config", "core.hooksPath", "linked"]));
    std::os::unix::fs::symlink(outside.path(), root.path().join("linked")).unwrap();
    assert!(reconciliation::inspect(&config::load(root.path()).unwrap()).is_err());
}

#[test]
fn commands_block_checks_on_noninteractive_drift_and_keep_plan_read_only() {
    let root = project(&["pre-commit"]);
    support::check(
        root.path(),
        "always-ok",
        &["pre-commit", "pre-push"],
        "git config --local please.executed yes\n",
    );
    for args in [&["sync"][..], &["run", "--at", "pre-push"][..]] {
        let output = cli(root.path(), args);
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains("terminal"));
        assert!(
            !git(root.path(), &["config", "--get", "please.executed"])
                .status
                .success()
        );
        assert!(!hook(root.path(), "pre-commit").exists());
    }
    ok(cli(root.path(), &["run", "--at", "pre-push", "--plan"]));
    assert!(!hook(root.path(), "pre-commit").exists());
    synchronize(root.path());
    ok(cli(root.path(), &["run", "--at", "pre-push"]));
    assert!(
        git(root.path(), &["config", "--get", "please.executed"])
            .status
            .success()
    );
}

#[test]
fn real_commit_hooks_forward_message_path_and_preserve_index_and_worktree() {
    let root = project(&["pre-commit", "commit-msg"]);
    checker(
        root.path(),
        "set -eu\ntest \"$PLEASE_EXECUTION_POINT\" = \"$PLEASE_GIT_HOOK\"\ncase \"$PLEASE_GIT_HOOK\" in\npre-commit) test \"$PLEASE_GIT_HOOK_ARG_COUNT\" = 0;;\ncommit-msg) test \"$PLEASE_GIT_HOOK_ARG_COUNT\" = 1; grep -q '^test: accepted' \"$PLEASE_GIT_HOOK_ARG_1\";;\n*) exit 19;;\nesac\n",
    );
    fs::write(root.path().join("file"), "staged\n").unwrap();
    ok(git(root.path(), &["add", "file"]));
    fs::write(root.path().join("file"), "unstaged\n").unwrap();
    let index = git(root.path(), &["ls-files", "--stage"]).stdout;
    synchronize(root.path());
    ok(hooked_git(root.path(), &["commit", "-m", "test: accepted"]));
    assert_eq!(
        fs::read_to_string(root.path().join("file")).unwrap(),
        "unstaged\n"
    );
    assert_eq!(git(root.path(), &["ls-files", "--stage"]).stdout, index);
    assert_eq!(
        please::report::read(root.path()).unwrap().plan.at,
        "commit-msg"
    );
    assert!(
        !hooked_git(root.path(), &["commit", "--allow-empty", "-m", "invalid"])
            .status
            .success()
    );
    let before = fs::read(hook(root.path(), "pre-commit")).unwrap();
    assignments_write(root.path(), &["pre-commit"]);
    let output = hooked_git(
        root.path(),
        &["commit", "--allow-empty", "-m", "test: drift"],
    );
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("please sync"));
    assert_eq!(fs::read(hook(root.path(), "pre-commit")).unwrap(), before);
}

#[test]
fn actual_push_hook_blocks_and_allows_pushes() {
    let root = project(&["pre-push"]);
    let remote = tempfile::tempdir().unwrap();
    ok(git(remote.path(), &["init", "--bare", "--quiet"]));
    ok(git(
        root.path(),
        &["commit", "--allow-empty", "-m", "test: initial"],
    ));
    checker(root.path(), "exit 23\n");
    synchronize(root.path());
    assert!(
        !hooked_git(
            root.path(),
            &[
                "push",
                remote.path().to_str().unwrap(),
                "HEAD:refs/heads/test"
            ]
        )
        .status
        .success()
    );
    assert!(
        !git(remote.path(), &["rev-parse", "--verify", "refs/heads/test"])
            .status
            .success()
    );
    checker(
        root.path(),
        "test \"$PLEASE_GIT_HOOK\" = pre-push && test \"$PLEASE_GIT_HOOK_ARG_COUNT\" = 2\n",
    );
    ok(hooked_git(
        root.path(),
        &[
            "push",
            remote.path().to_str().unwrap(),
            "HEAD:refs/heads/test",
        ],
    ));
}

#[test]
fn custom_hooks_directory_and_worktrees_use_git_resolution() {
    let root = project(&["pre-commit"]);
    ok(git(
        root.path(),
        &["config", "core.hooksPath", "custom hooks/nested"],
    ));
    synchronize(root.path());
    assert!(root.path().join("custom hooks/nested/pre-commit").exists());
    ok(hooked_git(
        root.path(),
        &["commit", "--allow-empty", "-m", "test: custom"],
    ));
    ok(git(root.path(), &["config", "--unset", "core.hooksPath"]));
    // Track configuration so the linked tree shares identical declarations.
    ok(git(root.path(), &["add", ".please"]));
    ok(git(root.path(), &["commit", "-m", "test: configuration"]));
    synchronize(root.path());
    let linked = tempfile::tempdir().unwrap();
    let path = linked.path().join("worktree");
    ok(git(
        root.path(),
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "linked",
            path.to_str().unwrap(),
        ],
    ));
    assert!(path.join(".git").is_file());
    assert!(
        reconciliation::inspect(&config::load(&path).unwrap())
            .unwrap()
            .synchronized()
    );
    ok(hooked_git(
        &path,
        &["commit", "--allow-empty", "-m", "test: linked"],
    ));
}

#[test]
fn invocation_validation_recursion_and_missing_executable_block_commits() {
    let root = project(&["pre-commit"]);
    synchronize(root.path());
    let p = config::load(root.path()).unwrap();
    assert!(
        reconciliation::prepare(
            &p,
            Some(HookInvocation {
                event: "pre-commit".into(),
                path: "checker.sh".into(),
                arguments: vec![]
            }),
            Some("pre-commit"),
            false
        )
        .is_err()
    );
    assert!(
        reconciliation::prepare(
            &p,
            Some(HookInvocation {
                event: "pre-commit".into(),
                path: hook(root.path(), "pre-commit"),
                arguments: vec![]
            }),
            Some("second"),
            false
        )
        .is_err()
    );
    checker(root.path(), "please run --at first\n");
    let output = hooked_git(
        root.path(),
        &["commit", "--allow-empty", "-m", "test: recurse"],
    );
    assert!(!output.status.success());
    let report = please::report::read(root.path()).unwrap();
    assert!(
        report.results[0]
            .stderr
            .as_ref()
            .unwrap()
            .text
            .contains("recursive Please Behave hook execution blocked")
    );
    // Invoke Git with a PATH containing Git/sh but no Please Behave.
    let bins = tempfile::tempdir().unwrap();
    let git_path = Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    std::os::unix::fs::symlink(
        String::from_utf8(git_path.stdout).unwrap().trim(),
        bins.path().join("git"),
    )
    .unwrap();
    let output = Command::new(bins.path().join("git"))
        .args(["commit", "--allow-empty", "-m", "test: unavailable"])
        .current_dir(root.path())
        .env("PATH", bins.path())
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("please")
            && String::from_utf8_lossy(&output.stderr).contains("not found"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn stale_approval_and_interrupted_changes_can_be_recovered() {
    let root = project(&["pre-commit", "pre-push"]);
    let p = config::load(root.path()).unwrap();
    let plan = reconciliation::inspect(&p).unwrap();
    executable(&hook(root.path(), "pre-push"), "#!/bin/sh\nexit 0\n");
    assert!(reconciliation::apply(&p, &plan).is_err());
    assert!(!hook(root.path(), "pre-commit").exists());
    synchronize(root.path());
    // Recover outdated managed content after an interrupted update.
    let path = hook(root.path(), "pre-commit");
    fs::write(
        &path,
        fs::read_to_string(&path)
            .unwrap()
            .replace("--at 'pre-commit'", "--at 'first'"),
    )
    .unwrap();
    assert!(
        !reconciliation::inspect(&config::load(root.path()).unwrap())
            .unwrap()
            .synchronized()
    );
    synchronize(root.path());
    assert!(
        fs::read_to_string(hook(root.path(), "pre-commit"))
            .unwrap()
            .contains("--at 'pre-commit'")
    );
}

#[test]
fn actual_terminal_approval_and_rejection_gate_execution() {
    use std::{
        io::{Read, Write},
        os::fd::FromRawFd,
    };
    fn terminal(root: &Path, args: &[&str], answer: &[u8]) -> std::process::ExitStatus {
        let (mut master_fd, mut slave_fd) = (-1, -1);
        // A real PTY exercises terminal detection; pipes deliberately cannot approve.
        assert_eq!(
            unsafe {
                libc::openpty(
                    &mut master_fd,
                    &mut slave_fd,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            },
            0
        );
        let mut master = unsafe { fs::File::from_raw_fd(master_fd) };
        let slave = unsafe { fs::File::from_raw_fd(slave_fd) };
        let mut child = Command::new(env!("CARGO_BIN_EXE_please"))
            .args(args)
            .current_dir(root)
            .env("NO_COLOR", "1")
            .stdin(slave.try_clone().unwrap())
            .stdout(slave.try_clone().unwrap())
            .stderr(slave)
            .spawn()
            .unwrap();
        master.write_all(answer).unwrap();
        let reader = std::thread::spawn(move || {
            let mut bytes = [0; 4096];
            while matches!(master.read(&mut bytes), Ok(n) if n > 0) {}
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if std::time::Instant::now() > deadline {
                child.kill().unwrap();
                panic!("terminal reconciliation timed out");
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        };
        reader.join().unwrap();
        status
    }
    let root = project(&["pre-commit"]);
    support::check(
        root.path(),
        "always-ok",
        &["pre-commit", "pre-push"],
        "git config --local please.executed yes\n",
    );
    assert_eq!(
        terminal(root.path(), &["run", "--at", "pre-push"], b"n\n").code(),
        Some(2)
    );
    assert!(!hook(root.path(), "pre-commit").exists());
    assert!(
        !git(root.path(), &["config", "--get", "please.executed"])
            .status
            .success()
    );
    assert!(terminal(root.path(), &["sync"], b"yes\n").success());
    assert!(hook(root.path(), "pre-commit").exists());
    fs::remove_file(hook(root.path(), "pre-commit")).unwrap();
    assert!(terminal(root.path(), &["run", "--at", "pre-push"], b"y\n").success());
    assert!(
        git(root.path(), &["config", "--get", "please.executed"])
            .status
            .success()
    );
}

#[test]
fn auto_approve_reconciles_noninteractively_without_executing_rules() {
    let root = project(&["pre-commit", "commit-msg", "pre-push"]);
    checker(
        root.path(),
        "git config --local please.executed yes\nexit 99\n",
    );
    let report = root.path().join(".please/reports/last.json");
    fs::write(&report, "prior report").unwrap();
    assert_eq!(cli(root.path(), &["sync"]).status.code(), Some(2));
    assert!(!hook(root.path(), "pre-commit").exists());
    let output = ok(cli(root.path(), &["sync", "--yes"]));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.starts_with("Planned changes:\n"));
    assert!(stderr.contains("  Create hook: "));
    assert!(!stderr.contains("Auto-approved"));
    assert_eq!(output.stdout, b"Synchronization completed successfully.\n");
    assert!(!stderr.contains("Apply these changes?"));
    for event in ["pre-commit", "commit-msg", "pre-push"] {
        assert!(hook(root.path(), event).is_file());
    }
    assert!(
        !git(root.path(), &["config", "--get", "please.executed"])
            .status
            .success()
    );
    assert_eq!(fs::read_to_string(&report).unwrap(), "prior report");
    assert!(!root.path().join(".editorconfig").exists());
    let before = fs::metadata(hook(root.path(), "pre-commit"))
        .unwrap()
        .modified()
        .unwrap();
    let repeated = ok(cli(root.path(), &["sync", "--yes"]));
    assert_eq!(repeated.stdout, b"Already synchronized.\n");
    assert!(repeated.stderr.is_empty());
    assert_eq!(
        before,
        fs::metadata(hook(root.path(), "pre-commit"))
            .unwrap()
            .modified()
            .unwrap()
    );
    // A fresh CI job can execute after setup without another approval.
    let path = root.path().join(".please/checks/always-ok/rule.toml");
    fs::write(
        &path,
        fs::read_to_string(&path)
            .unwrap()
            .replace("enabled = true", "enabled = false"),
    )
    .unwrap();
    ok(cli(root.path(), &["run", "--at", "first"]));
    let removed = ok(cli(root.path(), &["sync", "--yes"]));
    assert_eq!(removed.stdout, b"Synchronization completed successfully.\n");
    assert!(String::from_utf8_lossy(&removed.stderr).contains("  Remove owned hook: "));
    for event in reconciliation::GIT_EVENTS {
        assert!(!hook(root.path(), event).exists());
    }
}

#[test]
fn auto_approve_preserves_conflicts_and_full_configuration_validation() {
    let root = project(&["pre-commit", "pre-push"]);
    let path = hook(root.path(), "pre-push");
    let foreign = "#!/usr/bin/env python3\nprint('custom')\n";
    executable(&path, foreign);
    let report = root.path().join(".please/reports/last.json");
    fs::write(&report, "prior report").unwrap();
    let output = cli(root.path(), &["sync", "--yes"]);
    assert_eq!(output.status.code(), Some(2));
    assert_eq!(fs::read_to_string(&path).unwrap(), foreign);
    assert!(!hook(root.path(), "pre-commit").exists());
    fs::remove_file(&path).unwrap();
    assignments_write(root.path(), &["missing"]);
    assert_eq!(cli(root.path(), &["sync", "--yes"]).status.code(), Some(2));
    assert!(!hook(root.path(), "pre-commit").exists());
    assert_eq!(fs::read_to_string(&report).unwrap(), "prior report");
}

#[test]
fn auto_approve_cannot_bypass_hook_recursion_protection() {
    let root = project(&["pre-commit"]);
    checker(root.path(), "please sync --yes\n");
    ok(cli(root.path(), &["sync", "--yes"]));
    let before = fs::read(hook(root.path(), "pre-commit")).unwrap();
    // Invoking the managed wrapper directly exercises the guard without a commit.
    let output = hooked_git(root.path(), &["hook", "run", "pre-commit"]);
    assert!(!output.status.success());
    let report = please::report::read(root.path()).unwrap();
    assert!(
        report.results[0]
            .stderr
            .as_ref()
            .unwrap()
            .text
            .contains("recursive Please Behave hook execution blocked")
    );
    assert_eq!(fs::read(hook(root.path(), "pre-commit")).unwrap(), before);
}

#[test]
fn fixed_hooks_select_only_their_own_effective_rules() {
    let root = project(&["check", "first"]);
    // A local Rule Set overrides the Rule's own custom assignment.
    fs::create_dir_all(root.path().join(".please/rule-sets")).unwrap();
    support::check(
        root.path(),
        "set-rule",
        &["first"],
        "test \"$PLEASE_GIT_HOOK\" = commit-msg",
    );
    fs::write(root.path().join(".please/rule-sets/message.toml"), "id = 'message'\ndescription = 'Message checks'\nrules = ['set-rule']\nrun_at = ['commit-msg']\n").unwrap();
    for event in ["pre-commit", "pre-push"] {
        support::check(
            root.path(),
            event,
            &[event],
            &format!("test \"$PLEASE_GIT_HOOK\" = {event}"),
        );
    }
    ok(cli(root.path(), &["sync", "--yes"]));
    for (event, expected) in [
        ("pre-commit", "pre-commit"),
        ("commit-msg", "set-rule"),
        ("pre-push", "pre-push"),
    ] {
        ok(hooked_git(root.path(), &["hook", "run", event]));
        let report = please::report::read(root.path()).unwrap();
        assert_eq!(report.plan.at, event);
        assert_eq!(report.results.len(), 1);
        assert_eq!(report.results[0].id, expected);
        for point in ["check", "first"] {
            let error = reconciliation::prepare(
                &config::load(root.path()).unwrap(),
                Some(HookInvocation {
                    event: event.into(),
                    path: hook(root.path(), event),
                    arguments: vec![],
                }),
                Some(point),
                false,
            )
            .err()
            .unwrap();
            assert!(error.to_string().contains("must match its Git event"));
        }
    }
}

#[test]
fn direct_execution_blocks_nested_execution_and_reconciliation() {
    for point in ["check", "first", "pre-push"] {
        for nested in [format!("run --at {point}"), "sync --yes".into()] {
            let root = project(&[point]);
            // Bound the regression case: a missing guard admits one nested run,
            // which exits successfully instead of recursively launching more runs.
            checker(
                root.path(),
                &format!(
                    "if [ \"${{PLEASE_RECURSION_TEST:-}}\" = 1 ]; then\n echo 'nested execution admitted'\n exit 0\nfi\nexport PLEASE_RECURSION_TEST=1\nexec {:?} {nested}\n",
                    env!("CARGO_BIN_EXE_please")
                ),
            );
            if point == "pre-push" {
                synchronize(root.path());
            }
            let output = cli(root.path(), &["run", "--at", point]);
            assert_eq!(output.status.code(), Some(2), "{point}: {nested}");
            let report = please::report::read(root.path()).unwrap();
            let item = &report.results[0];
            assert_eq!(item.status, please::report::Status::Error);
            assert!(
                item.stderr
                    .as_ref()
                    .unwrap()
                    .text
                    .contains("recursive Please Behave hook execution blocked")
            );
            assert!(
                !item
                    .stdout
                    .as_ref()
                    .unwrap()
                    .text
                    .contains("nested execution admitted")
            );
            assert_eq!(report.results.len(), 1);
        }
    }
}

#[test]
fn only_effective_builtin_assignments_require_hooks() {
    let root = project(&["first", "check"]);
    synchronize(root.path());
    for event in reconciliation::GIT_EVENTS {
        assert!(!hook(root.path(), event).exists());
    }
    assignments_write(root.path(), &["pre-commit"]);
    synchronize(root.path());
    assert!(hook(root.path(), "pre-commit").exists());
    assert!(!hook(root.path(), "pre-push").exists());
    assignments_write(root.path(), &["check"]);
    synchronize(root.path());
    assert!(!hook(root.path(), "pre-commit").exists());
}

#[test]
fn explicit_points_execute_noninteractively_without_inspecting_or_changing_hooks() {
    let root = project(&["pre-commit"]);
    support::check(
        root.path(),
        "always-ok",
        &["check", "first", "pre-commit"],
        "git --version\n",
    );
    // A required hook is missing: execution must not ask to install it.
    for point in ["check", "first"] {
        let output = cli(root.path(), &["run", "--at", point]);
        ok(output);
        assert!(!hook(root.path(), "pre-commit").exists());
        assert!(!hook(root.path(), "pre-push").exists());
        assert_eq!(please::report::read(root.path()).unwrap().results.len(), 1);
    }
    ok(cli(root.path(), &["sync", "--yes"]));
    let file = hook(root.path(), "pre-commit");
    let stale = fs::read_to_string(&file)
        .unwrap()
        .replace("--at 'pre-commit'", "--at 'first'");
    fs::write(&file, &stale).unwrap();
    assert!(
        !reconciliation::inspect(&config::load(root.path()).unwrap())
            .unwrap()
            .synchronized()
    );
    let permissions = fs::metadata(&file).unwrap().permissions().mode();
    for point in ["check", "first"] {
        ok(cli(root.path(), &["run", "--at", point]));
        assert_eq!(fs::read_to_string(&file).unwrap(), stale);
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode(),
            permissions
        );
        assert!(!hook(root.path(), "pre-push").exists());
    }
    // Even hook locations that reconciliation rejects are irrelevant to direct execution.
    ok(git(
        root.path(),
        &["config", "core.hooksPath", ".please/config.toml"],
    ));
    for point in ["check", "first"] {
        ok(cli(root.path(), &["run", "--at", point]));
    }
    assert_eq!(fs::read_to_string(&file).unwrap(), stale);
    assert!(!root.path().join(".github").exists());
}
