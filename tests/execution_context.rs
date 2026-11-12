#![cfg(unix)]
mod support;
use please::{
    config,
    report::{self, Status},
};
use sha2::{Digest, Sha256};
use std::fs;
use support::write;

#[test]
fn validated_execution_point_overrides_inherited_context_for_checks_and_provider_processes() {
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
    support::configure_adapter(root.path());
    write(
        root.path(),
        ".please/config.toml",
        "project = 'My_App'\n[execution_points]\ncustom = ['custom-point']\n",
    );
    let points = [
        "pre-commit",
        "commit-msg",
        "pre-push",
        "check",
        "custom-point",
    ];
    // Inputs arrive as arguments; nothing from the harness environment reaches the rule.
    support::check_args(
        root.path(),
        "context",
        &points,
        "@context @param/label @rule/resource",
        r#"set -eu
test -z "$(env | grep '^PLEASE_' | grep -v '^PLEASE_ACTIVE_HOOK=' || true)"
point=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["execution_point"])' "$1")
test "$point" = "$EXPECTED_EXECUTION_POINT"
test "$2" = 'preserved'
test "$(cat "$3")" = bundled
printf '%s' "$point"
"#,
    );
    let rule = root.path().join(".please/checks/context/rule.toml");
    fs::write(
        &rule,
        format!(
            "{}\n[parameters.label]\ntype = 'string'\nvalue = 'preserved'\n",
            fs::read_to_string(&rule).unwrap()
        ),
    )
    .unwrap();
    write(root.path(), ".please/checks/context/resource", "bundled\n");
    write(root.path(), "evidence", "Evidence\n");
    write(
        root.path(),
        ".please/reviews/context/rule.toml",
        &format!(
            "id = 'context-review'\ndescription = 'Context review'\nrun_at = {points:?}\n[review]\nprompt = 'prompt.md'\n"
        ),
    );
    write(
        root.path(),
        ".please/reviews/context/prompt.md",
        "Inspect evidence.",
    );
    let fake = tempfile::tempdir().unwrap();
    support::context_provider(fake.path());
    let path = std::env::join_paths(
        std::iter::once(fake.path().to_path_buf())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let output = support::command(env!("CARGO_BIN_EXE_please"))
        .args(["sync", "--yes"])
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // A governor running in this repository must not activate Please Behave's guard.
    let governor_owner = format!(
        "{:x}",
        Sha256::digest(
            root.path()
                .join(".git")
                .canonicalize()
                .unwrap()
                .as_os_str()
                .as_encoded_bytes()
        )
    );
    // No --at exercises the default pre-push context too.
    for point in points.into_iter().map(Some).chain([None]) {
        let expected = point.unwrap_or("pre-push");
        let calls = fake.path().join("calls");
        fs::write(&calls, "").unwrap();
        let mut command = support::command(env!("CARGO_BIN_EXE_please"));
        command.args(["run"]);
        if let Some(point) = point {
            command.args(["--at", point]);
        }
        let output = command
            .current_dir(root.path())
            .env("PATH", &path)
            .env("PLEASE_EXECUTION_POINT", "spoofed")
            .env("PLEASE_CONTEXT", "spoofed")
            .env("PLEASE_RULE_DIRECTORY", "spoofed")
            .env("PLEASE_PARAM_LABEL", "spoofed")
            .env("PLEASE_GIT_HOOK", "spoofed")
            .env("HANNES_EXECUTION_POINT", "governor-point")
            .env("HANNES_ACTIVE_HOOK", &governor_owner)
            .env("HANNES_RULE_DIRECTORY", "/nonexistent/governor")
            .env("HANNES_PARAM_LABEL", "governor-value")
            .env("EXPECTED_EXECUTION_POINT", expected)
            .env("CONTEXT_CALLS", &calls)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{expected}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report = report::read(root.path()).unwrap();
        assert_eq!(report.overall, Status::Pass);
        let item = report.results.iter().find(|r| r.id == "context").unwrap();
        assert_eq!(item.stdout.as_ref().unwrap().text, expected);
        assert_eq!(fs::read_to_string(calls).unwrap(), "review\n");
    }
}
