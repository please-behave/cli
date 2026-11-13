#![cfg(unix)]
mod support;
use please::{
    VerificationMode, config, planner,
    provider::Configured,
    report::{self, Status},
};
use std::{fs, os::unix::fs::PermissionsExt, path::Path};
use support::write;

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
    please::init::init(root.path()).unwrap();
    fs::remove_dir_all(root.path().join(".please/checks")).unwrap();
    fs::remove_dir_all(root.path().join(".please/reviews")).unwrap();
    write(root.path(), "evidence", "Evidence\n");
    root
}
fn review(root: &Path, id: &str, provider: &str, point: &str) {
    write(
        root,
        &format!(".please/reviews/{id}/rule.toml"),
        &format!(
            "id = {id:?}\ndescription = 'Inspect evidence'\nrun_at = [{point:?}]\n[review]\nprovider = {provider:?}\nprompt = 'prompt.md'\n"
        ),
    );
    write(
        root,
        &format!(".please/reviews/{id}/prompt.md"),
        "Unique review instructions\n",
    );
}
fn provider(root: &Path, id: &str, mode: &str, requirements: &str) {
    write(
        root,
        &format!(".please/providers/{id}.toml"),
        &format!(
            "command = './tool'\nargs = [{mode:?}, '$HOME', '$(false)', '|', '']\ntimeout_seconds = 1\n[parameters]\nlabel = {id:?}\n{requirements}"
        ),
    );
}
fn tool(root: &Path) {
    write(
        root,
        "tool",
        r#"#!/usr/bin/env python3
import json, os, sys, time
mode = sys.argv[1]
assert sys.argv[2:] == ['$HOME', '$(false)', '|', '']
assert not [name for name in os.environ if name.startswith('PLEASE_')]
if mode == 'unread':
    time.sleep(30)
request = json.load(sys.stdin)
assert request['protocol_version'] == 1
assert request['execution_point'] == 'check' and os.path.isfile(request['context_file'])
prompt = request['prompt']
assert 'Unique review instructions' in prompt
assert request['response_schema']['required'] == ['reviews']
assert 'positive line number' in prompt
assert 'manifest paths' in prompt
reviews = request['reviews']
assert all(r['parameters']['label'] in ['first', 'second'] for r in reviews)
if mode == 'malformed':
    print('{"private-prompt-secret": "never expose"}')
    sys.exit(0)
if mode == 'nonzero':
    print('private-prompt-secret', file=sys.stderr)
    sys.exit(7)
if mode == 'timeout':
    time.sleep(30)
if mode == 'oversized':
    print('x' * 70000)
    sys.exit(0)
status = 'FAIL' if mode == 'fail' else 'ERROR' if mode == 'error' else 'PASS'
findings = [] if mode == 'ungrounded' else [{'message':'Evidence inspected','path':'evidence','line':1}]
print(json.dumps({'protocol_version':1,'reviews':[{'id': r['id'], 'status':status, 'reason':'Evidence inspected', 'findings':findings} for r in reviews]}))
"#,
    );
    fs::set_permissions(root.join("tool"), fs::Permissions::from_mode(0o755)).unwrap();
}
fn execute(root: &Path) -> report::Report {
    please::verify_project_at(
        config::load(root).unwrap(),
        &Configured,
        VerificationMode::AllRules,
        Some("check"),
        &mut |_| {},
    )
    .unwrap()
}
#[test]
fn stdin_results_and_provider_isolation() {
    let root = fixture();
    tool(root.path());
    provider(
        root.path(),
        "first",
        "ok",
        "[requirements]\nenv = ['PATH']\nexecutables = ['./tool', 'python3']\n",
    );
    provider(root.path(), "second", "ok", "");
    for (id, reference) in [("a", "first"), ("b", "first"), ("c", "second")] {
        review(root.path(), id, reference, "check");
    }
    let project = config::load(root.path()).unwrap();
    assert_eq!(
        planner::build(&project, Some("check"), false)
            .unwrap()
            .plan
            .planned_model_calls,
        2
    );
    let result = execute(root.path());
    assert_eq!(result.overall, Status::Pass, "{:?}", result.results);
    assert_eq!(result.results.len(), 3);
    assert!(result.provider_usage.is_none());
    assert_eq!(report::read(root.path()).unwrap().overall, Status::Pass);
}
#[test]
fn process_and_response_errors_never_pass_or_expose_raw_output() {
    for mode in [
        "malformed",
        "nonzero",
        "timeout",
        "unread",
        "oversized",
        "ungrounded",
    ] {
        let root = fixture();
        tool(root.path());
        provider(root.path(), "first", mode, "");
        review(root.path(), "a", "first", "check");
        if mode == "unread" {
            // Larger than the pipe capacity: stdin must remain inside the deadline.
            write(
                root.path(),
                ".please/reviews/a/prompt.md",
                &format!("Unique review instructions\n{}", "\\\n".repeat(14000)),
            );
        }
        let start = std::time::Instant::now();
        let result = execute(root.path());
        assert_eq!(
            result.overall,
            Status::Error,
            "{mode}: {:?}",
            result.results
        );
        assert!(start.elapsed().as_secs() < 10, "{mode}");
        assert!(
            !fs::read_to_string(root.path().join(".please/reports/last.json"))
                .unwrap()
                .contains("private-prompt-secret")
        );
    }
}
#[test]
fn runtime_requirements_are_deferred_until_selected_after_checks() {
    let root = fixture();
    tool(root.path());
    provider(
        root.path(),
        "first",
        "ok",
        "[requirements]\nenv = ['TEST_MISSING_CREDENTIAL_71F52']\n",
    );
    review(root.path(), "a", "first", "pre-push");
    // Unused and inapplicable providers do not require credentials, including plans.
    let project = config::load(root.path()).unwrap();
    assert!(
        planner::build(&project, Some("pre-push"), false)
            .unwrap()
            .plan
            .executable()
    );
    assert_eq!(execute(root.path()).overall, Status::Pass);
    review(root.path(), "a", "first", "check");
    support::check(root.path(), "gate", &["check"], "exit 1\n");
    let gated = execute(root.path());
    assert_eq!(gated.overall, Status::Fail);
    assert!(!format!("{:?}", gated.results).contains("required environment variable"));
    fs::remove_dir_all(root.path().join(".please/checks")).unwrap();
    let result = execute(root.path());
    assert_eq!(result.overall, Status::Error);
    assert!(format!("{:?}", result.results).contains("required environment variable"));
    provider(
        root.path(),
        "first",
        "ok",
        "[requirements]\nexecutables = ['please-missing-test-tool-71f52']\n",
    );
    assert!(format!("{:?}", execute(root.path()).results).contains("required executable"));
    provider(root.path(), "first", "ok", "");
    fs::remove_file(root.path().join("tool")).unwrap();
    assert_eq!(execute(root.path()).overall, Status::Error);
}
#[test]
fn adapter_configuration_is_strict_and_defaults_are_explicit() {
    let root = fixture();
    provider(root.path(), "first", "ok", "");
    review(root.path(), "a", "first", "check");
    let path = root.path().join(".please/providers/first.toml");
    let valid = fs::read_to_string(&path).unwrap();
    for invalid in [
        valid.replace("command = './tool'", "command = ''"),
        format!("provider = 'unsupported'\n{valid}"),
        format!("{valid}\n[requirements]\nenv = ['BAD=NAME']\n"),
        format!("unknown = true\n{valid}"),
        format!("api_key = 'forbidden'\n{valid}"),
    ] {
        fs::write(&path, invalid).unwrap();
        assert!(config::load(root.path()).is_err());
    }
    fs::write(
        &path,
        valid.replace("timeout_seconds = 1", "timeout_seconds = 0"),
    )
    .unwrap();
    assert_eq!(
        config::load(root.path())
            .unwrap()
            .review_provider(&config::load(root.path()).unwrap().reviews[0])
            .unwrap()
            .timeout_seconds,
        0
    );
    fs::write(path, valid).unwrap();
    let project = config::load(root.path()).unwrap();
    assert!(project.provider.is_none());
    assert_eq!(
        project
            .review_provider(&project.reviews[0])
            .unwrap()
            .command,
        "./tool"
    );
    fs::write(
        root.path().join(".please/provider.toml"),
        "command = 'missing-but-valid'\ntimeout_seconds = 5\n",
    )
    .unwrap();
    assert_eq!(
        config::load(root.path()).unwrap().provider.unwrap().command,
        "missing-but-valid"
    );
}
#[test]
fn completed_fail_and_error_respect_review_blocking() {
    let root = fixture();
    tool(root.path());
    provider(root.path(), "first", "fail", "");
    review(root.path(), "a", "first", "check");
    let path = root.path().join(".please/reviews/a/rule.toml");
    fs::write(
        &path,
        fs::read_to_string(&path)
            .unwrap()
            .replace("description =", "blocking = false\ndescription ="),
    )
    .unwrap();
    assert_eq!(execute(root.path()).overall, Status::Pass);
    for mode in ["error", "nonzero", "malformed", "timeout"] {
        provider(root.path(), "first", mode, "");
        let result = execute(root.path());
        assert_eq!(result.overall, Status::Pass);
        assert_eq!(
            result
                .results
                .iter()
                .find(|r| r.kind == "review")
                .unwrap()
                .status,
            Status::Error
        );
    }
    fs::write(
        &path,
        fs::read_to_string(&path)
            .unwrap()
            .replace("blocking = false", "blocking = true"),
    )
    .unwrap();
    assert_eq!(execute(root.path()).overall, Status::Error);
}

#[test]
fn protocol_versions_complete_coverage_and_usage_fail_closed() {
    for mode in [
        "wrong-version",
        "missing-version",
        "duplicate-key",
        "missing-review",
        "extra-review",
        "missing-path",
        "bad-usage",
        "fenced",
        "valid-usage",
    ] {
        let root = fixture();
        write(
            root.path(),
            "tool",
            r#"#!/usr/bin/env python3
import json, sys
request = json.load(sys.stdin)
mode = sys.argv[1]
r = {'id': request['reviews'][0]['id'], 'status':'PASS', 'reason':'Evidence inspected', 'findings':[{'message':'Evidence', 'path':'evidence', 'line':1}]}
response = {'protocol_version':1,'reviews':[r]}
if mode == 'wrong-version': response['protocol_version'] = 2
if mode == 'missing-version': del response['protocol_version']
if mode == 'missing-review': response['reviews'] = []
if mode == 'extra-review': response['reviews'].append(dict(r, id='unexpected'))
if mode == 'missing-path': del r['findings'][0]['path']
if mode in ['bad-usage', 'valid-usage']:
    response['usage'] = {'input_tokens':10,'cached_input_tokens':11 if mode == 'bad-usage' else 3,'output_tokens':4}
if mode == 'duplicate-key':
    print('{"protocol_version":1,"protocol_version":1,"reviews":[]}')
elif mode == 'fenced': print('```json\n' + json.dumps(response) + '\n```')
else: print(json.dumps(response))
"#,
        );
        fs::set_permissions(root.path().join("tool"), fs::Permissions::from_mode(0o755)).unwrap();
        write(
            root.path(),
            ".please/providers/first.toml",
            &format!("command = './tool'\nargs = [{mode:?}]\ntimeout_seconds = 5\n"),
        );
        review(root.path(), "a", "first", "check");
        let result = execute(root.path());
        assert_eq!(
            result.overall,
            if mode == "valid-usage" {
                Status::Pass
            } else {
                Status::Error
            },
            "{mode}: {:?}",
            result.results
        );
        if mode == "valid-usage" {
            assert_eq!(result.token_usage, Some(14));
            assert_eq!(
                report::read(root.path())
                    .unwrap()
                    .provider_usage
                    .unwrap()
                    .cached_input_tokens,
                3
            );
        }
    }
}

#[test]
fn structured_parameters_merge_per_review_without_changing_provider_definition() {
    let root = fixture();
    write(
        root.path(),
        ".please/provider.toml",
        "command = 'python3'\nargs = ['tool.py']\ntimeout_seconds = 5\n[parameters]\nvariant = 'quick'\nretained = [1, true, 'text']\n[parameters.tools]\nread = true\nsearch = true\n",
    );
    for id in ["a", "b"] {
        write(
            root.path(),
            &format!(".please/reviews/{id}/rule.toml"),
            &format!(
                "id = {id:?}\ndescription = 'Inspect evidence'\nrun_at = ['check']\n[review]\nprompt = 'prompt.md'\n{}",
                if id == "a" {
                    "[review.parameters]\nvariant = 'careful'\n[review.parameters.tools]\nread = false\n"
                } else {
                    ""
                }
            ),
        );
        write(
            root.path(),
            &format!(".please/reviews/{id}/prompt.md"),
            "Inspect evidence\n",
        );
    }
    write(
        root.path(),
        "tool.py",
        r#"import json, sys
request = json.load(sys.stdin)
assert request['protocol_version'] == 1
assert request['execution_point'] == 'check'
assert request['input']['manifest']
assert len(request['reviews']) == 2
for review in request['reviews']:
    p = review['parameters']
    assert p['retained'] == [1, True, 'text']
    assert p['variant'] == ('careful' if review['id'] == 'a' else 'quick')
    assert p['tools'] == ({'read':False} if review['id'] == 'a' else {'read':True,'search':True})
print(json.dumps({'protocol_version':1, 'reviews':[{'id':r['id'],'status':'PASS','reason':'Parameters delivered','findings':[{'message':'Evidence','path':'evidence','line':1}]} for r in request['reviews']]}))
"#,
    );
    let project = config::load(root.path()).unwrap();
    assert_eq!(
        planner::build(&project, Some("check"), false)
            .unwrap()
            .plan
            .planned_model_calls,
        1
    );
    assert_eq!(execute(root.path()).overall, Status::Pass);
    assert_eq!(
        config::load(root.path())
            .unwrap()
            .provider
            .unwrap()
            .parameters["tools"]["search"],
        true
    );
    for value in ["nan", "inf", "2026-01-01"] {
        write(
            root.path(),
            ".please/provider.toml",
            &format!(
                "command = 'fake-adapter'\ntimeout_seconds = 1\n[parameters]\ninvalid = {value}\n"
            ),
        );
        assert!(config::load(root.path()).is_err(), "{value}");
    }
}

#[test]
fn init_has_no_provider_and_selected_reviews_require_explicit_configuration() {
    let root = fixture();
    assert!(!root.path().join(".please/provider.toml").exists());
    assert!(config::load(root.path()).unwrap().provider.is_none());
    write(
        root.path(),
        ".please/reviews/a/rule.toml",
        "id = 'a'\ndescription = 'Inspect evidence'\nrun_at = ['check']\n[review]\nprompt = 'prompt.md'\n",
    );
    write(
        root.path(),
        ".please/reviews/a/prompt.md",
        "Inspect evidence\n",
    );
    let project = config::load(root.path()).unwrap();
    let plan = planner::build(&project, Some("check"), false).unwrap().plan;
    assert!(!plan.executable());
    assert!(
        plan.blocking_reasons
            .iter()
            .any(|r| r.contains("no adapter configured"))
    );
    assert!(
        planner::build(&project, Some("pre-push"), false)
            .unwrap()
            .plan
            .executable()
    );
    write(
        root.path(),
        ".please/reviews/a/rule.toml",
        "id = 'a'\ndescription = 'Inspect evidence'\nenabled = false\nrun_at = ['check']\n[review]\nprompt = 'prompt.md'\n",
    );
    assert!(
        planner::build(&config::load(root.path()).unwrap(), Some("check"), false)
            .unwrap()
            .plan
            .executable()
    );
}

#[test]
fn documented_dummy_runs_from_supporting_adapter_directory() {
    let root = fixture();
    write(
        root.path(),
        ".please/providers/adapters/dummy.py",
        include_str!("../examples/adapters/dummy.py"),
    );
    // This supporting file must never be recursively interpreted as a provider.
    write(
        root.path(),
        ".please/providers/adapters/data.toml",
        "not a provider definition",
    );
    write(
        root.path(),
        ".please/providers/dummy.toml",
        "command = 'python3'\nargs = ['.please/providers/adapters/dummy.py']\ntimeout_seconds = 5\n",
    );
    review(root.path(), "a", "dummy", "check");
    let result = execute(root.path());
    assert_eq!(result.overall, Status::Error);
    assert_eq!(
        result.results[0].reason,
        "Dummy adapter performs no AI assessment"
    );
    assert!(result.results[0].findings.is_empty());
}

#[test]
fn unknown_named_references_are_rejected_even_for_disabled_reviews() {
    let root = fixture();
    review(root.path(), "a", "missing", "check");
    let path = root.path().join(".please/reviews/a/rule.toml");
    for disabled in [false, true] {
        if disabled {
            let text = fs::read_to_string(&path).unwrap();
            fs::write(
                &path,
                text.replace("description =", "enabled = false\ndescription ="),
            )
            .unwrap();
        }
        let error = format!("{:#}", config::load(root.path()).unwrap_err());
        assert!(
            error.contains("unknown provider") && error.contains("missing"),
            "{error}"
        );
    }
}
