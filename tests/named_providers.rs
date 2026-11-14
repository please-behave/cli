mod support;
use please::{
    config, external, planner,
    provider::{Provider, Request},
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
    support::configure_adapter(root.path());
    fs::remove_dir_all(root.path().join(".please/checks")).unwrap();
    fs::remove_dir_all(root.path().join(".please/reviews")).unwrap();
    write(root.path(), "evidence", "Evidence\n");
    root
}
fn provider(root: &Path, directory: &str, id: &str, model: &str) {
    write(
        root,
        &format!("{directory}/providers/{id}.toml"),
        &format!(
            "command = 'fake-adapter'\ntimeout_seconds = 17\n[parameters]\nlabel = {model:?}\n"
        ),
    );
}
fn review(root: &Path, directory: &str, id: &str, reference: Option<&str>, parameters: &str) {
    let reference = reference
        .map(|id| format!("provider = {id:?}\n"))
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
    let root = project();
    provider(root.path(), ".please", "quality", "quality-model");
    provider(root.path(), ".please", "fast", "fast-model");
    review(root.path(), ".please", "named", Some("quality"), "");
    review(root.path(), ".please", "legacy", None, "");
    let p = config::load(root.path()).unwrap();
    assert_eq!(p.providers.len(), 2);
    assert_eq!(
        p.review_provider(&p.reviews[0]).unwrap().parameters,
        p.provider.as_ref().unwrap().parameters
    );
    assert_eq!(
        p.review_provider(&p.reviews[1]).unwrap().parameters["label"].as_str(),
        Some("quality-model")
    );
    let plan = planner::build(&p, Some("check"), false).unwrap().plan;
    assert_eq!(plan.planned_model_calls, 2);
    assert!(plan.scheduled[0].provider_config.is_none());
    assert_eq!(plan.scheduled[1].provider.as_deref(), Some("quality"));
    assert_eq!(
        plan.scheduled[1]
            .provider_config
            .as_ref()
            .unwrap()
            .timeout_seconds,
        17
    );
    fs::remove_file(root.path().join(".please/provider.toml")).unwrap();
    assert!(config::load(root.path()).unwrap().provider.is_none());
}

struct Recording(Mutex<Vec<(String, Vec<String>)>>);
impl Provider for Recording {
    fn review(&self, request: &Request<'_>) -> anyhow::Result<String> {
        let config = request.provider_config()?;
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
fn calls_and_saved_reports_keep_provider_and_parameter_boundaries() {
    let root = project();
    provider(root.path(), ".please", "first", "first-model");
    provider(root.path(), ".please", "second", "second-model");
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
    let provider = Recording(Mutex::new(vec![]));
    let result = please::verify_project_at(
        p,
        &provider,
        please::VerificationMode::AllRules,
        Some("check"),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(result.overall, Status::Pass);
    let calls = provider.0.lock().unwrap();
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
    assert_eq!(saved.plan.scheduled[2].provider.as_deref(), Some("second"));
    assert_eq!(
        saved.plan.scheduled[2]
            .provider_config
            .as_ref()
            .unwrap()
            .parameters["label"]
            .as_str(),
        Some("second-model")
    );
    let path = root.path().join(".please/reports/last.json");
    let mut json: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    json["plan"]["scheduled"][2]
        .as_object_mut()
        .unwrap()
        .remove("provider_config");
    fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    assert!(report::read(root.path()).is_err());
}

fn set(root: &Path, directory: &str, rules: &[&str]) {
    write(
        root,
        &format!("{directory}/config.toml"),
        &format!(
            "id = 'quality'\ndescription = 'Shared quality'\nrules = {rules:?}\nrun_at = ['check']\n"
        ),
    );
}
fn imports(root: &Path, remote: &Path, selections: &[&str]) {
    let mut text = "project = 'My_App'\n".to_owned();
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
fn imported_providers_resolve_across_sets_offline_and_reconcile_atomically() {
    let root = project();
    let remote = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    git(remote.path(), &["init", "--quiet", "--initial-branch=main"]);
    set(remote.path(), "definitions", &[]);
    provider(remote.path(), "definitions", "central", "central-model");
    set(remote.path(), "reviews", &["rules/reviews/external"]);
    review(remote.path(), "rules", "external", Some("central"), "");
    commit(remote.path());
    review(root.path(), ".please", "local", Some("central"), "");
    imports(root.path(), remote.path(), &["reviews", "definitions"]);
    let p = external::load(root.path(), true, Some(cache.path())).unwrap();
    assert_eq!(p.providers.len(), 1);
    assert_eq!(p.reviews.len(), 2);
    assert!(p.reviews.iter().all(
        |r| p.review_provider(r).unwrap().parameters["label"].as_str() == Some("central-model")
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
            .providers
            .len(),
        1
    );
    let repeated_state = fs::read(&state_path).unwrap();
    assert_eq!(state, repeated_state);
    // A local declaration cannot override centrally distributed settings.
    provider(root.path(), ".please", "central", "central-model");
    let error = format!(
        "{:#}",
        external::load(root.path(), true, Some(cache.path())).unwrap_err()
    );
    assert!(
        error.contains("duplicate provider") && error.contains("central"),
        "{error}"
    );
    assert_eq!(fs::read(&state_path).unwrap(), state);
    fs::remove_dir_all(root.path().join(".please/providers")).unwrap();
    // Distinct imported files collide even when their settings are identical.
    provider(remote.path(), "reviews", "central", "central-model");
    commit(remote.path());
    assert!(
        format!(
            "{:#}",
            external::load(root.path(), true, Some(cache.path())).unwrap_err()
        )
        .contains("duplicate provider")
    );
    assert_eq!(fs::read(&state_path).unwrap(), state);
    fs::remove_dir_all(remote.path().join("reviews/providers")).unwrap();
    fs::remove_dir_all(remote.path().join("definitions/providers")).unwrap();
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

#[cfg(unix)]
#[test]
fn provider_files_preserve_source_boundaries() {
    use std::os::unix::fs::symlink;
    let root = project();
    let outside = tempfile::tempdir().unwrap();
    provider(outside.path(), ".please", "quality", "model");
    fs::create_dir(root.path().join(".please/providers")).unwrap();
    symlink(
        outside.path().join(".please/providers/quality.toml"),
        root.path().join(".please/providers/quality.toml"),
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
        "quality/providers/central.toml",
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
id = request['reviews'][0]['id']
print(json.dumps({'protocol_version':1,'reviews':[{'id':id,'status':'PASS','reason':'Evidence inspected','findings':[{'message':'Evidence','path':'evidence','line':1}]}]}))
"#,
    );
    fs::set_permissions(root.path().join("tool"), fs::Permissions::from_mode(0o755)).unwrap();
    let p = external::load(root.path(), false, Some(cache.path())).unwrap();
    let result = please::verify_project_at(
        p,
        &please::provider::Configured,
        please::VerificationMode::AllRules,
        Some("check"),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(result.overall, Status::Pass, "{:?}", result.results);
}
