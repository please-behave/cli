mod support;
use please::{
    adapter::{Adapter, Request},
    config, external, planner,
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
    // REQ-0053, REQ-0055: imported dependencies resolve within their import; an ambiguous short name is rejected.
    let f = Fixture::new();
    write(
        &f.remote,
        "repository/ruleset.toml",
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
        &format!("{path}/ruleset.toml"),
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
        support::commit_all(&root);
        support::use_head_as_change_base();
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
        support::trust(&self.remote);
        self.untrusted_imports(selections);
    }
    fn untrusted_imports(&self, selections: &[(&str, &str)]) {
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
// REQ-0359, REQ-0362: an import validates and preserves its optional label and version.
fn imported_rule_set_name_is_validated_and_preserved() {
    let f = Fixture::new();
    let path = "repository/ruleset.toml";
    let definition = fs::read_to_string(f.remote.join(path)).unwrap();
    let mut selected_id = None;
    f.imports(&[("repository", "main")]);
    for (field, name) in [
        ("", None),
        (
            "name = 'Repository checks'\nversion = '2.3'\n",
            Some("Repository checks"),
        ),
    ] {
        if name.is_some() {
            write(&f.remote, path, &format!("{definition}{field}"));
            commit(&f.remote);
        }
        let project = f.load(true).unwrap();
        let set = &project.rule_sets[0];
        assert_eq!(set.name.as_deref(), name);
        assert_eq!(
            set.version.as_ref().and_then(toml::Value::as_str),
            name.map(|_| "2.3")
        );
        assert!(set.id.ends_with(":quality"));
        let plan = planner::build(&project, Some("check"), false).unwrap().plan;
        assert_eq!(plan.required_ids.len(), 1);
        assert!(plan.required_ids[0].ends_with(":required-files"));
        assert_eq!(
            plan.scheduled[0].origin.as_ref().unwrap().rule_sets,
            [set.id.clone()].into()
        );
        let id = &plan.required_ids[0];
        assert_eq!(selected_id.get_or_insert_with(|| id.clone()), id);
    }
    write(&f.remote, path, &format!("{definition}name = 'invalid '\n"));
    commit(&f.remote);
    let error = f.load(true).unwrap_err();
    assert!(format!("{error:#}").contains(&format!("{path}: name must contain 1..=80 characters")));
    write(&f.remote, path, &format!("{definition}version = '2'\n"));
    commit(&f.remote);
    let error = f.load(true).unwrap_err();
    assert!(format!("{error:#}").contains(&format!("{path}: Rule Set \"quality\": version")));
}

#[test]
fn imported_rule_versions_come_from_the_source_definitions() {
    // REQ-0351: a resolved import preserves source versions in both rule kinds.
    let f = Fixture::new();
    let check_path = "repository/checks/required-files/rule.toml";
    let check = fs::read_to_string(f.remote.join(check_path)).unwrap();
    write(&f.remote, check_path, &format!("version = '1.10'\n{check}"));
    write(
        &f.remote,
        "repository/reviews/advisory/rule.toml",
        "id = 'versioned-review'\ndescription = 'Review'\nversion = '0.3'\nenabled = false\n[review]\nprompt = 'prompt.md'\n",
    );
    let set_path = "repository/ruleset.toml";
    let set = fs::read_to_string(f.remote.join(set_path)).unwrap();
    write(
        &f.remote,
        set_path,
        &set.replace(
            "'repository/checks/required-files'",
            "'repository/checks/required-files', 'repository/reviews/advisory'",
        ),
    );
    let revision = commit(&f.remote);
    let unversioned = git(&f.remote, &["rev-parse", "HEAD^"]);
    f.imports(&[("repository", &revision), ("software", &unversioned)]);

    for refresh in [true, false] {
        let project = f.load(refresh).unwrap();
        let check_version = |suffix: &str| {
            project
                .checks
                .iter()
                .find(|check| check.id.ends_with(suffix))
                .unwrap()
                .version
                .as_deref()
        };
        assert_eq!(check_version(":repository:1:required-files"), Some("1.10"));
        assert_eq!(check_version(":software:1:required-files"), None);
        let review = project
            .reviews
            .iter()
            .find(|review| review.id.ends_with(":versioned-review"))
            .unwrap();
        assert_eq!(review.version.as_deref(), Some("0.3"));
    }

    write(&f.remote, check_path, &format!("version = 1.10\n{check}"));
    let invalid = commit(&f.remote);
    f.imports(&[("repository", &invalid)]);
    let error = format!("{:#}", f.load(true).unwrap_err());
    assert!(error.contains(check_path), "{error}");
    assert!(error.contains("rule \"required-files\""), "{error}");
    assert!(error.contains("version must be a string"), "{error}");
}

#[test]
fn resolution_state_accepts_zero_and_one_without_refresh_or_fallback() {
    // REQ-0331: existing cached imports retain strict validation under either version label.
    let f = Fixture::new();
    f.imports(&[("repository", "main")]);
    f.load(true).unwrap();
    let original: serde_json::Value = serde_json::from_slice(&f.state()).unwrap();
    assert_eq!(original["version"], 0);
    let path = f.root.join(".git/please/external-rule-sets.json");
    for version in [0, 1] {
        let mut state = original.clone();
        state["version"] = serde_json::json!(version);
        let bytes = serde_json::to_vec(&state).unwrap();
        fs::write(&path, &bytes).unwrap();
        f.load(false).unwrap();
        assert_eq!(f.state(), bytes);
    }
    for version in [
        serde_json::json!(true),
        serde_json::json!(-1),
        serde_json::json!(2),
        serde_json::json!(1.0),
        serde_json::json!("0"),
        serde_json::Value::Null,
    ] {
        let mut state = original.clone();
        state["version"] = version;
        fs::write(&path, state.to_string()).unwrap();
        assert!(f.load(false).is_err());
    }
}

#[test]
// REQ-0291: changed resolutions and cache restoration report changes; unchanged refreshes report no-op.
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
impl Adapter for Never {
    fn review(&self, _: &Request<'_>) -> anyhow::Result<String> {
        panic!("unexpected adapter call")
    }
}

#[test]
fn refreshing_an_import_needs_no_update_confirmation_or_announcement() {
    // REQ-0198: a moving import refreshes noninteractively without update management output.
    let f = Fixture::new();
    // No hook changes are needed, so any confirmation would concern the import itself.
    support::check(&f.root, "always-ok", &["check"], "exit 0");
    f.imports(&[("repository", "main")]);
    let sync = || {
        let output = support::command(env!("CARGO_BIN_EXE_please"))
            .current_dir(&f.root)
            .env("XDG_CACHE_HOME", &f.cache)
            .env("NO_COLOR", "1")
            .args(["sync"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout, b"Synchronization completed successfully.\n");
        assert!(output.stderr.is_empty(), "{output:?}");
    };
    sync();
    let previous = f.state();
    write(&f.remote, "repository/new-resource", "updated import\n");
    let revision = commit(&f.remote);
    sync();
    assert_ne!(f.state(), previous);
    let project = external::load(&f.root, false, Some(&f.cache.join("please"))).unwrap();
    assert!(
        project
            .effective
            .rules
            .values()
            .filter_map(|r| r.external.as_ref())
            .all(|o| o.commit == revision)
    );
}

#[test]
fn branch_tag_and_full_commit_share_one_immutable_revision() {
    // REQ-0197: branches, annotated tags and full hashes resolve to their exact commit.
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
    // REQ-0202: an invalid refresh leaves the prior validated resolution state usable and unchanged.
    let f = Fixture::new();
    f.imports(&[("repository", "main")]);
    f.load(true).unwrap();
    let previous = f.state();
    write(
        &f.remote,
        "repository/ruleset.toml",
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
    // REQ-0207: absent resolution state, missing cached content and modified definitions block loading.
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
    // REQ-0197: an unavailable reference fails instead of falling back to another revision.
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
    // REQ-0188: consumer imports cannot override imported rule properties.
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
            "../../../software/ruleset.toml",
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
    // REQ-0203: concurrent refreshes of one project publish complete, equivalent rule state.
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
// REQ-0282: import synchronization runs neither imported rules nor source Git hooks.
fn retrieval_never_executes_rules_or_remote_hooks_and_errors_hide_credentials() {
    // REQ-0200: credential-bearing import URLs fail without echoing their secret.
    // REQ-0204: synchronization executes neither imported rule scripts nor source Git hooks.
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
impl Adapter for ReviewMock {
    fn review(&self, request: &Request<'_>) -> anyhow::Result<String> {
        assert_eq!(request.reviews.len(), 1);
        assert_eq!(request.reviews[0].text, "Judge consumer documentation.\n");
        assert!(request.project.root.join("README.md").is_file());
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
        "repository/ruleset.toml",
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
    assert_eq!(report.overall, Status::Error);
    assert_eq!(report.results[0].exit_code, Some(7));
}

#[test]
fn source_instruction_paths_cannot_escape_the_rule_directory() {
    // REQ-0194: referenced disabled reviews still validate their prompt paths.
    // REQ-0195: imported prompt paths reject absolute paths, parent traversal and symlink escapes.
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
        let central = f.remote.join("repository/ruleset.toml");
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

#[cfg(unix)]
fn assert_sync_error(output: std::process::Output, expected: &str) {
    assert_eq!(output.status.code(), Some(2));
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(diagnostic.contains(expected), "{diagnostic}");
    assert!(!String::from_utf8_lossy(&output.stdout).contains("successfully"));
}

#[cfg(unix)]
#[test]
fn import_count_limit_fails_synchronization() {
    // REQ-0205: reject excessive imports before retrieval or state publication.
    let f = Fixture::new();
    f.imports(&vec![("repository", "main"); please::limits::MAX_RULES + 1]);
    assert_sync_error(
        cli(&f, &["sync", "--yes"]),
        "at most 256 external Rule Set and adapter imports",
    );
    assert!(!f.root.join(".git/please/external-rule-sets.json").exists());
}

#[cfg(unix)]
#[test]
fn expanded_repository_size_limits_fail_synchronization() {
    // REQ-0205: tiny compressed Git objects must not bypass expanded file/total bounds.
    for total in [false, true] {
        let f = Fixture::new();
        f.imports(&[("repository", "main")]);
        assert!(cli(&f, &["sync", "--yes"]).status.success());
        let previous = f.state();
        let file_bytes = please::limits::MAX_SNAPSHOT_FILE_BYTES;
        let count = if total {
            please::limits::MAX_SNAPSHOT_BYTES / file_bytes + 1
        } else {
            1
        };
        for index in 0..count {
            let file = fs::File::create(f.remote.join(format!("expanded-{index}"))).unwrap();
            file.set_len(file_bytes + u64::from(!total)).unwrap();
        }
        commit(&f.remote);
        let expected = if total {
            "cached repository exceeds 134217728 bytes"
        } else {
            "cached file exceeds 8388608 bytes"
        };
        assert_sync_error(cli(&f, &["sync", "--yes"]), expected);
        assert_eq!(f.state(), previous);
    }
}

/// Delegate all real Git work except one operation, which blocks after a readiness
/// marker. Each test owns its PATH and children; no process-global environment edits.
#[cfg(unix)]
fn blocked_git(f: &Fixture, operation: &str) -> (TempDir, std::process::Command) {
    use std::os::unix::fs::PermissionsExt;
    let real_git = support::command("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    assert!(real_git.status.success());
    let bin = tempfile::tempdir().unwrap();
    let script = bin.path().join("git");
    fs::write(&script, "#!/bin/sh\nfor arg do\n  if [ \"$arg\" = \"$TEST_BLOCK_OPERATION\" ]; then\n    echo $$ > \"$TEST_GIT_READY\"\n    exec sleep 120\n  fi\ndone\nexec \"$TEST_REAL_GIT\" \"$@\"\n").unwrap();
    fs::set_permissions(script, fs::Permissions::from_mode(0o755)).unwrap();
    let path = std::env::join_paths(
        std::iter::once(bin.path().to_path_buf())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let mut command = support::command(env!("CARGO_BIN_EXE_please"));
    command
        .current_dir(&f.root)
        .args(["sync", "--yes"])
        .env("XDG_CACHE_HOME", &f.cache)
        .env("PATH", path)
        .env(
            "TEST_REAL_GIT",
            String::from_utf8(real_git.stdout).unwrap().trim(),
        )
        .env("TEST_BLOCK_OPERATION", operation)
        .env("TEST_GIT_READY", bin.path().join("ready"))
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    (bin, command)
}

/// Kill and reap even on assertion failures; the blocked Git has its own group.
#[cfg(unix)]
struct BlockedSync {
    child: std::process::Child,
    git_pid: Option<i32>,
}
#[cfg(unix)]
impl Drop for BlockedSync {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(pid) = self.git_pid {
            unsafe {
                libc::kill(-pid, libc::SIGKILL);
            }
        }
    }
}

#[cfg(unix)]
fn wait_for_git(command: &mut std::process::Command, bin: &Path) -> BlockedSync {
    let mut process = BlockedSync {
        child: command.spawn().unwrap(),
        git_pid: None,
    };
    let started = std::time::Instant::now();
    loop {
        if let Ok(text) = fs::read_to_string(bin.join("ready")) {
            if let Ok(pid) = text.trim().parse() {
                process.git_pid = Some(pid);
                return process;
            }
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(10),
            "Git readiness deadline exceeded"
        );
        assert!(
            process.child.try_wait().unwrap().is_none(),
            "sync exited before Git readiness"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[cfg(unix)]
fn retrieval_deadline(operation: &str, seconds: u64, expected: &str) {
    let f = Fixture::new();
    f.imports(&[("repository", "main")]);
    let (bin, mut command) = blocked_git(&f, operation);
    let started = std::time::Instant::now();
    let mut process = wait_for_git(&mut command, bin.path());
    let status = loop {
        if let Some(status) = process.child.try_wait().unwrap() {
            break status;
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(seconds + 15),
            "sync did not enforce Git deadline"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    use std::io::Read;
    let mut stderr = Vec::new();
    process
        .child
        .stderr
        .take()
        .unwrap()
        .read_to_end(&mut stderr)
        .unwrap();
    assert_eq!(status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&stderr).contains(expected),
        "{}",
        String::from_utf8_lossy(&stderr)
    );
    assert!(started.elapsed() >= std::time::Duration::from_secs(seconds));
    assert!(!f.root.join(".git/please/external-rule-sets.json").exists());
    drop(process);
    assert!(cli(&f, &["sync", "--yes"]).status.success());
}

#[cfg(unix)]
#[test]
fn retrieval_deadline_fails_synchronization() {
    // REQ-0205: a stalled remote Git operation fails at the published deadline.
    retrieval_deadline(
        "clone",
        please::limits::EXTERNAL_FETCH_TIMEOUT_SECONDS,
        "Git operation timed out",
    );
}

#[cfg(unix)]
#[test]
fn local_git_deadline_fails_synchronization() {
    // REQ-0205: materialization is also bounded, without any remote service.
    retrieval_deadline(
        "read-tree",
        please::limits::EXTERNAL_GIT_TIMEOUT_SECONDS,
        "cannot materialize Git cache revision",
    );
}

#[cfg(unix)]
fn locked_file(path: &Path) -> fs::File {
    use std::os::fd::AsRawFd;
    let file = fs::File::open(path).unwrap();
    assert_eq!(
        unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
        0
    );
    file
}

#[cfg(unix)]
#[test]
fn bounded_state_and_shared_cache_lock_waits_fail_synchronization() {
    // REQ-0205: both project and shared-cache contention fail rather than wait forever.
    let f = Fixture::new();
    f.imports(&[("repository", "main")]);
    assert!(cli(&f, &["sync", "--yes"]).status.success());
    let previous = f.state();
    let repositories = f.cache.join("please/git");
    let repository = fs::read_dir(repositories)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    for path in [
        f.root.join(".git/please/resolution.lock"),
        repository.join("cache.lock"),
    ] {
        let lock = locked_file(&path);
        let mut command = support::command(env!("CARGO_BIN_EXE_please"));
        command
            .current_dir(&f.root)
            .env("XDG_CACHE_HOME", &f.cache)
            .args(["sync", "--yes"]);
        let started = std::time::Instant::now();
        let output =
            please::process::run(&mut command, std::time::Duration::from_secs(45)).unwrap();
        assert!(!output.timed_out, "sync did not enforce lock deadline");
        assert_eq!(output.status.code(), Some(2));
        assert!(
            output
                .stderr
                .text
                .contains("external cache busy; retry please sync")
        );
        assert!(
            started.elapsed()
                >= std::time::Duration::from_secs(please::limits::EXTERNAL_LOCK_TIMEOUT_SECONDS)
        );
        assert_eq!(f.state(), previous);
        drop(lock);
    }
    assert!(cli(&f, &["sync", "--yes"]).status.success());
}

#[cfg(unix)]
#[test]
fn killed_sync_releases_project_and_shared_cache_locks() {
    // REQ-0230: SIGKILL bypasses destructors while both actual import locks are held.
    use std::os::fd::AsRawFd;
    let f = Fixture::new();
    f.imports(&[("repository", "main")]);
    let (bin, mut command) = blocked_git(&f, "clone");
    let process = wait_for_git(&mut command, bin.path());
    let repository = fs::read_dir(f.cache.join("please/git"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let paths = [
        f.root.join(".git/please/resolution.lock"),
        repository.join("cache.lock"),
    ];
    for path in &paths {
        let file = fs::File::open(path).unwrap();
        assert_eq!(
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            -1
        );
        assert_eq!(
            std::io::Error::last_os_error().kind(),
            std::io::ErrorKind::WouldBlock
        );
    }
    drop(process);
    for path in &paths {
        drop(locked_file(path));
    }
    assert!(cli(&f, &["sync", "--yes"]).status.success());
    external::load(&f.root, false, Some(&f.cache.join("please"))).unwrap();
}

#[test]
fn cli_refresh_cached_plan_and_offline_hook_keep_existing_lifecycle() {
    let f = Fixture::new();
    f.imports(&[("repository", "main")]);
    let source = f.remote.join("repository/ruleset.toml");
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
    assert_eq!(diagnostic["plan_schema_version"], 0);
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
    assert!(!f.root.join(".please/reports/1.json").exists());
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

#[cfg(unix)]
#[test]
fn nested_sync_rejects_before_refreshing_imports() {
    // REQ-0314: rejection preserves the import state and resolved revisions before any write.
    let f = Fixture::new();
    f.imports(&[("repository", "main")]);
    assert!(cli(&f, &["sync", "--yes"]).status.success());
    let previous = f.state();
    let original = git(&f.remote, &["rev-parse", "HEAD"]);
    write(&f.remote, "repository/note.md", "new revision");
    let updated = commit(&f.remote);
    assert_ne!(original, updated);
    for arguments in ["sync", "sync --yes"] {
        support::check(
            &f.root,
            "nested-sync",
            &["check"],
            &format!("exec {:?} {arguments}", env!("CARGO_BIN_EXE_please")),
        );
        let output = cli(&f, &["run", "--at", "check"]);
        assert_eq!(output.status.code(), Some(2));
        let report = please::report::read(&f.root).unwrap();
        let result = report
            .results
            .iter()
            .find(|r| r.id == "nested-sync")
            .unwrap();
        assert_eq!(result.status, Status::Error);
        assert!(
            result
                .stderr
                .as_ref()
                .unwrap()
                .text
                .contains("recursive Please Behave hook execution blocked")
        );
        assert_eq!(f.state(), previous);
    }
    assert!(cli(&f, &["sync", "--yes"]).status.success());
    let state: serde_json::Value = serde_json::from_slice(&f.state()).unwrap();
    assert_eq!(
        state["resolutions"]
            .as_object()
            .unwrap()
            .values()
            .next()
            .unwrap()["commit"],
        updated
    );
}

#[cfg(unix)]
#[test]
fn check_can_sync_imports_in_another_repository() {
    // REQ-0314: the guard only rejects synchronization of the active repository.
    let outer = Fixture::new();
    let other = Fixture::new();
    other.imports(&[("repository", "main")]);
    assert!(cli(&other, &["sync", "--yes"]).status.success());
    let previous = other.state();
    write(&other.remote, "repository/note.md", "new revision");
    let updated = commit(&other.remote);
    support::check(
        &outer.root,
        "other-sync",
        &["check"],
        &format!(
            "export XDG_CACHE_HOME={:?}\ncd {:?} && exec {:?} sync --yes",
            other.cache,
            other.root,
            env!("CARGO_BIN_EXE_please")
        ),
    );
    let output = cli(&outer, &["run", "--at", "check"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_ne!(other.state(), previous);
    let state: serde_json::Value = serde_json::from_slice(&other.state()).unwrap();
    assert_eq!(
        state["resolutions"]
            .as_object()
            .unwrap()
            .values()
            .next()
            .unwrap()["commit"],
        updated
    );
}

#[test]
fn all_fixed_hooks_execute_imported_rules_offline() {
    let f = Fixture::new();
    f.imports(&[("repository", "main")]);
    write(
        &f.remote,
        "repository/ruleset.toml",
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
    // REQ-0203: different projects can refresh concurrently against one revision cache.
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
    // REQ-0199: authentication uses the host SSH command with Git terminal prompting disabled.
    // REQ-0201: failed-import diagnostics do not echo secret-bearing Git helper output.
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    f.imports(&[("repository", "main")]);
    let path = f.root.join(".please/config.toml");
    let text = fs::read_to_string(&path).unwrap().replace(
        f.remote.to_str().unwrap(),
        "ssh://git@example.invalid/rules.git",
    );
    fs::write(path, text).unwrap();
    support::trust_location("ssh://git@example.invalid/rules.git");
    let ssh = f.root.with_file_name("fake-ssh");
    fs::write(
        &ssh,
        "#!/bin/sh\ntest \"$GIT_TERMINAL_PROMPT\" = 0 || exit 99\necho 'Permission denied: SECRET' >&2\nexit 1\n",
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
        support::trust_location(url);
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
    support::trust(&other);
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
    support::trust(&f.remote);
    for (path, mapping) in mappings {
        text += &format!(
            "[[imports]]\ngit = {:?}\npath = {path:?}\nref = 'main'\n{mapping}\n",
            f.remote.to_str().unwrap()
        );
    }
    write(&f.root, ".please/config.toml", &text);
}
fn source_points(f: &Fixture, path: &str, points: &[&str]) {
    let file = f.remote.join(path).join("ruleset.toml");
    let text = fs::read_to_string(&file)
        .unwrap()
        .replace("['check']", &format!("{points:?}"));
    fs::write(file, text).unwrap();
}

#[test]
fn independent_mappings_resolve_inheritance_many_to_one_and_offline_execution() {
    // REQ-0189: mappings are import-local, allow many-to-one targets and drive offline selection.
    // REQ-0191: changing mappings preserves cached source definitions, identities and resolution state.
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
    let central = f.remote.join("repository/ruleset.toml");
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
        fs::read_to_string(contents.join("repository/ruleset.toml"))
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
    // REQ-0189: invalid mapping names and unknown sources or targets fail with import context.
    // REQ-0190: built-in execution points cannot be either end of a mapping.
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
    // REQ-0189: unmapped imported custom points require a project declaration.
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
    // REQ-0164: the project supplies independent parameter values for each import binding.
    // REQ-0145: duplicate imported references execute one check and one review per binding.
    let f = Fixture::new();
    write(
        &f.remote,
        "repository/ruleset.toml",
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
    impl Adapter for ParameterReview {
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
    // REQ-0192: multiple rule sets share rule directories referenced from the repository root.
    // REQ-0187, REQ-0193, REQ-0206: separate bindings, referenced definitions only, offline execution.
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
        "id = 'documentation'\ndescription = 'Shared review'\n[review]\nprompt = 'prompt.md'\n",
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
            &format!("{set}/ruleset.toml"),
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
    // REQ-0181: distinct external definitions with identical IDs are rejected even when their contents match.
    // REQ-0195: external rule references reject absolute, traversing and non-directory paths.
    for reference in [
        "../checks/outside",
        "/checks/outside",
        "rules/checks/../../outside",
        "repository/ruleset.toml",
        "repository/checks/required-files/rule.toml",
        "missing/checks/rule",
    ] {
        let f = Fixture::new();
        write(
            &f.remote,
            "repository/ruleset.toml",
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
        "repository/ruleset.toml",
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
    // REQ-0195: external rule directories and definitions reject symbolic link components.
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
            "repository/ruleset.toml",
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
// REQ-0228: mapped imported reviews receive the consumer execution point.
fn imported_and_local_checks_and_reviews_receive_the_consumer_execution_point() {
    let f = Fixture::new();
    write(
        &f.remote,
        "repository/ruleset.toml",
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
    support::context_adapter(fake.path());
    let path = std::env::join_paths(
        std::iter::once(fake.path().to_path_buf())
            .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let calls = fake.path().join("calls");
    let output = support::command(env!("CARGO_BIN_EXE_please"))
        .current_dir(&f.root)
        .env("PLEASE_CHANGE_BASE", "HEAD")
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
    assert_eq!(fs::read_to_string(calls).unwrap(), "review\nreview\n");
}

#[test]
fn imported_names_survive_namespacing_and_saved_status_uses_short_unique_labels() {
    // REQ-0136: an imported rule without a display name uses its own short ID.
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
        "repository/ruleset.toml",
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
        "repository/ruleset.toml",
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
        "id = 'documentation'\ndescription = 'Documentation'\n[review]\nprompt = 'prompt.md'\n",
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

#[cfg(unix)]
#[test]
fn coordinated_display_renames_preserve_local_and_imported_execution() {
    // REQ-0137: labels cannot affect identity, selection, order, delivered parameters or adapters.
    let f = Fixture::new();
    fs::remove_dir_all(f.root.join(".please/checks")).unwrap();
    fs::remove_dir_all(f.root.join(".please/reviews")).unwrap();
    write(&f.root, "evidence", "changed evidence\n");
    write(
        &f.remote,
        "repository/ruleset.toml",
        "id = 'quality'\ndescription = 'Label fixture'\nrun_at = ['check']\nrules = ['repository/checks/z-check', 'repository/reviews/a-review', 'repository/checks/disabled']\n",
    );
    fs::remove_dir_all(f.remote.join("repository/checks/required-files")).unwrap();
    let definitions = |root: &Path, directory: &str, label: &str, value: &str, imported: bool| {
        let adapter = if imported {
            "imported-chosen"
        } else {
            "local-chosen"
        };
        write(
            root,
            &format!("{directory}/adapters/{adapter}.toml"),
            "command = 'fake-adapter'\ntimeout_seconds = 17\n[parameters]\nmodel = 'synthetic-model'\n",
        );
        let parameter = if imported {
            "required = true".to_owned()
        } else {
            format!("value = {value:?}")
        };
        for (kind, id, body) in [
            (
                "checks",
                "z-check",
                "[check]\ncommand = 'sh @rule/index.sh @param/label'\n",
            ),
            (
                "reviews",
                "a-review",
                "[review]\nadapter = 'chosen'\nprompt = 'prompt.md'\n",
            ),
        ] {
            write(
                root,
                &format!("{directory}/{kind}/{id}/rule.toml"),
                &format!(
                    "id = {id:?}\nname = {label:?}\ndescription = 'Label fixture'\nrun_at = ['check']\n{body}[parameters.label]\ntype = 'string'\n{parameter}\n"
                ).replace("adapter = 'chosen'", &format!("adapter = {adapter:?}")),
            );
        }
        write(
            root,
            &format!("{directory}/checks/disabled/rule.toml"),
            &format!(
                "id = 'disabled'\nname = {label:?}\ndescription = 'Unselected label fixture'\nenabled = false\nrun_at = ['check']\n[check]\ncommand = 'exit 1'\n"
            ),
        );
        write(
            root,
            &format!("{directory}/checks/z-check/index.sh"),
            "printf '%s' \"$1\"\n",
        );
        write(
            root,
            &format!("{directory}/reviews/a-review/prompt.md"),
            "Inspect evidence.\n",
        );
    };
    definitions(&f.root, ".please", "Zulu local", "local-value", false);
    definitions(
        &f.remote,
        "repository",
        "Alpha imported",
        "imported-value",
        true,
    );
    commit(&f.remote);
    mapped_imports(
        &f,
        &[(
            "repository",
            "[imports.rule_parameters.z-check]\nlabel = 'imported-value'\n[imports.rule_parameters.a-review]\nlabel = 'imported-value'",
        )],
        &[],
    );
    struct Inspect(std::sync::Mutex<Vec<(String, String, String)>>);
    impl Adapter for Inspect {
        fn review(&self, request: &Request<'_>) -> anyhow::Result<String> {
            let review = request.reviews[0];
            let expected = if review.id.starts_with("git:") {
                "imported-value"
            } else {
                "local-value"
            };
            assert_eq!(request.parameters["label"], expected);
            assert_eq!(
                request.adapter_profile()?.parameters["model"],
                "synthetic-model"
            );
            self.0.lock().unwrap().push((
                review.id.clone(),
                review.adapter.clone().unwrap(),
                request.parameters["label"].clone(),
            ));
            Ok(serde_json::json!({"reviews":[{"id":review.id,"status":"PASS","reason":"Evidence inspected","findings":[{"message":"Evidence", "path":"evidence", "line":1}]}]}).to_string())
        }
    }
    let run = || {
        let adapter = Inspect(std::sync::Mutex::new(Vec::new()));
        let report = please::verify_project_at(
            f.load(true).unwrap(),
            &adapter,
            please::VerificationMode::AllRules,
            Some("check"),
            &mut |_| {},
        )
        .unwrap();
        assert_eq!(report.overall, Status::Pass);
        assert_eq!(report.results.len(), 4);
        assert_eq!(report.plan.required_ids.len(), 4);
        assert_eq!(report.plan.disabled_rules.len(), 2);
        assert!(
            report
                .plan
                .disabled_rules
                .iter()
                .all(|r| r.id.ends_with("disabled"))
        );
        let selection: Vec<_> = report
            .plan
            .scheduled
            .iter()
            .map(|r| {
                (
                    r.id.clone(),
                    r.kind.clone(),
                    r.selected_by.clone(),
                    r.adapter.clone(),
                    serde_json::to_value(&r.adapter_profile).unwrap(),
                )
            })
            .collect();
        let outputs: Vec<_> = report
            .results
            .iter()
            .filter(|r| r.kind == "check")
            .map(|r| (r.id.clone(), r.stdout.as_ref().unwrap().text.clone()))
            .collect();
        assert_eq!(outputs.len(), 2);
        for (id, output) in &outputs {
            assert_eq!(
                output,
                if id.starts_with("git:") {
                    "imported-value"
                } else {
                    "local-value"
                }
            );
        }
        let mut calls = adapter.0.into_inner().unwrap();
        calls.sort();
        assert_eq!(calls.len(), 2);
        (report.plan.required_ids, selection, outputs, calls)
    };
    let before = run();
    // Reverse the labels' lexical order and give checks/reviews colliding names.
    definitions(&f.root, ".please", "Alpha renamed", "local-value", false);
    definitions(
        &f.remote,
        "repository",
        "Zulu renamed",
        "imported-value",
        true,
    );
    commit(&f.remote);
    assert_eq!(run(), before);
    let loaded = f.load(false).unwrap();
    let plan = planner::build(&loaded, Some("check"), false).unwrap().plan;
    for rule in &plan.scheduled {
        assert_eq!(
            rule.name.as_deref(),
            Some(if rule.id.starts_with("git:") {
                "Zulu renamed"
            } else {
                "Alpha renamed"
            })
        );
    }
}

#[test]
fn coordinated_project_and_worktree_resolutions_remain_independent() {
    // REQ-0229: refresh each distinct resolution in a shared cache without rewriting either neighbor.
    let f = Fixture::new();
    let mut revisions = Vec::new();
    for index in 0..4 {
        write(&f.remote, "repository/revision", &index.to_string());
        revisions.push(commit(&f.remote));
        git(&f.remote, &["branch", &format!("consumer-{index}")]);
    }
    f.imports(&[("repository", "consumer-0")]);
    commit(&f.root);
    let linked = f._directory.path().join("linked");
    git(
        &f.root,
        &[
            "worktree",
            "add",
            "--quiet",
            "-b",
            "linked",
            linked.to_str().unwrap(),
        ],
    );
    let other = f._directory.path().join("other");
    git(
        &f.root,
        &[
            "clone",
            "--quiet",
            "--no-local",
            f.root.to_str().unwrap(),
            other.to_str().unwrap(),
        ],
    );
    let roots = [&f.root, &linked, &other];
    let cache = f.cache.join("please");
    let state_paths: Vec<_> = roots
        .iter()
        .map(|root| {
            root.join(git(
                root,
                &["rev-parse", "--git-path", "please/external-rule-sets.json"],
            ))
        })
        .collect();
    let sync = |root: &Path| {
        let output = support::command(env!("CARGO_BIN_EXE_please"))
            .current_dir(root)
            .env("XDG_CACHE_HOME", &f.cache)
            .args(["sync", "--yes"])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    };
    for (index, root) in roots.iter().enumerate() {
        let path = root.join(".please/config.toml");
        fs::write(
            &path,
            fs::read_to_string(&path)
                .unwrap()
                .replace("consumer-0", &format!("consumer-{index}")),
        )
        .unwrap();
        sync(root);
        let loaded = external::load(root, false, Some(&cache)).unwrap();
        assert!(
            loaded
                .effective
                .rules
                .values()
                .filter_map(|r| r.external.as_ref())
                .map(|o| o.commit.as_str())
                .collect::<Vec<_>>()
                == [revisions[index].as_str()]
        );
    }
    let mut states: Vec<_> = state_paths.iter().map(|p| fs::read(p).unwrap()).collect();
    assert!(states[0] != states[1] && states[1] != states[2] && states[0] != states[2]);
    for (index, root) in roots.iter().enumerate() {
        git(
            &f.remote,
            &[
                "update-ref",
                &format!("refs/heads/consumer-{index}"),
                &revisions[3],
            ],
        );
        sync(root);
        for (neighbor, path) in state_paths.iter().enumerate() {
            let bytes = fs::read(path).unwrap();
            if neighbor == index {
                assert_ne!(bytes, states[neighbor]);
                states[neighbor] = bytes;
            } else {
                assert_eq!(bytes, states[neighbor]);
            }
        }
        let loaded = external::load(root, false, Some(&cache)).unwrap();
        assert!(
            loaded
                .effective
                .rules
                .values()
                .filter_map(|r| r.external.as_ref())
                .map(|o| o.commit.as_str())
                .collect::<Vec<_>>()
                == [revisions[3].as_str()]
        );
    }
}

#[cfg(unix)]
#[test]
fn coordinated_declined_terminal_approval_keeps_refreshed_imports() {
    // REQ-0294: sync persists a moving import before terminal hook approval, even when declined.
    use std::{
        io::{Read, Write},
        os::fd::FromRawFd,
    };
    let f = Fixture::new();
    f.imports(&[("repository", "main")]);
    assert!(cli(&f, &["sync", "--yes"]).status.success());
    let previous = f.state();
    write(&f.remote, "repository/revision", "refreshed\n");
    let revision = commit(&f.remote);
    support::check(&f.root, "needs-hook", &["pre-commit"], "exit 0");
    let hook = f.root.join(".git/hooks/pre-commit");
    let foreign = b"#!/bin/sh\n# Keep this user hook\n";
    fs::write(&hook, foreign).unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    let (mut master_fd, mut slave_fd) = (-1, -1);
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut master_fd,
                &mut slave_fd,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        },
        0
    );
    let mut master = unsafe { fs::File::from_raw_fd(master_fd) };
    let slave = unsafe { fs::File::from_raw_fd(slave_fd) };
    let mut child = support::command(env!("CARGO_BIN_EXE_please"))
        .current_dir(&f.root)
        .env("XDG_CACHE_HOME", &f.cache)
        .env("NO_COLOR", "1")
        .args(["sync"])
        .stdin(slave.try_clone().unwrap())
        .stdout(slave.try_clone().unwrap())
        .stderr(slave)
        .spawn()
        .unwrap();
    // Wait for the approval prompt before answering; prove refresh already persisted at that boundary.
    let state = f.root.join(".git/please/external-rule-sets.json");
    let reader = std::thread::spawn(move || {
        let mut transcript = Vec::new();
        let mut byte = [0];
        let mut answered = false;
        while matches!(master.read(&mut byte), Ok(1)) {
            transcript.push(byte[0]);
            if !answered && String::from_utf8_lossy(&transcript).contains("[y/N]") {
                let persisted = fs::read(&state).unwrap();
                assert_ne!(persisted, previous);
                assert!(String::from_utf8_lossy(&persisted).contains(&revision));
                master.write_all(b"n\n").unwrap();
                answered = true;
            }
        }
        assert!(answered, "{}", String::from_utf8_lossy(&transcript));
        transcript
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if std::time::Instant::now() > deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("terminal sync timed out: {:?}", reader.join().unwrap());
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let transcript = reader.join().unwrap();
    assert_eq!(
        status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&transcript)
    );
    assert_eq!(fs::read(hook).unwrap(), foreign);
    // Loading without refresh after making the source unavailable proves the resolution survived rejection.
    let expected = git(&f.remote, &["rev-parse", "HEAD"]);
    fs::rename(&f.remote, f.remote.with_extension("offline")).unwrap();
    let loaded = external::load(&f.root, false, Some(&f.cache.join("please"))).unwrap();
    let origins: Vec<_> = loaded
        .effective
        .rules
        .values()
        .filter_map(|r| r.external.as_ref())
        .collect();
    assert!(!origins.is_empty());
    assert!(origins.iter().all(|o| o.commit == expected));
}

#[cfg(unix)]
#[test]
fn coordinated_sync_preserves_governed_files_through_hook_lifecycle_and_failure() {
    // REQ-0298: import refresh and hook creation/update/removal/failure leave the governed tree intact.
    use std::{collections::BTreeMap, os::unix::fs::PermissionsExt};
    fn snapshot(root: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in fs::read_dir(directory).unwrap() {
            let path = entry.unwrap().path();
            let relative = path.strip_prefix(root).unwrap();
            if relative == Path::new(".git") || relative == Path::new(".please/reports") {
                continue;
            }
            if path.is_dir() {
                snapshot(root, &path, files);
            } else {
                files.insert(relative.to_path_buf(), fs::read(path).unwrap());
            }
        }
    }
    let f = Fixture::new();
    f.imports(&[("repository", "main")]);
    write(&f.root, "src/example.rs", "fn main() {}\n");
    write(&f.root, "README.md", "Governed documentation\n");
    write(&f.root, "untracked.bin", "\0untracked bytes\n");
    let sync = |success: bool| {
        let mut before = BTreeMap::new();
        snapshot(&f.root, &f.root, &mut before);
        let output = cli(&f, &["sync", "--yes"]);
        assert_eq!(output.status.success(), success, "{output:?}");
        let mut after = BTreeMap::new();
        snapshot(&f.root, &f.root, &mut after);
        assert_eq!(after, before);
        output
    };
    sync(true);
    let previous = f.state();
    write(&f.remote, "repository/revision", "advanced\n");
    let revision = commit(&f.remote);
    sync(true);
    assert_ne!(f.state(), previous);
    assert!(String::from_utf8_lossy(&f.state()).contains(&revision));
    support::check(&f.root, "needs-hook", &["pre-commit"], "exit 0");
    sync(true);
    let hook = f.root.join(".git/hooks/pre-commit");
    let original = fs::read_to_string(&hook).unwrap();
    assert!(original.contains("--at 'pre-commit'"));
    fs::write(&hook, original.replace("--at 'pre-commit'", "--at 'check'")).unwrap();
    let output = sync(true);
    assert!(String::from_utf8_lossy(&output.stderr).contains("Update hook"));
    assert_eq!(fs::read_to_string(&hook).unwrap(), original);
    support::check(&f.root, "needs-hook", &["check"], "exit 0");
    sync(true);
    assert!(!hook.exists());
    support::check(&f.root, "needs-hook", &["pre-commit"], "exit 0");
    let foreign = "#!/usr/bin/env python3\nprint('foreign hook')\n";
    fs::write(&hook, foreign).unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    let output = sync(false);
    assert!(String::from_utf8_lossy(&output.stderr).contains("hook"));
    assert_eq!(fs::read_to_string(hook).unwrap(), foreign);
}

/// Runs the executable with an empty user trust store of its own, outside the project.
fn please_with_own_trust(f: &Fixture, args: &[&str]) -> std::process::Output {
    support::command(env!("CARGO_BIN_EXE_please"))
        .args(args)
        .current_dir(&f.root)
        .env("XDG_CACHE_HOME", &f.cache)
        .env("XDG_CONFIG_HOME", f.root.parent().unwrap().join("config"))
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

#[test]
fn untrusted_repositories_block_sync_plan_and_run_before_retrieval() {
    // REQ-0334: no retrieval happens and the error names the repository and the trust command.
    // REQ-0337: the unattended confirmation option does not establish trust.
    let f = Fixture::new();
    f.untrusted_imports(&[("repository", "main")]);
    for args in [&["sync", "--yes"][..], &["run", "--plan"], &["run"]] {
        let output = please_with_own_trust(&f, args);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("is not trusted"), "{stderr}");
        assert!(
            stderr.contains(&format!("please trust {}", f.remote.display())),
            "{stderr}"
        );
        assert!(!f.cache.join("please").exists(), "{args:?} retrieved");
    }
    let listed = please_with_own_trust(&f, &["trust", "--list"]);
    assert_eq!(listed.status.code(), Some(0));
    assert!(listed.stdout.is_empty(), "{listed:?}");
}

#[test]
fn trust_commands_grant_and_revoke_trust_for_the_whole_repository() {
    // REQ-0334: revocation blocks execution without network access.
    // REQ-0335: the decision lives in the user store, not in the project.
    // REQ-0336: references, directories and later revisions need no further decision.
    // REQ-0337: only the explicit commands change trust.
    let f = Fixture::new();
    f.untrusted_imports(&[("repository", "main")]);
    let granted = please_with_own_trust(&f, &["trust", f.remote.to_str().unwrap()]);
    assert_eq!(granted.status.code(), Some(0), "{granted:?}");
    assert!(String::from_utf8_lossy(&granted.stdout).starts_with("Trusted "));
    let again = please_with_own_trust(&f, &["trust", f.remote.to_str().unwrap()]);
    assert!(String::from_utf8_lossy(&again.stdout).starts_with("Already trusted "));
    let store = f.root.parent().unwrap().join("config/please/trust.toml");
    assert!(store.is_file());
    assert!(!f.root.join(".please/trust.toml").exists());
    assert!(!f.root.join(".git/please/trust.toml").exists());

    let first = please_with_own_trust(&f, &["sync", "--yes"]);
    assert_eq!(first.status.code(), Some(0), "{first:?}");
    git(&f.remote, &["tag", "v1"]);
    write(&f.remote, "repository/new-resource", "later revision\n");
    commit(&f.remote);
    f.untrusted_imports(&[("repository", "main"), ("software", "v1")]);
    let output = please_with_own_trust(&f, &["sync", "--yes"]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let plan = please_with_own_trust(&f, &["run", "--plan"]);
    assert_eq!(plan.status.code(), Some(0), "{plan:?}");

    let revoked = please_with_own_trust(&f, &["trust", "--remove", f.remote.to_str().unwrap()]);
    assert_eq!(revoked.status.code(), Some(0), "{revoked:?}");
    for args in [&["run", "--plan"][..], &["run"], &["sync", "--yes"]] {
        let output = please_with_own_trust(&f, args);
        assert_eq!(output.status.code(), Some(2), "{args:?}: {output:?}");
        assert!(String::from_utf8_lossy(&output.stderr).contains("is not trusted"));
    }
    let listed = please_with_own_trust(&f, &["trust", "--list"]);
    assert!(listed.stdout.is_empty(), "{listed:?}");
}

#[test]
fn repositories_are_identified_by_lexically_normalized_location() {
    // REQ-0338: spelling variants coincide; other aliases and adapters' rules do not apply.
    use please::external::trust::identity;
    let https = identity("https://github.com/org/repo");
    for variant in [
        "HTTPS://GitHub.com/org/repo.git",
        "https://github.com:443/org/repo/",
        "https://github.com/org/repo.git/",
    ] {
        assert_eq!(identity(variant), https, "{variant}");
    }
    assert_ne!(identity("https://github.com/org/Repo"), https);
    assert_ne!(identity("git@github.com:org/repo.git"), https);
    assert_ne!(identity("ssh://git@github.com/org/repo"), https);
    assert_eq!(
        identity("git@GitHub.com:org/repo.git"),
        identity("git@github.com:org/repo")
    );
    let f = Fixture::new();
    let canonical = identity(f.remote.to_str().unwrap());
    assert_eq!(canonical, f.remote.to_str().unwrap());
    assert_eq!(identity(&format!("{canonical}/")), canonical);
    assert_eq!(identity(&format!("file://{canonical}")), canonical);
    #[cfg(unix)]
    {
        let alias = f.remote.with_file_name("alias");
        std::os::unix::fs::symlink(&f.remote, &alias).unwrap();
        assert_eq!(identity(alias.to_str().unwrap()), canonical);
    }
}
