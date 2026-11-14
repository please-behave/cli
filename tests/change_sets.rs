#![cfg(unix)]
mod support;
use please::{
    change_set::{self, State},
    config, git_snapshot, planner,
    review_input::PreparedReviewInput,
};
use std::{fs, path::Path};
use support::write;

fn git(root: &Path, args: &[&str]) -> String {
    let out = support::command("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap().trim().into()
}
fn fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "-q", "--initial-branch=main"]);
    please::init::init(root.path()).unwrap();
    support::configure_adapter(root.path());
    fs::remove_dir_all(root.path().join(".please/checks")).unwrap();
    fs::remove_dir_all(root.path().join(".please/reviews")).unwrap();
    root
}
fn review(root: &Path, id: &str, point: &str, extra: &str) {
    write(
        root,
        &format!(".please/reviews/{id}/rule.toml"),
        &format!(
            "id = {id:?}\ndescription = 'Review'\nrun_at = [{point:?}]\n[review]\nprompt = 'prompt.md'\n{extra}"
        ),
    );
    write(root, &format!(".please/reviews/{id}/prompt.md"), "Judge\n");
}
fn files(set: &change_set::ChangeSet) -> Vec<(&str, State)> {
    set.files
        .iter()
        .map(|c| (c.path.as_str(), c.state))
        .collect()
}

#[test]
fn pre_commit_changes_are_the_staged_ones() {
    let root = fixture();
    write(root.path(), "kept", "one\n");
    write(root.path(), "gone", "two\n");
    support::commit_all(root.path());
    write(root.path(), "kept", "changed\n");
    write(root.path(), "added", "new\n");
    fs::remove_file(root.path().join("gone")).unwrap();
    write(root.path(), "unstaged", "never staged\n");
    git(root.path(), &["add", "kept", "added", "gone"]);
    for point in ["pre-commit", "commit-msg"] {
        let set = change_set::collect(root.path(), point, None).unwrap();
        assert_eq!(
            files(&set),
            [
                ("added", State::Added),
                ("gone", State::Deleted),
                ("kept", State::Modified)
            ]
        );
    }
    let set = change_set::collect(root.path(), "pre-commit", None).unwrap();
    let patch = change_set::patch(root.path(), &set.source, &["kept".into()]).unwrap();
    assert!(patch.contains("+changed") && !patch.contains("new"));
}

#[test]
fn a_direct_pre_push_run_takes_the_commits_not_on_any_remote() {
    let root = fixture();
    // An unborn branch has nothing to push.
    assert!(
        change_set::collect(root.path(), "pre-push", None)
            .unwrap()
            .files
            .is_empty()
    );
    write(root.path(), "pushed", "one\n");
    support::commit_all(root.path());
    let pushed = git(root.path(), &["rev-parse", "HEAD"]);
    git(
        root.path(),
        &["update-ref", "refs/remotes/origin/main", &pushed],
    );
    git(
        root.path(),
        &["remote", "add", "origin", "https://example.invalid/x.git"],
    );
    write(root.path(), "local", "two\n");
    support::commit_all(root.path());
    write(root.path(), "uncommitted", "three\n");
    let set = change_set::collect(root.path(), "pre-push", None).unwrap();
    assert_eq!(files(&set), [("local", State::Added)]);
    // A remote that has everything leaves nothing to review.
    let head = git(root.path(), &["rev-parse", "HEAD"]);
    git(
        root.path(),
        &["update-ref", "refs/remotes/origin/main", &head],
    );
    assert!(
        change_set::collect(root.path(), "pre-push", None)
            .unwrap()
            .files
            .is_empty()
    );
}

#[test]
fn reviews_without_changes_are_omitted_and_triggers_limit_the_scope() {
    let root = fixture();
    review(root.path(), "all", "pre-commit", "");
    review(
        root.path(),
        "docs",
        "pre-commit",
        "[review.trigger]\nchanged_files = ['docs/**']\n",
    );
    write(root.path(), "src.txt", "source\n");
    write(root.path(), "docs/a.md", "# A\n");
    support::commit_all(root.path());
    let plan = |root: &Path| {
        planner::build(&config::load(root).unwrap(), Some("pre-commit"), false).unwrap()
    };
    let prepared = plan(root.path());
    assert!(prepared.plan.scheduled.is_empty());
    assert_eq!(
        prepared
            .plan
            .omitted
            .iter()
            .map(|o| (o.rule.id.as_str(), o.reason.as_str()))
            .collect::<Vec<_>>(),
        [("all", "no_changes"), ("docs", "trigger_not_matched")]
    );
    write(root.path(), "src.txt", "changed\n");
    git(root.path(), &["add", "src.txt"]);
    let prepared = plan(root.path());
    assert_eq!(prepared.plan.scheduled.len(), 1);
    assert_eq!(prepared.plan.scheduled[0].id, "all");
    assert_eq!(prepared.plan.context.scopes[0].files, ["src.txt"]);
    write(root.path(), "docs/a.md", "# Changed\n");
    git(root.path(), &["add", "docs/a.md"]);
    let prepared = plan(root.path());
    let input = prepared.review_input.unwrap();
    assert_eq!(input.scopes["all"].files.len(), 2);
    assert_eq!(input.scopes["docs"].files.len(), 1);
    assert_eq!(input.scopes["docs"].files[0].path, "docs/a.md");
}

#[test]
fn scope_files_carry_content_unless_deleted_binary_or_switched_off() {
    let root = fixture();
    review(
        root.path(),
        "full",
        "pre-commit",
        "include_change_set = true\n",
    );
    review(
        root.path(),
        "names",
        "pre-commit",
        "include_content = false\n",
    );
    write(root.path(), "text", "old\n");
    write(root.path(), "gone", "old\n");
    support::commit_all(root.path());
    write(root.path(), "text", "new text\n");
    fs::remove_file(root.path().join("gone")).unwrap();
    fs::write(root.path().join("blob.bin"), [0u8, 159, 146, 150]).unwrap();
    support::stage_all(root.path());
    let project = config::load(root.path()).unwrap();
    let snapshot = git_snapshot::capture(root.path()).unwrap();
    let changes = change_set::collect(root.path(), "pre-commit", None).unwrap();
    let active: Vec<_> = project.reviews.iter().collect();
    let input = PreparedReviewInput::prepare(&project, &active, &snapshot, &changes).unwrap();
    let full = &input.scopes["full"];
    let by = |name: &str| full.files.iter().find(|f| f.path == name).unwrap();
    assert_eq!(by("text").content.as_deref(), Some("new text\n"));
    assert!(by("gone").content.is_none() && by("gone").change == State::Deleted);
    assert!(by("blob.bin").content.is_none() && by("blob.bin").binary);
    let patch = full.change_set.as_deref().unwrap();
    assert!(patch.contains("+new text") && patch.contains("-old"));
    let names = &input.scopes["names"];
    assert_eq!(names.files.len(), full.files.len());
    assert!(names.files.iter().all(|f| f.content.is_none()) && names.change_set.is_none());
}

#[test]
fn content_above_the_bound_ends_input_preparation_as_an_error() {
    let root = fixture();
    review(root.path(), "full", "pre-commit", "");
    write(root.path(), "big", &"x".repeat(300 * 1024));
    support::stage_all(root.path());
    let project = config::load(root.path()).unwrap();
    let snapshot = git_snapshot::capture(root.path()).unwrap();
    let changes = change_set::collect(root.path(), "pre-commit", None).unwrap();
    let active: Vec<_> = project.reviews.iter().collect();
    let error = PreparedReviewInput::prepare(&project, &active, &snapshot, &changes)
        .unwrap_err()
        .to_string();
    assert!(error.contains("exceeds"), "{error}");
}

#[test]
fn check_without_a_base_blocks_every_review() {
    let root = fixture();
    review(root.path(), "all", "check", "");
    let out = support::command(env!("CARGO_BIN_EXE_please"))
        .current_dir(root.path())
        .args(["run", "--at", "check", "--plan", "--json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stdout).contains("PLEASE_CHANGE_BASE"));
}
