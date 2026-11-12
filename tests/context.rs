//! CTX-001: the structured run context follows its JSON Schema in real Git scenarios.
#![cfg(unix)]
mod support;
use please::config;
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Output,
    sync::OnceLock,
};
use support::write;

const POINTS: [&str; 4] = ["pre-commit", "commit-msg", "pre-push", "check"];
const ZERO: &str = "0000000000000000000000000000000000000000";

fn validator() -> &'static jsonschema::Validator {
    static VALIDATOR: OnceLock<jsonschema::Validator> = OnceLock::new();
    VALIDATOR.get_or_init(|| {
        let schema: Value =
            serde_json::from_str(include_str!("../docs/schemas/context.schema.json")).unwrap();
        jsonschema::validator_for(&schema).unwrap()
    })
}
fn assert_valid(value: &Value) {
    let errors: Vec<String> = validator()
        .iter_errors(value)
        .map(|error| error.to_string())
        .collect();
    assert!(errors.is_empty(), "{errors:#?}\n{value:#}");
}

fn path_with_binary() -> std::ffi::OsString {
    std::env::join_paths(
        std::iter::once(
            Path::new(env!("CARGO_BIN_EXE_please"))
                .parent()
                .unwrap()
                .to_path_buf(),
        )
        .chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap()
}
fn git(root: &Path, args: &[&str]) -> Output {
    support::command("git")
        .args(args)
        .current_dir(root)
        .env("PATH", path_with_binary())
        .env_remove("PLEASE_CHANGE_BASE")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .output()
        .unwrap()
}
fn ok(output: Output) -> Output {
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}
fn text(output: Output) -> String {
    String::from_utf8(ok(output).stdout)
        .unwrap()
        .trim()
        .to_owned()
}
fn please(root: &Path, args: &[&str], environment: &[(&str, &str)]) -> Output {
    support::command(env!("CARGO_BIN_EXE_please"))
        .args(args)
        .current_dir(root)
        .env("NO_COLOR", "1")
        .env_remove("PLEASE_CHANGE_BASE")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .envs(environment.iter().copied())
        .output()
        .unwrap()
}

/// A committed project whose check fails unless the context file is present.
fn project(settings: &str) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    ok(git(
        root.path(),
        &["init", "--quiet", "--initial-branch=main"],
    ));
    config::init(root.path()).unwrap();
    write(
        root.path(),
        ".please/config.toml",
        &format!("project = 'My_App'\n{settings}"),
    );
    support::check_args(
        root.path(),
        "capture",
        &POINTS,
        "@context",
        "test -s \"$1\" || exit 2\n",
    );
    write(root.path(), "a.txt", "one\n");
    write(root.path(), "b.txt", "bee\n");
    write(root.path(), "d.txt", "dee\n");
    commit(root.path(), "test: initial");
    root
}
fn commit(root: &Path, message: &str) {
    ok(git(root, &["add", "-A"]));
    ok(git(root, &["commit", "--quiet", "-m", message]));
}
fn hooks(root: &Path) {
    ok(please(root, &["sync", "--yes"], &[]));
}

/// Every stored context, oldest first, each validated against the published schema.
fn contexts(root: &Path) -> Vec<(PathBuf, Value)> {
    // Contexts live in the worktree-local Git directory, also for linked worktrees.
    let base = PathBuf::from(text(git(
        root,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "please/context",
        ],
    )));
    let mut directories: Vec<_> = fs::read_dir(&base)
        .map(|entries| entries.map(|entry| entry.unwrap().path()).collect())
        .unwrap_or_default();
    directories.sort();
    directories
        .into_iter()
        .map(|directory| {
            let value: Value =
                serde_json::from_slice(&fs::read(directory.join("context.json")).unwrap()).unwrap();
            assert_valid(&value);
            (directory, value)
        })
        .collect()
}
fn latest(root: &Path, point: &str) -> (PathBuf, Value) {
    contexts(root)
        .into_iter()
        .rfind(|(_, value)| value["execution_point"] == point)
        .unwrap_or_else(|| panic!("no context for {point}"))
}
fn run_check(root: &Path) -> (PathBuf, Value) {
    please(root, &["run", "--at", "check"], &[]);
    latest(root, "check")
}
fn patch(directory: &Path, section: &Value) -> String {
    let file = section["file"].as_str().expect("patch file");
    fs::read_to_string(directory.join(file)).unwrap()
}
fn strings(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item.as_str().or_else(|| item["path"].as_str()).unwrap())
        .collect()
}
fn find<'a>(entries: &'a Value, path: &str) -> &'a Value {
    entries
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["path"] == path)
        .unwrap_or_else(|| panic!("{path} not listed in {entries}"))
}

#[test]
fn schema_is_strict_and_requires_the_stable_sections() {
    let root = project("");
    let (_, context) = run_check(root.path());
    for mutation in [
        |value: &mut Value| {
            value.as_object_mut().unwrap().remove("run_id");
        },
        |value: &mut Value| {
            value["unexpected"] = Value::Bool(true);
        },
        |value: &mut Value| {
            value["schema_version"] = Value::from(2);
        },
        |value: &mut Value| {
            value["workspace"]["staged"] = Value::from("none");
        },
    ] {
        let mut changed = context.clone();
        mutation(&mut changed);
        assert!(!validator().is_valid(&changed), "{changed:#}");
    }
    for section in ["repository", "head", "workspace"] {
        assert!(context[section].is_object());
    }
    assert!(context.get("push").is_none() && context.get("commit_message").is_none());
}

#[test]
fn unborn_repository_has_no_head_and_a_dirty_workspace() {
    let root = tempfile::tempdir().unwrap();
    ok(git(
        root.path(),
        &["init", "--quiet", "--initial-branch=main"],
    ));
    config::init(root.path()).unwrap();
    support::check_args(
        root.path(),
        "capture",
        &POINTS,
        "@context",
        "test -s \"$1\"\n",
    );
    let (_, context) = run_check(root.path());
    assert!(context["head"]["commit"].is_null() && context["head"]["message"].is_null());
    assert_eq!(context["head"]["branch"], "main");
    assert_eq!(context["workspace"]["clean"], false);
    assert!(strings(&context["workspace"]["untracked"]).contains(&".please/config.toml"));
}

#[test]
fn clean_workspace_reports_head_branch_and_message() {
    let root = project("");
    let (directory, context) = run_check(root.path());
    assert_eq!(context["workspace"]["clean"], true);
    for list in ["staged", "unstaged", "untracked", "conflicted"] {
        assert!(context["workspace"][list].as_array().unwrap().is_empty());
    }
    assert_eq!(
        context["head"]["commit"],
        text(git(root.path(), &["rev-parse", "HEAD"]))
    );
    assert_eq!(context["head"]["message"], "test: initial");
    assert_eq!(context["repository"]["linked_worktree"], false);
    assert_eq!(context["repository"]["shallow"], false);
    assert!(context["workspace"]["staged_patch"]["file"].is_null());
    assert!(directory.join("context.json").is_file());
}

#[test]
fn staged_unstaged_untracked_and_partially_staged_files_are_separated() {
    let root = project("");
    let path = root.path();
    write(path, "a.txt", "two\n");
    ok(git(path, &["add", "a.txt"]));
    write(path, "a.txt", "three\n");
    ok(git(path, &["mv", "b.txt", "c.txt"]));
    fs::remove_file(path.join("d.txt")).unwrap();
    write(path, "u.txt", "new\n");
    let (directory, context) = run_check(path);
    let workspace = &context["workspace"];
    assert_eq!(workspace["clean"], false);
    assert_eq!(find(&workspace["staged"], "a.txt")["status"], "modified");
    let renamed = find(&workspace["staged"], "c.txt");
    assert_eq!(
        (renamed["status"].as_str(), renamed["old_path"].as_str()),
        (Some("renamed"), Some("b.txt"))
    );
    assert_eq!(find(&workspace["unstaged"], "a.txt")["status"], "modified");
    assert_eq!(find(&workspace["unstaged"], "d.txt")["status"], "deleted");
    assert_eq!(strings(&workspace["untracked"]), ["u.txt"]);
    assert!(patch(&directory, &workspace["staged_patch"]).contains("+two"));
    assert!(patch(&directory, &workspace["unstaged_patch"]).contains("+three"));
}

#[test]
fn merge_conflicts_are_listed_separately() {
    let root = project("");
    let path = root.path();
    ok(git(path, &["checkout", "--quiet", "-b", "other"]));
    write(path, "a.txt", "other\n");
    commit(path, "test: other");
    ok(git(path, &["checkout", "--quiet", "main"]));
    write(path, "a.txt", "main\n");
    commit(path, "test: main");
    assert!(!git(path, &["merge", "other"]).status.success());
    let (_, context) = run_check(path);
    assert_eq!(strings(&context["workspace"]["conflicted"]), ["a.txt"]);
    assert_eq!(context["workspace"]["clean"], false);
}

#[test]
fn unusual_paths_are_listed_verbatim() {
    let root = project("");
    let path = root.path();
    write(path, "with space/ünïcode file.txt", "x\n");
    let (_, context) = run_check(path);
    assert!(strings(&context["workspace"]["untracked"]).contains(&"with space/ünïcode file.txt"));
    assert_eq!(context["workspace"]["unrepresentable_paths"], 0);
}

#[test]
fn detached_head_linked_worktree_and_shallow_clone_are_reported() {
    let root = project("");
    let path = root.path();
    let worktree = tempfile::tempdir().unwrap();
    let linked = worktree.path().join("linked");
    ok(git(
        path,
        &[
            "worktree",
            "add",
            "--quiet",
            linked.to_str().unwrap(),
            "-b",
            "side",
        ],
    ));
    let (_, context) = run_check(&linked);
    assert_eq!(context["repository"]["linked_worktree"], true);
    assert_eq!(context["head"]["branch"], "side");
    ok(git(path, &["checkout", "--quiet", "--detach"]));
    assert!(run_check(path).1["head"]["branch"].is_null());
    let clone = tempfile::tempdir().unwrap();
    let target = clone.path().join("shallow");
    ok(git(
        clone.path(),
        &[
            "clone",
            "--quiet",
            "--depth",
            "1",
            &format!("file://{}", path.display()),
            target.to_str().unwrap(),
        ],
    ));
    assert_eq!(run_check(&target).1["repository"]["shallow"], true);
}

#[test]
fn commit_message_and_staged_files_reach_commit_msg() {
    let root = project("");
    let path = root.path();
    hooks(path);
    write(path, "a.txt", "two\n");
    ok(git(path, &["add", "a.txt"]));
    ok(git(
        path,
        &["commit", "--quiet", "-m", "test: update a\n\nBody line"],
    ));
    let (_, message) = latest(path, "commit-msg");
    assert!(
        message["commit_message"]["text"]
            .as_str()
            .unwrap()
            .contains("Body line")
    );
    assert!(
        message["commit_message"]["file"]
            .as_str()
            .unwrap()
            .ends_with("COMMIT_EDITMSG")
    );
    assert_eq!(
        find(&message["workspace"]["staged"], "a.txt")["status"],
        "modified"
    );
    assert_eq!(message["head"]["message"], "test: initial");
    let (_, earlier) = latest(path, "pre-commit");
    assert!(earlier.get("commit_message").is_none());
    assert_eq!(
        find(&earlier["workspace"]["staged"], "a.txt")["status"],
        "modified"
    );
}

fn remote(path: &Path) -> tempfile::TempDir {
    let bare = tempfile::tempdir().unwrap();
    ok(git(bare.path(), &["init", "--bare", "--quiet"]));
    ok(git(
        path,
        &["remote", "add", "origin", bare.path().to_str().unwrap()],
    ));
    bare
}

#[test]
fn push_updates_describe_create_update_delete_and_unpushed_heads() {
    let root = project("");
    let path = root.path();
    let _bare = remote(path);
    hooks(path);
    let first = text(git(path, &["rev-parse", "HEAD"]));
    ok(git(
        path,
        &["push", "--quiet", "origin", "HEAD:refs/heads/main"],
    ));
    let (directory, context) = latest(path, "pre-push");
    let update = &context["push"]["updates"][0];
    assert_eq!(context["push"]["remote"], "origin");
    assert_eq!(
        (update["kind"].as_str(), update["remote_sha"].is_null()),
        (Some("create"), true)
    );
    assert_eq!(update["is_head"], true);
    assert_eq!(strings(&update["commits"]), [first.as_str()]);
    assert!(strings(&update["changed_files"]).contains(&"a.txt"));
    assert!(patch(&directory, &update["patch"]).contains("a.txt"));

    write(path, "a.txt", "two\n");
    commit(path, "test: second");
    let second = text(git(path, &["rev-parse", "HEAD"]));
    ok(git(
        path,
        &["push", "--quiet", "origin", "HEAD:refs/heads/main"],
    ));
    let (directory, context) = latest(path, "pre-push");
    let update = &context["push"]["updates"][0];
    assert_eq!(update["kind"], "update");
    assert_eq!(update["remote_sha"], first.as_str());
    assert_eq!(strings(&update["commits"]), [second.as_str()]);
    assert_eq!(
        find(&update["changed_files"], "a.txt")["status"],
        "modified"
    );
    assert!(patch(&directory, &update["patch"]).contains("+two"));

    ok(git(
        path,
        &["push", "--quiet", "origin", "HEAD~1:refs/heads/older"],
    ));
    let (_, context) = latest(path, "pre-push");
    let update = &context["push"]["updates"][0];
    assert_eq!(
        (update["kind"].as_str(), update["is_head"].as_bool()),
        (Some("create"), Some(false))
    );
    assert!(update["commits"].as_array().unwrap().is_empty());

    ok(git(
        path,
        &["push", "--quiet", "origin", ":refs/heads/older"],
    ));
    let (_, context) = latest(path, "pre-push");
    let update = &context["push"]["updates"][0];
    assert_eq!(update["kind"], "delete");
    assert_eq!(update["local_sha"], ZERO);
    assert_eq!(update["range_available"], false);
}

#[test]
fn several_refs_and_tags_in_one_push_are_listed_with_their_own_patches() {
    let root = project("");
    let path = root.path();
    let _bare = remote(path);
    hooks(path);
    write(path, "a.txt", "two\n");
    commit(path, "test: second");
    ok(git(path, &["tag", "v1"]));
    ok(git(
        path,
        &[
            "push",
            "--quiet",
            "origin",
            "HEAD:refs/heads/one",
            "HEAD~1:refs/heads/two",
            "refs/tags/v1",
        ],
    ));
    let (_, context) = latest(path, "pre-push");
    let updates = context["push"]["updates"].as_array().unwrap();
    assert_eq!(updates.len(), 3);
    let refs: Vec<_> = updates
        .iter()
        .map(|u| u["remote_ref"].as_str().unwrap())
        .collect();
    assert!(
        refs.contains(&"refs/tags/v1") && refs.contains(&"refs/heads/one"),
        "{refs:?}"
    );
    let files: std::collections::BTreeSet<_> = updates
        .iter()
        .filter_map(|update| update["patch"]["file"].as_str())
        .collect();
    assert_eq!(
        files.len(),
        updates
            .iter()
            .filter(|u| u["patch"]["file"].is_string())
            .count()
    );
}

#[test]
fn dirty_workspace_is_visible_at_pre_push() {
    let root = project("");
    let path = root.path();
    let _bare = remote(path);
    hooks(path);
    write(path, "a.txt", "changed after the last commit\n");
    ok(git(
        path,
        &["push", "--quiet", "origin", "HEAD:refs/heads/main"],
    ));
    let (_, context) = latest(path, "pre-push");
    assert_eq!(context["workspace"]["clean"], false);
    assert_eq!(
        find(&context["workspace"]["unstaged"], "a.txt")["status"],
        "modified"
    );
    assert_eq!(context["push"]["updates"][0]["is_head"], true);
}

#[test]
fn explicit_change_base_adds_net_changes_since_that_commit() {
    let root = project("");
    let path = root.path();
    let base = text(git(path, &["rev-parse", "HEAD"]));
    write(path, "a.txt", "two\n");
    commit(path, "test: second");
    write(path, "u.txt", "new\n");
    please(
        path,
        &["run", "--at", "check"],
        &[("PLEASE_CHANGE_BASE", &base)],
    );
    let (directory, context) = latest(path, "check");
    let changes = &context["changes"];
    assert_eq!(changes["base"], base.as_str());
    assert_eq!(find(&changes["files"], "a.txt")["status"], "modified");
    assert_eq!(find(&changes["files"], "u.txt")["status"], "untracked");
    assert!(patch(&directory, &changes["patch"]).contains("+two"));
    assert!(run_check(path).1.get("changes").is_none());
}

#[test]
fn oversized_sections_are_marked_truncated_instead_of_failing_the_run() {
    let root = project("[context]\nmax_section_bytes = 1024\n");
    let path = root.path();
    write(path, "a.txt", &"long line of text\n".repeat(400));
    commit(path, &format!("test: long\n\n{}", "x".repeat(3000)));
    write(path, "a.txt", &"another long line\n".repeat(400));
    for index in 0..80 {
        write(
            path,
            &format!("generated/file-{index:03}-with-a-long-name.txt"),
            "x\n",
        );
    }
    let (_, context) = run_check(path);
    assert_eq!(context["head"]["message_truncated"], true);
    assert!(context["head"]["message"].as_str().unwrap().len() <= 1024);
    assert_eq!(context["workspace"]["truncated"], true);
    assert_eq!(context["workspace"]["clean"], false);
    assert_eq!(context["workspace"]["unstaged_patch"]["truncated"], true);
    assert!(context["workspace"]["unstaged_patch"]["file"].is_null());
}

#[test]
fn only_the_configured_number_of_runs_is_retained() {
    let root = project("[context]\nretain_runs = 3\n");
    for _ in 0..6 {
        ok(please(root.path(), &["run", "--at", "check"], &[]));
    }
    let remaining = contexts(root.path());
    assert_eq!(remaining.len(), 3);
    let ids: Vec<_> = remaining
        .iter()
        .map(|(_, value)| value["run_id"].as_str().unwrap().to_owned())
        .collect();
    let mut sorted = ids.clone();
    sorted.sort();
    assert_eq!(ids, sorted);
}

#[test]
fn context_settings_are_validated() {
    for (setting, expected) in [
        ("retain_runs = 0", "context.retain_runs"),
        ("retain_runs = 10001", "context.retain_runs"),
        ("max_section_bytes = 1023", "context.max_section_bytes"),
    ] {
        let root = project(&format!("[context]\n{setting}\n"));
        let output = please(root.path(), &["run", "--at", "check"], &[]);
        assert_eq!(output.status.code(), Some(2));
        assert!(String::from_utf8_lossy(&output.stderr).contains(expected));
    }
}

#[test]
fn an_unusable_change_base_omits_the_section_instead_of_failing_the_run() {
    let root = project("");
    for base in [
        "0123456789abcdef0123456789abcdef01234567",
        "no-such-ref",
        "",
    ] {
        let output = please(
            root.path(),
            &["run", "--at", "check"],
            &[("PLEASE_CHANGE_BASE", base)],
        );
        assert_eq!(output.status.code(), Some(0), "{base:?}");
    }
    let (_, context) = latest(root.path(), "check");
    assert!(context.get("changes").is_none());
}
