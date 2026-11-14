//! The changed files and change set of an execution point (DEC-0024): staged for
//! pre-commit and commit-msg, pushed commits for pre-push, net changes against
//! `PLEASE_CHANGE_BASE` for every other point.
use crate::{
    git_snapshot::{self, excluded, git, path_output},
    limits::{MAX_DIFF_BYTES, MAX_SNAPSHOT_FILES},
    reconciliation::HookContext,
};
use anyhow::{Context, Result, ensure};
use serde::Serialize;
use std::{collections::BTreeMap, path::Path, time::Duration};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    Added,
    Modified,
    Deleted,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Change {
    pub path: String,
    pub state: State,
}

/// Where the change set comes from; it decides how the patch is produced.
#[derive(Debug, Clone)]
pub enum Source {
    Staged,
    /// Pairs of (from, to) commit or tree IDs, one per pushed reference.
    Pushed(Vec<(String, String)>),
    Base(String),
}

#[derive(Debug, Clone)]
pub struct ChangeSet {
    pub source: Source,
    pub files: Vec<Change>,
}
impl ChangeSet {
    pub fn base(&self) -> Option<&str> {
        match &self.source {
            Source::Base(commit) => Some(commit),
            _ => None,
        }
    }
    /// A change set without files, for callers that never select by changes.
    pub fn empty() -> Self {
        Self {
            source: Source::Staged,
            files: Vec::new(),
        }
    }
}

/// Parse `git diff --name-status -z` output, or the path list of `ls-files -z` for
/// untracked files, which are additions.
pub fn records(text: &str, untracked: bool) -> Result<Vec<(State, &str)>> {
    ensure!(
        !text.contains('\u{fffd}') && (text.is_empty() || text.ends_with('\0')),
        "unsupported or incomplete changed filename output"
    );
    let mut fields = text.split_terminator('\0');
    let mut result = Vec::new();
    while let Some(first) = fields.next() {
        if untracked {
            result.push((State::Added, first));
            continue;
        }
        let state = match first {
            "A" => State::Added,
            "D" => State::Deleted,
            "M" | "T" => State::Modified,
            other => anyhow::bail!("unsupported change state {other:?}; resolve conflicts first"),
        };
        result.push((
            state,
            fields
                .next()
                .context("incomplete changed filename output")?,
        ));
    }
    Ok(result)
}

fn run(root: &Path, arguments: &[&str]) -> Result<String> {
    let output = path_output(git(root).args(arguments), Duration::from_secs(30))?;
    ensure!(
        output.status.success() && !output.timed_out && !output.stdout.truncated,
        "Git {} failed, timed out or exceeded the 128 MiB snapshot budget",
        arguments.first().copied().unwrap_or_default()
    );
    Ok(output.stdout.text)
}

fn files(root: &Path, arguments: &[&str]) -> Result<BTreeMap<String, State>> {
    let mut map = BTreeMap::new();
    let text = run(root, arguments)?;
    for (state, path) in records(&text, false)? {
        crate::config::relative(Path::new(path))?;
        if !excluded(Path::new(path)) {
            map.insert(path.to_owned(), state);
        }
        ensure!(
            map.len() <= MAX_SNAPSHOT_FILES,
            "changed path limit exceeded"
        );
    }
    Ok(map)
}

const DIFF: [&str; 6] = [
    "diff",
    "--no-ext-diff",
    "--no-textconv",
    "--ignore-submodules=none",
    "--no-renames",
    "--name-status",
];

/// Collect the changes of an execution point. A direct or preview run at pre-push has no
/// pushed references from Git; it then takes the commits not yet on any remote.
pub fn collect(root: &Path, point: &str, hook: Option<&HookContext>) -> Result<ChangeSet> {
    git_snapshot::require_root(root)?;
    match point {
        "pre-commit" | "commit-msg" => {
            let mut arguments = DIFF.to_vec();
            arguments.extend(["--cached", "-z", "--"]);
            let map = files(root, &arguments)?;
            Ok(ChangeSet {
                source: Source::Staged,
                files: map
                    .into_iter()
                    .map(|(path, state)| Change { path, state })
                    .collect(),
            })
        }
        "pre-push" => {
            let ranges = pushed_ranges(root, hook)?;
            let mut map = BTreeMap::new();
            for (from, to) in &ranges {
                let mut arguments = DIFF.to_vec();
                arguments.extend(["-z", from, to, "--"]);
                for (path, state) in files(root, &arguments)? {
                    map.insert(path, state);
                }
            }
            Ok(ChangeSet {
                source: Source::Pushed(ranges),
                files: map
                    .into_iter()
                    .map(|(path, state)| Change { path, state })
                    .collect(),
            })
        }
        _ => {
            let base = std::env::var("PLEASE_CHANGE_BASE").map_err(|_| {
                anyhow::anyhow!(
                    "PLEASE_CHANGE_BASE must explicitly name a baseline commit for reviews at {point}"
                )
            })?;
            let (commit, map) = git_snapshot::changed_paths(root, &base)?;
            Ok(ChangeSet {
                source: Source::Base(commit),
                files: map
                    .into_iter()
                    .map(|(path, state)| Change { path, state })
                    .collect(),
            })
        }
    }
}

fn is_zero(sha: &str) -> bool {
    sha.bytes().all(|b| b == b'0')
}
fn is_object_id(sha: &str) -> bool {
    matches!(sha.len(), 40 | 64) && sha.bytes().all(|b| b.is_ascii_hexdigit())
}

fn pushed_ranges(root: &Path, hook: Option<&HookContext>) -> Result<Vec<(String, String)>> {
    let Some(hook) = hook.filter(|hook| hook.event() == "pre-push") else {
        let remote = default_remote(root)?;
        return Ok(unpushed(root, "HEAD", remote.as_deref())?
            .into_iter()
            .collect());
    };
    let remote = hook
        .arguments()
        .first()
        .map(|name| name.to_string_lossy().into_owned());
    let mut ranges = Vec::new();
    for line in hook.push_input().lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        ensure!(fields.len() == 4, "malformed pre-push reference line");
        let (local, remote_sha) = (fields[1], fields[3]);
        ensure!(
            is_object_id(local) && is_object_id(remote_sha),
            "malformed object ID in pre-push input"
        );
        if is_zero(local) {
            continue;
        }
        if is_zero(remote_sha) {
            ranges.extend(unpushed(root, local, remote.as_deref())?);
        } else {
            run(
                root,
                &["cat-file", "-e", &format!("{remote_sha}^{{commit}}")],
            )
            .with_context(
                || "the remote tip is unknown locally; fetch it to review the pushed commits",
            )?;
            ranges.push((remote_sha.to_owned(), local.to_owned()));
        }
    }
    Ok(ranges)
}

/// The remote of the current branch's upstream, else `origin`, else no remote (every
/// remote-tracking reference counts).
fn default_remote(root: &Path) -> Result<Option<String>> {
    let branch = run(root, &["symbolic-ref", "-q", "--short", "HEAD"]).ok();
    if let Some(branch) = branch.as_deref().map(str::trim).filter(|b| !b.is_empty()) {
        let key = format!("branch.{branch}.remote");
        if let Ok(remote) = run(root, &["config", "--get", &key]) {
            let remote = remote.trim();
            if !remote.is_empty() && remote != "." {
                return Ok(Some(remote.to_owned()));
            }
        }
    }
    let remotes = run(root, &["remote"])?;
    Ok(remotes
        .lines()
        .any(|name| name == "origin")
        .then(|| "origin".to_owned()))
}

/// The commits reachable from `tip` but from no remote-tracking reference of `remote`,
/// as a (parent of the oldest, tip) range; none when everything is already pushed.
fn unpushed(root: &Path, tip: &str, remote: Option<&str>) -> Result<Option<(String, String)>> {
    // An unborn branch has no commits to push.
    if run(
        root,
        &["rev-parse", "--verify", "-q", &format!("{tip}^{{commit}}")],
    )
    .is_err()
    {
        return Ok(None);
    }
    let remotes = remote.map_or("--remotes".to_owned(), |name| format!("--remotes={name}"));
    let list = run(root, &["rev-list", tip, "--not", &remotes])?;
    let Some(oldest) = list.lines().last() else {
        return Ok(None);
    };
    let parent = format!("{oldest}^");
    let base = match run(root, &["rev-parse", "--verify", "-q", &parent]) {
        Ok(parent) => parent.trim().to_owned(),
        Err(_) => {
            let output = crate::process::run(
                git(root).args(["hash-object", "-t", "tree", "--stdin"]),
                Duration::from_secs(10),
            )?;
            ensure!(output.status.success(), "cannot resolve the empty tree");
            output.stdout.text.trim().to_owned()
        }
    };
    Ok(Some((base, tip.to_owned())))
}

/// The patch of the given paths for this change set. Untracked files have no patch.
/// A patch above the bound is an error, never truncated.
pub fn patch(root: &Path, source: &Source, paths: &[String]) -> Result<String> {
    if paths.is_empty() {
        return Ok(String::new());
    }
    let sections: Vec<Vec<&str>> = match source {
        Source::Staged => vec![vec!["--cached"]],
        Source::Pushed(ranges) => ranges
            .iter()
            .map(|(from, to)| vec![from.as_str(), to.as_str()])
            .collect(),
        Source::Base(commit) => vec![vec![commit.as_str()]],
    };
    let mut text = String::new();
    for revisions in sections {
        let mut command = git(root);
        command.args([
            "--literal-pathspecs",
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--ignore-submodules=none",
            "--no-renames",
        ]);
        command.args(&revisions).arg("--").args(paths);
        let output = crate::process::run(&mut command, Duration::from_secs(20))
            .context("Git diff failed")?;
        ensure!(
            output.status.success() && !output.timed_out,
            "Git diff failed or timed out"
        );
        text.push_str(&output.stdout.text);
        ensure!(
            !output.stdout.truncated && text.len() <= MAX_DIFF_BYTES,
            "change set exceeds {MAX_DIFF_BYTES} bytes; narrow the review triggers or set include_change_set = false"
        );
    }
    Ok(text)
}
