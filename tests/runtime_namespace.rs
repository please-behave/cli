#![cfg(unix)]
mod support;
use please::{
    Progress, VerificationMode, config,
    provider::{Provider, Request},
    report::{self, Status},
};
use std::{fs, process::Command};
use support::{check, write};

struct Never;
impl Provider for Never {
    fn review(&self, _: &Request<'_>) -> anyhow::Result<String> {
        panic!("no provider call expected")
    }
}

fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    assert!(
        Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(root.path())
            .status()
            .unwrap()
            .success()
    );
    config::init(root.path()).unwrap();
    fs::remove_dir_all(root.path().join(".please/checks")).unwrap();
    fs::remove_dir_all(root.path().join(".please/reviews")).unwrap();
    root
}

#[test]
fn product_prefix_is_reserved_for_rules_and_providers() {
    for id in ["please-plan", "please-snapshot", "please-tree-stability"] {
        let root = fixture();
        check(root.path(), id, &["check"], "exit 0");
        let error = config::load(root.path()).unwrap_err();
        assert!(format!("{error:#}").contains("IDs beginning please- are reserved"));
    }
    let root = fixture();
    check(root.path(), "hannes-plan", &["check"], "exit 0");
    assert!(config::load(root.path()).is_ok());
    write(
        root.path(),
        ".please/providers/please-provider.toml",
        "provider = 'codex-cli'\ntimeout_seconds = 5\n",
    );
    assert!(
        format!("{:#}", config::load(root.path()).unwrap_err())
            .contains("IDs beginning please- are reserved")
    );
}

#[test]
fn library_phases_and_saved_integrity_results_use_product_namespace() {
    let root = fixture();
    write(
        root.path(),
        ".hannes/config.toml",
        "not valid product configuration",
    );
    write(root.path(), ".hannes/reports/last.json", "governor state");
    let mut phases = Vec::new();
    let report = please::verify_project_at(
        config::load(root.path()).unwrap(),
        &Never,
        VerificationMode::ChecksOnly,
        Some("check"),
        &mut |event| {
            if let Progress::Phase(message) = event {
                phases.push(message);
            }
        },
    )
    .unwrap();
    assert_eq!(report.overall, Status::Pass);
    assert_eq!(
        phases,
        [
            "Preparing execution plan and working files …",
            "Checks only: reviews omitted; no provider call.",
            "No active checks configured.",
            "Capturing working files …",
            "Checking final working file state …",
            "Saving report …",
        ]
    );
    assert_eq!(
        fs::read_to_string(root.path().join(".hannes/reports/last.json")).unwrap(),
        "governor state"
    );
    check(
        root.path(),
        "mutate",
        &["check"],
        "printf changed > changed",
    );
    let report = please::verify_project_at(
        config::load(root.path()).unwrap(),
        &Never,
        VerificationMode::AllRules,
        Some("check"),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.schema_version, 4);
    assert_eq!(report.overall, Status::Error);
    assert!(
        report
            .results
            .iter()
            .any(|item| item.id == "please-tree-stability")
    );
    assert!(
        report::read(root.path())
            .unwrap()
            .results
            .iter()
            .any(|item| item.id == "please-tree-stability")
    );

    // Incomplete input blocks execution and records the product's plan result.
    let oversized = fs::File::create(root.path().join("oversized")).unwrap();
    oversized
        .set_len(please::limits::MAX_SNAPSHOT_FILE_BYTES + 1)
        .unwrap();
    let report = please::verify_project_at(
        config::load(root.path()).unwrap(),
        &Never,
        VerificationMode::AllRules,
        Some("check"),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.overall, Status::Error);
    assert!(report.results.iter().any(|item| item.id == "please-plan"));
    assert!(
        report
            .results
            .iter()
            .all(|item| !item.id.starts_with("hannes-"))
    );
}

#[test]
fn cli_provider_clears_product_model_without_using_governor_model() {
    let root = fixture();
    write(root.path(), "evidence", "Evidence\n");
    write(
        root.path(),
        ".please/reviews/review/rule.toml",
        "id = 'review'\ndescription = 'Review evidence'\nrun_at = ['check']\n[review]\nprompt = 'prompt.md'\n",
    );
    write(
        root.path(),
        ".please/reviews/review/prompt.md",
        "Inspect evidence.",
    );
    write(
        root.path(),
        ".please/provider.toml",
        "provider = 'cli'\ncommand = 'python3'\nargs = ['tool.py']\ntimeout_seconds = 5\n",
    );
    write(
        root.path(),
        "tool.py",
        r#"import json, os, sys
assert 'PLEASE_MODEL' not in os.environ
assert os.environ['HANNES_MODEL'] == 'governor-model'
assert os.environ['PLEASE_EXECUTION_POINT'] == 'check'
sys.stdin.read()
print(json.dumps({'reviews': [{'id': 'review', 'status': 'PASS', 'reason': 'Evidence inspected', 'findings': [{'message': 'Evidence inspected', 'path': 'evidence', 'line': 1}]}]}))
"#,
    );
    let output = Command::new(env!("CARGO_BIN_EXE_please"))
        .args(["run", "--at", "check"])
        .current_dir(root.path())
        .env("PLEASE_MODEL", "inherited-product-model")
        .env("HANNES_MODEL", "governor-model")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(report::read(root.path()).unwrap().overall, Status::Pass);
}
