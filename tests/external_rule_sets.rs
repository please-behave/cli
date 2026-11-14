mod support;
use please::{
    config, external, planner,
    provider::{Provider, Request},
    report::Status,
};
use std::{
    fs,
    path::{Path, PathBuf},
};
use tempfile::TempDir;

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
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}
fn write(root: &Path, path: &str, text: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}
fn commit(root: &Path) -> String {
    git(root, &["add", "."]);
    git(root, &["commit", "--quiet", "-m", "fixture"]);
    git(root, &["rev-parse", "HEAD"])
}

#[cfg(unix)]
#[test]
fn check_dependencies_are_qualified_per_binding_and_consumer_references_are_exact() {
    let f = Fixture::new();
    write(
        &f.remote,
        "repository/config.toml",
        "id = 'quality'\ndescription = 'Dependencies'\nrun_at = ['check']\nrules = ['repository/checks/build', 'repository/checks/test']\n",
    );
    for (id, depends) in [("build", "[]"), ("test", "['build']")] {
        write(
            &f.remote,
            &format!("repository/checks/{id}/rule.toml"),
            &format!(
                "id = '{id}'\ndescription = 'Dependency check'\ndepends_on = {depends}\n[check]\ncommand = 'git --version'\n"
            ),
        );
    }
    let revision = commit(&f.remote);
    f.imports(&[("repository", &revision), ("repository", &revision)]);
    let mut loaded = f.load(true).unwrap();
    let tests: Vec<_> = loaded
        .checks
        .iter()
        .filter(|c| c.id.ends_with(":test"))
        .collect();
    assert_eq!(tests.len(), 2);
    assert_ne!(tests[0].depends_on, tests[1].depends_on);
    for check in tests {
        assert_eq!(
            check.depends_on,
            [check.id.strip_suffix("test").unwrap().to_owned() + "build"]
        );
    }
    let imported = loaded
        .checks
        .iter()
        .find(|c| c.id.ends_with(":test"))
        .unwrap()
        .id
        .clone();
    support::check(&f.root, "consumer", &["check"], "exit 0");
    let path = f.root.join(".please/checks/consumer/rule.toml");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("depends_on = [{imported:?}]\n{text}")).unwrap();
    loaded = f.load(false).unwrap();
    let report = please::verify_project_at(
        loaded,
        &Never,
        please::VerificationMode::AllRules,
        Some("check"),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.overall, Status::Pass);
    assert_eq!(please::report::read(&f.root).unwrap().results.len(), 5);
    fs::write(&path, format!("depends_on = ['test']\n{text}")).unwrap();
    let error = planner::build(&f.load(false).unwrap(), Some("check"), false)
        .err()
        .unwrap();
    assert!(format!("{error:#}").contains("ambiguous"));
}
fn definitions(root: &Path, path: &str, script: &str) {
    write(
        root,
        &format!("{path}/config.toml"),
        &format!(
            "id = 'quality'\ndescription = 'Reusable rules'\nrules = ['{path}/checks/required-files']\nrun_at = ['check']\n"
        ),
    );
    write(
        root,
        &format!("{path}/checks/required-files/rule.toml"),
        "id = 'required-files'\ndescription = 'Inspect consuming project'\nenabled = true\n[check]\ncommand = 'sh @rule/index.sh'\ntimeout_seconds = 5\n",
    );
    write(
        root,
        &format!("{path}/checks/required-files/index.sh"),
        script,
    );
}
struct Fixture {
    _directory: TempDir,
    root: PathBuf,
    remote: PathBuf,
    cache: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let base = directory.path().canonicalize().unwrap();
        let root = base.join("project");
        let remote = base.join("remote");
        let cache = base.join("cache");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&remote).unwrap();
        fs::create_dir(&cache).unwrap();
        git(&root, &["init", "--quiet", "--initial-branch=main"]);
        git(&remote, &["init", "--quiet", "--initial-branch=main"]);
        please::init::init(&root).unwrap();
        support::configure_adapter(&root);
        write(
            &root,
            ".please/config.toml",
            r#"project = "My_App"
max_display_files = 5
[execution_points]
custom = []
"#,
        );
        definitions(&remote, "repository", "git --version");
        definitions(&remote, "software", "git --version");
        commit(&remote);
        Self {
            _directory: directory,
            root,
            remote,
            cache,
        }
    }
    fn imports(&self, selections: &[(&str, &str)]) {
        let base = r#"project = "My_App"
max_display_files = 5
[execution_points]
custom = []
"#;
        let mut text = base.to_owned();
        for (path, reference) in selections {
            text += &format!(
                "\n[[imports]]\ngit = {:?}\npath = {path:?}\nref = {reference:?}\n",
                self.remote.to_str().unwrap()
            );
        }
        write(&self.root, ".please/config.toml", &text);
    }
    fn load(&self, refresh: bool) -> anyhow::Result<config::Project> {
        external::load(&self.root, refresh, Some(&self.cache))
    }
    fn state(&self) -> Vec<u8> {
        fs::read(self.root.join(".git/please/external-rule-sets.json")).unwrap()
    }
    fn origin(&self) -> please::external::Origin {
        self.load(false)
            .unwrap()
            .effective
            .rules
            .values()
            .find_map(|r| r.external.clone())
            .unwrap()
    }
    fn contents(&self) -> PathBuf {
        let origin = self.origin();
        self.cache
            .join("git")
            .join(origin.repository)
            .join(origin.commit)
            .join("contents")
    }
}
#[test]
fn sync_output_tracks_import_state_and_cache_changes_without_hook_changes() {
    let f = Fixture::new();
    support::check(&f.root, "always-ok", &["check"], "exit 0");
    let sync = |expected: &[u8]| {
        let output = support::command(env!("CARGO_BIN_EXE_please"))
            .args(["sync", "--yes"])
            .current_dir(&f.root)
            .env("XDG_CACHE_HOME", &f.cache)
            .env("NO_COLOR", "1")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(0), "{output:?}");
        assert_eq!(output.stdout, expected);
        assert!(output.stderr.is_empty(), "{output:?}");
        assert!(!f.root.join(".git/hooks/pre-push").exists());
    };
    let changed = b"Synchronization completed successfully.\n";
    let unchanged = b"Already synchronized.\n";
    sync(unchanged);
    f.imports(&[("repository", "main")]);
    sync(changed);
    sync(unchanged);

    write(
        &f.remote,
        "repository/checks/required-files/index.sh",
        "exit 0\n",
    );
    let revision = commit(&f.remote);
    sync(changed);
    sync(unchanged);

    let repository = fs::read_dir(f.cache.join("please/git"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    fs::remove_dir_all(repository.join(&revision)).unwrap();
    sync(changed);
    sync(unchanged);
    fs::remove_dir_all(repository.join("objects.git")).unwrap();
    sync(changed);
    sync(unchanged);
    fs::remove_file(f.root.join(".git/please/external-rule-sets.json")).unwrap();
    sync(changed);
    sync(unchanged);

    f.imports(&[]);
    sync(changed);
    sync(unchanged);
}

struct Never;
impl Provider for Never {
    fn review(&self, _: &Request<'_>) -> anyhow::Result<String> {
        panic!("unexpected provider call")
    }
}

#[test]
fn branch_tag_and_full_commit_share_one_immutable_revision() {
    let f = Fixture::new();
    git(&f.remote, &["tag", "-a", "v1.0.0", "-m", "tag"]);
    let commit = git(&f.remote, &["rev-parse", "HEAD"]);
    f.imports(&[("repository", "main"), ("software", "v1.0.0")]);
    let project = f.load(true).unwrap();
    let origins: Vec<_> = project
        .effective
        .rules
        .values()
        .filter_map(|r| r.external.as_ref())
        .collect();
    assert_eq!(origins.len(), 2);
    assert!(origins.iter().all(|o| o.commit == commit));
    let repository = f.cache.join("git").join(&origins[0].repository);
    assert_eq!(
        fs::read_dir(repository)
            .unwrap()
            .filter(|e| e.as_ref().unwrap().path().join("receipt.json").exists())
            .count(),
        1
    );
    f.imports(&[("repository", &commit)]);
    f.load(true).unwrap();
    assert_eq!(f.origin().commit, commit);
}

#[test]
fn local_and_identical_external_ids_are_isolated_and_order_is_deterministic() {
    let f = Fixture::new();
    support::check(&f.root, "required-files", &["check"], "git --version");
    f.imports(&[("repository", "main"), ("software", "main")]);
    let a = f.load(true).unwrap();
    let plan = planner::build(&a, Some("check"), false).unwrap().plan;
    assert_eq!(plan.scheduled.len(), 3);
    assert!(plan.scheduled.iter().any(|r| r.id == "required-files"));
    assert!(plan.scheduled.iter().all(|r| {
        !r.origin
            .as_ref()
            .unwrap()
            .source
            .to_string_lossy()
            .contains("cache/")
    }));
    f.imports(&[("software", "main"), ("repository", "main")]);
    let b = f.load(true).unwrap();
    assert_eq!(
        serde_json::to_value(&a.effective.rules).unwrap(),
        serde_json::to_value(&b.effective.rules).unwrap()
    );
    let report = please::verify_project_at(
        b,
        &Never,
        please::VerificationMode::AllRules,
        Some("check"),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.overall, Status::Pass);
    assert_eq!(report.results.len(), 3);
    please::report::read(&f.root).unwrap();
}

#[test]
fn execution_targets_consumer_and_source_resources_are_explicit() {
    let f = Fixture::new();
    write(&f.root, "README.md", "consumer");
    // The script locates its own source resources next to itself.
    definitions(
        &f.remote,
        "repository",
        "test -f README.md && test -f \"$(dirname \"$0\")/resource.txt\"",
    );
    write(
        &f.remote,
        "repository/checks/required-files/resource.txt",
        "source",
    );
    commit(&f.remote);
    f.imports(&[("repository", "main")]);
    let project = f.load(true).unwrap();
    let report = please::verify_project_at(
        project,
        &Never,
        please::VerificationMode::AllRules,
        Some("check"),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.overall, Status::Pass);
    assert_eq!(report.results.len(), 1);
}

#[test]
fn offline_execution_uses_last_resolution_and_refresh_moves_only_mutable_refs() {
    let f = Fixture::new();
    let original = git(&f.remote, &["rev-parse", "HEAD"]);
    f.imports(&[("repository", "main")]);
    f.load(true).unwrap();
    let contents = f.contents();
    let timestamp = contents.metadata().unwrap().modified().unwrap();
    write(&f.remote, "repository/note.md", "new");
    let updated = commit(&f.remote);
    assert_eq!(f.origin().commit, original);
    f.load(true).unwrap();
    assert_eq!(f.origin().commit, updated);
    assert_eq!(contents.metadata().unwrap().modified().unwrap(), timestamp);
    f.imports(&[("repository", &original)]);
    f.load(true).unwrap();
    assert_eq!(f.origin().commit, original);
    fs::rename(&f.remote, f.remote.with_extension("offline")).unwrap();
    let project = f.load(false).unwrap();
    let report = please::verify_project_at(
        project,
        &Never,
        please::VerificationMode::AllRules,
        Some("check"),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.overall, Status::Pass);
    let previous = f.state();
    f.imports(&[("repository", "main")]);
    assert!(f.load(true).is_err());
    assert_eq!(f.state(), previous);
}

#[test]
fn invalid_refresh_preserves_previous_valid_state_without_claiming_success() {
    let f = Fixture::new();
    f.imports(&[("repository", "main")]);
    f.load(true).unwrap();
    let previous = f.state();
    write(
        &f.remote,
        "repository/config.toml",
        "id = \"quality\"\ndescription = \"Reusable rules\"\nrules = [\"repository/checks/missing\"]\nrun_at = [\"check\"]\n",
    );
    commit(&f.remote);
    let error = format!("{:#}", f.load(true).unwrap_err());
    assert!(error.contains("missing or unsafe"), "{error}");
    assert_eq!(f.state(), previous);
    f.load(false).unwrap();
}

#[test]
fn missing_state_missing_cache_and_corruption_fail_closed() {
    let f = Fixture::new();
    f.imports(&[("repository", "main")]);
    assert!(format!("{:#}", f.load(false).unwrap_err()).contains("please sync"));
    f.load(true).unwrap();
    let contents = f.contents();
    fs::write(
        contents.join("repository/checks/required-files/rule.toml"),
        "corrupted",
    )
    .unwrap();
    assert!(format!("{:#}", f.load(false).unwrap_err()).contains("integrity"));
    assert!(f.load(true).is_err());
    fs::remove_dir_all(contents.parent().unwrap()).unwrap();
    assert!(format!("{:#}", f.load(false).unwrap_err()).contains("please sync"));
    f.load(true).unwrap();
}

#[test]
fn unavailable_references_and_repository_are_actionable() {
    let f = Fixture::new();
    f.imports(&[("repository", "not-a-reference")]);
    assert!(format!("{:#}", f.load(true).unwrap_err()).contains("reference"));
    f.imports(&[("repository", "main")]);
    fs::remove_dir_all(&f.remote).unwrap();
    assert!(format!("{:#}", f.load(true).unwrap_err()).contains("repository unavailable"));
}

#[test]
fn repository_root_is_a_governance_root() {
    let f = Fixture::new();
    fs::remove_dir_all(f.remote.join("repository")).unwrap();
    fs::remove_dir_all(f.remote.join("software")).unwrap();
    definitions(&f.remote, ".", "git --version");
    commit(&f.remote);
    f.imports(&[(".", "main")]);
    assert_eq!(f.load(true).unwrap().rule_sets.len(), 1);
}

#[test]
fn repeated_imports_are_separate_bindings_and_consumer_overrides_are_rejected() {
    let f = Fixture::new();
    f.imports(&[("repository", "main"), ("./repository/", "main")]);
    let p = f.load(true).unwrap();
    let plan = planner::build(&p, Some("check"), false).unwrap().plan;
    assert_eq!(plan.scheduled.len(), 2);
    assert_ne!(plan.scheduled[0].id, plan.scheduled[1].id);
    f.imports(&[("repository", "main")]);
    let path = f.root.join(".please/config.toml");
    let text = fs::read_to_string(&path).unwrap();
    for field in [
        "enabled = false",
        "rules = []",
        "run_at = []",
        "id = \"override\"",
    ] {
        fs::write(&path, format!("{text}\n{field}\n")).unwrap();
        assert!(f.load(true).is_err(), "{field}");
    }
}

#[test]
fn traversal_and_symlink_escapes_are_rejected() {
    let f = Fixture::new();
    for path in ["../software", "/tmp", "repository/../software"] {
        f.imports(&[(path, "main")]);
        assert!(format!("{:#}", f.load(true).unwrap_err()).contains("path"));
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink("../software", f.remote.join("link")).unwrap();
        fs::remove_file(f.remote.join("repository/checks/required-files/index.sh")).unwrap();
        std::os::unix::fs::symlink(
            "../../../software/config.toml",
            f.remote.join("repository/checks/required-files/index.sh"),
        )
        .unwrap();
        commit(&f.remote);
        f.imports(&[("link", "main")]);
        assert!(format!("{:#}", f.load(true).unwrap_err()).contains("symlink"));
        f.imports(&[("repository", "main")]);
        let p = f.load(true).unwrap();
        let check = p.checks.iter().find(|c| c.id.starts_with("git:")).unwrap();
        let result = please::checks::execute(&p, check);
        assert_eq!(result.status, Status::Error);
        assert!(result.reason.contains("symlink"));
    }
}

#[test]
fn concurrent_refreshes_publish_complete_state_and_share_checkout() {
    let f = Fixture::new();
    f.imports(&[("repository", "main"), ("software", "main")]);
    std::thread::scope(|scope| {
        let a = scope.spawn(|| f.load(true).unwrap());
        let b = scope.spawn(|| f.load(true).unwrap());
        assert_eq!(
            serde_json::to_value(a.join().unwrap().effective.rules).unwrap(),
            serde_json::to_value(b.join().unwrap().effective.rules).unwrap()
        );
    });
    assert_eq!(f.load(false).unwrap().rule_sets.len(), 2);
}

#[test]
fn retrieval_never_executes_rules_or_remote_hooks_and_errors_hide_credentials() {
    let f = Fixture::new();
    definitions(&f.remote, "repository", "sh -c 'touch SHOULD_NOT_EXIST'");
    commit(&f.remote);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let hook = f.remote.join(".git/hooks/post-checkout");
        fs::write(
            &hook,
            format!("#!/bin/sh\ntouch {}/HOOK_RAN\n", f.root.display()),
        )
        .unwrap();
        fs::set_permissions(hook, fs::Permissions::from_mode(0o755)).unwrap();
    }
    f.imports(&[("repository", "main")]);
    f.load(true).unwrap();
    assert!(!f.root.join("SHOULD_NOT_EXIST").exists());
    assert!(!f.root.join("HOOK_RAN").exists());
    let path = f.root.join(".please/config.toml");
    let text = fs::read_to_string(&path).unwrap().replace(
        f.remote.to_str().unwrap(),
        "https://user:SECRET@example.invalid/rules",
    );
    fs::write(path, text).unwrap();
    let error = format!("{:#}", f.load(true).unwrap_err());
    assert!(!error.contains("SECRET"), "{error}");
    assert!(error.contains("credentials"), "{error}");
}

struct ReviewMock;
impl Provider for ReviewMock {
    fn review(&self, request: &Request<'_>) -> anyhow::Result<String> {
        assert_eq!(request.reviews.len(), 1);
        assert_eq!(request.reviews[0].text, "Judge consumer documentation.\n");
        assert!(request.project.root.join("README.md").is_file());
        assert_eq!(request.reviews[0].files, ["README.md"]);
        Ok(serde_json::json!({"reviews":[{
            "id":request.reviews[0].id, "status":"PASS", "reason":"Documentation is present",
            "findings":[{"message":"Consumer README exists","path":"README.md","line":1}]
        }]})
        .to_string())
    }
}
#[test]
fn imported_reviews_use_source_instructions_and_consumer_evidence() {
    let f = Fixture::new();
    write(&f.root, "README.md", "# Consumer\n");
    write(
        &f.remote,
        "repository/config.toml",
        r#"id = "reviews"
description = "Reusable reviews"
rules = ["repository/reviews/documentation"]
run_at = ["check"]
"#,
    );
    fs::remove_dir_all(f.remote.join("repository/checks")).unwrap();
    write(
        &f.remote,
        "repository/reviews/documentation/rule.toml",
        r#"id = "documentation"
description = "Review documentation"
enabled = true
[review]
prompt = "criteria.md"
files = ["README.md"]
context = [{ path = "README.md", headings = ["Consumer"] }]
"#,
    );
    write(
        &f.remote,
        "repository/reviews/documentation/criteria.md",
        "Judge consumer documentation.\n",
    );
    commit(&f.remote);
    f.imports(&[("repository", "main")]);
    let project = f.load(true).unwrap();
    let report = please::verify_project_at(
        project,
        &ReviewMock,
        please::VerificationMode::AllRules,
        Some("check"),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.overall, Status::Pass);
    assert!(report.results[0].id.starts_with("git:"));
    please::report::read(&f.root).unwrap();
}

#[test]
fn imported_check_failure_propagates_and_no_unselected_rule_executes() {
    let f = Fixture::new();
    definitions(&f.remote, "repository", "sh -c 'exit 7'");
    commit(&f.remote);
    f.imports(&[("repository", "main")]);
    let project = f.load(true).unwrap();
    assert!(
        planner::build(&project, Some("pre-commit"), false)
            .unwrap()
            .plan
            .scheduled
            .is_empty()
    );
    let report = please::verify_project_at(
        project,
        &Never,
        please::VerificationMode::AllRules,
        Some("check"),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.overall, Status::Fail);
    assert_eq!(report.results[0].exit_code, Some(7));
}

#[test]
fn source_instruction_paths_cannot_escape_the_rule_directory() {
    for instructions in [
        "../software/criteria.md",
        "/tmp/criteria.md",
        "escape/criteria.md",
    ] {
        let f = Fixture::new();
        write(
            &f.remote,
            "repository/reviews/review/rule.toml",
            &format!(
                r#"id = "review"
description = "Disabled review still validates"
enabled = false
[review]
prompt = {instructions:?}
"#
            ),
        );
        let central = f.remote.join("repository/config.toml");
        let text = fs::read_to_string(&central).unwrap().replace(
            "repository/checks/required-files",
            "repository/reviews/review",
        );
        fs::write(central, text).unwrap();
        write(&f.remote, "software/criteria.md", "Outside boundary");
        #[cfg(unix)]
        std::os::unix::fs::symlink("../software", f.remote.join("repository/escape")).unwrap();
        commit(&f.remote);
        f.imports(&[("repository", "main")]);
        assert!(f.load(true).is_err(), "{instructions}");
    }
}

fn cli(f: &Fixture, args: &[&str]) -> std::process::Output {
    support::command(env!("CARGO_BIN_EXE_please"))
        .current_dir(&f.root)
        .env("XDG_CACHE_HOME", &f.cache)
        .env("NO_COLOR", "1")
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn cli_refresh_cached_plan_and_offline_hook_keep_existing_lifecycle() {
    let f = Fixture::new();
    f.imports(&[("repository", "main")]);
    let source = f.remote.join("repository/config.toml");
    fs::write(
        &source,
        fs::read_to_string(&source)
            .unwrap()
            .replace("['check']", "['pre-commit']"),
    )
    .unwrap();
    commit(&f.remote);
    let plan = cli(&f, &["run", "--at", "pre-commit", "--plan", "--json"]);
    assert_eq!(plan.status.code(), Some(2));
    let diagnostic: serde_json::Value = serde_json::from_slice(&plan.stdout).unwrap();
    assert_eq!(diagnostic["plan_schema_version"], 3);
    assert_eq!(diagnostic["status"], "blocked");
    assert!(
        !diagnostic["blocking_reasons"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(
        !f.root.join(".git/please").exists(),
        "missing-state preview must remain read-only"
    );
    let refresh = cli(&f, &["sync", "--yes"]);
    assert!(
        refresh.status.success(),
        "{}",
        String::from_utf8_lossy(&refresh.stderr)
    );
    assert!(!f.root.join(".please/reports/last.json").exists());
    assert!(f.root.join(".git/hooks/pre-commit").exists());
    fs::rename(&f.remote, f.remote.with_extension("offline")).unwrap();
    let plan = cli(&f, &["run", "--at", "pre-commit", "--plan", "--json"]);
    assert!(
        plan.status.success(),
        "{}",
        String::from_utf8_lossy(&plan.stderr)
    );
    let run = cli(&f, &["run", "--at", "pre-commit"]);
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    let mut paths = vec![
        Path::new(env!("CARGO_BIN_EXE_please"))
            .parent()
            .unwrap()
            .to_path_buf(),
    ];
    paths.extend(std::env::split_paths(&std::env::var_os("PATH").unwrap()));
    let hook = support::command(f.root.join(".git/hooks/pre-commit"))
        .current_dir(&f.root)
        .env("XDG_CACHE_HOME", &f.cache)
        .env("PATH", std::env::join_paths(paths).unwrap())
        .output()
        .unwrap();
    assert!(
        hook.status.success(),
        "{}",
        String::from_utf8_lossy(&hook.stderr)
    );
}

#[test]
fn all_fixed_hooks_execute_imported_rules_offline() {
    let f = Fixture::new();
    f.imports(&[("repository", "main")]);
    write(
        &f.remote,
        "repository/config.toml",
        "id = 'quality'\ndescription = 'Reusable rules'\nrules = ['repository/checks/required-files']\nrun_at = ['pre-commit', 'commit-msg', 'pre-push']\n",
    );
    write(
        &f.remote,
        "repository/checks/required-files/rule.toml",
        "id = 'required-files'\ndescription = 'Report the execution point'\n[check]\ncommand = 'sh @rule/index.sh @context'\n",
    );
    write(
        &f.remote,
        "repository/checks/required-files/index.sh",
        "python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))[\"execution_point\"], end=\"\")' \"$1\"\n",
    );
    commit(&f.remote);
    let setup = cli(&f, &["sync", "--yes"]);
    assert!(
        setup.status.success(),
        "{}",
        String::from_utf8_lossy(&setup.stderr)
    );
    fs::rename(&f.remote, f.remote.with_extension("offline")).unwrap();
    let paths = std::env::join_paths(
        std::iter::once(
            Path::new(env!("CARGO_BIN_EXE_please"))
                .parent()
                .unwrap()
                .to_path_buf(),
        )
        .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    for event in ["pre-commit", "commit-msg", "pre-push"] {
        let output = support::command(f.root.join(".git/hooks").join(event))
            .current_dir(&f.root)
            .env("XDG_CACHE_HOME", &f.cache)
            .env("PATH", &paths)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report = please::report::read(&f.root).unwrap();
        assert_eq!(report.plan.at, event);
        let imported = report
            .results
            .iter()
            .find(|result| result.id.starts_with("git:"))
            .unwrap();
        assert_eq!(imported.status, Status::Pass);
        assert_eq!(imported.stdout.as_ref().unwrap().text, event);
    }
}

#[test]
fn removed_and_changed_imports_require_their_own_resolution() {
    let f = Fixture::new();
    f.imports(&[("repository", "main")]);
    f.load(true).unwrap();
    f.imports(&[("software", "main")]);
    assert!(format!("{:#}", f.load(false).unwrap_err()).contains("no validated revision"));
    f.load(true).unwrap();
    f.imports(&[]);
    f.load(true).unwrap();
    f.imports(&[("software", "main")]);
    assert!(f.load(false).is_err());
}

#[test]
fn separate_projects_resolving_concurrently_share_one_cache() {
    let f = Fixture::new();
    f.imports(&[("repository", "main")]);
    let second = f.root.with_file_name("second");
    fs::create_dir(&second).unwrap();
    git(&second, &["init", "--quiet"]);
    please::init::init(&second).unwrap();
    fs::copy(
        f.root.join(".please/config.toml"),
        second.join(".please/config.toml"),
    )
    .unwrap();
    std::thread::scope(|scope| {
        let a = scope.spawn(|| f.load(true).unwrap());
        let b = scope.spawn(|| external::load(&second, true, Some(&f.cache)).unwrap());
        let a = a.join().unwrap();
        let b = b.join().unwrap();
        assert_eq!(
            serde_json::to_value(a.effective.rules).unwrap(),
            serde_json::to_value(b.effective.rules).unwrap()
        );
    });
}

#[test]
fn sha256_repositories_and_file_urls_use_native_git_resolution() {
    let f = Fixture::new();
    // Recreate only the temporary source using Git's alternate object format.
    fs::remove_dir_all(&f.remote).unwrap();
    fs::create_dir(&f.remote).unwrap();
    git(
        &f.remote,
        &[
            "init",
            "--quiet",
            "--initial-branch=main",
            "--object-format=sha256",
        ],
    );
    definitions(&f.remote, "repository", "git --version");
    let commit = commit(&f.remote);
    assert_eq!(commit.len(), 64);
    f.imports(&[("repository", &commit)]);
    let path = f.root.join(".please/config.toml");
    let text = fs::read_to_string(&path).unwrap().replace(
        f.remote.to_str().unwrap(),
        &format!("file://{}", f.remote.display()),
    );
    fs::write(path, text).unwrap();
    f.load(true).unwrap();
    assert_eq!(f.origin().commit, commit);
}

#[test]
fn malformed_configuration_does_not_echo_url_credentials() {
    let f = Fixture::new();
    let path = f.root.join(".please/config.toml");
    for text in [
        "[[imports]]\ngit = \"https://user:SECRET@example.invalid/rules\" INVALID\n",
        "[[imports]]\ngit = \"https://user:SECRET@example.invalid/rules\"\npath = \".\"\nref = \"main\"\nenabled = false\n",
    ] {
        fs::write(&path, text).unwrap();
        let error = format!("{:#}", f.load(true).unwrap_err());
        assert!(!error.contains("SECRET"), "{error}");
    }
}

#[cfg(unix)]
#[test]
fn authentication_failure_hides_helper_diagnostics() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    f.imports(&[("repository", "main")]);
    let path = f.root.join(".please/config.toml");
    let text = fs::read_to_string(&path).unwrap().replace(
        f.remote.to_str().unwrap(),
        "ssh://git@example.invalid/rules.git",
    );
    fs::write(path, text).unwrap();
    let ssh = f.root.with_file_name("fake-ssh");
    fs::write(
        &ssh,
        "#!/bin/sh\necho 'Permission denied: SECRET' >&2\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&ssh, fs::Permissions::from_mode(0o755)).unwrap();
    let output = support::command(env!("CARGO_BIN_EXE_please"))
        .current_dir(&f.root)
        .env("XDG_CACHE_HOME", &f.cache)
        .env("GIT_SSH_COMMAND", ssh)
        .env("GIT_SSH_VARIANT", "ssh")
        .args(["sync", "--yes"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.contains("authentication failed"), "{output}");
    assert!(!output.contains("SECRET"), "{output}");
}

#[test]
fn unselected_directories_are_not_interpreted_as_configuration() {
    let f = Fixture::new();
    write(&f.remote, "unselected/broken.toml", "INVALID TOML");
    write(&f.remote, "software/arbitrary.toml", "INVALID TOML");
    commit(&f.remote);
    f.imports(&[("repository", "main")]);
    f.load(true).unwrap();
}

#[test]
fn unsafe_repository_selectors_are_rejected_before_retrieval() {
    let f = Fixture::new();
    f.imports(&[("repository", "main")]);
    let path = f.root.join(".please/config.toml");
    let text = fs::read_to_string(&path).unwrap();
    for url in [
        "ext::sh -c touch-UNEXPECTED",
        "custom-helper::somewhere",
        "-h",
        "https://user:SECRET@example.invalid/rules",
        "ssh://user:SECRET@example.invalid/rules",
    ] {
        fs::write(&path, text.replace(f.remote.to_str().unwrap(), url)).unwrap();
        let error = format!("{:#}", f.load(true).unwrap_err());
        assert!(!error.contains("SECRET"), "{error}");
        assert!(!f.cache.join("git").exists(), "{url}");
    }
}

#[cfg(unix)]
#[test]
fn ssh_ports_and_ipv6_authorities_reach_host_git_without_password_confusion() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    f.imports(&[("repository", "main")]);
    let path = f.root.join(".please/config.toml");
    let text = fs::read_to_string(&path).unwrap();
    let ssh = f.root.with_file_name("fake-ssh");
    fs::write(&ssh, "#!/bin/sh\necho 'Permission denied' >&2\nexit 1\n").unwrap();
    fs::set_permissions(&ssh, fs::Permissions::from_mode(0o755)).unwrap();
    for url in [
        "ssh://git@example.invalid:2222/rules.git",
        "ssh://example.invalid:2222/rules.git",
        "ssh://[::1]:2222/rules.git",
    ] {
        fs::write(&path, text.replace(f.remote.to_str().unwrap(), url)).unwrap();
        let output = support::command(env!("CARGO_BIN_EXE_please"))
            .current_dir(&f.root)
            .env("XDG_CACHE_HOME", &f.cache)
            .env("GIT_SSH_COMMAND", &ssh)
            .env("GIT_SSH_VARIANT", "ssh")
            .args(["sync", "--yes"])
            .output()
            .unwrap();
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("authentication failed"),
            "{url}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn separate_repositories_with_identical_paths_and_ids_remain_distinct() {
    let f = Fixture::new();
    let other = f.remote.with_file_name("other");
    fs::create_dir(&other).unwrap();
    git(&other, &["init", "--quiet", "--initial-branch=main"]);
    definitions(&other, "repository", "git --version");
    commit(&other);
    f.imports(&[("repository", "main")]);
    let path = f.root.join(".please/config.toml");
    let text = fs::read_to_string(&path).unwrap();
    fs::write(
        path,
        format!(
            "{text}\n[[imports]]\ngit = {:?}\npath = \"repository\"\nref = \"main\"\n",
            other.to_str().unwrap()
        ),
    )
    .unwrap();
    let project = f.load(true).unwrap();
    let selected = planner::build(&project, Some("check"), false)
        .unwrap()
        .plan
        .scheduled;
    assert_eq!(selected.len(), 2);
    assert_ne!(selected[0].id, selected[1].id);
    assert_ne!(
        selected[0]
            .origin
            .as_ref()
            .unwrap()
            .external
            .as_ref()
            .unwrap()
            .repository,
        selected[1]
            .origin
            .as_ref()
            .unwrap()
            .external
            .as_ref()
            .unwrap()
            .repository
    );
}

#[test]
fn moved_tags_refresh_but_cached_execution_remains_on_the_validated_commit() {
    let f = Fixture::new();
    git(&f.remote, &["tag", "shared"]);
    f.imports(&[("repository", "shared")]);
    f.load(true).unwrap();
    let original = f.origin().commit;
    write(&f.remote, "repository/change.md", "new tag target");
    let updated = commit(&f.remote);
    git(&f.remote, &["tag", "--force", "shared"]);
    assert_eq!(f.origin().commit, original);
    f.load(true).unwrap();
    assert_eq!(f.origin().commit, updated);
}

fn mapped_imports(f: &Fixture, mappings: &[(&str, &str)], custom: &[&str]) {
    let mut text = format!("project = 'My_App'\n[execution_points]\ncustom = {custom:?}\n");
    for (path, mapping) in mappings {
        text += &format!(
            "[[imports]]\ngit = {:?}\npath = {path:?}\nref = 'main'\n{mapping}\n",
            f.remote.to_str().unwrap()
        );
    }
    write(&f.root, ".please/config.toml", &text);
}
fn source_points(f: &Fixture, path: &str, points: &[&str]) {
    let file = f.remote.join(path).join("config.toml");
    let text = fs::read_to_string(&file)
        .unwrap()
        .replace("['check']", &format!("{points:?}"));
    fs::write(file, text).unwrap();
}

#[test]
fn independent_mappings_resolve_inheritance_many_to_one_and_offline_execution() {
    let f = Fixture::new();
    source_points(&f, "repository", &["lint", "test"]);
    source_points(&f, "software", &["lint"]);
    let member = f.remote.join("repository/checks/required-files/rule.toml");
    fs::write(
        &member,
        fs::read_to_string(&member)
            .unwrap()
            .replace("[check]", "run_at = ['own']\n[check]"),
    )
    .unwrap();
    // Additional selected Rules inherit the set assignments.
    write(
        &f.remote,
        "repository/checks/standalone/rule.toml",
        "id = 'standalone'\ndescription = 'Standalone'\nrun_at = ['lint', 'test']\n[check]\ncommand = 'sh @rule/index.sh'\n",
    );
    write(
        &f.remote,
        "repository/checks/standalone/index.sh",
        "git --version\n",
    );
    // Disabled Rules validate, keep metadata and never enter execution results.
    write(
        &f.remote,
        "repository/checks/disabled/rule.toml",
        "id = 'disabled'\ndescription = 'Disabled'\nenabled = false\nrun_at = ['test']\n[check]\ncommand = 'sh @rule/index.sh'\n",
    );
    write(&f.remote, "repository/checks/disabled/index.sh", "exit 1\n");
    let central = f.remote.join("repository/config.toml");
    let text = fs::read_to_string(&central).unwrap().replace(
        "'repository/checks/required-files'",
        "'repository/checks/required-files', 'repository/checks/standalone', 'repository/checks/disabled'",
    );
    fs::write(central, text).unwrap();
    commit(&f.remote);
    mapped_imports(
        &f,
        &[
            (
                "repository",
                "[imports.execution_points]\nlint = 'verify'\ntest = 'verify'\nown = 'audit'",
            ),
            ("software", "[imports.execution_points]\nlint = 'audit'"),
        ],
        &["verify", "audit"],
    );
    let project = f.load(true).unwrap();
    let verify = planner::build(&project, Some("verify"), false)
        .unwrap()
        .plan;
    assert_eq!(verify.scheduled.len(), 2);
    assert_eq!(
        planner::build(&project, Some("audit"), false)
            .unwrap()
            .plan
            .scheduled
            .len(),
        1
    );
    assert!(project.checks.iter().any(|r| r.id.ends_with(":standalone")));
    let identities = project.effective.rules.keys().cloned().collect::<Vec<_>>();
    let before = serde_json::to_value(&project.effective.rules).unwrap();
    let contents = f.contents();
    assert!(
        fs::read_to_string(contents.join("repository/config.toml"))
            .unwrap()
            .contains("lint")
    );
    let state = f.state();
    fs::rename(&f.remote, f.remote.with_extension("offline")).unwrap();
    let offline = f.load(false).unwrap();
    assert_eq!(
        before,
        serde_json::to_value(&offline.effective.rules).unwrap()
    );
    let report = please::verify_project_at(
        offline,
        &Never,
        please::VerificationMode::AllRules,
        Some("verify"),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.overall, Status::Pass);
    assert_eq!(report.results.len(), 2);
    assert_eq!(f.state(), state);
    // Editing only mappings needs no refresh, changes no Rule identity or cache.
    mapped_imports(
        &f,
        &[
            (
                "repository",
                "[imports.execution_points]\nlint = 'verify'\ntest = 'verify'\nown = 'verify'",
            ),
            ("software", "[imports.execution_points]\nlint = 'verify'"),
        ],
        &["verify"],
    );
    let project = f.load(false).unwrap();
    assert_eq!(
        planner::build(&project, Some("verify"), false)
            .unwrap()
            .plan
            .scheduled
            .len(),
        3
    );
    assert_eq!(
        project.effective.rules.keys().cloned().collect::<Vec<_>>(),
        identities
    );
    assert_eq!(f.state(), state);
}

#[test]
fn invalid_import_mappings_fail_with_source_target_and_import_context() {
    let f = Fixture::new();
    source_points(&f, "repository", &["quality"]);
    commit(&f.remote);
    mapped_imports(
        &f,
        &[(
            "repository",
            "[imports.execution_points]\nquality = 'verify'",
        )],
        &["verify"],
    );
    f.load(true).unwrap();
    let state = f.state();
    for (source, target, reason) in [
        ("check", "verify", "built-in source"),
        ("pre-commit", "verify", "built-in source"),
        ("commit-msg", "verify", "built-in source"),
        ("pre-push", "verify", "built-in source"),
        ("quality", "check", "built-in target"),
        ("quality", "pre-commit", "built-in target"),
        ("quality", "commit-msg", "built-in target"),
        ("quality", "pre-push", "built-in target"),
        ("unknown", "verify", "unknown source"),
        ("quality", "missing", "unknown target"),
        ("", "verify", "invalid execution-point"),
        ("quality", "", "invalid execution-point"),
        ("Quality", "verify", "invalid execution-point"),
        ("quality", "bad/name", "invalid execution-point"),
    ] {
        mapped_imports(
            &f,
            &[(
                "repository",
                &format!("[imports.execution_points]\n{source:?} = {target:?}"),
            )],
            &["verify"],
        );
        let error = format!("{:#}", f.load(false).unwrap_err());
        assert!(
            error.contains("repository")
                && error.contains(source)
                && error.contains(target)
                && error.contains(reason),
            "{error}"
        );
        assert_eq!(f.state(), state);
    }
    for structure in [
        "execution_points = []",
        "execution_points = 'verify'",
        "execution_points = { quality = ['verify'] }",
        "execution_points = { quality = { target = 'verify' } }",
    ] {
        mapped_imports(&f, &[("repository", structure)], &["verify"]);
        assert!(f.load(false).is_err(), "{structure}");
    }
    mapped_imports(
        &f,
        &[(
            "repository",
            "[imports.execution_points]\nquality = 'verify'",
        )],
        &[],
    );
    assert!(format!("{:#}", f.load(false).unwrap_err()).contains("declare"));
}

#[test]
fn unmapped_custom_points_require_consumer_declarations_and_keep_names() {
    let f = Fixture::new();
    source_points(&f, "repository", &["quality", "audit"]);
    commit(&f.remote);
    mapped_imports(&f, &[("repository", "")], &["quality", "audit"]);
    let project = f.load(true).unwrap();
    assert_eq!(
        planner::build(&project, Some("quality"), false)
            .unwrap()
            .plan
            .scheduled
            .len(),
        1
    );
    mapped_imports(
        &f,
        &[(
            "repository",
            "[imports.execution_points]\nquality = 'verify'",
        )],
        &["verify", "audit"],
    );
    let project = f.load(false).unwrap();
    assert_eq!(
        planner::build(&project, Some("audit"), false)
            .unwrap()
            .plan
            .scheduled
            .len(),
        1
    );
    mapped_imports(
        &f,
        &[(
            "repository",
            "[imports.execution_points]\nquality = 'verify'",
        )],
        &["verify"],
    );
    assert!(
        format!("{:#}", f.load(false).unwrap_err()).contains("unknown resulting execution point")
    );
}

#[cfg(unix)]
#[test]
fn built_in_hooks_execute_imported_and_local_rules_with_message_context() {
    let f = Fixture::new();
    source_points(&f, "repository", &["pre-commit", "commit-msg", "pre-push"]);
    write(
        &f.remote,
        "repository/checks/required-files/rule.toml",
        "id = 'required-files'\ndescription = 'Use the commit message context'\n[check]\ncommand = 'sh @rule/index.sh @context'\n",
    );
    write(
        &f.remote,
        "repository/checks/required-files/index.sh",
        "point=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))[\"execution_point\"])' \"$1\")\nif [ \"$point\" = commit-msg ]; then\n  file=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))[\"commit_message\"][\"file\"])' \"$1\")\n  test -s \"$file\"\nelse\n  git --version\nfi\n",
    );
    commit(&f.remote);
    f.imports(&[("repository", "main")]);
    write(
        &f.root,
        ".please/checks/local/rule.toml",
        "id = 'local'\ndescription = 'Standalone local hook Rule'\nrun_at = ['pre-commit']\n[check]\ncommand = 'sh @rule/index.sh'\n",
    );
    write(&f.root, ".please/checks/local/index.sh", "git --version\n");
    write(
        &f.root,
        ".please/rule-sets/local.toml",
        "id = 'local-set'\ndescription = 'Local set hook assignments'\nrules = ['always-ok']\nrun_at = ['pre-commit', 'commit-msg', 'pre-push']\n",
    );
    let setup = cli(&f, &["sync", "--yes"]);
    assert!(
        setup.status.success(),
        "{}",
        String::from_utf8_lossy(&setup.stderr)
    );
    let paths = std::env::join_paths(
        std::iter::once(
            Path::new(env!("CARGO_BIN_EXE_please"))
                .parent()
                .unwrap()
                .to_path_buf(),
        )
        .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let message = f.root.join(".git/COMMIT_EDITMSG");
    fs::write(&message, "test: message\n").unwrap();
    fs::rename(&f.remote, f.remote.with_extension("offline")).unwrap();
    for event in ["pre-commit", "commit-msg", "pre-push"] {
        let path = f.root.join(".git/hooks").join(event);
        assert!(path.is_file());
        let mut command = support::command(path);
        command
            .current_dir(&f.root)
            .env("XDG_CACHE_HOME", &f.cache)
            .env("PATH", &paths);
        if event == "commit-msg" {
            command.arg(&message);
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{event}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let report = please::report::read(&f.root).unwrap();
        assert!(report.results.iter().any(|r| r.id.starts_with("git:")));
        assert!(report.results.iter().any(|r| r.id == "always-ok"));
        assert_eq!(
            report.results.len(),
            if event == "pre-commit" { 3 } else { 2 }
        );
    }
    assert!(!f.root.join(".git/hooks/check").exists());
    assert!(!f.root.join(".github").exists());
}

#[test]
fn check_only_configuration_creates_no_hooks_or_workflows() {
    let f = Fixture::new();
    f.imports(&[]);
    let file = f.root.join(".please/checks/always-ok/rule.toml");
    fs::write(
        &file,
        fs::read_to_string(&file)
            .unwrap()
            .replace("pre-push", "check"),
    )
    .unwrap();
    let setup = cli(&f, &["sync", "--yes"]);
    assert!(
        setup.status.success(),
        "{}",
        String::from_utf8_lossy(&setup.stderr)
    );
    for event in ["pre-commit", "commit-msg", "pre-push", "check"] {
        assert!(!f.root.join(".git/hooks").join(event).exists());
    }
    let run = cli(&f, &["run", "--at", "check"]);
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(please::report::read(&f.root).unwrap().results.len(), 1);
    assert!(!f.root.join(".github").exists());
}

#[test]
fn explicit_points_execute_imported_rules_offline_with_missing_and_outdated_hooks() {
    let f = Fixture::new();
    source_points(&f, "repository", &["check", "quality"]);
    commit(&f.remote);
    mapped_imports(
        &f,
        &[(
            "repository",
            "[imports.execution_points]\nquality = 'release'",
        )],
        &["release"],
    );
    let setup = cli(&f, &["sync", "--yes"]);
    assert!(
        setup.status.success(),
        "{}",
        String::from_utf8_lossy(&setup.stderr)
    );
    let hook = f.root.join(".git/hooks/pre-push");
    let original = fs::read_to_string(&hook).unwrap();
    fs::remove_file(&hook).unwrap();
    fs::rename(&f.remote, f.remote.with_extension("offline")).unwrap();
    for before in [
        None,
        Some(original.replace("--at 'pre-push'", "--at 'check'")),
    ] {
        if let Some(text) = &before {
            fs::write(&hook, text).unwrap();
        }
        for point in ["check", "release"] {
            let output = cli(&f, &["run", "--at", point]);
            assert!(
                output.status.success(),
                "{point}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let report = please::report::read(&f.root).unwrap();
            assert_eq!(report.overall, Status::Pass);
            assert_eq!(report.results.len(), 1);
            assert!(report.results[0].id.starts_with("git:"));
            assert_eq!(fs::read_to_string(&hook).ok(), before);
            assert!(!f.root.join(".git/hooks/pre-commit").exists());
            assert!(!f.root.join(".git/hooks/commit-msg").exists());
        }
    }
    assert!(!f.root.join(".github").exists());
}

#[cfg(unix)]
#[test]
fn import_parameters_preserve_bindings_mappings_resources_and_reference_deduplication() {
    let f = Fixture::new();
    write(
        &f.remote,
        "repository/config.toml",
        "id = 'quality'\ndescription = 'Parameterized checks and reviews'\nrules = ['repository/checks/required-files', 'repository/checks/required-files', 'repository/reviews/parameter-review']\nrun_at = ['lint', 'test']\n",
    );
    let check_path = f.remote.join("repository/checks/required-files/rule.toml");
    let declarations = "\n[parameters.scopes_file]\ntype = 'file'\nrequired = true\n[parameters.label]\ntype = 'string'\nrequired = true\n[parameters.optional]\ntype = 'file'\n";
    fs::write(
        &check_path,
        format!(
            "{}{declarations}",
            fs::read_to_string(&check_path).unwrap().replace(
                "command = 'sh @rule/index.sh'",
                "command = 'sh @rule/index.sh @rule/resource @param/scopes_file @param/label'"
            )
        ),
    )
    .unwrap();
    write(
        &f.remote,
        "repository/checks/required-files/resource",
        "bundled\n",
    );
    write(
        &f.remote,
        "repository/checks/required-files/index.sh",
        r#"
test "$(cat "$1")" = bundled || exit 1
test "$(cat "$2")" = "$3" || exit 1
printf '%s:%s\n' "$3" "$2"
"#,
    );
    write(
        &f.remote,
        "repository/reviews/parameter-review/rule.toml",
        &format!(
            "id = 'parameter-review'\ndescription = 'Imported parameter review'\n[review]\nprompt = 'prompt.md'\n{declarations}"
        ),
    );
    write(
        &f.remote,
        "repository/reviews/parameter-review/prompt.md",
        "Inspect the configured scope file.",
    );
    commit(&f.remote);
    write(&f.root, "frontend/scopes.txt", "frontend\n");
    write(&f.root, "backend/scopes.txt", "backend\n");
    let binding = |label: &str| {
        format!(
            "[imports.execution_points]\nlint = 'verify'\ntest = 'verify'\n[imports.rule_parameters.required-files]\nlabel = {label:?}\nscopes_file = '{label}/scopes.txt'\n[imports.rule_parameters.parameter-review]\nlabel = {label:?}\nscopes_file = '{label}/scopes.txt'\n"
        )
    };
    mapped_imports(
        &f,
        &[
            ("repository", &binding("frontend")),
            ("repository", &binding("backend")),
        ],
        &["verify"],
    );
    let p = f.load(true).unwrap();
    let plan = planner::build(&p, Some("verify"), false).unwrap().plan;
    assert_eq!(plan.scheduled.len(), 4);
    assert_eq!(plan.planned_model_calls, 2);
    let ids: std::collections::BTreeSet<_> = plan.scheduled.iter().map(|r| &r.id).collect();
    assert_eq!(ids.len(), 4);
    assert!(
        plan.scheduled
            .iter()
            .all(|r| r.origin.as_ref().unwrap().external.is_some())
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&f.state()).unwrap()["resolutions"]
            .as_object()
            .unwrap()
            .len(),
        1
    );
    fs::rename(&f.remote, f.remote.with_extension("offline")).unwrap();
    struct ParameterReview;
    impl Provider for ParameterReview {
        fn review(&self, request: &Request<'_>) -> anyhow::Result<String> {
            let label = &request.parameters["label"];
            let path = &request.parameters["scopes_file"];
            assert!(Path::new(path).starts_with(&request.project.root));
            assert_eq!(fs::read_to_string(path).unwrap().trim(), label);
            Ok(serde_json::json!({"reviews":request.reviews.iter().map(|r| serde_json::json!({"id":r.id, "status":"PASS", "reason":"Scope file inspected", "findings":[{"message":"Scopes available", "path":format!("{label}/scopes.txt"), "line":1}]})).collect::<Vec<_>>()}).to_string())
        }
    }
    let report = please::verify_project_at(
        f.load(false).unwrap(),
        &ParameterReview,
        please::VerificationMode::AllRules,
        Some("verify"),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.overall, Status::Pass);
    assert_eq!(report.results.len(), 4);
    let outputs: Vec<_> = report
        .results
        .iter()
        .filter_map(|i| i.stdout.as_ref().map(|s| s.text.as_str()))
        .collect();
    assert!(outputs.iter().any(|o| o.starts_with("frontend:")));
    assert!(outputs.iter().any(|o| o.starts_with("backend:")));
    please::report::read(&f.root).unwrap();
}

#[test]
fn import_parameter_values_cannot_leak_to_other_imports_or_undeclared_names() {
    let f = Fixture::new();
    let path = f.remote.join("repository/checks/required-files/rule.toml");
    fs::write(
        &path,
        format!(
            "{}\n[parameters.label]\ntype = 'string'\nrequired = true\n",
            fs::read_to_string(&path).unwrap()
        ),
    )
    .unwrap();
    commit(&f.remote);
    mapped_imports(
        &f,
        &[
            (
                "repository",
                "[imports.rule_parameters.required-files]\nlabel = 'first'",
            ),
            ("repository", ""),
        ],
        &[],
    );
    let p = f.load(true).unwrap();
    let report = please::verify_project_at(
        p,
        &Never,
        please::VerificationMode::AllRules,
        Some("check"),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.overall, Status::Error);
    assert_eq!(
        report
            .results
            .iter()
            .filter(|r| r.status == Status::Pass)
            .count(),
        1
    );
    let error = report
        .results
        .iter()
        .find(|r| r.status == Status::Error)
        .unwrap();
    assert!(error.reason.contains("label") && error.reason.contains("missing"));
    mapped_imports(
        &f,
        &[(
            "repository",
            "[imports.rule_parameters.required-files]\nlabel = 'first'\nundeclared = 'secret'",
        )],
        &[],
    );
    let report = please::verify_project_at(
        f.load(false).unwrap(),
        &Never,
        please::VerificationMode::AllRules,
        Some("check"),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.overall, Status::Error);
    assert!(report.results[0].reason.contains("undeclared"));
    assert!(report.results[0].reason.contains("not declared"));
}

#[test]
fn shared_rules_are_reused_across_sets_and_points_offline() {
    let f = Fixture::new();
    fs::remove_dir_all(f.root.join(".please/checks")).unwrap();
    fs::remove_dir_all(f.root.join(".please/reviews")).unwrap();
    fs::create_dir_all(f.remote.join("rules/checks")).unwrap();
    fs::rename(
        f.remote.join("repository/checks/required-files"),
        f.remote.join("rules/checks/shared"),
    )
    .unwrap();
    write(
        &f.remote,
        "rules/checks/shared/resource.txt",
        "shared resource",
    );
    write(
        &f.remote,
        "rules/checks/shared/index.sh",
        "test -s README.md && test -s \"$(dirname \"$0\")/resource.txt\"\n",
    );
    write(
        &f.remote,
        "rules/reviews/shared/rule.toml",
        "id = 'documentation'\ndescription = 'Shared review'\n[review]\nprompt = 'prompt.md'\nfiles = ['README.md']\n",
    );
    write(
        &f.remote,
        "rules/reviews/shared/prompt.md",
        "Judge consumer documentation.\n",
    );
    // Unreferenced definitions are not part of either import.
    write(&f.remote, "rules/checks/unused/rule.toml", "invalid toml");
    for (set, point) in [("repository", "pre-push"), ("software", "check")] {
        write(
            &f.remote,
            &format!("{set}/config.toml"),
            &format!(
                "id = {set:?}\ndescription = 'Shared rules'\nrules = ['rules/checks/shared', 'rules/checks/shared', 'rules/reviews/shared']\nrun_at = [{point:?}]\n"
            ),
        );
    }
    commit(&f.remote);
    write(&f.root, "README.md", "consumer");
    f.imports(&[("repository", "main"), ("software", "main")]);
    let project = f.load(true).unwrap();
    let checks: Vec<_> = project
        .checks
        .iter()
        .filter(|r| r.id.starts_with("git:"))
        .collect();
    let reviews: Vec<_> = project
        .reviews
        .iter()
        .filter(|r| r.id.starts_with("git:"))
        .collect();
    assert_eq!(checks.len(), 2);
    assert_eq!(reviews.len(), 2);
    assert_eq!(*checks[0].command, *checks[1].command);
    assert_eq!(reviews[0].text, reviews[1].text);
    for point in ["pre-push", "check"] {
        let plan = planner::build(&project, Some(point), false).unwrap().plan;
        assert_eq!(plan.scheduled.len(), 2);
        assert!(
            plan.scheduled
                .iter()
                .any(|r| r.id.ends_with(":required-files"))
        );
    }
    fs::rename(&f.remote, f.remote.with_extension("offline")).unwrap();
    let report = please::verify_project_at(
        f.load(false).unwrap(),
        &ReviewMock,
        please::VerificationMode::AllRules,
        Some("check"),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.overall, Status::Pass);
    assert_eq!(report.results.len(), 2);
}

#[test]
fn external_rule_references_reject_unsafe_paths_and_duplicate_ids() {
    for reference in [
        "../checks/outside",
        "/checks/outside",
        "rules/checks/../../outside",
        "repository/config.toml",
        "repository/checks/required-files/rule.toml",
        "missing/checks/rule",
    ] {
        let f = Fixture::new();
        write(
            &f.remote,
            "repository/config.toml",
            &format!(
                "id = 'set'\ndescription = 'Unsafe reference'\nrules = [{reference:?}]\nrun_at = ['check']\n"
            ),
        );
        commit(&f.remote);
        f.imports(&[("repository", "main")]);
        assert!(f.load(true).is_err(), "accepted {reference}");
    }
    let f = Fixture::new();
    write(
        &f.remote,
        "repository/config.toml",
        "id = 'set'\ndescription = 'Duplicate IDs'\nrules = ['repository/checks/required-files', 'software/checks/required-files']\nrun_at = ['check']\n",
    );
    commit(&f.remote);
    f.imports(&[("repository", "main")]);
    let error = format!("{:#}", f.load(true).unwrap_err());
    assert!(error.contains("duplicate"), "{error}");
}

#[cfg(unix)]
#[test]
fn shared_rule_references_reject_symlink_components() {
    for component in [
        "rules",
        "rules/checks/shared",
        "rules/checks/shared/rule.toml",
    ] {
        let f = Fixture::new();
        write(
            &f.remote,
            "rules/checks/shared/rule.toml",
            "id = 'shared'\ndescription = 'Shared rule'\n[check]\ncommand = 'sh @rule/index.sh'\n",
        );
        write(
            &f.remote,
            "repository/config.toml",
            "id = 'set'\ndescription = 'Symlink'\nrules = ['rules/checks/shared']\nrun_at = ['check']\n",
        );
        let path = f.remote.join(component);
        let target = path.with_extension("original");
        fs::rename(&path, &target).unwrap();
        std::os::unix::fs::symlink(target.file_name().unwrap(), path).unwrap();
        commit(&f.remote);
        f.imports(&[("repository", "main")]);
        assert!(f.load(true).is_err(), "accepted symlink {component}");
    }
}

#[cfg(unix)]
#[test]
fn imported_and_local_checks_and_reviews_receive_the_consumer_execution_point() {
    let f = Fixture::new();
    write(
        &f.remote,
        "repository/config.toml",
        "id = 'quality'\ndescription = 'Context rules'\nrules = ['repository/checks/required-files', 'repository/reviews/context']\nrun_at = ['source-point']\n",
    );
    let script = r#"set -eu
point=$(python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["execution_point"], end="")' "$1")
test "$point" = verify
test "$2" = preserved
test "$(cat "$3")" = bundled
printf '%s' "$point"
"#;
    write(
        &f.remote,
        "repository/checks/required-files/index.sh",
        script,
    );
    write(
        &f.remote,
        "repository/checks/required-files/resource",
        "bundled\n",
    );
    let path = f.remote.join("repository/checks/required-files/rule.toml");
    fs::write(
        &path,
        format!(
            "{}\n[parameters.label]\ntype = 'string'\nrequired = true\n",
            fs::read_to_string(&path).unwrap().replace(
                "command = 'sh @rule/index.sh'",
                "command = 'sh @rule/index.sh @context @param/label @rule/resource'"
            )
        ),
    )
    .unwrap();
    write(
        &f.remote,
        "repository/reviews/context/rule.toml",
        "id = 'imported-review'\ndescription = 'Imported context'\n[review]\nprompt = 'prompt.md'\n",
    );
    write(
        &f.remote,
        "repository/reviews/context/prompt.md",
        "Inspect evidence.",
    );
    commit(&f.remote);
    mapped_imports(
        &f,
        &[(
            "repository",
            "[imports.execution_points]\nsource-point = 'verify'\n[imports.rule_parameters.required-files]\nlabel = 'preserved'",
        )],
        &["verify"],
    );
    support::check_args(
        &f.root,
        "local-context",
        &["verify"],
        "@context @param/label @rule/resource",
        script,
    );
    let path = f.root.join(".please/checks/local-context/rule.toml");
    fs::write(
        &path,
        format!(
            "{}\n[parameters.label]\ntype = 'string'\nvalue = 'preserved'\n",
            fs::read_to_string(&path).unwrap()
        ),
    )
    .unwrap();
    write(
        &f.root,
        ".please/checks/local-context/resource",
        "bundled\n",
    );
    write(
        &f.root,
        ".please/reviews/local-context/rule.toml",
        "id = 'local-review'\ndescription = 'Local context'\nrun_at = ['verify']\n[review]\nprompt = 'prompt.md'\n",
    );
    write(
        &f.root,
        ".please/reviews/local-context/prompt.md",
        "Inspect evidence.",
    );
    write(&f.root, "evidence", "Evidence\n");
    let setup = cli(&f, &["sync", "--yes"]);
    assert!(
        setup.status.success(),
        "{}",
        String::from_utf8_lossy(&setup.stderr)
    );
    fs::rename(&f.remote, f.remote.with_extension("offline")).unwrap();
    let fake = tempfile::tempdir().unwrap();
    support::context_provider(fake.path());
    let path = std::env::join_paths(
        std::iter::once(fake.path().to_path_buf())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let calls = fake.path().join("calls");
    let output = support::command(env!("CARGO_BIN_EXE_please"))
        .current_dir(&f.root)
        .args(["run", "--at", "verify"])
        .env("XDG_CACHE_HOME", &f.cache)
        .env("PATH", path)
        .env("PLEASE_EXECUTION_POINT", "source-point")
        .env("EXPECTED_EXECUTION_POINT", "verify")
        .env("CONTEXT_CALLS", &calls)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = please::report::read(&f.root).unwrap();
    assert_eq!(report.overall, Status::Pass);
    assert_eq!(report.results.len(), 4);
    for check in report.results.iter().filter(|r| r.kind == "check") {
        assert_eq!(check.stdout.as_ref().unwrap().text, "verify");
    }
    assert_eq!(fs::read_to_string(calls).unwrap(), "review\n");
}

#[test]
fn imported_names_survive_namespacing_and_saved_status_uses_short_unique_labels() {
    let f = Fixture::new();
    let path = f.remote.join("repository/checks/required-files/rule.toml");
    fs::write(
        &path,
        format!(
            "name = 'Required files'\n{}",
            fs::read_to_string(&path).unwrap()
        ),
    )
    .unwrap();
    commit(&f.remote);
    f.imports(&[
        ("repository", "main"),
        ("repository", "main"),
        ("software", "main"),
    ]);
    let project = f.load(true).unwrap();
    let mut progress = String::new();
    let report = please::verify_project_at(
        project,
        &Never,
        please::VerificationMode::AllRules,
        Some("check"),
        &mut |event| progress.push_str(&please::terminal::render_progress(&event, false)),
    )
    .unwrap();
    let names = report.plan.display_names();
    assert_eq!(names.len(), 3);
    assert!(
        names
            .values()
            .any(|name| name == "Required files [repository]")
    );
    assert!(
        names
            .values()
            .any(|name| name == "Required files [repository] #2")
    );
    assert!(names.values().any(|name| name == "required-files"));
    assert!(!progress.contains("git:"));
    let saved = please::report::read(&f.root).unwrap();
    assert!(!please::terminal::render(&saved, false).contains("git:"));
    assert!(please::terminal::render_details(&saved, false).contains("git:"));
    assert!(saved.results.iter().all(|item| item.id.starts_with("git:")));
}

#[cfg(unix)]
#[test]
fn shared_entrypoints_execute_each_rule_offline() {
    let f = Fixture::new();
    write(
        &f.remote,
        "repository/config.toml",
        "id = 'quality'\ndescription = 'Shared entrypoints'\nrules = ['rules/checks/pass', 'rules/checks/fail', 'rules/checks/error', 'rules/checks/missing', 'rules/checks/disabled']\nrun_at = ['check']\n",
    );
    write(
        &f.remote,
        "scripts/shared.sh",
        "test -s README.md || exit 2\ncat \"$1\"\nexit \"$2\"\n",
    );
    for (id, entrypoint, enabled) in [
        ("pass", "scripts/shared.sh", true),
        ("fail", "scripts/shared.sh", true),
        ("error", "scripts/shared.sh", true),
        ("missing", "scripts/missing.sh", true),
        ("disabled", "scripts/missing.sh", false),
    ] {
        write(
            &f.remote,
            &format!("rules/checks/{id}/rule.toml"),
            &format!(
                "id = '{id}'\ndescription = 'Shared check'\nenabled = {enabled}\n[parameters.code]\ntype = 'string'\nrequired = true\n[check]\ncommand = 'sh @source/{entrypoint} @rule/resource @param/code'\n"
            ),
        );
        write(&f.remote, &format!("rules/checks/{id}/resource"), id);
        write(
            &f.remote,
            &format!("rules/checks/{id}/{entrypoint}"),
            "exit 2",
        );
    }
    // Neither the selected set nor the declaring rule supplies the shared path.
    write(&f.remote, "repository/scripts/shared.sh", "exit 2");
    commit(&f.remote);
    write(&f.root, "README.md", "consumer\n");
    f.imports(&[("repository", "main")]);
    let mut configuration = fs::read_to_string(f.root.join(".please/config.toml")).unwrap();
    for (id, code) in [
        ("pass", "0"),
        ("fail", "1"),
        ("error", "2"),
        ("missing", "0"),
    ] {
        configuration += &format!("\n[imports.rule_parameters.{id}]\ncode = '{code}'\n");
    }
    write(&f.root, ".please/config.toml", &configuration);
    let p = f.load(true).unwrap();
    let shared: Vec<_> = p
        .checks
        .iter()
        .filter(|c| c.command.iter().any(|p| p == "@source/scripts/shared.sh"))
        .collect();
    assert_eq!(shared.len(), 3);
    assert!(shared.iter().all(|c| *c.command == *shared[0].command));
    fs::rename(&f.remote, f.remote.with_extension("offline")).unwrap();
    let p = f.load(false).unwrap();
    assert_eq!(
        planner::build(&p, Some("check"), false)
            .unwrap()
            .plan
            .scheduled
            .len(),
        4
    );
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
    for (label, status) in [
        ("pass", Status::Pass),
        ("fail", Status::Fail),
        ("error", Status::Error),
    ] {
        let result = report
            .results
            .iter()
            .find(|r| r.stdout.as_ref().is_some_and(|s| s.text == label))
            .unwrap();
        assert_eq!(result.status, status);
        assert!(result.blocking);
    }
    let missing = report.results.iter().find(|r| r.stdout.is_none()).unwrap();
    assert_eq!(missing.status, Status::Error);
    assert!(missing.reason.contains("scripts/missing.sh"));
    please::report::read(&f.root).unwrap();
}

#[test]
fn imported_shared_entrypoints_reject_unsafe_paths() {
    for entrypoint in [
        "../outside.sh",
        "scripts/../../outside.sh",
        "/tmp/outside.sh",
        "C:/outside.sh",
        "scripts\\outside.sh",
        "scripts/.",
    ] {
        let f = Fixture::new();
        write(
            &f.remote,
            "repository/checks/required-files/rule.toml",
            &format!(
                "id = 'required-files'\ndescription = 'Invalid path'\nenabled = false\n[check]\ncommand = \"sh @source/{entrypoint}\"\n"
            ),
        );
        fs::create_dir_all(f.remote.join("scripts")).unwrap();
        write(&f.remote, "scripts/data", "not an entrypoint");
        commit(&f.remote);
        f.imports(&[("repository", "main")]);
        assert!(f.load(true).is_err(), "accepted {entrypoint:?}");
    }
}

#[cfg(unix)]
#[test]
fn imported_shared_entrypoints_reject_symlink_escapes() {
    use std::os::unix::fs::symlink;
    for link_directory in [false, true] {
        let f = Fixture::new();
        let outside = tempfile::tempdir().unwrap();
        write(outside.path(), "shared.sh", "exit 0");
        write(
            &f.remote,
            "repository/checks/required-files/rule.toml",
            "id = 'required-files'\ndescription = 'Unsafe shared script'\n[check]\ncommand = 'sh @source/scripts/shared.sh'\n",
        );
        if link_directory {
            symlink(outside.path(), f.remote.join("scripts")).unwrap();
        } else {
            fs::create_dir(f.remote.join("scripts")).unwrap();
            symlink(
                outside.path().join("shared.sh"),
                f.remote.join("scripts/shared.sh"),
            )
            .unwrap();
        }
        commit(&f.remote);
        f.imports(&[("repository", "main")]);
        let p = f.load(true).unwrap();
        let check = p.checks.iter().find(|c| c.id.starts_with("git:")).unwrap();
        let result = please::checks::execute(&p, check);
        assert_eq!(result.status, Status::Error);
    }
}

#[test]
fn imported_commands_plan_and_execute_offline() {
    let command = "'git --version'";
    let f = Fixture::new();
    write(
        &f.remote,
        "repository/checks/required-files/rule.toml",
        &format!(
            "id = 'required-files'\ndescription = 'Native Git command'\n[check]\ncommand = {command}\n"
        ),
    );
    commit(&f.remote);
    f.imports(&[("repository", "main"), ("repository", "main")]);
    f.load(true).unwrap();
    fs::rename(&f.remote, f.remote.with_extension("offline")).unwrap();
    let p = f.load(false).unwrap();
    assert_eq!(
        planner::build(&p, Some("check"), false)
            .unwrap()
            .plan
            .scheduled
            .len(),
        2
    );
    let report = please::verify_project_at(
        p,
        &Never,
        please::VerificationMode::AllRules,
        Some("check"),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.overall, Status::Pass);
    assert_eq!(report.results.len(), 2);
    assert!(
        report
            .results
            .iter()
            .all(|r| r.stdout.as_ref().unwrap().text.starts_with("git version"))
    );
}

#[cfg(unix)]
#[test]
fn imported_path_references_use_cached_provenance_offline() {
    use std::os::unix::fs::PermissionsExt;
    for executable in ["sh @rule/index.sh", "'@source/shared scripts/probe.sh'"] {
        let f = Fixture::new();
        let script = "#!/bin/sh\ntest -s README.md || exit 2\ntest \"$PWD\" = \"$(dirname \"$1\")\" || exit 2\ncase \"$2\" in */repository/checks/required-files/index.sh) ;; *) exit 2;; esac\ncmp \"$2\" \"$3\" || exit 2\nprintf 'cached paths passed'\n";
        write(
            &f.remote,
            "repository/checks/required-files/index.sh",
            script,
        );
        write(&f.remote, "shared scripts/probe.sh", script);
        fs::set_permissions(
            f.remote.join("shared scripts/probe.sh"),
            fs::Permissions::from_mode(0o755),
        )
        .unwrap();
        write(&f.root, "README.md", "# Consumer\n");
        let command = format!(
            "{executable} @repo/README.md @rule/index.sh '@source/shared scripts/probe.sh'"
        );
        write(
            &f.remote,
            "repository/checks/required-files/rule.toml",
            &format!(
                "id = 'required-files'\ndescription = 'Reference probe'\n[check]\ncommand = {}\n",
                serde_json::to_string(&command).unwrap()
            ),
        );
        commit(&f.remote);
        f.imports(&[("repository", "main")]);
        f.load(true).unwrap();
        fs::rename(&f.remote, f.remote.with_extension("offline")).unwrap();
        // Consumer paths must never shadow imported rule/source resources.
        write(&f.root, "shared scripts/probe.sh", "exit 2");
        write(&f.root, "index.sh", "exit 2");
        let p = f.load(false).unwrap();
        let check = p
            .checks
            .iter()
            .find(|c| c.id.contains("required-files"))
            .unwrap();
        assert!(check.source_root.starts_with(&f.cache));
        assert!(planner::build(&p, Some("check"), false).is_ok());
        let result = please::checks::execute(&p, check);
        assert_eq!(
            result.status,
            Status::Pass,
            "{} {:?}",
            result.reason,
            result.stderr
        );
        assert_eq!(result.stdout.unwrap().text, "cached paths passed");
    }
}

#[test]
fn exclusive_resources_coordinate_local_and_imported_bindings() {
    let f = Fixture::new();
    for path in ["repository", "software"] {
        let rule = f
            .remote
            .join(format!("{path}/checks/required-files/rule.toml"));
        fs::write(
            &rule,
            format!(
                "{}resource = 'shared'\n",
                fs::read_to_string(&rule).unwrap()
            ),
        )
        .unwrap();
    }
    let revision = commit(&f.remote);
    f.imports(&[("repository", &revision), ("software", &revision)]);
    support::check(&f.root, "a-local", &["check"], "git --version");
    let rule = f.root.join(".please/checks/a-local/rule.toml");
    fs::write(
        &rule,
        format!(
            "{}resource = 'shared'\n",
            fs::read_to_string(&rule).unwrap()
        ),
    )
    .unwrap();
    let project = f.load(true).unwrap();
    assert_eq!(
        project
            .checks
            .iter()
            .filter(|c| c.resource.as_deref() == Some("shared"))
            .count(),
        3
    );
    let mut running = std::collections::BTreeSet::new();
    let report = please::verify_project_at(
        project,
        &Never,
        please::VerificationMode::AllRules,
        Some("check"),
        &mut |event| match event {
            please::Progress::RuleStarted { id, .. } => {
                assert!(
                    running.is_empty(),
                    "shared resource overlapped: {running:?}"
                );
                running.insert(id.to_owned());
            }
            please::Progress::RuleFinished { item, .. } => {
                assert!(running.remove(&item.id));
            }
            _ => {}
        },
    )
    .unwrap();
    assert_eq!(report.overall, Status::Pass);
    assert_eq!(report.results.len(), 3);
}

#[test]
fn reference_overrides_preserve_paths_mapping_identity_and_offline_reports() {
    let f = Fixture::new();
    write(
        &f.remote,
        "repository/config.toml",
        r#"
id = "quality"
description = "Reference overrides"
run_at = ["lint"]
rules = [
    { id = "repository/checks/required-files", run_at = ["test", "check"] },
    { id = "repository/checks/required-files", run_at = ["test"] },
    { id = "repository/reviews/documentation", run_at = ["test"] },
]
"#,
    );
    write(
        &f.remote,
        "repository/reviews/documentation/rule.toml",
        "id = 'documentation'\ndescription = 'Documentation'\n[review]\nprompt = 'prompt.md'\nfiles = ['README.md']\n",
    );
    write(
        &f.remote,
        "repository/reviews/documentation/prompt.md",
        "Judge consumer documentation.\n",
    );
    write(&f.root, "README.md", "# Consumer\n");
    commit(&f.remote);
    mapped_imports(
        &f,
        &[("repository", "[imports.execution_points]\ntest = 'verify'")],
        &["verify", "lint"],
    );
    let project = f.load(true).unwrap();
    let plan = planner::build(&project, Some("verify"), false)
        .unwrap()
        .plan;
    assert_eq!(plan.scheduled.len(), 2);
    assert!(
        planner::build(&project, Some("lint"), false)
            .unwrap()
            .plan
            .scheduled
            .is_empty()
    );
    let check_id = project
        .checks
        .iter()
        .find(|c| c.id.ends_with(":required-files"))
        .unwrap()
        .id
        .clone();
    assert_eq!(
        project.effective.rules[&check_id].execution_points,
        ["check".into(), "verify".into()].into()
    );
    let ids = plan.required_ids.clone();
    fs::rename(&f.remote, f.remote.with_extension("offline")).unwrap();
    let project = f.load(false).unwrap();
    assert_eq!(
        planner::build(&project, Some("verify"), false)
            .unwrap()
            .plan
            .required_ids,
        ids
    );
    let report = please::verify_project_at(
        project,
        &ReviewMock,
        please::VerificationMode::AllRules,
        Some("verify"),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(report.overall, Status::Pass);
    assert_eq!(report.results.len(), 2);
    let saved = please::report::read(&f.root).unwrap();
    assert_eq!(saved.plan.required_ids, ids);
    assert_eq!(
        saved
            .plan
            .scheduled
            .iter()
            .find(|r| r.id == check_id)
            .unwrap()
            .origin
            .as_ref()
            .unwrap()
            .execution_points,
        ["check".into(), "verify".into()].into()
    );
}
