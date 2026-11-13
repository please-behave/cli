//! EXT-002: independently distributed named provider packages.
#![cfg(unix)]
mod support;
use please::{
    VerificationMode, config, external, planner,
    provider::Configured,
    report::{self, Status},
};
use std::{
    fs,
    path::{Path, PathBuf},
};

fn git(root: &Path, args: &[&str]) -> String {
    let result = support::command("git")
        .current_dir(root)
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    String::from_utf8(result.stdout).unwrap().trim().to_owned()
}
fn commit(root: &Path) -> String {
    git(root, &["add", "."]);
    git(root, &["commit", "--quiet", "-m", "test: record fixture"]);
    git(root, &["rev-parse", "HEAD"])
}
struct Fixture {
    _temporary: tempfile::TempDir,
    rules: PathBuf,
    providers: PathBuf,
    consumer: PathBuf,
    cache: PathBuf,
    rules_ref: String,
    provider_ref: String,
}
impl Fixture {
    fn new() -> Self {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().canonicalize().unwrap();
        let rules = root.join("Rules Repo");
        let providers = root.join("Provider Repo");
        let consumer = root.join("Consumer Repo");
        for path in [&rules, &providers, &consumer] {
            fs::create_dir(path).unwrap();
            git(path, &["init", "--quiet"]);
        }
        support::write(
            &rules,
            "policy/config.toml",
            "id = 'policy'\ndescription = 'Review evidence'\nrun_at = ['check']\nrules = ['rules/reviews/evidence']\n",
        );
        support::write(
            &rules,
            "rules/reviews/evidence/rule.toml",
            "id = 'evidence'\ndescription = 'Inspect evidence'\n[review]\nprovider = 'quality'\nprompt = 'prompt.md'\ninclude_diff = false\nfiles = ['evidence']\n",
        );
        support::write(
            &rules,
            "rules/reviews/evidence/prompt.md",
            "Inspect the provided evidence.\n",
        );
        support::write(
            &providers,
            "packages/quality/provider.toml",
            "command = 'python3'\nargs = ['@provider/adapter.py', '@source/shared message.txt']\ntimeout_seconds = 5\n[parameters]\nmodel = 'offline-fixture'\n[requirements]\nexecutables = ['python3']\n",
        );
        support::write(&providers, "shared message.txt", "shared resource\n");
        support::write(
            &providers,
            "packages/quality/adapter.py",
            r#"import json, pathlib, sys
request = json.load(sys.stdin)
assert pathlib.Path(sys.argv[1]).read_text() == 'shared resource\n'
assert request['reviews'][0]['parameters']['model'] == 'offline-fixture'
pathlib.Path('.git/provider-called').write_text('called')
if len(sys.argv) > 2 and sys.argv[2] == 'tamper':
    pathlib.Path(__file__).write_text('# modified during invocation\n')
print(json.dumps({'protocol_version':1, 'reviews':[{'id':r['id'],'status':'PASS','reason':'Evidence inspected','findings':[{'message':'File inspected','path':'evidence','line':1}]} for r in request['reviews']]}))
"#,
        );
        let rules_ref = commit(&rules);
        let provider_ref = commit(&providers);
        please::init::init(&consumer).unwrap();
        fs::remove_dir_all(consumer.join(".please/checks")).unwrap();
        fs::remove_dir_all(consumer.join(".please/reviews")).unwrap();
        support::write(&consumer, "evidence", "Verified evidence\n");
        support::write(&consumer, ".gitignore", ".please/reports/\n");
        let fixture = Self {
            _temporary: temporary,
            rules,
            providers,
            consumer,
            cache: root.join("cache"),
            rules_ref,
            provider_ref,
        };
        fixture.configure(true, true);
        fixture
    }
    fn configure(&self, rules: bool, provider: bool) {
        let mut text = "project = 'Provider consumer'\n".to_owned();
        if rules {
            text += &format!(
                "[[imports]]\ngit = {:?}\npath = 'policy'\nref = {:?}\n",
                self.rules.to_str().unwrap(),
                self.rules_ref
            );
        }
        if provider {
            text += &format!(
                "[[provider_imports]]\nid = 'quality'\ngit = {:?}\npath = 'packages/quality'\nref = {:?}\n",
                self.providers.to_str().unwrap(),
                self.provider_ref
            );
        }
        support::write(&self.consumer, ".please/config.toml", &text);
    }
    fn load(&self, refresh: bool) -> anyhow::Result<config::Project> {
        external::load(&self.consumer, refresh, Some(&self.cache))
    }
    fn execute(&self, project: config::Project) -> report::Report {
        please::verify_project_at(
            project,
            &Configured,
            VerificationMode::AllRules,
            Some("check"),
            &mut |_| {},
        )
        .unwrap()
    }
    fn cached(&self, project: &config::Project) -> PathBuf {
        let origin = &project.external_providers["quality"].origin;
        self.cache
            .join("git")
            .join(&origin.repository)
            .join(&origin.commit)
            .join("contents")
    }
}

#[test]
fn separate_provider_and_ruleset_repositories_run_offline() {
    let f = Fixture::new();
    let project = f.load(true).unwrap();
    assert_eq!(project.providers.len(), 1);
    let plan = planner::build(&project, Some("check"), false).unwrap().plan;
    assert_eq!(
        plan.scheduled[0].provider_origin.as_ref().unwrap().commit,
        f.provider_ref
    );
    assert_eq!(
        plan.scheduled[0].provider_config.as_ref().unwrap().args[0],
        "@provider/adapter.py"
    );
    fs::rename(&f.rules, f.rules.with_extension("offline")).unwrap();
    fs::rename(&f.providers, f.providers.with_extension("offline")).unwrap();
    let report = f.execute(f.load(false).unwrap());
    assert_eq!(report.overall, Status::Pass, "{:?}", report.results);
    assert_eq!(report.plan.plan_id, plan.plan_id);
    assert!(f.consumer.join(".git/provider-called").is_file());
    assert!(!f.consumer.join(".please/providers/quality.toml").exists());
    assert!(!f.consumer.join(".please/adapters").exists());
    assert_eq!(report::read(&f.consumer).unwrap().overall, Status::Pass);
    assert!(please::terminal::render_details(&report, false).contains("Provider source: quality"));
}

#[test]
fn shared_repository_and_independent_revisions_are_supported() {
    let mut f = Fixture::new();
    support::write(
        &f.rules,
        "packages/quality/provider.toml",
        &fs::read_to_string(f.providers.join("packages/quality/provider.toml")).unwrap(),
    );
    support::write(
        &f.rules,
        "packages/quality/adapter.py",
        &fs::read_to_string(f.providers.join("packages/quality/adapter.py")).unwrap(),
    );
    support::write(&f.rules, "shared message.txt", "shared resource\n");
    f.rules_ref = commit(&f.rules);
    f.providers = f.rules.clone();
    f.provider_ref = f.rules_ref.clone();
    f.configure(true, true);
    let before = f.execute(f.load(true).unwrap());
    assert_eq!(before.overall, Status::Pass);
    assert_eq!(fs::read_dir(f.cache.join("git")).unwrap().count(), 1);
    let path = f.providers.join("packages/quality/provider.toml");
    fs::write(
        &path,
        fs::read_to_string(&path).unwrap() + "\n# A separately selected revision\n",
    )
    .unwrap();
    f.provider_ref = commit(&f.providers);
    f.configure(true, true);
    let after = f.execute(f.load(true).unwrap());
    assert_eq!(after.overall, Status::Pass);
    assert_ne!(before.plan.plan_id, after.plan.plan_id);
    assert_ne!(
        after.plan.scheduled[0]
            .origin
            .as_ref()
            .unwrap()
            .external
            .as_ref()
            .unwrap()
            .commit,
        after.plan.scheduled[0]
            .provider_origin
            .as_ref()
            .unwrap()
            .commit
    );
}

#[test]
fn provider_only_imports_need_no_rules_and_never_invoke_adapters() {
    let f = Fixture::new();
    f.configure(false, true);
    let project = f.load(true).unwrap();
    assert_eq!(project.providers.len(), 1);
    assert!(project.rule_sets.is_empty());
    assert!(project.reviews.is_empty());
    assert!(!f.consumer.join(".git/provider-called").exists());
    f.configure(false, false);
    f.load(true).unwrap();
    assert!(
        !f.consumer
            .join(".git/please/external-rule-sets.json")
            .exists()
    );
    f.configure(true, false);
    assert!(format!("{:#}", f.load(true).unwrap_err()).contains("unknown provider"));
}

#[test]
fn collisions_and_unsafe_resources_fail_without_publishing_state() {
    let f = Fixture::new();
    f.load(true).unwrap();
    let state = f.consumer.join(".git/please/external-rule-sets.json");
    let previous = fs::read(&state).unwrap();
    let central = f.consumer.join(".please/config.toml");
    let original = fs::read_to_string(&central).unwrap();
    let repeated = original.split_once("[[provider_imports]]").unwrap().1;
    fs::write(
        &central,
        format!("{original}\n[[provider_imports]]{repeated}"),
    )
    .unwrap();
    assert!(f.load(true).is_err());
    assert_eq!(fs::read(&state).unwrap(), previous);
    fs::write(&central, original).unwrap();
    support::write(
        &f.consumer,
        ".please/providers/quality.toml",
        "command = 'unused'\ntimeout_seconds = 5\n",
    );
    assert!(format!("{:#}", f.load(true).unwrap_err()).contains("duplicate provider"));
    fs::remove_file(f.consumer.join(".please/providers/quality.toml")).unwrap();
    for token in [
        "@provider/../adapter.py",
        "@source/../adapter.py",
        "@provider//absolute",
        "@unknown/adapter.py",
        "@provider/missing",
        "@provider",
    ] {
        let path = f.providers.join("packages/quality/provider.toml");
        fs::write(
            &path,
            format!("command = 'python3'\nargs = [{token:?}]\ntimeout_seconds = 5\n"),
        )
        .unwrap();
        let revision = commit(&f.providers);
        let text = fs::read_to_string(&central)
            .unwrap()
            .replace(&f.provider_ref, &revision);
        fs::write(&central, &text).unwrap();
        assert!(f.load(true).is_err(), "accepted {token}");
        assert_eq!(fs::read(&state).unwrap(), previous);
        fs::write(&central, text.replace(&revision, &f.provider_ref)).unwrap();
    }
    assert!(f.load(false).is_ok());
    assert!(!f.consumer.join(".git/provider-called").exists());
}

#[test]
fn cached_provider_tampering_is_fatal_even_for_advisory_reviews() {
    let f = Fixture::new();
    let project = f.load(true).unwrap();
    let path = f.cached(&project).join("packages/quality/adapter.py");
    fs::write(&path, "# changed after loading\n").unwrap();
    assert!(f.load(false).is_err());
    let mut project = project;
    project.reviews[0].blocking = false;
    let report = f.execute(project);
    assert_eq!(report.overall, Status::Error);
    assert!(
        report
            .results
            .iter()
            .any(|r| r.configuration_error && r.status == Status::Error)
    );
    assert!(!f.consumer.join(".git/provider-called").exists());
}

#[test]
fn provider_changes_during_execution_invalidate_the_verdict() {
    let mut f = Fixture::new();
    let path = f.providers.join("packages/quality/provider.toml");
    let text = fs::read_to_string(&path).unwrap().replace(
        "'@source/shared message.txt'",
        "'@source/shared message.txt', 'tamper'",
    );
    fs::write(path, text).unwrap();
    f.provider_ref = commit(&f.providers);
    f.configure(true, true);
    let mut project = f.load(true).unwrap();
    project.reviews[0].blocking = false;
    let report = f.execute(project);
    assert!(f.consumer.join(".git/provider-called").exists());
    assert_eq!(report.overall, Status::Error);
    assert!(report.results.iter().any(|r| r.configuration_error));
}

#[test]
fn imported_provider_symlinks_are_rejected() {
    let mut f = Fixture::new();
    std::os::unix::fs::symlink("adapter.py", f.providers.join("packages/quality/link.py")).unwrap();
    let path = f.providers.join("packages/quality/provider.toml");
    fs::write(
        &path,
        fs::read_to_string(&path)
            .unwrap()
            .replace("@provider/adapter.py", "@provider/link.py"),
    )
    .unwrap();
    f.provider_ref = commit(&f.providers);
    f.configure(true, true);
    assert!(f.load(true).is_err());
    assert!(!f.consumer.join(".git/provider-called").exists());
}

#[test]
fn executable_references_and_literal_arguments_preserve_token_boundaries() {
    use std::os::unix::fs::PermissionsExt;
    let mut f = Fixture::new();
    let adapter = f.providers.join("packages/quality/adapter.py");
    let text = fs::read_to_string(&adapter).unwrap().replace(
        "request = json.load(sys.stdin)",
        "assert sys.argv[2:5] == ['@literal', '--value=@provider/not-a-reference', '$HOME']\nassert pathlib.Path(sys.argv[5]).read_text() == 'consumer data\\n'\nrequest = json.load(sys.stdin)",
    );
    fs::write(&adapter, format!("#!/usr/bin/env python3\n{text}")).unwrap();
    fs::set_permissions(&adapter, fs::Permissions::from_mode(0o755)).unwrap();
    support::write(&f.consumer, "consumer data.txt", "consumer data\n");
    support::write(
        &f.providers,
        "packages/quality/provider.toml",
        "command = '@provider/adapter.py'\nargs = ['@source/shared message.txt', '@@literal', '--value=@provider/not-a-reference', '$HOME', '@repo/consumer data.txt']\ntimeout_seconds = 5\n[parameters]\nmodel = 'offline-fixture'\n[requirements]\nexecutables = ['@provider/adapter.py', 'python3']\n",
    );
    f.provider_ref = commit(&f.providers);
    f.configure(true, true);
    let report = f.execute(f.load(true).unwrap());
    assert_eq!(report.overall, Status::Pass, "{:?}", report.results);
}

#[test]
fn imported_header_review_uses_an_independent_cached_adapter() {
    let mut f = Fixture::new();
    support::write(
        &f.rules,
        "policy/config.toml",
        "id = 'policy'\ndescription = 'Commit wording'\nrun_at = ['commit-msg']\nrules = ['rules/reviews/evidence']\n",
    );
    support::write(
        &f.rules,
        "rules/reviews/evidence/rule.toml",
        "id = 'evidence'\ndescription = 'Inspect header'\n[review]\nprovider = 'quality'\ninput = 'commit-header'\nprompt = 'prompt.md'\n",
    );
    support::write(
        &f.providers,
        "packages/quality/adapter.py",
        r#"import json, sys
request = json.load(sys.stdin)
assert request['input']['commit_header'] == 'fix: preserve focus'
assert request['input']['manifest'] == request['input']['excerpts'] == []
assert request['head'] is None and request['input']['diff'] == ''
assert request['response_schema']['properties']['reviews']['items']['properties']['findings']['minItems'] == 1
print(json.dumps({'protocol_version':1,'reviews':[{'id':r['id'],'status':'PASS','reason':'Header inspected','findings':[{'source':'commit-header','path':None,'line':1,'message':'Specific imperative summary'}]} for r in request['reviews']]}))
"#,
    );
    f.rules_ref = commit(&f.rules);
    f.provider_ref = commit(&f.providers);
    f.configure(true, true);
    let project = f.load(true).unwrap();
    please::reconciliation::reconcile(&project, &mut |_| Ok(true)).unwrap();
    support::write(
        &f.consumer,
        ".git/message",
        "fix: preserve focus\n\nUnsent body\n",
    );
    let (hook, _) = please::reconciliation::prepare(
        &project,
        Some(please::reconciliation::HookInvocation {
            event: "commit-msg".into(),
            path: f.consumer.join(".git/hooks/commit-msg"),
            arguments: vec![".git/message".into()],
        }),
        Some("commit-msg"),
        false,
    )
    .unwrap();
    let report = please::verify_project_with_context(
        project,
        &Configured,
        VerificationMode::AllRules,
        Some("commit-msg"),
        hook.as_ref(),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.overall, Status::Pass, "{:?}", report.results);
    assert!(report.plan.scheduled[0].provider_origin.is_some());
    assert_eq!(
        report.plan.input.commit_header.as_deref(),
        Some("fix: preserve focus")
    );
    assert!(!f.consumer.join(".please/adapters").exists());
}
