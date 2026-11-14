use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::Path,
    process::{Command, Stdio},
    time::Duration,
};

use crate::limits::{
    MAX_CONFIG_DEPTH, MAX_DIFF_BYTES, MAX_SNAPSHOT_BYTES as MAX_TOTAL,
    MAX_SNAPSHOT_FILE_BYTES as MAX_FILE, MAX_SNAPSHOT_FILES as MAX_FILES,
};
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FileState {
    pub sha256: String,
    pub bytes: u64,
    pub executable: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub head: Option<String>,
    pub fingerprint: String,
    pub files: BTreeMap<String, FileState>,
    pub complete: bool,
    pub limitations: Vec<String>,
}
#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Changes {
    pub state: String,
    pub total_files: Option<usize>,
    pub sample_paths: Vec<String>,
    pub head_changed: bool,
    pub line_counts: Option<String>,
}
pub(crate) fn git(root: &Path) -> Command {
    let mut c = Command::new("git");
    c.current_dir(root)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .stdin(Stdio::null());
    c
}
// These outputs are repository data, not logs. Use the snapshot byte budget;
// stderr and every ordinary process/provider capture retain their smaller bound.
fn path_output(command: &mut Command, timeout: Duration) -> Result<crate::process::Output> {
    crate::process::run_with_stdout_limit(command, timeout, MAX_TOTAL as usize)
}
pub fn require_root(root: &Path) -> Result<()> {
    let canonical_root = root.canonicalize()?;
    let root = canonical_root.as_path();
    let output = crate::process::run(
        git(root).args(["rev-parse", "--show-toplevel"]),
        Duration::from_secs(10),
    )?;
    ensure!(
        output.status.success() && !output.timed_out,
        "run Please Behave inside a Git repository (git init first)"
    );
    let found = Path::new(output.stdout.text.trim()).canonicalize()?;
    ensure!(
        found == root,
        "run Please Behave from the repository root: {}",
        found.display()
    );
    Ok(())
}
fn excluded(path: &Path) -> bool {
    path.starts_with(".git") || path.starts_with(crate::config::path("reports"))
}
fn add_config(
    root: &Path,
    paths: &mut BTreeSet<String>,
    limitations: &mut Vec<String>,
) -> Result<()> {
    for path in crate::discovery::snapshot_inputs(root)? {
        if excluded(&path) {
            continue;
        }
        if path.starts_with(crate::config::path(""))
            && path.components().count() > MAX_CONFIG_DEPTH + 2
        {
            limitations.push(format!("configuration nesting exceeds {MAX_CONFIG_DEPTH}"));
            continue;
        }
        if paths.len() >= MAX_FILES {
            limitations.push(format!("file count exceeds {MAX_FILES}"));
            break;
        }
        if let Some(s) = path.to_str() {
            #[cfg(windows)]
            let path = s.replace('\\', "/");
            #[cfg(not(windows))]
            let path = s.to_owned();
            paths.insert(path);
        } else {
            limitations.push("non-UTF-8 configuration path".into());
        }
    }
    Ok(())
}
pub fn capture(root: &Path) -> Result<Snapshot> {
    let canonical_root = root.canonicalize()?;
    let root = canonical_root.as_path();
    require_root(root)?;
    let mut limitations = Vec::new();
    let head_out = crate::process::run(
        git(root).args(["rev-parse", "--verify", "HEAD"]),
        Duration::from_secs(10),
    )?;
    ensure!(!head_out.timed_out, "Git HEAD lookup timed out");
    let head = if head_out.status.success() {
        Some(head_out.stdout.text.trim().to_owned())
    } else {
        let symbolic = crate::process::run(
            git(root).args(["symbolic-ref", "-q", "HEAD"]),
            Duration::from_secs(10),
        )?;
        ensure!(symbolic.status.success(), "cannot resolve Git HEAD");
        None
    };
    // NUL-separated paths: whitespace and newlines in file names remain unambiguous.
    let list = path_output(
        git(root).args([
            "-c",
            "core.quotepath=false",
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ]),
        Duration::from_secs(30),
    )?;
    ensure!(
        list.status.success() && !list.timed_out,
        "Git file enumeration failed"
    );
    // The process capture is deliberately bounded. Never claim complete coverage after truncation.
    if list.stdout.truncated {
        limitations.push("Git file list exceeded the 128 MiB snapshot budget".into());
    }
    let mut paths = BTreeSet::new();
    for raw in list.stdout.text.split_terminator('\0') {
        if raw.contains('\u{fffd}') {
            limitations.push("unsupported non-UTF-8 path".into());
            continue;
        }
        if !excluded(Path::new(raw)) {
            paths.insert(raw.to_owned());
            if paths.len() > MAX_FILES {
                limitations.push(format!("file count exceeds {MAX_FILES}"));
                break;
            }
        }
    }
    add_config(root, &mut paths, &mut limitations)?;
    let mut files = BTreeMap::new();
    let mut total = 0;
    for path in paths {
        if files.len() >= MAX_FILES {
            limitations.push(format!("file count exceeds {MAX_FILES}"));
            break;
        }
        let full = root.join(&path);
        crate::config::relative(Path::new(&path))?;
        let meta = match full.symlink_metadata() {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                limitations.push(format!("cannot inspect {path}: {e}"));
                continue;
            }
        };
        if meta.file_type().is_symlink() || !meta.is_file() {
            limitations.push(format!(
                "unsupported symlink/submodule/special file: {path}"
            ));
            continue;
        }
        // Canonical containment also prevents traversing a symlink in a parent directory.
        if !full.canonicalize()?.starts_with(root) {
            limitations.push(format!("outside root: {path}"));
            continue;
        }
        if meta.len() > MAX_FILE || total + meta.len() > MAX_TOTAL {
            limitations.push(format!("hash budget exceeded: {path}"));
            continue;
        }
        let mut bytes = Vec::new();
        match fs::File::open(&full).and_then(|f| f.take(MAX_FILE + 1).read_to_end(&mut bytes)) {
            Ok(_) if bytes.len() as u64 <= MAX_FILE => {}
            _ => {
                limitations.push(format!("unreadable/oversized file: {path}"));
                continue;
            }
        }
        total += bytes.len() as u64;
        #[cfg(unix)]
        let executable = {
            use std::os::unix::fs::PermissionsExt;
            meta.permissions().mode() & 0o111 != 0
        };
        #[cfg(not(unix))]
        let executable = false;
        files.insert(
            path,
            FileState {
                sha256: format!("{:x}", Sha256::digest(&bytes)),
                bytes: bytes.len() as u64,
                executable,
            },
        );
    }
    let fingerprint = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(&head, &files))?)
    );
    Ok(Snapshot {
        head,
        fingerprint,
        files,
        complete: limitations.is_empty(),
        limitations,
    })
}
pub fn compare(before: &Snapshot, after: &Snapshot, limit: usize) -> Changes {
    let paths: BTreeSet<_> = before.files.keys().chain(after.files.keys()).collect();
    let changed: Vec<_> = paths
        .into_iter()
        .filter(|p| before.files.get(*p) != after.files.get(*p))
        .cloned()
        .collect();
    let head_changed = before.head != after.head;
    let complete = before.complete && after.complete;
    let state = if !complete {
        "UNKNOWN"
    } else if head_changed || !changed.is_empty() {
        "STALE"
    } else {
        "CURRENT"
    };
    Changes {
        state: state.into(),
        total_files: complete.then_some(changed.len()),
        sample_paths: changed.into_iter().take(limit.min(5)).collect(),
        head_changed,
        line_counts: None,
    }
}
pub fn bounded_diff(root: &Path, has_head: bool) -> Result<String> {
    bounded_diff_for_paths(root, has_head, None)
}
pub fn bounded_diff_for_paths(
    root: &Path,
    has_head: bool,
    paths: Option<&[String]>,
) -> Result<String> {
    let mut cmd = git(root);
    if let Some(paths) = paths {
        ensure!(!paths.is_empty(), "focused diff requires entry files");
        cmd.arg("--literal-pathspecs");
    }
    cmd.args(["diff", "--no-ext-diff", "--no-textconv", "--no-color"]);
    if has_head {
        cmd.arg("HEAD");
    }
    if let Some(paths) = paths {
        cmd.arg("--").args(paths);
    } else {
        cmd.args(["--", "."]).arg(format!(
            ":(exclude){}",
            crate::config::path("reports").display()
        ));
    }
    let out = crate::process::run(&mut cmd, Duration::from_secs(20)).context("Git diff failed")?;
    ensure!(
        out.status.success() && !out.timed_out,
        "Git diff failed or timed out"
    );
    ensure!(
        !out.stdout.truncated && out.stdout.text.len() <= MAX_DIFF_BYTES,
        "change set exceeds {MAX_DIFF_BYTES} bytes; commit smaller changes or set include_diff = false"
    );
    Ok(out.stdout.text)
}

/// Git status is orientation only; never used for since-verification comparisons.
pub fn working_summary(root: &Path) -> Result<String> {
    let output = path_output(
        git(root).args(["status", "--porcelain=v1", "-z", "--untracked-files=all"]),
        Duration::from_secs(20),
    )?;
    ensure!(
        output.status.success() && !output.timed_out && !output.stdout.truncated,
        "Git status unavailable, timed out or truncated"
    );
    let mut entries = output.stdout.text.split_terminator('\0');
    let (mut staged, mut unstaged, mut untracked) = (0, 0, 0);
    while let Some(entry) = entries.next() {
        let bytes = entry.as_bytes();
        ensure!(
            bytes.len() >= 3 && bytes[2] == b' ',
            "invalid Git status record"
        );
        let rename = b"RC".contains(&bytes[0]) || b"RC".contains(&bytes[1]);
        if rename {
            entries.next();
        }
        if excluded(Path::new(&entry[3..])) {
            continue;
        }
        if &bytes[..2] == b"??" {
            untracked += 1;
        } else {
            if bytes[0] != b' ' {
                staged += 1;
            }
            if bytes[1] != b' ' {
                unstaged += 1;
            }
        }
    }
    Ok(format!(
        "Git status: {staged} staged, {unstaged} unstaged, {untracked} untracked paths (categories may overlap; not changes since verification)"
    ))
}

/// Explicit net comparison for triggers, separate from snapshot freshness.
pub fn changed_paths(root: &Path, base: &str) -> Result<(String, BTreeSet<String>)> {
    require_root(root)?;
    ensure!(
        !base.is_empty() && base.len() <= 1024 && !base.contains('\0'),
        "invalid PLEASE_CHANGE_BASE"
    );
    let resolved = crate::process::run(
        git(root).args([
            "rev-parse",
            "--verify",
            "--end-of-options",
            &format!("{base}^{{commit}}"),
        ]),
        Duration::from_secs(10),
    )?;
    ensure!(
        resolved.status.success() && !resolved.timed_out && !resolved.stdout.truncated,
        "cannot resolve PLEASE_CHANGE_BASE; fetch the baseline commit in shallow clones"
    );
    let commit = resolved.stdout.text.trim().to_owned();
    ensure!(
        matches!(commit.len(), 40 | 64) && commit.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid baseline commit ID"
    );
    let index = path_output(
        git(root).args(["ls-files", "-v", "-z"]),
        Duration::from_secs(30),
    )?;
    ensure!(
        index.status.success() && !index.timed_out && !index.stdout.truncated,
        "cannot inspect index flags for change comparison"
    );
    ensure!(
        index
            .stdout
            .text
            .split_terminator('\0')
            .all(|entry| entry.starts_with("H ")),
        "change comparison requires ordinary tracked files; remove assume-unchanged/skip-worktree flags and resolve index conflicts"
    );
    let tracked: BTreeSet<_> = index
        .stdout
        .text
        .split_terminator('\0')
        .map(|entry| &entry[2..])
        .collect();
    let mut paths = BTreeSet::new();
    for arguments in [
        vec![
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--ignore-submodules=none",
            "--no-renames",
            "--name-only",
            "-z",
            &commit,
            "--",
        ],
        vec!["ls-files", "--others", "--exclude-standard", "-z"],
    ] {
        let output = path_output(git(root).args(arguments), Duration::from_secs(30))?;
        ensure!(
            output.status.success() && !output.timed_out && !output.stdout.truncated,
            "change comparison unavailable, timed out or exceeded the 128 MiB snapshot budget; ensure baseline tree is available"
        );
        let text = output.stdout.text;
        ensure!(
            !text.contains('\u{fffd}') && (text.is_empty() || text.ends_with('\0')),
            "unsupported or incomplete changed filename output"
        );
        for path in text.split_terminator('\0') {
            crate::config::relative(Path::new(path))?;
            if !excluded(Path::new(path)) {
                paths.insert(path.to_owned());
            }
            ensure!(paths.len() <= MAX_FILES, "changed path limit exceeded");
        }
    }
    let mut inputs = BTreeSet::new();
    let mut limitations = Vec::new();
    add_config(root, &mut inputs, &mut limitations)?;
    ensure!(
        limitations.is_empty(),
        "incomplete governance input comparison: {limitations:?}"
    );
    paths.extend(
        inputs
            .into_iter()
            .filter(|path| !tracked.contains(path.as_str())),
    );
    ensure!(paths.len() <= MAX_FILES, "changed path limit exceeded");
    Ok((commit, paths))
}
