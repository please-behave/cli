mod support;
use please::{
    config, external, planner,
    provider::{Provider, Request},
    report::{self, Status},
};
use std::{fs, path::Path, process::Command, sync::Mutex};
use support::write;

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
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
    config::init(root.path()).unwrap();
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
            "provider = 'codex-cli'\ntimeout_seconds = 17\nmodel = {model:?}\nmcp_servers = []\n[review_profile]\nreasoning_effort = 'low'\ntool_output_token_limit = 512\n"
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
    assert_eq!(p.review_provider(&p.reviews[0]).model, p.provider.model);
    assert_eq!(
        p.review_provider(&p.reviews[1]).model.as_deref(),
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
    assert_eq!(
        config::load(root.path()).unwrap().provider.model,
        config::ProviderConfig::scaffold().model
    );
}

#[test]
fn missing_invalid_and_credential_bearing_configurations_fail_closed() {
    let root = project();
    review(root.path(), ".please", "named", Some("missing"), "");
    let path = root.path().join(".please/reviews/named/rule.toml");
    let text = fs::read_to_string(&path)
        .unwrap()
        .replace("run_at =", "enabled = false\nrun_at =");
    fs::write(&path, text).unwrap();
    let error = format!("{:#}", config::load(root.path()).unwrap_err());
    assert!(
        error.contains("named") && error.contains("missing"),
        "{error}"
    );
    provider(root.path(), ".please", "missing", "model");
    let config_path = root.path().join(".please/providers/missing.toml");
    let valid = fs::read_to_string(&config_path).unwrap();
    for invalid in [
        valid.replace("codex-cli", "gemini"),
        valid.replace("17", "0"),
        valid.replace("'low'", "'unknown'"),
        valid.replace("512", "1"),
        valid.replace("mcp_servers = []", "mcp_servers = ['*']"),
        format!("api_token = 'unsupported'\n{valid}"),
        format!("{valid}\nsecret = 'unsupported'"),
        valid.replace("model = \"model\"", "model = ''"),
    ] {
        fs::write(&config_path, invalid).unwrap();
        let error = format!("{:#}", config::load(root.path()).unwrap_err());
        assert!(error.contains("missing"), "{error}");
    }
    fs::write(&config_path, valid).unwrap();
    assert!(config::load(root.path()).is_ok());
    fs::rename(&config_path, config_path.with_file_name("bad.id.toml")).unwrap();
    assert!(config::load(root.path()).is_err());
}

struct Recording(Mutex<Vec<(String, Vec<String>)>>);
impl Provider for Recording {
    fn review(&self, request: &Request<'_>) -> anyhow::Result<String> {
        let config = request.provider_config()?;
        self.0.lock().unwrap().push((
            config.model.clone().unwrap(),
            request.reviews.iter().map(|r| r.id.clone()).collect(),
        ));
        Ok(serde_json::json!({"reviews":request.reviews.iter().map(|r| serde_json::json!({"id":r.id,"status":"PASS","reason":"Evidence inspected","findings":[{"message":"Evidence","path":"evidence","line":1}]})).collect::<Vec<_>>()} ).to_string())
    }
}
#[test]
fn batching_and_saved_reports_keep_provider_and_parameter_boundaries() {
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
        4
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
    assert_eq!(calls.len(), 4);
    assert!(
        calls
            .iter()
            .any(|(m, ids)| m == "first-model" && ids == &["a", "b"])
    );
    assert!(calls.iter().any(|(_, ids)| ids == &["d", "e"]));
    let saved = report::read(root.path()).unwrap();
    assert_eq!(saved.plan.scheduled[2].provider.as_deref(), Some("second"));
    assert_eq!(
        saved.plan.scheduled[2]
            .provider_config
            .as_ref()
            .unwrap()
            .model
            .as_deref(),
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
    assert!(
        p.reviews
            .iter()
            .all(|r| p.review_provider(r).model.as_deref() == Some("central-model"))
    );
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
    assert_eq!(recorder.0.lock().unwrap().len(), 1);
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
fn codex_invocations_use_each_named_configuration() {
    use std::os::unix::fs::PermissionsExt;
    let root = project();
    for (id, model) in [("first", "first-model"), ("second", "second-model")] {
        provider(root.path(), ".please", id, model);
        review(root.path(), ".please", id, Some(id), "");
    }
    let second = root.path().join(".please/providers/second.toml");
    let text = fs::read_to_string(&second)
        .unwrap()
        .replace("'low'", "'high'")
        .replace("512", "1024")
        .replace("mcp_servers = []", "mcp_servers = ['allowed']");
    fs::write(second, text).unwrap();
    let fake = tempfile::tempdir().unwrap();
    let script = fake.path().join("codex");
    fs::write(&script, r#"#!/usr/bin/env python3
import json, sys
if sys.argv[1] == 'mcp':
    print('[{"name":"allowed"},{"name":"other"}]')
else:
    args = sys.argv[1:]
    model = args[args.index('--model') + 1]
    if model == 'first-model':
        id, effort, limit, allowed = 'first', 'low', 512, 'false'
    elif model == 'second-model':
        id, effort, limit, allowed = 'second', 'high', 1024, 'true'
    else:
        raise AssertionError(model)
    assert 'model_reasoning_effort="' + effort + '"' in args
    assert 'tool_output_token_limit=' + str(limit) in args
    assert 'mcp_servers.allowed.enabled=' + allowed in args
    assert 'mcp_servers.other.enabled=false' in args
    assert '--sandbox' in args and 'read-only' in args
    assert '--ask-for-approval' in args and 'never' in args
    sys.stdin.read()
    with open(args[args.index('--output-last-message') + 1], 'w') as output:
        json.dump({'reviews':[{'id':id,'status':'PASS','reason':'Settings delivered','findings':[{'message':'Evidence','path':'evidence','line':1}]}]}, output)
"#).unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    let p = config::load(root.path()).unwrap();
    let result = please::verify_project_at(
        p,
        &please::provider::Codex::new(&script),
        please::VerificationMode::AllRules,
        Some("check"),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(result.overall, Status::Pass, "{:?}", result.results);
    assert_eq!(
        report::read(root.path()).unwrap().plan.planned_model_calls,
        2
    );
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
        "provider = 'cli'\ncommand = './tool'\nmodel = 'central-model'\ntimeout_seconds = 5\n[requirements]\nenv = ['PLEASE_MODEL']\nexecutables = ['./tool']\n",
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
assert os.environ['PLEASE_MODEL'] == 'central-model'
prompt = sys.stdin.read()
line = next(line for line in prompt.splitlines() if line.startswith('Configured reviews (JSON data): '))
id = json.loads(line.split(': ', 1)[1])[0]['id']
print(json.dumps({'reviews':[{'id':id,'status':'PASS','reason':'Evidence inspected','findings':[{'message':'Evidence','path':'evidence','line':1}]}]}))
"#,
    );
    fs::set_permissions(root.path().join("tool"), fs::Permissions::from_mode(0o755)).unwrap();
    let p = external::load(root.path(), false, Some(cache.path())).unwrap();
    let result = please::verify_project_at(
        p,
        &please::provider::Configured::default(),
        please::VerificationMode::AllRules,
        Some("check"),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(result.overall, Status::Pass, "{:?}", result.results);
}
