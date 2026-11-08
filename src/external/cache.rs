//! Shared immutable checkouts and bounded host-Git operations.
use super::hash;
use crate::{config, limits::*, process};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

pub(super) fn directory() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("XDG_CACHE_HOME") {
        let path = PathBuf::from(path);
        ensure!(path.is_absolute(), "XDG_CACHE_HOME must be absolute");
        return Ok(path.join("please"));
    }
    let home =
        PathBuf::from(std::env::var_os("HOME").context("HOME unavailable; set XDG_CACHE_HOME")?);
    ensure!(home.is_absolute(), "HOME must be absolute");
    Ok(home
        .join(if cfg!(target_os = "macos") {
            "Library/Caches"
        } else {
            ".cache"
        })
        .join("please"))
}

pub(super) fn create_directory(path: &Path) -> Result<()> {
    // Refuse symlink components even in a pre-existing cache hierarchy.
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        create_directory(parent)?;
    }
    match fs::create_dir(path) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e.into()),
    }
    let metadata = path.symlink_metadata()?;
    ensure!(
        metadata.is_dir() && !metadata.file_type().is_symlink(),
        "cache directory must not be a symlink"
    );
    Ok(())
}

pub(super) struct Lock(File);
impl Lock {
    pub(super) fn acquire(path: &Path) -> Result<Self> {
        Self::open(path, true)
    }
    pub(super) fn read(path: &Path) -> Result<Self> {
        Self::open(path, false)
    }
    fn open(path: &Path, write: bool) -> Result<Self> {
        if let Ok(metadata) = path.symlink_metadata() {
            ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "unsafe external cache lock"
            );
        }
        let mut options = OpenOptions::new();
        options
            .create(write)
            .truncate(false)
            .read(true)
            .write(write);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NOFOLLOW).mode(0o600);
        }
        let file = options
            .open(path)
            .context("cannot open external cache lock")?;
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            let start = Instant::now();
            loop {
                if unsafe {
                    libc::flock(
                        file.as_raw_fd(),
                        (if write { libc::LOCK_EX } else { libc::LOCK_SH }) | libc::LOCK_NB,
                    )
                } == 0
                {
                    break;
                }
                let error = std::io::Error::last_os_error();
                ensure!(
                    error.kind() == std::io::ErrorKind::WouldBlock,
                    "external cache lock failed: {error}"
                );
                ensure!(
                    start.elapsed() < Duration::from_secs(EXTERNAL_LOCK_TIMEOUT_SECONDS),
                    "external cache busy; retry please sync"
                );
                thread::sleep(Duration::from_millis(20));
            }
        }
        #[cfg(not(unix))]
        bail!("external Git rule sets require Unix cache locking");
        Ok(Self(file))
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::fd::AsRawFd;
            unsafe {
                libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
            }
        }
    }
}

fn command(root: &Path, repository: Option<&Path>) -> Command {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .stdin(Stdio::null())
        .args([
            "--no-pager",
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "protocol.ext.allow=never",
            "-c",
            "protocol.file.allow=always",
        ])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .env("GIT_OPTIONAL_LOCKS", "0");
    // A hook/embedding process may carry repository-selection variables.
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "GIT_COMMON_DIR",
        "GIT_OBJECT_DIRECTORY",
        "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    ] {
        command.env_remove(name);
    }
    if let Some(repository) = repository {
        command.arg("--git-dir").arg(repository);
    }
    command
}

pub(super) fn git(
    root: &Path,
    repository: Option<&Path>,
    args: &[&str],
    remote: bool,
) -> Result<String> {
    let output = process::run(
        command(root, repository).args(args),
        Duration::from_secs(if remote {
            EXTERNAL_FETCH_TIMEOUT_SECONDS
        } else {
            EXTERNAL_GIT_TIMEOUT_SECONDS
        }),
    )
    .map_err(|_| anyhow::anyhow!("cannot run Git; check Git installation and cache permissions"))?;
    ensure!(
        !output.timed_out,
        "Git operation timed out; check repository access and retry please sync"
    );
    if !output.status.success() {
        // Git stderr may contain credentials or helper diagnostics. Never echo it.
        let error = output.stderr.text.to_ascii_lowercase();
        if error.contains("authentication")
            || error.contains("permission denied")
            || error.contains("could not read username")
            || error.contains("403")
        {
            bail!("Git authentication failed; configure host Git credentials or SSH access");
        }
        if error.contains("couldn't find remote ref")
            || error.contains("not our ref")
            || error.contains("unadvertised object")
            || error.contains("invalid refspec")
        {
            bail!("invalid or unavailable Git reference; check the configured ref");
        }
        if remote {
            bail!(
                "Git repository unavailable or reference inaccessible; check repository access and configured ref"
            );
        }
        bail!("Git cache operation failed; check cache permissions and run please sync");
    }
    ensure!(
        !output.stdout.truncated,
        "Git output exceeded the supported capture limit"
    );
    Ok(output.stdout.text.trim().to_owned())
}

fn repository_directory(cache: &Path, repository: &str) -> Result<PathBuf> {
    ensure!(
        repository.len() == 64 && repository.bytes().all(|c| c.is_ascii_hexdigit()),
        "invalid cache repository identity"
    );
    Ok(cache.join("git").join(repository))
}

pub(super) fn resolve(
    root: &Path,
    cache: &Path,
    repository: &str,
    url: &str,
    reference: &str,
    changed: &mut bool,
) -> Result<String> {
    let directory = repository_directory(cache, repository)?;
    create_directory(&directory).context("cannot create Git repository cache")?;
    let _lock = Lock::acquire(&directory.join("cache.lock"))?;
    let bare = directory.join("objects.git");
    if !bare.try_exists()? {
        let temporary = tempfile::tempdir_in(&directory)?;
        git(
            root,
            None,
            &[
                "clone",
                "--bare",
                "--quiet",
                "--no-local",
                "--no-hardlinks",
                "--no-recurse-submodules",
                "--template=",
                "--",
                url,
                temporary
                    .path()
                    .to_str()
                    .context("cache path must be UTF-8")?,
            ],
            true,
        )?;
        // Clone negotiates the remote object format (including SHA-256). Keep no
        // remote URL in the published cache; refresh uses the configured URL.
        git(
            root,
            Some(temporary.path()),
            &["config", "--remove-section", "remote.origin"],
            false,
        )?;
        fs::rename(temporary.path(), &bare)?;
        *changed = true;
    }
    ensure!(
        bare.symlink_metadata()?.is_dir() && !bare.symlink_metadata()?.file_type().is_symlink(),
        "unsafe Git object cache"
    );
    let destination = format!("refs/please/{}", hash(reference));
    // Fetch the selector itself, never a fallback branch, and force movable tags/branches.
    let refspec = format!("+{reference}:{destination}");
    git(
        root,
        Some(&bare),
        &[
            "fetch",
            "--quiet",
            "--no-write-fetch-head",
            "--no-tags",
            "--no-recurse-submodules",
            "--",
            url,
            &refspec,
        ],
        true,
    )?;
    let commit = git(
        root,
        Some(&bare),
        &[
            "rev-parse",
            "--verify",
            &format!("{destination}^{{commit}}"),
        ],
        false,
    )
    .context("invalid Git reference: selector must identify a commit")?;
    validate_commit(&commit)?;
    if reference.len() == commit.len() && reference.bytes().all(|c| c.is_ascii_hexdigit()) {
        ensure!(
            reference.eq_ignore_ascii_case(&commit),
            "Git resolved a different commit than requested"
        );
    }
    Ok(commit)
}

fn validate_commit(commit: &str) -> Result<()> {
    ensure!(
        [40, 64].contains(&commit.len()) && commit.bytes().all(|c| c.is_ascii_hexdigit()),
        "invalid resolved Git commit; run please sync"
    );
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Receipt {
    commit: String,
    digest: String,
}

fn digest(root: &Path) -> Result<String> {
    fn walk(
        root: &Path,
        directory: &Path,
        depth: usize,
        hasher: &mut Sha256,
        files: &mut usize,
        bytes: &mut u64,
    ) -> Result<()> {
        ensure!(
            depth <= MAX_EXTERNAL_CACHE_DEPTH,
            "cached repository nesting exceeds {MAX_EXTERNAL_CACHE_DEPTH}"
        );
        let mut entries = fs::read_dir(directory)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            *files += 1;
            ensure!(
                *files <= MAX_SNAPSHOT_FILES,
                "cached repository exceeds {MAX_SNAPSHOT_FILES} entries"
            );
            let path = entry.path();
            let relative = path
                .strip_prefix(root)?
                .to_str()
                .context("cached paths must be UTF-8")?;
            hasher.update((relative.len() as u64).to_le_bytes());
            hasher.update(relative.as_bytes());
            let metadata = path.symlink_metadata()?;
            if metadata.file_type().is_symlink() {
                hasher.update(b"link");
                let target = fs::read_link(&path)?;
                let target = target.to_str().context("cached symlink must be UTF-8")?;
                hasher.update((target.len() as u64).to_le_bytes());
                hasher.update(target.as_bytes());
            } else if metadata.is_dir() {
                hasher.update(b"directory");
                walk(root, &path, depth + 1, hasher, files, bytes)?;
            } else {
                ensure!(metadata.is_file(), "unsupported cached repository entry");
                ensure!(
                    metadata.len() <= MAX_SNAPSHOT_FILE_BYTES,
                    "cached file exceeds {MAX_SNAPSHOT_FILE_BYTES} bytes"
                );
                *bytes += metadata.len();
                ensure!(
                    *bytes <= MAX_SNAPSHOT_BYTES,
                    "cached repository exceeds {MAX_SNAPSHOT_BYTES} bytes"
                );
                hasher.update(b"file");
                hasher.update(metadata.len().to_le_bytes());
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    hasher.update([u8::from(metadata.permissions().mode() & 0o111 != 0)]);
                }
                let mut content = Vec::new();
                File::open(&path)?
                    .take(MAX_SNAPSHOT_FILE_BYTES + 1)
                    .read_to_end(&mut content)?;
                ensure!(
                    content.len() as u64 == metadata.len(),
                    "cached file changed while reading"
                );
                hasher.update(content);
            }
        }
        Ok(())
    }
    ensure!(
        root.symlink_metadata()?.is_dir() && !root.symlink_metadata()?.file_type().is_symlink(),
        "unsafe cached checkout"
    );
    let mut hasher = Sha256::new();
    walk(root, root, 0, &mut hasher, &mut 0, &mut 0)?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn validate_revision(revision: &Path, commit: &str) -> Result<PathBuf> {
    ensure!(
        !revision.symlink_metadata()?.file_type().is_symlink(),
        "unsafe cached revision"
    );
    let receipt_path = revision.join("receipt.json");
    ensure!(
        !receipt_path.symlink_metadata()?.file_type().is_symlink(),
        "unsafe cache receipt"
    );
    let receipt: Receipt =
        serde_json::from_str(&config::read_bounded(&receipt_path, MAX_METADATA_BYTES)?)?;
    ensure!(
        receipt.commit == commit,
        "cache receipt identifies a different Git revision"
    );
    let contents = revision.join("contents");
    ensure!(
        receipt.digest == digest(&contents)?,
        "cached checkout integrity check failed"
    );
    Ok(contents)
}

pub(super) fn checkout(
    root: &Path,
    cache: &Path,
    repository: &str,
    commit: &str,
    refresh: bool,
    changed: &mut bool,
) -> Result<PathBuf> {
    validate_commit(commit)?;
    let directory = repository_directory(cache, repository)?;
    crate::config::source_path(cache, &Path::new("git").join(repository))
        .context("missing or unsafe Git repository cache")?;
    let _lock = if refresh {
        Lock::acquire(&directory.join("cache.lock"))?
    } else {
        Lock::read(&directory.join("cache.lock"))?
    };
    let revision = directory.join(commit);
    if revision.symlink_metadata().is_ok() {
        // Never silently accept or overwrite a corrupt immutable revision.
        return validate_revision(&revision, commit);
    }
    ensure!(refresh, "missing cached revision");
    let bare = crate::config::source_path(&directory, Path::new("objects.git"))
        .context("missing or unsafe Git object cache")?;
    let temporary = tempfile::tempdir_in(&directory)?;
    let contents = temporary.path().join("contents");
    fs::create_dir(&contents)?;
    let index = temporary.path().join("index");
    let isolated = |args: &[&str]| -> Result<()> {
        let mut cmd = command(root, Some(&bare));
        cmd.env("GIT_INDEX_FILE", &index)
            .env("GIT_WORK_TREE", &contents)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env_remove("GIT_CONFIG_COUNT")
            .env_remove("GIT_CONFIG_PARAMETERS")
            .args(["-c", "core.autocrlf=false", "-c", "core.symlinks=true"])
            .args(args);
        let output = process::run(&mut cmd, Duration::from_secs(EXTERNAL_GIT_TIMEOUT_SECONDS))?;
        ensure!(
            output.status.success() && !output.timed_out,
            "cannot materialize Git cache revision"
        );
        Ok(())
    };
    // Plumbing does not execute checkout hooks or initialize submodules.
    // Disabling global/system config prevents user smudge filters during extraction.
    isolated(&["read-tree", commit])?;
    isolated(&[
        "checkout-index",
        "--all",
        &format!("--prefix={}/", contents.display()),
    ])?;
    fs::remove_file(index)?;
    // Git checkout-index does not initialize gitlinks; reject these instead of partial coverage.
    let submodules = Arc::new(AtomicBool::new(false));
    let found = submodules.clone();
    let mut line = Vec::new();
    let output = process::run_with_stdout_observer(
        command(root, Some(&bare)).args(["ls-tree", "-r", "--format=%(objecttype)", commit]),
        Duration::from_secs(EXTERNAL_GIT_TIMEOUT_SECONDS),
        move |bytes| {
            for &byte in bytes {
                if byte == b'\n' {
                    if line == b"commit" {
                        found.store(true, Ordering::Relaxed);
                    }
                    line.clear();
                } else if line.len() < 16 {
                    line.push(byte);
                }
            }
        },
    )?;
    ensure!(
        output.status.success() && !output.timed_out,
        "cannot validate cached Git tree"
    );
    ensure!(
        !submodules.load(Ordering::Relaxed),
        "external repositories with submodules are unsupported"
    );
    let receipt = Receipt {
        commit: commit.into(),
        digest: digest(&contents)?,
    };
    let mut file = File::create(temporary.path().join("receipt.json"))?;
    file.write_all(&serde_json::to_vec(&receipt)?)?;
    file.sync_all()?;
    fs::rename(temporary.path(), &revision).context("cannot publish cached revision")?;
    *changed = true;
    File::open(&directory)?.sync_all()?;
    validate_revision(&revision, commit)
}
