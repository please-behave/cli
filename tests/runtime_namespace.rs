#![cfg(unix)]
mod support;
use please::{
    Progress, VerificationMode, config,
    provider::{Provider, Request},
    report::{self, Status},
};
use std::fs;
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
        support::command("git")
            .args(["init", "--quiet"])
            .current_dir(root.path())
            .status()
            .unwrap()
            .success()
    );
    config::init(root.path()).unwrap();
    support::configure_adapter(root.path());
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
        "command = 'fake-adapter'\ntimeout_seconds = 5\n",
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
            "Checks only: reviews not run; no provider call.",
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
