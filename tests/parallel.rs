//! CHK-001: bounded parallel checks retain report, snapshot and review contracts.
mod support;
use clap::Parser;
use please::{
    cli::{Cli, Words},
    config, planner,
    provider::{Provider, Request},
    report::{self, Status},
};
use std::fs;
use tempfile::TempDir;
struct Never;
impl Provider for Never {
    fn review(&self, _: &Request<'_>) -> anyhow::Result<String> {
        panic!("unexpected review");
    }
}
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
    config::init(root.path()).unwrap();
    support::configure_adapter(root.path());
    root
}
fn verify(root: &TempDir) -> report::Report {
    please::verify_project_at(
        config::load(root.path()).unwrap(),
        &Never,
        please::VerificationMode::AllRules,
        Some("check"),
        &mut |_| {},
    )
    .unwrap()
}
#[test]
fn defaults_overrides_validation_and_semantic_identity() {
    let root = project();
    let mut loaded = config::load(root.path()).unwrap();
    assert_eq!(loaded.config.execution.max_parallel.get(), 4);
    let before = planner::build(&loaded, Some("check"), false)
        .unwrap()
        .plan
        .plan_id;
    loaded.config.execution.max_parallel = 1.try_into().unwrap();
    assert_eq!(
        planner::build(&loaded, Some("check"), false)
            .unwrap()
            .plan
            .plan_id,
        before
    );
    let config_path = root.path().join(".please/config.toml");
    let text = fs::read_to_string(&config_path).unwrap();
    fs::write(
        &config_path,
        text.replace("max_parallel = 4", "max_parallel = 8"),
    )
    .unwrap();
    assert_eq!(
        planner::build(&config::load(root.path()).unwrap(), Some("check"), false)
            .unwrap()
            .plan
            .plan_id,
        before
    );
    for value in ["0", "-1", "1.5", "'four'", "184467440737095516160"] {
        support::write(
            root.path(),
            ".please/config.toml",
            &format!("project = 'Example'\n[execution]\nmax_parallel = {value}\n"),
        );
        assert!(config::load(root.path()).is_err(), "accepted {value}");
    }
    support::write(
        root.path(),
        ".please/config.toml",
        "project = 'Example'\n[execution]\nmax_parallel = 2\n",
    );
    assert_eq!(
        config::load(root.path())
            .unwrap()
            .config
            .execution
            .max_parallel
            .get(),
        2
    );
    for value in ["0", "-1", "1.5", "184467440737095516160"] {
        assert!(Cli::try_parse_from(["please", "run", "--max-parallel", value]).is_err());
    }
    let cli =
        Cli::try_parse_from(["please", "run", "--at", "check", "--max-parallel", "6"]).unwrap();
    assert!(matches!(cli.command, Words::Run { max_parallel: Some(n), .. } if n.get() == 6));
    for resource in ["", " leading", "trailing ", "two\nlines", "escape\u{1b}"] {
        support::check(root.path(), "invalid", &["check"], "exit 0");
        let path = root.path().join(".please/checks/invalid/rule.toml");
        let text = fs::read_to_string(&path)
            .unwrap()
            .replace("enabled = true", "enabled = false");
        fs::write(
            path,
            format!(
                "{text}resource = {}\n",
                serde_json::to_string(resource).unwrap()
            ),
        )
        .unwrap();
        assert!(
            config::load(root.path()).is_err(),
            "accepted resource {resource:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn independent_processes_overlap_and_captures_remain_attributed() {
    let root = project();
    let signals = tempfile::tempdir().unwrap();
    for (id, other) in [("a", "b"), ("b", "a")] {
        support::check(
            root.path(),
            id,
            &["check"],
            &format!(
                "touch '{dir}/{id}'\ni=0\nwhile [ ! -f '{dir}/{other}' ]; do i=$((i+1)); [ $i -lt 500 ] || exit 2; sleep 0.01; done\nprintf '{id} stdout'\nprintf '{id} stderr' >&2\n",
                dir = signals.path().display()
            ),
        );
    }
    let report = verify(&root);
    assert_eq!(report.overall, Status::Pass);
    assert_eq!(
        report
            .results
            .iter()
            .map(|i| i.id.as_str())
            .collect::<Vec<_>>(),
        ["a", "b"]
    );
    for item in &report.results {
        assert_eq!(
            item.stdout.as_ref().unwrap().text,
            format!("{} stdout", item.id)
        );
        assert_eq!(
            item.stderr.as_ref().unwrap().text,
            format!("{} stderr", item.id)
        );
    }
    let saved = report::read(root.path()).unwrap();
    assert_eq!(saved.schema_version, 4);
    assert_eq!(saved.plan.plan_schema_version, 3);
    assert!(please::terminal::render_details(&saved, false).contains("b stderr"));
    // A CLI override wins over a configured serial limit, and leaves it untouched.
    let config_path = root.path().join(".please/config.toml");
    let text = fs::read_to_string(&config_path)
        .unwrap()
        .replace("max_parallel = 4", "max_parallel = 1");
    fs::write(&config_path, &text).unwrap();
    for id in ["a", "b"] {
        fs::remove_file(signals.path().join(id)).unwrap();
    }
    let output = support::command(env!("CARGO_BIN_EXE_please"))
        .args(["run", "--at", "check", "--max-parallel", "2"])
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(fs::read_to_string(config_path).unwrap(), text);
}

#[cfg(unix)]
#[test]
fn failures_timeouts_and_spawn_errors_release_resources() {
    for script in ["exit 1", "exit 2", "sleep 30"] {
        let root = project();
        support::check(root.path(), "a", &["check"], script);
        support::check(root.path(), "b", &["check"], "printf recovered");
        for id in ["a", "b"] {
            let path = root.path().join(format!(".please/checks/{id}/rule.toml"));
            let text = fs::read_to_string(&path)
                .unwrap()
                .replace("timeout_seconds = 10", "timeout_seconds = 1");
            fs::write(path, format!("{text}resource = 'shared'\n")).unwrap();
        }
        let report = verify(&root);
        assert_ne!(report.results[0].status, Status::Pass);
        assert_eq!(report.results[1].status, Status::Pass);
        let path = root.path().join(".please/checks/a/rule.toml");
        fs::write(
            &path,
            fs::read_to_string(&path)
                .unwrap()
                .replace("sh @rule/entrypoint", "please-missing-executable"),
        )
        .unwrap();
        let report = verify(&root);
        assert_eq!(report.results[0].status, Status::Error);
        assert_eq!(report.results[1].status, Status::Pass);
    }
}

#[cfg(unix)]
#[test]
fn cli_override_controls_serial_execution() {
    let root = project();
    let signals = tempfile::tempdir().unwrap();
    support::write(
        root.path(),
        ".please/config.toml",
        "project = 'Example'\n[execution]\nmax_parallel = 4\n",
    );
    support::check(
        root.path(),
        "a",
        &["check"],
        &format!("touch '{}/finished'", signals.path().display()),
    );
    support::check(
        root.path(),
        "b",
        &["check"],
        &format!("test -f '{}/finished'", signals.path().display()),
    );
    let output = support::command(env!("CARGO_BIN_EXE_please"))
        .args(["run", "--at", "check", "--max-parallel", "1"])
        .current_dir(root.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
