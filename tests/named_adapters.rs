mod support;
use please::{
    adapter::{Adapter, Request},
    config, external, planner,
    report::{self, Status},
};
use std::{fs, path::Path, sync::Mutex};
use support::write;

fn git(root: &Path, args: &[&str]) -> String {
    let output = support::command("git")
        .current_dir(root)
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}
fn project() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "--quiet", "--initial-branch=main"]);
    please::init::init(root.path()).unwrap();
    support::commit_all(root.path());
    support::use_head_as_change_base();
    support::configure_adapter(root.path());
    fs::remove_dir_all(root.path().join(".please/checks")).unwrap();
    fs::remove_dir_all(root.path().join(".please/reviews")).unwrap();
    write(root.path(), "evidence", "Evidence\n");
    root
}
fn adapter(root: &Path, directory: &str, id: &str, model: &str) {
    write(
        root,
        &format!("{directory}/adapters/{id}.toml"),
        &format!(
            "command = 'fake-adapter'\ntimeout_seconds = 17\n[parameters]\nlabel = {model:?}\n"
        ),
    );
}
fn review(root: &Path, directory: &str, id: &str, reference: Option<&str>, parameters: &str) {
    let reference = reference
        .map(|id| format!("adapter = {id:?}\n"))
        .unwrap_or_default();
    write(
        root,
        &format!("{directory}/reviews/{id}/rule.toml"),
        &format!(
            "id = {id:?}\ndescription = 'Review evidence'\nrun_at = ['check']\n{parameters}\n[review]\n{reference}prompt = 'prompt.md'\n"
        ),
    );
    write(
        root,
        &format!("{directory}/reviews/{id}/prompt.md"),
        "Inspect evidence.\n",
    );
}

#[test]
fn named_references_load_without_changing_legacy_defaults() {
    // REQ-0214: explicit adapter references and the optional default resolve independently.
    let root = project();
    adapter(root.path(), ".please", "quality", "quality-model");
    adapter(root.path(), ".please", "fast", "fast-model");
    review(root.path(), ".please", "named", Some("quality"), "");
    review(root.path(), ".please", "legacy", None, "");
    let p = config::load(root.path()).unwrap();
    assert_eq!(p.adapters.len(), 2);
    assert_eq!(
        p.review_adapter(&p.reviews[0]).unwrap().parameters,
        p.adapter.as_ref().unwrap().parameters
    );
    assert_eq!(
        p.review_adapter(&p.reviews[1]).unwrap().parameters["label"].as_str(),
        Some("quality-model")
    );
    let plan = planner::build(&p, Some("check"), false).unwrap().plan;
    assert_eq!(plan.planned_model_calls, 2);
    assert!(plan.scheduled[0].adapter_profile.is_none());
    assert_eq!(plan.scheduled[1].adapter.as_deref(), Some("quality"));
    assert_eq!(
        plan.scheduled[1]
            .adapter_profile
            .as_ref()
            .unwrap()
            .timeout_seconds,
        17
    );
    fs::remove_file(root.path().join(".please/adapter.toml")).unwrap();
    assert!(config::load(root.path()).unwrap().adapter.is_none());
}

struct Recording(Mutex<Vec<(String, Vec<String>)>>);
impl Adapter for Recording {
    fn review(&self, request: &Request<'_>) -> anyhow::Result<String> {
        let config = request.adapter_profile()?;
        self.0.lock().unwrap().push((
            config
                .parameters
                .get("label")
                .and_then(|v| v.as_str())
                .unwrap_or("default")
                .to_owned(),
            request.reviews.iter().map(|r| r.id.clone()).collect(),
        ));
        Ok(serde_json::json!({"reviews":request.reviews.iter().map(|r| serde_json::json!({"id":r.id,"status":"PASS","reason":"Evidence inspected","findings":[{"message":"Evidence","path":"evidence","line":1}]})).collect::<Vec<_>>()} ).to_string())
    }
}
#[test]
fn calls_and_saved_reports_keep_adapter_and_parameter_boundaries() {
    let root = project();
    adapter(root.path(), ".please", "first", "first-model");
    adapter(root.path(), ".please", "second", "second-model");
    for (id, reference, parameters) in [
        ("a", Some("first"), ""),
        ("b", Some("first"), ""),
        ("c", Some("second"), ""),
        ("d", None, ""),
        ("e", None, ""),
        (
            "f",
            Some("first"),
            "[parameters.label]\ntype = 'string'\nvalue = 'other'\n",
        ),
    ] {
        review(root.path(), ".please", id, reference, parameters);
    }
    let p = config::load(root.path()).unwrap();
    assert_eq!(
        planner::build(&p, Some("check"), false)
            .unwrap()
            .plan
            .planned_model_calls,
        6
    );
    let adapter = Recording(Mutex::new(vec![]));
    let result = please::verify_project_at(
        p,
        &adapter,
        please::VerificationMode::AllRules,
        Some("check"),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(result.overall, Status::Pass);
    let calls = adapter.0.lock().unwrap();
    assert_eq!(calls.len(), 6);
    assert!(calls.iter().all(|(_, ids)| ids.len() == 1));
    assert!(
        calls
            .iter()
            .any(|(m, ids)| m == "first-model" && ids == &["a"])
    );
    assert!(calls.iter().any(|(_, ids)| ids == &["d"]));
    assert!(calls.iter().any(|(_, ids)| ids == &["e"]));
    let saved = report::read(root.path()).unwrap();
    // REQ-0339: each result names the adapter entry that produced it; the default has no ID.
    let adapters: Vec<_> = saved
        .results
        .iter()
        .map(|item| (item.id.as_str(), item.adapter.as_deref()))
        .collect();
    assert_eq!(
        adapters,
        [
            ("a", Some("first")),
            ("b", Some("first")),
            ("c", Some("second")),
            ("d", None),
            ("e", None),
            ("f", Some("first")),
        ]
    );
    assert_eq!(saved.plan.scheduled[2].adapter.as_deref(), Some("second"));
    assert_eq!(
        saved.plan.scheduled[2]
            .adapter_profile
            .as_ref()
            .unwrap()
            .parameters["label"]
            .as_str(),
        Some("second-model")
    );
    let path = root.path().join(".please/reports/1.json");
    let mut json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    let valid = serde_json::to_vec(&json).unwrap();
    json["results"][0]["adapter"] = "second".into();
    fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    assert!(report::read(root.path()).is_err());
    fs::write(&path, valid).unwrap();
    json["plan"]["scheduled"][2]
        .as_object_mut()
        .unwrap()
        .remove("adapter_profile");
    fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    assert!(report::read(root.path()).is_err());
}

fn set(root: &Path, directory: &str, rules: &[&str]) {
    write(
        root,
        &format!("{directory}/ruleset.toml"),
        &format!(
            "id = 'quality'\ndescription = 'Shared quality'\nrules = {rules:?}\nrun_at = ['check']\n"
        ),
    );
}
fn imports(root: &Path, remote: &Path, selections: &[&str]) {
    let mut text = "project = 'My_App'\n".to_owned();
    support::trust(remote);
    for path in selections {
        text += &format!(
            "[[imports]]\ngit = {:?}\npath = {path:?}\nref = 'main'\n",
            remote.to_str().unwrap()
        );
    }
    write(root, ".please/config.toml", &text);
}
fn commit(root: &Path) {
    git(root, &["add", "."]);
    git(root, &["commit", "--quiet", "-m", "fixture"]);
}

#[test]
fn imported_adapters_resolve_across_sets_offline_and_reconcile_atomically() {
    // REQ-0210: rule-set adapters cannot override local or other imported adapter definitions.
    let root = project();
    let remote = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    git(remote.path(), &["init", "--quiet", "--initial-branch=main"]);
    set(remote.path(), "definitions", &[]);
    adapter(remote.path(), "definitions", "central", "central-model");
    set(remote.path(), "reviews", &["rules/reviews/external"]);
    review(remote.path(), "rules", "external", Some("central"), "");
    commit(remote.path());
    review(root.path(), ".please", "local", Some("central"), "");
    imports(root.path(), remote.path(), &["reviews", "definitions"]);
    let p = external::load(root.path(), true, Some(cache.path())).unwrap();
    assert_eq!(p.adapters.len(), 1);
    assert_eq!(p.reviews.len(), 2);
    assert!(p.reviews.iter().all(
        |r| p.review_adapter(r).unwrap().parameters["label"].as_str() == Some("central-model")
    ));
    let original = planner::build(&p, Some("check"), false).unwrap().plan;
    let state_path = root.path().join(".git/please/external-rule-sets.json");
    let state = fs::read(&state_path).unwrap();
    imports(root.path(), remote.path(), &["definitions", "reviews"]);
    let offline = external::load(root.path(), false, Some(cache.path())).unwrap();
    assert_eq!(
        planner::build(&offline, Some("check"), false)
            .unwrap()
            .plan
            .plan_id,
        original.plan_id
    );
    let recorder = Recording(Mutex::new(vec![]));
    assert_eq!(
        please::verify_project_at(
            offline,
            &recorder,
            please::VerificationMode::AllRules,
            Some("check"),
            &mut |_| {}
        )
        .unwrap()
        .overall,
        Status::Pass
    );
    assert_eq!(recorder.0.lock().unwrap().len(), 2);
    // Same immutable definition may be imported repeatedly without a conflict.
    imports(
        root.path(),
        remote.path(),
        &["definitions", "reviews", "definitions"],
    );
    assert_eq!(
        external::load(root.path(), true, Some(cache.path()))
            .unwrap()
            .adapters
            .len(),
        1
    );
    let repeated_state = fs::read(&state_path).unwrap();
    assert_eq!(state, repeated_state);
    // A local declaration cannot override centrally distributed settings.
    adapter(root.path(), ".please", "central", "central-model");
    let error = format!(
        "{:#}",
        external::load(root.path(), true, Some(cache.path())).unwrap_err()
    );
    assert!(
        error.contains("duplicate adapter") && error.contains("central"),
        "{error}"
    );
    assert_eq!(fs::read(&state_path).unwrap(), state);
    fs::remove_dir_all(root.path().join(".please/adapters")).unwrap();
    // Distinct imported files with identical definitions share one adapter.
    adapter(remote.path(), "reviews", "central", "central-model");
    commit(remote.path());
    assert_eq!(
        external::load(root.path(), true, Some(cache.path()))
            .unwrap()
            .adapters
            .len(),
        1
    );
    let state = fs::read(&state_path).unwrap();
    // A differing definition with the same identifier is rejected.
    adapter(remote.path(), "reviews", "central", "other-model");
    commit(remote.path());
    assert!(
        format!(
            "{:#}",
            external::load(root.path(), true, Some(cache.path())).unwrap_err()
        )
        .contains("duplicate adapter")
    );
    assert_eq!(fs::read(&state_path).unwrap(), state);
    fs::remove_dir_all(remote.path().join("reviews/adapters")).unwrap();
    fs::remove_dir_all(remote.path().join("definitions/adapters")).unwrap();
    commit(remote.path());
    let error = format!(
        "{:#}",
        external::load(root.path(), true, Some(cache.path())).unwrap_err()
    );
    assert!(
        error.contains("central") && error.contains("review"),
        "{error}"
    );
    assert_eq!(fs::read(&state_path).unwrap(), state);
    // Offline resolution retains the previously validated revision.
    assert!(external::load(root.path(), false, Some(cache.path())).is_ok());
}

#[test]
fn identical_adapters_of_different_revisions_coexist() {
    // REQ-0210: an adapter shipped by rule sets pinned at different commits of one repository is one adapter while its definition is unchanged.
    let root = project();
    let remote = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    git(remote.path(), &["init", "--quiet", "--initial-branch=main"]);
    for directory in ["first", "second"] {
        set(remote.path(), directory, &[]);
        adapter(remote.path(), directory, "central", "central-model");
    }
    commit(remote.path());
    git(remote.path(), &["tag", "v1"]);
    write(remote.path(), "second/NOTES.md", "Unrelated change\n");
    commit(remote.path());
    support::trust(remote.path());
    let configure = |first: &str, second: &str| {
        let text: String = [("first", first), ("second", second)]
            .iter()
            .map(|(path, reference)| {
                format!(
                    "[[imports]]\ngit = {:?}\npath = {path:?}\nref = {reference:?}\n",
                    remote.path().to_str().unwrap()
                )
            })
            .collect();
        write(
            root.path(),
            ".please/config.toml",
            &format!("project = 'My_App'\n{text}"),
        );
    };
    configure("v1", "main");
    assert_eq!(
        external::load(root.path(), true, Some(cache.path()))
            .unwrap()
            .adapters
            .len(),
        1
    );
    adapter(remote.path(), "second", "central", "other-model");
    commit(remote.path());
    let error = format!(
        "{:#}",
        external::load(root.path(), true, Some(cache.path())).unwrap_err()
    );
    assert!(error.contains("duplicate adapter"), "{error}");
}

#[cfg(unix)]
#[test]
fn adapter_files_preserve_source_boundaries() {
    use std::os::unix::fs::symlink;
    let root = project();
    let outside = tempfile::tempdir().unwrap();
    adapter(outside.path(), ".please", "quality", "model");
    fs::create_dir(root.path().join(".please/adapters")).unwrap();
    symlink(
        outside.path().join(".please/adapters/quality.toml"),
        root.path().join(".please/adapters/quality.toml"),
    )
    .unwrap();
    assert!(config::load(root.path()).is_err());
}

#[cfg(unix)]
#[test]
fn imported_cli_commands_follow_existing_explicit_import_trust() {
    use std::os::unix::fs::PermissionsExt;
    let root = project();
    let remote = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    git(remote.path(), &["init", "--quiet", "--initial-branch=main"]);
    set(remote.path(), "quality", &["rules/reviews/external"]);
    review(remote.path(), "rules", "external", Some("central"), "");
    write(
        remote.path(),
        "quality/adapters/central.toml",
        "command = './tool'\ntimeout_seconds = 5\n[parameters]\nlabel = 'central-model'\n[requirements]\nexecutables = ['./tool']\n",
    );
    commit(remote.path());
    imports(root.path(), remote.path(), &["quality"]);
    // Refresh validates configuration without executing imported commands or requirements.
    assert!(external::load(root.path(), true, Some(cache.path())).is_ok());
    write(
        root.path(),
        "tool",
        r#"#!/usr/bin/env python3
import json, os, sys
request = json.load(sys.stdin)
assert request['reviews'][0]['parameters']['label'] == 'central-model'
print(json.dumps({'protocol_version':1,'reason':'Evidence inspected','findings':[{'message':'Evidence','path':'evidence','line':1}]}))
"#,
    );
    fs::set_permissions(root.path().join("tool"), fs::Permissions::from_mode(0o755)).unwrap();
    let p = external::load(root.path(), false, Some(cache.path())).unwrap();
    let result = please::verify_project_at(
        p,
        &please::adapter::Configured,
        please::VerificationMode::AllRules,
        Some("check"),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(result.overall, Status::Pass, "{:?}", result.results);
}
