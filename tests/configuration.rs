mod support;
use please::{
    config, external, planner,
    provider::{Provider, Request},
    report::{self, Status},
};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tempfile::TempDir;

fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}
fn git(root: &Path, args: &[&str]) -> String {
    let out = support::command("git")
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
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().into()
}
fn project() -> TempDir {
    let d = tempfile::tempdir().unwrap();
    git(d.path(), &["init", "--quiet", "--initial-branch=main"]);
    write(
        d.path(),
        ".please/config.toml",
        "project = 'My_App'\n[execution_points]\ncustom = ['release']\n",
    );
    write(d.path(), "README.md", "# My_App\n");
    support::configure_adapter(d.path());
    d
}
fn check(root: &Path, id: &str, attributes: &str, implementation: &str, script: &str) {
    write(
        root,
        &format!(".please/checks/{id}/rule.toml"),
        &format!(
            "id = {id:?}\ndescription = 'Test check'\n{attributes}\n[check]\n{implementation}\n"
        ),
    );
    write(root, &format!(".please/checks/{id}/index.sh"), script);
}
fn review(root: &Path, attributes: &str) {
    write(
        root,
        ".please/reviews/design/rule.toml",
        &format!(
            "id = 'design'\ndescription = 'Review design'\n{attributes}\n[review]\nprompt = 'prompt.md'\n"
        ),
    );
    write(
        root,
        ".please/reviews/design/prompt.md",
        "Inspect documented expectations.\n",
    );
}
fn error(root: &Path) -> String {
    format!("{:#}", config::load(root).unwrap_err())
}
struct Never;
impl Provider for Never {
    fn review(&self, _: &Request<'_>) -> anyhow::Result<String> {
        panic!("unexpected provider call")
    }
}
struct Mock(&'static str);
impl Provider for Mock {
    fn review(&self, request: &Request<'_>) -> anyhow::Result<String> {
        assert_eq!(
            request.reviews[0].text,
            "Inspect documented expectations.\n"
        );
        if self.0 == "unavailable" {
            anyhow::bail!("provider unavailable");
        }
        if self.0 == "invalid" {
            return Ok("invalid JSON".into());
        }
        Ok(serde_json::json!({"reviews": [{"id": request.reviews[0].id, "status": self.0,
            "reason": "Documented behavior", "findings": [{"message": "Evidence", "path": "README.md", "line": 1}]}]}).to_string())
    }
}

#[test]
fn fixed_discovery_and_defaults() {
    let d = project();
    assert!(config::load(d.path()).unwrap().checks.is_empty());
    check(
        d.path(),
        "readme",
        "run_at = ['check']",
        "command = '@rule/index.sh'",
        "exit 0\n",
    );
    // Supporting data and unrelated TOML documents are never parsed as governance.
    write(
        d.path(),
        ".please/checks/readme/gitleaks.toml",
        "arbitrary data, not TOML",
    );
    write(d.path(), ".please/arbitrary/invalid.toml", "not TOML");
    let p = config::load(d.path()).unwrap();
    assert!(p.checks[0].enabled);
    assert_eq!(p.checks[0].timeout_seconds, 30);
    let plan = planner::build(&p, Some("check"), false).unwrap().plan;
    assert_eq!(plan.plan_schema_version, 3);
    assert_eq!(plan.scheduled.len(), 1);
    assert!(plan.scheduled[0].blocking);
    write(
        d.path(),
        ".please/checks/readme/gitleaks.toml",
        "changed bundled data",
    );
    let changed = config::load(d.path()).unwrap();
    assert_ne!(
        plan.plan_id,
        planner::build(&changed, Some("check"), false)
            .unwrap()
            .plan
            .plan_id
    );
}

#[test]
fn strict_configuration_and_implementation_validation() {
    for (attributes, implementation) in [
        ("run_at = ['unknown']", "command = '@rule/index.sh'"),
        (
            "run_at = ['check']\nunknown = true",
            "command = '@rule/index.sh'",
        ),
        (
            "run_at = ['check']",
            "command = '@rule/index.sh'\nunknown = true",
        ),
        ("run_at = ['check']", "command = ''"),
        (
            "run_at = ['check']",
            "command = '@rule/index.sh'\ntimeout_seconds = 0",
        ),
        ("", "command = '@rule/index.sh'"),
    ] {
        let d = project();
        check(d.path(), "readme", attributes, implementation, "exit 0");
        assert!(
            config::load(d.path()).is_err(),
            "{attributes}: {implementation}"
        );
    }
    let d = project();
    for text in [
        "version = 4\nproject = 'My_App'",
        "version = 3\nproject = 'My_App'",
        "version = 99\nproject = 'My_App'",
        "project = 'My_App'\nunknown = true",
        "project = 'My_App'\n[[checks]]\nid = 'legacy'",
    ] {
        write(d.path(), ".please/config.toml", text);
        assert!(config::load(d.path()).is_err());
    }
}

#[test]
fn definitions_are_required_but_disabled_implementations_may_be_missing() {
    let d = project();
    fs::create_dir_all(d.path().join(".please/checks/readme")).unwrap();
    assert!(error(d.path()).contains("rule.toml"));
    check(
        d.path(),
        "readme",
        "enabled = false",
        "command = 'sh @rule/missing.sh'",
        "exit 0",
    );
    assert!(config::load(d.path()).is_ok());
    fs::remove_dir_all(d.path().join(".please/checks")).unwrap();
    fs::create_dir_all(d.path().join(".please/reviews/design")).unwrap();
    assert!(error(d.path()).contains("rule.toml"));
    review(d.path(), "enabled = false");
    fs::remove_file(d.path().join(".please/reviews/design/prompt.md")).unwrap();
    assert!(config::load(d.path()).is_ok());
}

#[test]
fn resource_filenames_are_confined() {
    for name in [
        "",
        ".",
        "..",
        "../index.sh",
        "/tmp/index.sh",
        "scripts/../index.sh",
        "C:/index.sh",
        "helpers\\index.sh",
        "C:\\index.sh",
    ] {
        let d = project();
        review(d.path(), "run_at = ['check']");
        let path = d.path().join(".please/reviews/design/rule.toml");
        fs::write(
            &path,
            fs::read_to_string(&path)
                .unwrap()
                .replace("'prompt.md'", &format!("{name:?}")),
        )
        .unwrap();
        assert!(config::load(d.path()).is_err(), "accepted prompt {name:?}");
    }
    let d = project();
    review(d.path(), "run_at = ['check']");
    write(
        d.path(),
        ".please/reviews/design/rule.toml",
        "id = 'design'\ndescription = 'Review'\nrun_at = ['check']\n[review]\nprompt = 'scripts/prompt.md'\n",
    );
    assert!(config::load(d.path()).is_err());
}

#[test]
fn local_sets_override_union_and_validate_scope() {
    let d = project();
    check(
        d.path(),
        "readme",
        "run_at = ['pre-commit']",
        "command = '@rule/index.sh'",
        "exit 0",
    );
    for (set, at) in [("repository", "check"), ("software", "release")] {
        write(
            d.path(),
            &format!(".please/rule-sets/{set}.toml"),
            &format!(
                "id = {set:?}\ndescription = 'Reusable rules'\nrules = ['readme']\nrun_at = [{at:?}]"
            ),
        );
    }
    let p = config::load(d.path()).unwrap();
    assert_eq!(
        p.effective.rules["readme"]
            .execution_points
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["check", "release"]
    );
    assert_eq!(
        planner::build(&p, Some("check"), false)
            .unwrap()
            .plan
            .scheduled
            .len(),
        1
    );
    assert!(
        planner::build(&p, Some("pre-commit"), false)
            .unwrap()
            .plan
            .scheduled
            .is_empty()
    );
    let path = ".please/rule-sets/repository.toml";
    let original = fs::read_to_string(d.path().join(path)).unwrap();
    for attribute in [
        "enabled = true",
        "blocking = false",
        "git = 'source'",
        "path = 'source'",
        "ref = 'main'",
    ] {
        write(d.path(), path, &format!("{original}\n{attribute}"));
        assert!(config::load(d.path()).is_err(), "accepted {attribute}");
    }
    write(d.path(), path, &original.replace("'readme'", "'missing'"));
    assert!(config::load(d.path()).is_err());
    write(d.path(), path, &original.replace("'readme'", "'software'"));
    assert!(config::load(d.path()).is_err());
}

#[test]
fn duplicate_ids_and_exactly_one_implementation() {
    let d = project();
    check(
        d.path(),
        "design",
        "run_at = ['check']",
        "command = '@rule/index.sh'",
        "exit 0",
    );
    review(d.path(), "run_at = ['check']");
    assert!(error(d.path()).contains("duplicate"));
    fs::remove_dir_all(d.path().join(".please/reviews")).unwrap();
    let path = ".please/checks/design/rule.toml";
    let original = fs::read_to_string(d.path().join(path)).unwrap();
    write(
        d.path(),
        path,
        &format!("{original}\n[review]\nprompt = 'prompt.md'"),
    );
    assert!(config::load(d.path()).is_err());
}

#[cfg(unix)]
#[test]
fn direct_and_interpreter_execution_keep_consumer_and_resources() {
    use std::os::unix::fs::PermissionsExt;
    for interpreter in ["", "sh "] {
        let d = project();
        check(
            d.path(),
            "readme",
            "run_at = ['check']",
            &format!("command = '{interpreter}@rule/index.sh @rule/data.txt'"),
            "#!/bin/sh\ntest -s README.md || exit 1\ncat \"$1\"\n",
        );
        write(d.path(), ".please/checks/readme/data.txt", "bundled data\n");
        if interpreter.is_empty() {
            fs::set_permissions(
                d.path().join(".please/checks/readme/index.sh"),
                fs::Permissions::from_mode(0o755),
            )
            .unwrap();
        }
        let p = config::load(d.path()).unwrap();
        let result = please::verify_project_at(
            p,
            &Never,
            please::VerificationMode::AllRules,
            Some("check"),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(result.overall, Status::Pass);
        assert_eq!(
            result.results[0].stdout.as_ref().unwrap().text,
            "bundled data\n"
        );
        report::read(d.path()).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn blocking_failures_errors_and_disabled_rules() {
    for (implementation, script, expected) in [
        ("command = 'sh @rule/index.sh'", "exit 1", Status::Fail),
        ("command = 'sh @rule/index.sh'", "exit 2", Status::Error),
        (
            "command = 'please-missing-interpreter @rule/index.sh'",
            "exit 0",
            Status::Error,
        ),
        (
            "command = 'sh @rule/index.sh'\ntimeout_seconds = 1",
            "sleep 4",
            Status::Error,
        ),
        ("command = '@rule/index.sh'", "exit 0", Status::Error),
    ] {
        let d = project();
        check(
            d.path(),
            "readme",
            "run_at = ['pre-push']",
            implementation,
            script,
        );
        let result = please::verify(d.path(), &Never).unwrap();
        assert_eq!(result.overall, expected, "{implementation} {script}");
        assert_eq!(result.results[0].status, expected);
        let saved = report::read(d.path()).unwrap();
        assert_eq!(saved.schema_version, 4);
        assert!(saved.results[0].blocking);
    }
    let d = project();
    check(
        d.path(),
        "readme",
        "enabled = false",
        "command = '@rule/index.sh'",
        "exit 1",
    );
    let p = config::load(d.path()).unwrap();
    let plan = planner::build(&p, None, false).unwrap().plan;
    assert!(plan.scheduled.is_empty());
    assert_eq!(plan.disabled_rules[0].id, "readme");
    let result = please::verify(d.path(), &Never).unwrap();
    assert!(result.results.is_empty());
}

#[test]
fn reviews_preserve_provider_contract_and_nonblocking_failures() {
    for (attributes, verdict, expected) in [
        ("", "PASS", Status::Pass),
        ("", "FAIL", Status::Fail),
        ("blocking = false", "FAIL", Status::Pass),
        ("blocking = false", "ERROR", Status::Pass),
        ("blocking = false", "unavailable", Status::Pass),
        ("blocking = false", "invalid", Status::Pass),
    ] {
        let d = project();
        review(d.path(), &format!("run_at = ['pre-push']\n{attributes}"));
        let result = please::verify(d.path(), &Mock(verdict)).unwrap();
        assert_eq!(result.overall, expected, "{verdict}");
        report::read(d.path()).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn failed_check_prevents_reviews() {
    let d = project();
    check(
        d.path(),
        "readme",
        "run_at = ['pre-push']",
        "command = 'sh @rule/index.sh'",
        "exit 1",
    );
    review(d.path(), "run_at = ['pre-push']");
    let result = please::verify(d.path(), &Mock("PASS")).unwrap();
    assert_eq!(result.overall, Status::Fail);
    assert_eq!(result.results[1].status, Status::NotRun);
}

#[test]
fn checks_reject_a_blocking_property() {
    for attribute in ["blocking = false", "blocking = true"] {
        let d = project();
        check(
            d.path(),
            "readme",
            &format!("run_at = ['pre-push']\n{attribute}"),
            "command = '@rule/index.sh'",
            "exit 0",
        );
        let error = config::load(d.path()).unwrap_err();
        assert!(format!("{error:#}").contains("blocking"), "{attribute}");
    }
}

#[cfg(unix)]
#[test]
fn all_designated_paths_reject_symlinks() {
    use std::os::unix::fs::symlink;
    for relative in [
        "checks/readme/index.sh",
        "checks/readme/rule.toml",
        "checks/readme",
        "checks",
        "config.toml",
        "reviews/design/prompt.md",
        "reviews/design/rule.toml",
        "reviews/design",
        "reviews",
    ] {
        let d = project();
        check(
            d.path(),
            "readme",
            "run_at = ['check']",
            "command = '@rule/index.sh'",
            "exit 0",
        );
        review(d.path(), "run_at = ['check']");
        let source = d.path().join(".please").join(relative);
        let outside = d.path().join("outside");
        fs::rename(&source, &outside).unwrap();
        symlink(&outside, &source).unwrap();
        if relative == "checks/readme/index.sh" {
            let p = config::load(d.path()).unwrap();
            assert_eq!(
                please::checks::execute(&p, &p.checks[0]).status,
                Status::Error
            );
        } else {
            assert!(config::load(d.path()).is_err(), "followed {relative}");
        }
    }
}

struct External {
    _temp: TempDir,
    root: PathBuf,
    remote: PathBuf,
    cache: PathBuf,
}
impl External {
    fn new() -> Self {
        let d = project();
        let root = d.path().canonicalize().unwrap();
        let remote = root.join("remote");
        let cache = root.join("cache");
        fs::create_dir(&remote).unwrap();
        fs::create_dir(&cache).unwrap();
        git(&remote, &["init", "--quiet", "--initial-branch=main"]);
        for name in ["repository", "software"] {
            write(
                &remote,
                &format!("{name}/config.toml"),
                &format!(
                    "id = {name:?}\ndescription = 'Reusable set'\nrules = ['{name}/checks/readme']\nrun_at = ['check']"
                ),
            );
            write(
                &remote,
                &format!("{name}/checks/readme/rule.toml"),
                "id = 'readme'\ndescription = 'Consumer README'\n[check]\ncommand = 'sh @rule/index.sh @rule/data.txt'\n",
            );
            write(
                &remote,
                &format!("{name}/checks/readme/index.sh"),
                "test -s README.md || exit 1\ntest -s \"$1\" || exit 1\n",
            );
            write(
                &remote,
                &format!("{name}/checks/readme/data.txt"),
                "bundled resource\n",
            );
        }
        let f = Self {
            _temp: d,
            root,
            remote,
            cache,
        };
        f.commit();
        f
    }
    fn commit(&self) -> String {
        git(&self.remote, &["add", "."]);
        git(&self.remote, &["commit", "--quiet", "-m", "fixture"]);
        git(&self.remote, &["rev-parse", "HEAD"])
    }
    fn imports(&self, selections: &[(&str, &str)]) {
        let mut text = "project = 'My_App'\n[execution_points]\ncustom = []\n".to_string();
        for (path, reference) in selections {
            text += &format!(
                "[[imports]]\ngit = {:?}\npath = {path:?}\nref = {reference:?}\n",
                self.remote.to_str().unwrap()
            );
        }
        write(&self.root, ".please/config.toml", &text);
    }
    fn load(&self, refresh: bool) -> anyhow::Result<config::Project> {
        external::load(&self.root, refresh, Some(&self.cache))
    }
}

#[cfg(unix)]
#[test]
fn external_refs_cache_offline_isolation_and_consumer_execution() {
    let f = External::new();
    git(&f.remote, &["tag", "v1.0.0"]);
    let commit = git(&f.remote, &["rev-parse", "HEAD"]);
    // Local and two external scopes all use the same rule ID without collision.
    check(
        &f.root,
        "readme",
        "run_at = ['check']",
        "command = 'sh @rule/index.sh'",
        "exit 0",
    );
    for reference in ["main", "v1.0.0", &commit] {
        f.imports(&[("repository", reference), ("software", reference)]);
        let p = f.load(true).unwrap();
        assert_eq!(p.checks.len(), 3);
        let origins: Vec<_> = p
            .effective
            .rules
            .values()
            .filter_map(|r| r.external.as_ref())
            .collect();
        assert_eq!(origins.len(), 2);
        assert!(origins.iter().all(|o| o.commit == commit));
        assert_eq!(p.checks.len(), 3);
        let p = f.load(false).unwrap();
        assert_eq!(
            planner::build(&p, Some("check"), false)
                .unwrap()
                .plan
                .scheduled
                .len(),
            3
        );
    }
    fs::rename(&f.remote, f.root.join("offline-remote")).unwrap();
    let p = f.load(false).unwrap();
    // Keep test fixture cache/remote contents outside the consumer snapshot.
    write(&f.root, ".gitignore", "cache/\noffline-remote/\n");
    let result = please::verify_project_at(
        p,
        &Never,
        please::VerificationMode::AllRules,
        Some("check"),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(result.overall, Status::Pass);
    assert_eq!(result.results.len(), 3);
    report::read(&f.root).unwrap();
}

#[test]
fn external_invalid_roots_import_overrides_and_scope_references_fail() {
    let f = External::new();
    f.imports(&[("missing", "main")]);
    assert!(f.load(true).is_err());
    f.imports(&[("repository", "main")]);
    let file = f.remote.join("repository/config.toml");
    let original = fs::read_to_string(&file).unwrap();
    for suffix in [
        "\nproject = 'Consumer'",
        "\nimports = []",
        "\nenabled = true",
        "\nblocking = false",
    ] {
        fs::write(&file, format!("{original}{suffix}")).unwrap();
        f.commit();
        assert!(f.load(true).is_err(), "accepted {suffix}");
    }
    fs::write(
        &file,
        original.replace("repository/checks/readme", "repository/checks/missing"),
    )
    .unwrap();
    f.commit();
    assert!(f.load(true).is_err());
    fs::write(&file, &original).unwrap();
    fs::remove_file(f.remote.join("repository/checks/readme/rule.toml")).unwrap();
    f.commit();
    assert!(f.load(true).is_err());
    let central = f.root.join(".please/config.toml");
    let text = fs::read_to_string(&central).unwrap();
    for extra in [
        "enabled = false",
        "run_at = ['check']",
        "id = 'override'",
        "blocking = false",
        "description = 'Override'",
    ] {
        fs::write(&central, format!("{text}{extra}\n")).unwrap();
        assert!(f.load(true).is_err(), "accepted import {extra}");
    }
}

#[test]
fn initialization_is_one_time() {
    let d = tempfile::tempdir().unwrap();
    git(d.path(), &["init", "--quiet"]);
    assert_eq!(please::init::init(d.path()).unwrap(), 9);
    config::load(d.path()).unwrap();
    assert!(
        !fs::read_to_string(d.path().join(".please/config.toml"))
            .unwrap()
            .contains("version")
    );
    assert!(
        d.path()
            .join(".please/checks/always-ok/rule.toml")
            .is_file()
    );
    assert_eq!(
        please::init::init(d.path()).unwrap_err().to_string(),
        "Already initialized (.please/ exists)."
    );
    assert_eq!(
        please::verify(d.path(), &Never).unwrap().overall,
        Status::Pass
    );
    report::read(d.path()).unwrap();
}

#[test]
fn checks_only_reports_selected_reviews_as_not_run() {
    // A blocking review, with and without a trigger: checks-only needs no change base and
    // makes no provider call; the review is NOT_RUN and the checks decide the result.
    for trigger in [false, true] {
        let d = project();
        review(d.path(), "run_at = ['pre-push']\nblocking = true");
        if trigger {
            let path = d.path().join(".please/reviews/design/rule.toml");
            let text = fs::read_to_string(&path).unwrap();
            fs::write(
                &path,
                format!("{text}\n[review.trigger]\nchanged_files = ['README.md']\n"),
            )
            .unwrap();
        }
        let p = config::load(d.path()).unwrap();
        let plan = planner::build(&p, None, true).unwrap().plan;
        assert!(plan.executable());
        assert_eq!(plan.scheduled.len(), 1);
        let report = please::verify_project_with_mode(
            p,
            &Never,
            please::VerificationMode::ChecksOnly,
            &mut |_| {},
        )
        .unwrap();
        assert!(report.checks_only);
        assert_eq!(report.overall, Status::Pass);
        let [item] = &report.results[..] else {
            panic!("one review result expected")
        };
        assert_eq!(
            (item.kind.as_str(), item.status),
            ("review", Status::NotRun)
        );
        assert!(item.reason.contains("Checks only"));
        report::read(d.path()).unwrap();
    }
}

#[test]
fn prepared_input_is_shared_without_rereading_resources() {
    let d = project();
    review(d.path(), "run_at = ['pre-push']");
    let path = d.path().join(".please/reviews/design/rule.toml");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(
        &path,
        format!("{text}\n[[review.context]]\npath = 'README.md'\n"),
    )
    .unwrap();
    let p = config::load(d.path()).unwrap();
    let prepared = planner::build(&p, None, false).unwrap();
    let input = prepared.review_input.as_ref().unwrap();
    assert!(input.excerpts[0].text.contains("My_App"));
    // Rendering the prepared request performs no filesystem reread.
    fs::remove_file(d.path().join("README.md")).unwrap();
    fs::remove_file(d.path().join(".please/reviews/design/prompt.md")).unwrap();
    assert!(input.excerpts[0].text.contains("My_App"));
    assert_eq!(p.reviews[0].text, "Inspect documented expectations.\n");
    let rendered = please::provider::prompt(&Request {
        execution_point: "pre-push",
        parameters: &Default::default(),
        project: &p,
        reviews: &[&p.reviews[0]],
        snapshot: &prepared.snapshot,
        input,
    })
    .unwrap();
    assert!(rendered.contains("My_App"));
}

#[test]
fn saved_reports_reject_inconsistent_metadata() {
    let d = project();
    review(d.path(), "run_at = ['pre-push']\nblocking = false");
    let r = please::verify(d.path(), &Mock("FAIL")).unwrap();
    let original = serde_json::to_value(r).unwrap();
    for case in ["missing-rule", "blocking", "point", "plan", "overall"] {
        let mut value = original.clone();
        match case {
            "missing-rule" => value["results"] = serde_json::json!([]),
            "blocking" => value["results"][0]["blocking"] = true.into(),
            "point" => value["plan"]["at"] = "release".into(),
            "plan" => value["plan"] = serde_json::Value::Null,
            _ => value["overall"] = "FAIL".into(),
        }
        write(d.path(), ".please/reports/last.json", &value.to_string());
        assert!(report::read(d.path()).is_err(), "accepted {case}");
    }
}

#[cfg(unix)]
#[test]
fn local_rules_share_entrypoints_with_independent_results() {
    use std::os::unix::fs::PermissionsExt;
    for interpreter in ["", "sh "] {
        let d = project();
        write(
            d.path(),
            "scripts/shared.sh",
            "#!/bin/sh\ntest -s README.md || exit 2\ncat \"$1\"\nexit \"$2\"\n",
        );
        fs::set_permissions(
            d.path().join("scripts/shared.sh"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        for (id, code) in [("pass", "0"), ("fail", "1"), ("error", "2")] {
            check(
                d.path(),
                id,
                &format!(
                    "run_at = ['check']\n[parameters.code]\ntype = 'string'\nvalue = '{code}'"
                ),
                &format!(
                    "command = '{interpreter}@source/scripts/shared.sh @rule/resource @param/code'"
                ),
                "exit 2",
            );
            write(d.path(), &format!(".please/checks/{id}/resource"), id);
            // A rule-relative decoy must never override a source-relative path.
            write(
                d.path(),
                &format!(".please/checks/{id}/scripts/shared.sh"),
                "exit 2",
            );
        }
        for (id, attributes) in [
            ("missing", "run_at = ['check']"),
            ("disabled", "enabled = false"),
        ] {
            check(
                d.path(),
                id,
                attributes,
                "command = 'sh @source/scripts/missing.sh'",
                "exit 0",
            );
            write(
                d.path(),
                &format!(".please/checks/{id}/scripts/missing.sh"),
                "exit 0",
            );
        }
        let p = config::load(d.path()).unwrap();
        let shared: Vec<_> = p
            .checks
            .iter()
            .filter(|c| matches!(c.id.as_str(), "pass" | "fail" | "error"))
            .collect();
        assert!(shared.iter().all(|c| *c.command == *shared[0].command));
        let report = please::verify_project_at(
            p,
            &Never,
            please::VerificationMode::AllRules,
            Some("check"),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(report.overall, Status::Error);
        assert_eq!(report.results.len(), 4);
        for (id, status) in [
            ("pass", Status::Pass),
            ("fail", Status::Fail),
            ("error", Status::Error),
            ("missing", Status::Error),
        ] {
            let result = report.results.iter().find(|r| r.id == id).unwrap();
            assert_eq!(result.status, status, "{id}: {}", result.reason);
            if id == "missing" {
                assert!(result.reason.contains("scripts/missing.sh"));
            } else {
                assert_eq!(result.stdout.as_ref().unwrap().text, id);
                assert!(result.blocking);
            }
        }
    }
}

#[cfg(unix)]
#[test]
fn shared_commands_reject_symlinks_at_execution() {
    use std::os::unix::fs::symlink;
    for target in ["scripts/shared.sh", "scripts", ".please/checks/readme"] {
        let d = project();
        let outside = tempfile::tempdir().unwrap();
        check(
            d.path(),
            "readme",
            "run_at = ['check']",
            "command = 'sh @source/scripts/shared.sh'",
            "exit 0",
        );
        write(d.path(), "scripts/shared.sh", "exit 0");
        let p = config::load(d.path()).unwrap();
        let original = d.path().join(target);
        let moved = outside.path().join("replacement");
        fs::rename(&original, &moved).unwrap();
        symlink(&moved, &original).unwrap();
        if target == ".please/checks/readme" {
            assert!(config::load(d.path()).is_err());
        } else {
            let reloaded = config::load(d.path()).unwrap();
            assert_eq!(
                please::checks::execute(&reloaded, &reloaded.checks[0]).status,
                Status::Error,
                "accepted {target}"
            );
        }
        let result = please::checks::execute(&p, &p.checks[0]);
        assert_eq!(
            result.status,
            Status::Error,
            "accepted replacement of {target}"
        );
        assert!(result.stdout.is_none());
    }
    // Disabled rules still reject unsafe paths; missing files alone are allowed.
    let d = project();
    check(
        d.path(),
        "readme",
        "enabled = false",
        "command = '@source/scripts/shared.sh'",
        "exit 0",
    );
    symlink("../outside", d.path().join("scripts")).unwrap();
    let p = config::load(d.path()).unwrap();
    assert_eq!(
        please::checks::execute(&p, &p.checks[0]).status,
        Status::Error
    );
    fs::remove_file(d.path().join("scripts")).unwrap();
    fs::create_dir_all(d.path().join("scripts/shared.sh")).unwrap();
    let p = config::load(d.path()).unwrap();
    assert_eq!(
        please::checks::execute(&p, &p.checks[0]).status,
        Status::Error
    );
}

#[test]
fn shared_entrypoints_recheck_missing_and_nonregular_files_before_execution() {
    for replace_with_directory in [false, true] {
        let d = project();
        check(
            d.path(),
            "readme",
            "run_at = ['check']",
            "command = '@source/scripts/shared.sh'",
            "exit 0",
        );
        write(d.path(), "scripts/shared.sh", "exit 0");
        let p = config::load(d.path()).unwrap();
        fs::remove_file(d.path().join("scripts/shared.sh")).unwrap();
        if replace_with_directory {
            fs::create_dir(d.path().join("scripts/shared.sh")).unwrap();
        }
        let result = please::checks::execute(&p, &p.checks[0]);
        assert_eq!(result.status, Status::Error);
        assert!(result.stdout.is_none());
    }
}

#[test]
fn string_commands_parse_arguments_without_a_shell() {
    for (command, expected) in [
        (
            "python3 scripts/check-docs.py",
            vec!["python3", "scripts/check-docs.py"],
        ),
        (
            r#""tools/my tool" 'path with spaces' "two words""#,
            vec!["tools/my tool", "path with spaces", "two words"],
        ),
        (
            r#"tool '' "" pre"two words"post"#,
            vec!["tool", "", "", "pretwo wordspost"],
        ),
        (
            r#"tool path\ with\ spaces "a\"b" 'a\b' a\\b"#,
            vec!["tool", "path with spaces", "a\"b", "a\\b", "a\\b"],
        ),
        (
            r#"tool '$HOME `literal`' \$HOME "\$HOME" "|&;<>()#" \; *.md ~ [ab]?"#,
            vec![
                "tool",
                "$HOME `literal`",
                "$HOME",
                "$HOME",
                "|&;<>()#",
                ";",
                "*.md",
                "~",
                "[ab]?",
            ],
        ),
        (" \ttool\targument  ", vec!["tool", "argument"]),
    ] {
        let d = project();
        check(
            d.path(),
            "parse",
            "enabled = false",
            &format!("command = {}", serde_json::to_string(command).unwrap()),
            "exit 2",
        );
        let p = config::load(d.path()).unwrap();
        assert_eq!(&*p.checks[0].command, &expected, "{command}");
    }
}

#[test]
fn string_commands_reject_invalid_syntax() {
    for command in [
        "",
        " \t ",
        "''",
        "\" \"",
        "tool 'unfinished",
        "tool \"unfinished",
        r"tool \",
        "tool @unknown/file",
        "tool @rule",
        "tool @rule/",
        "tool @source//file",
        "tool @repo/../outside",
        "tool @rule/../outside",
        "tool @source/a/../../outside",
        "tool @rule/./file",
        "tool @repo/C:/file",
        "tool '@source/dir\\file'",
        "tool @source//absolute",
        "tool @literal",
        "tool | other",
        "tool || other",
        "tool && other",
        "tool &",
        "tool; other",
        "tool > output",
        "tool < input",
        "tool $(other)",
        "tool `other`",
        "tool $HOME",
        "tool \"$HOME\"",
        "tool \"`other`\"",
        "(tool)",
        "tool # comment",
        "tool\nother",
        "tool\r\nother",
        "tool\0",
        "tool '\n'",
        "tool \\\nother",
    ] {
        let d = project();
        check(
            d.path(),
            "invalid",
            "enabled = false",
            &format!("command = {}", serde_json::to_string(command).unwrap()),
            "exit 0",
        );
        let message = error(d.path());
        assert!(
            message.contains(".please/checks/invalid/rule.toml"),
            "{command:?}: {message}"
        );
        assert!(message.contains("command"), "{command:?}: {message}");
    }
}

#[cfg(unix)]
#[test]
fn string_command_arguments_reach_the_process_unchanged() {
    let d = project();
    write(
        d.path(),
        "scripts/argument probe.sh",
        "printf '<%s>\\n' \"$@\"\n",
    );
    check(
        d.path(),
        "arguments",
        "run_at = ['check']",
        r#"command = '''sh "scripts/argument probe.sh" "" 'two words' '$HOME $(touch unwanted) *.md' escaped\ space "a\"b"'''"#,
        "exit 2",
    );
    let p = config::load(d.path()).unwrap();
    let result = please::checks::execute(&p, &p.checks[0]);
    assert_eq!(result.status, Status::Pass, "{}", result.reason);
    assert_eq!(
        result.stdout.unwrap().text,
        "<>\n<two words>\n<$HOME $(touch unwanted) *.md>\n<escaped space>\n<a\"b>\n"
    );
    assert!(!d.path().join("unwanted").exists());
}

#[cfg(unix)]
#[test]
fn command_representations_execute_equivalently() {
    use std::os::unix::fs::PermissionsExt;
    for (code, expected) in [
        (0, Status::Pass),
        (1, Status::Fail),
        (2, Status::Error),
        (7, Status::Fail),
    ] {
        let d = project();
        let relative = "scripts/check with spaces.sh";
        write(
            d.path(),
            relative,
            &format!(
                "#!/bin/sh\ntest -s README.md || exit 2\ncat \"$1\"\nprintf '\\n%s' \"$2\"\nprintf diagnostic >&2\nexit {code}\n"
            ),
        );
        let executable = d.path().join(relative);
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        let quoted = format!("\"{}\"", executable.display());
        let mut previous = None;
        for implementation in [
            format!(
                "command = {}",
                serde_json::to_string(&format!("{quoted} @rule/resource @param/label")).unwrap()
            ),
            format!(
                "command = {}",
                serde_json::to_string(&format!("sh {quoted} @rule/resource @param/label")).unwrap()
            ),
        ] {
            check(
                d.path(),
                "equivalent",
                "run_at = ['check']\n[parameters.label]\ntype = 'string'\nvalue = 'independent'",
                &implementation,
                "exit 2",
            );
            write(d.path(), ".please/checks/equivalent/resource", "bundled");
            let p = config::load(d.path()).unwrap();
            let result = please::checks::execute(&p, &p.checks[0]);
            assert_eq!(
                result.status, expected,
                "{implementation}: {}",
                result.reason
            );
            assert!(result.blocking);
            assert_eq!(result.exit_code, Some(code));
            let output = (
                result.reason,
                result.stdout.unwrap().text,
                result.stderr.unwrap().text,
            );
            assert_eq!(output.1, "bundled\nindependent");
            assert_eq!(output.2, "diagnostic");
            if let Some(previous) = &previous {
                assert_eq!(&output, previous, "{implementation}");
            }
            previous = Some(output);
        }
    }
}

#[test]
fn commands_validate_without_execution() {
    for implementation in [
        "",
        "command = []",
        "command = ['']",
        "command = ['   ']",
        "command = ['git', 1]",
        r#"command = ["git", "\u0000"]"#,
        "command = ['git']\nentrypoint = 'index.sh'",
        "command = ['git']\ninterpreter = 'sh'",
        "command = 'git --version'\nentrypoint = 'index.sh'",
        "command = 'git --version'\ninterpreter = 'sh'",
        "command = 'git --version'\nentrypoint = 'index.sh'\ninterpreter = 'sh'",
    ] {
        let d = project();
        check(
            d.path(),
            "invalid",
            "enabled = false",
            implementation,
            "exit 0",
        );
        assert!(config::load(d.path()).is_err(), "accepted {implementation}");
    }
    let command = "\"please-no-such-command ''\"";
    let d = project();
    check(
        d.path(),
        "missing",
        "run_at = ['check']",
        &format!("command = {command}"),
        "exit 0",
    );
    check(
        d.path(),
        "disabled",
        "enabled = false",
        &format!("command = {command}"),
        "exit 0",
    );
    let p = config::load(d.path()).unwrap();
    assert_eq!(
        planner::build(&p, Some("check"), false)
            .unwrap()
            .plan
            .scheduled
            .len(),
        1
    );
    let report = please::verify_project_at(
        p,
        &Never,
        please::VerificationMode::AllRules,
        Some("check"),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.results.len(), 1);
    assert_eq!(report.results[0].status, Status::Error);
}

#[cfg(unix)]
#[test]
fn commands_preserve_literal_arguments_and_rule_context() {
    for (code, expected) in [
        (0, Status::Pass),
        (1, Status::Fail),
        (2, Status::Error),
        (7, Status::Fail),
    ] {
        let d = project();
        let literal = "two words; $HOME $(echo expanded) *.md";
        let script = format!(
            "test -s README.md || exit 2; cat \"$3\"; printf '\\n<%s>\\n<%s>\\n%s' \"$1\" \"$2\" \"$4\"; exit {code}"
        );
        write(d.path(), "scripts/probe.sh", &script);
        let command = serde_json::to_string(&format!(
            "sh @source/scripts/probe.sh '' '{literal}' @rule/resource @param/label"
        ))
        .unwrap();
        check(
            d.path(),
            "native",
            "run_at = ['check']\n[parameters.label]\ntype = 'string'\nvalue = 'independent'",
            &format!("command = {command}"),
            "exit 2",
        );
        write(d.path(), ".please/checks/native/resource", "bundled");
        let p = config::load(d.path()).unwrap();
        let plan = planner::build(&p, Some("check"), false).unwrap().plan;
        assert_eq!(plan.scheduled.len(), 1);
        let result = please::checks::execute(&p, &p.checks[0]);
        assert_eq!(result.status, expected, "{}", result.reason);
        assert!(result.blocking);
        assert_eq!(result.exit_code, Some(code));
        assert_eq!(
            result.stdout.unwrap().text,
            format!("bundled\n<>\n<{literal}>\nindependent")
        );
    }
}

#[test]
fn direct_commands_use_the_existing_process_runner() {
    let d = project();
    check(
        d.path(),
        "native",
        "run_at = ['check']",
        "command = 'git --version'",
        "exit 2",
    );
    let p = config::load(d.path()).unwrap();
    let before = planner::build(&p, Some("check"), false)
        .unwrap()
        .plan
        .plan_id;
    let result = please::checks::execute(&p, &p.checks[0]);
    assert_eq!(result.status, Status::Pass);
    assert!(result.stdout.unwrap().text.starts_with("git version"));
    check(
        d.path(),
        "native",
        "run_at = ['check']",
        "command = 'git --help'",
        "exit 2",
    );
    let p = config::load(d.path()).unwrap();
    assert_ne!(
        before,
        planner::build(&p, Some("check"), false)
            .unwrap()
            .plan
            .plan_id
    );
}

#[cfg(unix)]
#[test]
fn local_path_references_execute_with_consumer_working_directory() {
    use std::os::unix::fs::PermissionsExt;
    let d = project();
    let script = "#!/bin/sh\ntest -s README.md || exit 2\nprintf '<%s>\\n' \"$PWD\" \"$@\"\n";
    write(d.path(), "scripts/shared tool.sh", script);
    write(d.path(), ".please/checks/paths/rule tool.sh", script);
    for relative in [
        "scripts/shared tool.sh",
        ".please/checks/paths/rule tool.sh",
    ] {
        fs::set_permissions(d.path().join(relative), fs::Permissions::from_mode(0o755)).unwrap();
    }
    for executable in [
        "@rule/rule tool.sh",
        "@source/scripts/shared tool.sh",
        "@repo/scripts/shared tool.sh",
    ] {
        let command = format!(
            "\"{executable}\" \"@rule/rule tool.sh\" @repo/README.md \"@source/scripts/shared tool.sh\" @@literal"
        );
        check(
            d.path(),
            "paths",
            "run_at = ['check']",
            &format!("command = {}", serde_json::to_string(&command).unwrap()),
            "exit 2",
        );
        let p = config::load(d.path()).unwrap();
        let result = please::checks::execute(&p, &p.checks[0]);
        assert_eq!(result.status, Status::Pass, "{}", result.reason);
        let root = d.path().canonicalize().unwrap();
        assert_eq!(
            result.stdout.unwrap().text,
            format!(
                "<{}>\n<{}>\n<{}>\n<{}>\n<@literal>\n",
                root.display(),
                root.join(".please/checks/paths/rule tool.sh").display(),
                root.join("README.md").display(),
                root.join("scripts/shared tool.sh").display()
            )
        );
    }
}

#[test]
fn reference_overrides_drive_check_review_plans_and_saved_reports() {
    let d = project();
    check(
        d.path(),
        "readme",
        "run_at = ['pre-commit']",
        "command = 'git --version'",
        "",
    );
    check(d.path(), "inherited", "", "command = 'git --version'", "");
    review(d.path(), "run_at = ['pre-commit']");
    write(
        d.path(),
        ".please/rule-sets/quality.toml",
        r#"
id = "quality"
description = "Reference assignments"
run_at = ["pre-push"]
rules = [
    { id = "readme", run_at = ["check", "release"] },
    { id = "design", run_at = ["check", "release"] },
    "inherited",
]
"#,
    );
    let project = config::load(d.path()).unwrap();
    let plan = planner::build(&project, Some("check"), false).unwrap().plan;
    assert!(plan.executable());
    assert_eq!(plan.required_ids, ["readme", "design"]);
    assert_eq!(
        planner::build(&project, Some("pre-commit"), false)
            .unwrap()
            .plan
            .scheduled
            .len(),
        0
    );
    assert_eq!(
        planner::build(&project, Some("pre-push"), false)
            .unwrap()
            .plan
            .required_ids,
        ["inherited"]
    );
    // Checks-only plans keep reviews scheduled; they are reported NOT_RUN at execution.
    let checks_only = planner::build(&project, Some("check"), true).unwrap().plan;
    assert!(checks_only.executable());
    assert_eq!(checks_only.required_ids, ["readme", "design"]);
    for at in ["check", "release"] {
        let project = config::load(d.path()).unwrap();
        let report = please::verify_project_at(
            project,
            &Mock("PASS"),
            please::VerificationMode::AllRules,
            Some(at),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(report.overall, Status::Pass);
        assert_eq!(report.results.len(), 2);
        let saved = report::read(d.path()).unwrap();
        assert_eq!(saved.plan.required_ids, plan.required_ids);
        for rule in &saved.plan.scheduled {
            let origin = rule.origin.as_ref().unwrap();
            assert_eq!(
                origin.execution_points,
                ["check".into(), "release".into()].into()
            );
            assert_eq!(origin.rule_sets, ["quality".into()].into());
        }
    }
}
