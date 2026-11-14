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
assert request['protocol_version'] == 2
assert request['execution_point'] == 'check' and os.path.isfile(request['context_file'])
prompt = request['prompt']
assert 'Unique review instructions' in prompt
assert request['response_schema']['required'] == ['reason', 'findings']
assert 'positive line number' in prompt
assert 'manifest paths' in prompt
reviews = request['reviews']
assert all(r['parameters']['label'] in ['first', 'second'] for r in reviews)
if mode == 'malformed':
    print('{"original-adapter-output": true}')
    sys.exit(0)
if mode == 'nonzero':
    print('original-adapter-error', file=sys.stderr)
    sys.exit(7)
if mode == 'error':
    print('original-adapter-error', file=sys.stderr)
    sys.exit(2)
if mode == 'timeout':
    time.sleep(30)
if mode == 'oversized':
    print('x' * 70000)
    sys.exit(0)
findings = [] if mode == 'ungrounded' else [{'message':'Evidence inspected','path':'evidence','line':1}]
print(json.dumps({'protocol_version':2,'reason':'Evidence inspected','findings':findings}))
sys.exit(1 if mode == 'fail' else 3 if mode == 'structured-error' else 0)
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
        3
    );
    let result = execute(root.path());
    assert_eq!(result.overall, Status::Pass, "{:?}", result.results);
    assert_eq!(result.results.len(), 3);
    assert!(result.provider_usage.is_none());
    assert_eq!(report::read(root.path()).unwrap().overall, Status::Pass);
}
#[test]
fn process_and_response_errors_never_pass_and_keep_original_output() {
    for mode in [
        "malformed",
        "nonzero",
        "error",
        "structured-error",
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
        // REQ-0131: the original output of an ERROR is retained unmasked in the report.
        let item = result.results.iter().find(|r| r.kind == "review").unwrap();
        let output = |capture: &Option<please::process::Capture>| {
            capture.as_ref().map(|c| c.text.clone()).unwrap_or_default()
        };
        match mode {
            "nonzero" => {
                assert_eq!(item.exit_code, Some(7));
                assert!(output(&item.stderr).contains("original-adapter-error"));
                assert!(item.reason.contains("please status --details"));
            }
            "error" => assert!(output(&item.stderr).contains("original-adapter-error")),
            "malformed" => assert!(output(&item.stdout).contains("original-adapter-output")),
            "structured-error" => {
                assert_eq!(item.exit_code, Some(3));
                assert!(output(&item.stdout).contains("Evidence inspected"));
            }
            "timeout" => {
                // The killed process leaves no exit status, but its captures are kept.
                assert_eq!(item.exit_code, None);
                assert!(item.stdout.is_some() && item.stderr.is_some());
            }
            "oversized" => {
                let stdout = item.stdout.as_ref().unwrap();
                assert!(stdout.truncated && stdout.total_bytes > 64 * 1024);
                assert_eq!(stdout.text.len(), 64 * 1024);
            }
            _ => {}
        }
        let saved = report::read(root.path()).unwrap();
        let saved = saved.results.iter().find(|r| r.kind == "review").unwrap();
        assert_eq!(saved.stderr.is_some(), item.stderr.is_some(), "{mode}");
        assert_eq!(saved.stdout.is_some(), item.stdout.is_some(), "{mode}");
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
fn protocol_versions_and_complete_coverage_fail_closed_while_usage_is_tolerant() {
    for mode in [
        "wrong-version",
        "legacy-version",
        "missing-version",
        "duplicate-key",
        "legacy-envelope",
        "status-field",
        "id-field",
        "blank-explanation",
        "long-explanation",
        "missing-path",
        "bad-usage",
        "zero-usage",
        "fenced",
        "valid-usage",
        "explanation",
    ] {
        let root = fixture();
        write(
            root.path(),
            "tool",
            r#"#!/usr/bin/env python3
import json, sys
request = json.load(sys.stdin)
mode = sys.argv[1]
r = {'reason':'Evidence inspected', 'findings':[{'message':'Evidence', 'path':'evidence', 'line':1}]}
response = dict(r, protocol_version=2)
if mode == 'wrong-version': response['protocol_version'] = 3
if mode == 'legacy-version': response['protocol_version'] = 1
if mode == 'missing-version': del response['protocol_version']
if mode == 'legacy-envelope': response = {'protocol_version':2, 'reviews':[dict(r, id=request['reviews'][0]['id'], status='PASS')]}
if mode == 'status-field': response['status'] = 'PASS'
if mode == 'id-field': response['id'] = request['reviews'][0]['id']
if mode == 'blank-explanation': response['explanation'] = '  '
if mode == 'long-explanation': response['explanation'] = 'x' * 8001
if mode == 'explanation': response['explanation'] = 'First line\nSecond line'
if mode == 'missing-path': del r['findings'][0]['path']
if mode in ['bad-usage', 'valid-usage']:
    response['usage'] = {'input_tokens':10,'cached_input_tokens':11 if mode == 'bad-usage' else 3,'output_tokens':4}
if mode == 'zero-usage':
    response['usage'] = {'input_tokens':0,'cached_input_tokens':-1,'output_tokens':'many','reasoning_output_tokens':None}
if mode == 'duplicate-key':
    print('{"protocol_version":2,"protocol_version":2,"reason":"x","findings":[]}')
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
            if matches!(
                mode,
                "valid-usage" | "bad-usage" | "zero-usage" | "explanation"
            ) {
                Status::Pass
            } else {
                Status::Error
            },
            "{mode}: {:?}",
            result.results
        );
        if mode == "explanation" {
            let item = result.results.iter().find(|r| r.kind == "review").unwrap();
            assert_eq!(item.explanation.as_deref(), Some("First line\nSecond line"));
        }
        if mode == "zero-usage" {
            assert_eq!(result.token_usage, None);
            assert_eq!(result.provider_usage, None);
        }
        if mode == "bad-usage" {
            assert_eq!(result.token_usage, Some(14));
        }
        if mode == "valid-usage" {
            assert_eq!(result.token_usage, Some(14));
            assert_eq!(
                report::read(root.path())
                    .unwrap()
                    .provider_usage
                    .unwrap()
                    .cached_input_tokens,
                Some(3)
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
assert request['protocol_version'] == 2
assert request['execution_point'] == 'check'
assert request['input']['manifest']
assert len(request['reviews']) == 1
for review in request['reviews']:
    p = review['parameters']
    assert p['retained'] == [1, True, 'text']
    assert p['variant'] == ('careful' if review['id'] == 'a' else 'quick')
    assert p['tools'] == ({'read':False} if review['id'] == 'a' else {'read':True,'search':True})
print(json.dumps({'protocol_version':2, 'reason':'Parameters delivered', 'findings':[{'message':'Evidence','path':'evidence','line':1}]}))
"#,
    );
    let project = config::load(root.path()).unwrap();
    assert_eq!(
        planner::build(&project, Some("check"), false)
            .unwrap()
            .plan
            .planned_model_calls,
        2
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
    assert!(result.results[0].reason.contains("not 0 or 1"));
    assert!(
        result.results[0]
            .stderr
            .as_ref()
            .is_some_and(|c| c.text.contains("Dummy adapter performs no AI assessment"))
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
