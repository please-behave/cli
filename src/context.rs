//! Structured run context: Git facts for the execution point, written once per run.
//!
//! The context is a read-only JSON document (`docs/schemas/context.schema.json`) that checks receive
//! as the `@context` argument and review adapters as `context_file`. It never contains file
//! contents, only paths, states and patches.
use crate::{config::Project, git_snapshot, process, reconciliation::HookContext};
use anyhow::{Context as _, Result, bail, ensure};
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

pub const SCHEMA_VERSION: u32 = 1;
const TIMEOUT: Duration = Duration::from_secs(30);
static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Git<'a> {
    root: &'a Path,
    limit: usize,
}
impl Git<'_> {
    fn out(&self, arguments: &[&str]) -> Result<process::Output> {
        let mut command = git_snapshot::git(self.root);
        command.args(arguments);
        let output = process::run_with_stdout_limit(&mut command, TIMEOUT, self.limit)?;
        ensure!(!output.timed_out, "git {} timed out", arguments[0]);
        Ok(output)
    }
    fn text(&self, arguments: &[&str]) -> Result<String> {
        let output = self.out(arguments)?;
        ensure!(
            output.status.success() && !output.stdout.truncated,
            "git {} failed or exceeded context.max_section_bytes: {}",
            arguments[0],
            output.stderr.text.trim()
        );
        Ok(output.stdout.text)
    }
}

/// Truncate on a character boundary without exceeding `limit` bytes.
fn bounded(text: &str, limit: usize) -> (String, bool) {
    if text.len() <= limit {
        return (text.to_owned(), false);
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_owned(), true)
}

fn status_name(letter: char) -> &'static str {
    match letter {
        'A' => "added",
        'D' => "deleted",
        'R' => "renamed",
        'C' => "copied",
        'T' => "typechange",
        'U' => "unmerged",
        _ => "modified",
    }
}

fn entry(path: &str, status: &str, old_path: Option<&str>) -> Value {
    json!({"path": path, "status": status, "old_path": old_path})
}

/// Accumulates entries under an estimated byte budget and counts what did not fit.
struct Listing {
    entries: Vec<Value>,
    used: usize,
    limit: usize,
    omitted: usize,
}
impl Listing {
    fn new(limit: usize) -> Self {
        Self {
            entries: Vec::new(),
            used: 0,
            limit,
            omitted: 0,
        }
    }
    fn push(&mut self, value: Value, weight: usize) {
        if self.used + weight > self.limit {
            self.omitted += 1;
        } else {
            self.used += weight;
            self.entries.push(value);
        }
    }
}

fn repository(git: &Git) -> Result<Value> {
    let text = git.text(&[
        "rev-parse",
        "--path-format=absolute",
        "--git-dir",
        "--git-common-dir",
        "--is-shallow-repository",
    ])?;
    let lines: Vec<_> = text.lines().collect();
    ensure!(lines.len() == 3, "unexpected rev-parse output");
    Ok(json!({
        "git_dir": lines[0],
        "common_dir": lines[1],
        "linked_worktree": lines[0] != lines[1],
        "shallow": lines[2] == "true",
    }))
}

fn head(git: &Git, limit: usize) -> Result<(Value, Option<String>)> {
    let resolved = git.out(&["rev-parse", "--verify", "-q", "HEAD"])?;
    let commit = resolved
        .status
        .success()
        .then(|| resolved.stdout.text.trim().to_owned());
    let branch = git.out(&["symbolic-ref", "-q", "--short", "HEAD"])?;
    let branch = branch
        .status
        .success()
        .then(|| branch.stdout.text.trim().to_owned());
    let (message, truncated) = match &commit {
        Some(commit) => {
            let output = git.out(&["log", "-1", "--format=%B", "--end-of-options", commit])?;
            ensure!(output.status.success(), "cannot read the HEAD message");
            let (text, cut) = bounded(output.stdout.text.trim_end_matches('\n'), limit);
            (Some(text), cut || output.stdout.truncated)
        }
        None => (None, false),
    };
    Ok((
        json!({
            "commit": commit,
            "branch": branch,
            "message": message,
            "message_truncated": truncated,
        }),
        commit,
    ))
}

/// Write one patch file when it fits the section limit; report its state either way.
fn patch(git: &Git, directory: &Path, name: &str, arguments: &[&str]) -> Result<Value> {
    let output = git.out(arguments)?;
    ensure!(output.status.success(), "git diff failed for {name}");
    if output.stdout.truncated {
        return Ok(json!({"file": null, "bytes": output.stdout.total_bytes, "truncated": true}));
    }
    if output.stdout.text.is_empty() {
        return Ok(json!({"file": null, "bytes": 0, "truncated": false}));
    }
    write_private(&directory.join(name), output.stdout.text.as_bytes())?;
    Ok(json!({"file": name, "bytes": output.stdout.total_bytes, "truncated": false}))
}

/// Entries of `git status --porcelain=v2 -z`, split by category under the section limits.
struct Status {
    staged: Listing,
    unstaged: Listing,
    untracked: Listing,
    conflicted: Listing,
    untracked_paths: Vec<String>,
    total: usize,
    unrepresentable: usize,
}

fn parse_status(text: &str, limit: usize) -> Result<Status> {
    let mut tokens = text.split_terminator('\0');
    let mut status = Status {
        staged: Listing::new(limit),
        unstaged: Listing::new(limit),
        untracked: Listing::new(limit),
        conflicted: Listing::new(limit),
        untracked_paths: Vec::new(),
        total: 0,
        unrepresentable: 0,
    };
    while let Some(record) = tokens.next() {
        let kind = record.chars().next().unwrap_or(' ');
        let fields: Vec<&str> = match kind {
            '1' => record.splitn(9, ' ').collect(),
            '2' => record.splitn(10, ' ').collect(),
            'u' => record.splitn(11, ' ').collect(),
            '?' => record.splitn(2, ' ').collect(),
            _ => continue,
        };
        let original = (kind == '2').then(|| tokens.next()).flatten();
        let Some(path) = fields.last().copied().filter(|path| !path.is_empty()) else {
            bail!("unexpected git status output");
        };
        status.total += 1;
        // Invalid UTF-8 shows up as U+FFFD; count such paths instead of listing a lossy name.
        if path.contains('\u{fffd}') || original.is_some_and(|old| old.contains('\u{fffd}')) {
            status.unrepresentable += 1;
            continue;
        }
        let weight = path.len() + original.map_or(0, str::len) + 48;
        match kind {
            '?' => {
                status.untracked_paths.push(path.to_owned());
                status.untracked.push(json!(path), weight);
            }
            'u' => status.conflicted.push(json!(path), weight),
            _ => {
                let mut states = fields[1].chars();
                let (index, tree) = (states.next().unwrap_or('.'), states.next().unwrap_or('.'));
                if index != '.' {
                    let old = original.filter(|_| matches!(index, 'R' | 'C'));
                    status
                        .staged
                        .push(entry(path, status_name(index), old), weight);
                }
                if tree != '.' {
                    let old = original.filter(|_| matches!(tree, 'R' | 'C'));
                    status
                        .unstaged
                        .push(entry(path, status_name(tree), old), weight);
                }
            }
        }
    }
    Ok(status)
}

fn workspace(git: &Git, directory: &Path, limit: usize) -> Result<(Value, Vec<String>)> {
    let output = git.out(&["status", "--porcelain=v2", "-z", "--untracked-files=all"])?;
    ensure!(output.status.success(), "git status failed");
    let cut_output = output.stdout.truncated;
    let mut text = output.stdout.text;
    if cut_output {
        // Keep only complete NUL-terminated records of an oversized listing.
        text.truncate(text.rfind('\0').map_or(0, |end| end + 1));
    }
    let Status {
        staged,
        unstaged,
        untracked,
        conflicted,
        untracked_paths,
        total,
        unrepresentable,
    } = parse_status(&text, limit)?;
    let omitted = staged.omitted + unstaged.omitted + untracked.omitted + conflicted.omitted;
    let value = json!({
        "clean": total == 0 && !cut_output,
        "staged": staged.entries,
        "unstaged": unstaged.entries,
        "untracked": untracked.entries,
        "conflicted": conflicted.entries,
        "unrepresentable_paths": unrepresentable,
        "omitted_entries": omitted,
        "truncated": omitted > 0 || cut_output,
        "staged_patch": patch(
            git,
            directory,
            "staged.patch",
            &["diff", "--cached", "--no-color", "--no-ext-diff", "--no-textconv", "-M"],
        )?,
        "unstaged_patch": patch(
            git,
            directory,
            "unstaged.patch",
            &["diff", "--no-color", "--no-ext-diff", "--no-textconv", "-M"],
        )?,
    });
    Ok((value, untracked_paths))
}

/// Parse `git diff --name-status -z -M` output into entries.
fn name_status(text: &str, limit: usize) -> Result<(Vec<Value>, usize)> {
    let mut tokens = text.split_terminator('\0');
    let mut listing = Listing::new(limit);
    while let Some(code) = tokens.next() {
        let letter = code.chars().next().unwrap_or('M');
        let first = tokens.next().context("incomplete name-status output")?;
        let second = matches!(letter, 'R' | 'C').then(|| tokens.next()).flatten();
        let (path, old) = second.map_or((first, None), |new| (new, Some(first)));
        ensure!(
            !path.contains('\u{fffd}') && old.is_none_or(|old| !old.contains('\u{fffd}')),
            "unsupported non-UTF-8 path in changed files"
        );
        let weight = path.len() + old.map_or(0, str::len) + 48;
        listing.push(entry(path, status_name(letter), old), weight);
    }
    Ok((listing.entries, listing.omitted))
}

struct Section {
    files: Vec<Value>,
    omitted: usize,
    truncated: bool,
    patch: Value,
}

fn diff_section(
    git: &Git,
    directory: &Path,
    name: &str,
    limit: usize,
    range: &[&str],
) -> Result<Section> {
    let mut status = vec![
        "diff",
        "--name-status",
        "-z",
        "-M",
        "--no-ext-diff",
        "--no-textconv",
    ];
    status.extend(range);
    let listing = git.out(&status)?;
    ensure!(listing.status.success(), "git diff failed for {name}");
    let (files, omitted) = if listing.stdout.truncated {
        (Vec::new(), 0)
    } else {
        name_status(&listing.stdout.text, limit)?
    };
    let mut patch_arguments = vec!["diff", "--no-color", "--no-ext-diff", "--no-textconv", "-M"];
    patch_arguments.extend(range);
    Ok(Section {
        files,
        omitted,
        truncated: listing.stdout.truncated,
        patch: patch(git, directory, name, &patch_arguments)?,
    })
}

fn is_zero(sha: &str) -> bool {
    sha.bytes().all(|byte| byte == b'0')
}
fn is_object_id(sha: &str) -> bool {
    matches!(sha.len(), 40 | 64) && sha.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn push(
    git: &Git,
    directory: &Path,
    hook: &HookContext,
    head: Option<&str>,
    limit: usize,
) -> Result<Option<Value>> {
    if hook.event() != "pre-push" {
        return Ok(None);
    }
    let remote = hook
        .arguments()
        .first()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut updates = Vec::new();
    for (index, line) in hook.push_input().lines().enumerate() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        ensure!(fields.len() == 4, "malformed pre-push reference line");
        let (local_ref, local, remote_ref, remote_sha) =
            (fields[0], fields[1], fields[2], fields[3]);
        ensure!(
            is_object_id(local) && is_object_id(remote_sha),
            "malformed object ID in pre-push input"
        );
        let kind = if is_zero(local) {
            "delete"
        } else if is_zero(remote_sha) {
            "create"
        } else {
            "update"
        };
        let mut commits = Listing::new(limit);
        let mut section = None;
        let mut available = kind != "delete";
        if kind != "delete" {
            let list = if kind == "update" {
                git.out(&["rev-list", &format!("{remote_sha}..{local}")])?
            } else {
                git.out(&["rev-list", local, "--not", &format!("--remotes={remote}")])?
            };
            if list.status.success() && !list.stdout.truncated {
                let shas: Vec<&str> = list.stdout.text.lines().collect();
                for sha in &shas {
                    commits.push(json!(sha), sha.len() + 4);
                }
                let base = if kind == "update" {
                    Some(remote_sha.to_owned())
                } else if let Some(oldest) = shas.last() {
                    let parent =
                        git.out(&["rev-parse", "--verify", "-q", &format!("{oldest}^")])?;
                    Some(if parent.status.success() {
                        parent.stdout.text.trim().to_owned()
                    } else {
                        git.text(&["hash-object", "-t", "tree", "--stdin"])?
                            .trim()
                            .to_owned()
                    })
                } else {
                    None
                };
                if let Some(base) = base {
                    let name = format!("push-{index}.patch");
                    section = Some(diff_section(
                        git,
                        directory,
                        &name,
                        limit,
                        &[base.as_str(), local],
                    )?);
                }
            } else {
                available = false;
            }
        }
        let section = section.unwrap_or(Section {
            files: Vec::new(),
            omitted: 0,
            truncated: false,
            patch: json!({"file": null, "bytes": 0, "truncated": false}),
        });
        updates.push(json!({
            "local_ref": local_ref,
            "local_sha": local,
            "remote_ref": remote_ref,
            "remote_sha": (!is_zero(remote_sha)).then_some(remote_sha),
            "kind": kind,
            "is_head": head == Some(local),
            "range_available": available,
            "commits": commits.entries,
            "omitted_commits": commits.omitted,
            "changed_files": section.files,
            "omitted_files": section.omitted,
            "files_truncated": section.truncated,
            "patch": section.patch,
        }));
    }
    Ok(Some(json!({"remote": remote, "updates": updates})))
}

fn changes(
    git: &Git,
    directory: &Path,
    untracked: &[String],
    limit: usize,
) -> Result<Option<Value>> {
    let Ok(base) = std::env::var("PLEASE_CHANGE_BASE") else {
        return Ok(None);
    };
    // The variable is set by CI for review triggers. An unusable value must not fail runs
    // that never use it, so the section is silently omitted.
    let resolved = (!base.is_empty() && base.len() <= 1024 && !base.contains('\0'))
        .then(|| {
            git.out(&[
                "rev-parse",
                "--verify",
                "--end-of-options",
                &format!("{base}^{{commit}}"),
            ])
        })
        .transpose()?
        .filter(|output| output.status.success())
        .map(|output| output.stdout.text.trim().to_owned())
        .filter(|commit| is_object_id(commit));
    let Some(commit) = resolved else {
        return Ok(None);
    };
    let mut section = diff_section(git, directory, "changes.patch", limit, &[commit.as_str()])?;
    let mut listing = Listing::new(limit);
    for path in untracked {
        listing.push(entry(path, "untracked", None), path.len() + 48);
    }
    section.files.extend(listing.entries);
    section.omitted += listing.omitted;
    Ok(Some(json!({
        "base": commit,
        "files": section.files,
        "omitted_files": section.omitted,
        "files_truncated": section.truncated,
        "patch": section.patch,
    })))
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(path, bytes).with_context(|| format!("cannot write {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Remove the oldest completed run directories beyond the retention, never the current one.
fn prune(base: &Path, keep: usize, current: &str) {
    let Ok(entries) = fs::read_dir(base) else {
        return;
    };
    let mut runs: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    runs.sort();
    let surplus = runs.len().saturating_sub(keep);
    for name in runs
        .into_iter()
        .take(surplus)
        .filter(|name| name != current)
    {
        let _ = fs::remove_dir_all(base.join(name));
    }
}

/// Write the context for this run and return the absolute path of its JSON document.
pub fn prepare(project: &Project, point: &str, hook: Option<&HookContext>) -> Result<PathBuf> {
    let limit = usize::try_from(project.config.context.max_section_bytes)?;
    let git = Git {
        root: &project.root,
        limit,
    };
    let base = PathBuf::from(
        git.text(&[
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "please/context",
        ])?
        .trim(),
    );
    let run_id = format!(
        "{}-{}-{}",
        chrono::Utc::now().format("%Y%m%dT%H%M%S%3fZ"),
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let directory = base.join(&run_id);
    fs::create_dir_all(&directory)
        .with_context(|| format!("cannot create {}", directory.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?;
    }
    prune(&base, project.config.context.retain_runs, &run_id);

    let (head_value, head_commit) = head(&git, limit)?;
    let (workspace_value, untracked) = workspace(&git, &directory, limit)?;
    let mut document = json!({
        "schema_version": SCHEMA_VERSION,
        "run_id": run_id,
        "execution_point": point,
        "repository": repository(&git)?,
        "head": head_value,
        "workspace": workspace_value,
    });
    let object = document.as_object_mut().expect("context is an object");
    if let Some(hook) = hook {
        // The managed hook always passes exactly one message file; other shapes omit the section.
        if hook.event() == "commit-msg" && hook.arguments().len() == 1 {
            let path = hook.commit_message_path(&project.root)?;
            let text = fs::read(&path)
                .with_context(|| format!("cannot read commit message {}", path.display()))?;
            let (text, truncated) = bounded(&String::from_utf8_lossy(&text), limit);
            object.insert(
                "commit_message".into(),
                json!({"file": path.to_string_lossy(), "text": text, "truncated": truncated}),
            );
        }
        if let Some(value) = push(&git, &directory, hook, head_commit.as_deref(), limit)? {
            object.insert("push".into(), value);
        }
    }
    if let Some(value) = changes(&git, &directory, &untracked, limit)? {
        object.insert("changes".into(), value);
    }
    let path = directory.join("context.json");
    write_private(&path, serde_json::to_string_pretty(&document)?.as_bytes())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_records_are_split_by_category_with_renames_and_conflicts() {
        let text = [
            "1 M. N... 100644 100644 100644 a b a.txt",
            "1 .M N... 100644 100644 100644 a b a.txt",
            "2 R. N... 100644 100644 100644 a b R100 new name.txt",
            "old name.txt",
            "u UU N... 100644 100644 100644 100644 a b c conflict.txt",
            "? untracked file.txt",
        ]
        .join("\0")
            + "\0";
        let status = parse_status(&text, 10_000).unwrap();
        assert_eq!(status.total, 5);
        assert_eq!(status.staged.entries.len(), 2);
        assert_eq!(status.staged.entries[1]["old_path"], "old name.txt");
        assert_eq!(status.staged.entries[1]["status"], "renamed");
        assert_eq!(status.unstaged.entries.len(), 1);
        assert_eq!(status.conflicted.entries[0], "conflict.txt");
        assert_eq!(status.untracked_paths, ["untracked file.txt"]);
        assert_eq!(status.unrepresentable, 0);
    }

    #[test]
    fn invalid_utf8_paths_are_counted_instead_of_listed() {
        let text = "? bad\u{fffd}.txt\0? good.txt\0";
        let status = parse_status(text, 10_000).unwrap();
        assert_eq!((status.total, status.unrepresentable), (2, 1));
        assert_eq!(status.untracked_paths, ["good.txt"]);
    }

    #[test]
    fn entries_over_the_limit_are_counted_as_omitted() {
        let text = "? one.txt\0? two.txt\0? three.txt\0";
        let status = parse_status(text, 100).unwrap();
        assert_eq!(status.untracked.entries.len(), 1);
        assert_eq!(status.untracked.omitted, 2);
    }
}
