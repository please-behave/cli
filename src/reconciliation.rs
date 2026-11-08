//! Desired-state inspection and approved reconciliation, independent of rule execution.
use crate::{config::Project, process};
use anyhow::{Context, Result, bail, ensure};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs::{self, Permissions},
    io::{IsTerminal, Write},
    path::{Component, Path, PathBuf},
    process::Command,
    time::Duration,
};

pub const GIT_EVENTS: &[&str] = &["commit-msg", "pre-commit", "pre-push"];
const BEGIN: &str = "# BEGIN PLEASE MANAGED";
const END: &str = "# END PLEASE MANAGED";
const HEADER: &str = "#!/bin/sh\n# Please Behave hook wrapper\n";
const GUARD: &str = "PLEASE_ACTIVE_HOOK";

#[derive(Debug, Clone)]
pub struct HookInvocation {
    pub event: String,
    pub path: PathBuf,
    pub arguments: Vec<OsString>,
}

/// Validated context supplied only to deterministic checker processes.
pub struct HookContext {
    invocation: HookInvocation,
    owner: String,
}
impl HookContext {
    pub fn configure(&self, command: &mut Command) {
        command.env(GUARD, &self.owner);
        command.env("PLEASE_GIT_HOOK", &self.invocation.event);
        command.env(
            "PLEASE_GIT_HOOK_ARG_COUNT",
            self.invocation.arguments.len().to_string(),
        );
        for (index, argument) in self.invocation.arguments.iter().enumerate() {
            command.env(format!("PLEASE_GIT_HOOK_ARG_{}", index + 1), argument);
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileState {
    text: String,
    mode: u32,
}
#[derive(Debug, PartialEq, Eq)]
struct Change {
    path: PathBuf,
    before: Option<FileState>,
    after: Option<FileState>,
}
#[derive(Debug, PartialEq, Eq)]
pub struct Plan {
    directory: PathBuf,
    owner: String,
    changes: Vec<Change>,
}
impl Plan {
    pub fn synchronized(&self) -> bool {
        self.changes.is_empty()
    }
    pub fn display(&self) -> String {
        self.changes
            .iter()
            .map(|change| {
                let action = match (&change.before, &change.after) {
                    (None, _) => "Create hook",
                    (_, None) => "Remove owned hook",
                    _ => "Update hook managed section/permissions (preserve unmanaged content)",
                };
                format!("  {action}: {}\n", change.path.display())
            })
            .collect()
    }
}

fn git(root: &Path, arguments: &[&str]) -> Result<String> {
    let output = process::run(
        Command::new("git")
            .current_dir(root)
            .env("GIT_OPTIONAL_LOCKS", "0")
            .args(arguments),
        Duration::from_secs(10),
    )?;
    ensure!(
        !output.timed_out && output.status.success() && !output.stdout.truncated,
        "cannot resolve Git hook location: {}",
        output.stderr.text
    );
    Ok(output
        .stdout
        .text
        .strip_suffix('\n')
        .unwrap_or(&output.stdout.text)
        .to_owned())
}

/// Reject links at every existing component, including dangling links.
fn safe_path(path: &Path) -> Result<()> {
    ensure!(
        path.is_absolute(),
        "hook path must be absolute: {}",
        path.display()
    );
    let mut current = PathBuf::new();
    for component in path.components() {
        ensure!(
            !matches!(component, Component::ParentDir),
            "unsafe hook path: {}",
            path.display()
        );
        current.push(component);
        match current.symlink_metadata() {
            Ok(metadata) => ensure!(
                !metadata.file_type().is_symlink(),
                "symlink conflict: {}; use a real repository-local hook directory",
                current.display()
            ),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(error).with_context(|| format!("inspect {}", current.display()));
            }
        }
    }
    Ok(())
}

fn repository_identity(project: &Project) -> Result<(PathBuf, String)> {
    let common = PathBuf::from(git(
        &project.root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?)
    .canonicalize()?;
    let owner = format!(
        "{:x}",
        Sha256::digest(common.as_os_str().as_encoded_bytes())
    );
    Ok((common, owner))
}

pub(crate) fn configure_guard(project: &Project, command: &mut Command) -> Result<()> {
    let (_, owner) = repository_identity(project)?;
    command.env(GUARD, owner);
    Ok(())
}

fn location(project: &Project) -> Result<(PathBuf, String)> {
    let (common, owner) = repository_identity(project)?;
    let directory = PathBuf::from(git(
        &project.root,
        &["rev-parse", "--path-format=absolute", "--git-path", "hooks"],
    )?);
    ensure!(
        directory == common.join("hooks")
            || (directory.starts_with(&project.root) && directory != project.root),
        "ambiguous/shared hook directory {}; set a repository-local core.hooksPath or use Git's default hooks directory",
        directory.display()
    );
    safe_path(&directory)?;
    if directory.try_exists()? {
        ensure!(
            directory.is_dir(),
            "hook location is not a directory: {}",
            directory.display()
        );
    }
    Ok((directory, owner))
}

#[cfg(unix)]
fn mode(permissions: Permissions) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    permissions.mode() & 0o7777
}
#[cfg(not(unix))]
fn mode(_: Permissions) -> u32 {
    0o755
}
#[cfg(unix)]
fn permissions(mode: u32) -> Permissions {
    use std::os::unix::fs::PermissionsExt;
    Permissions::from_mode(mode)
}
#[cfg(not(unix))]
fn permissions(_: u32) -> Permissions {
    unimplemented!("Git hook reconciliation requires Unix executable permissions")
}

fn read(path: &Path) -> Result<Option<FileState>> {
    safe_path(path)?;
    let metadata = match path.symlink_metadata() {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("inspect {}", path.display())),
    };
    ensure!(
        metadata.is_file(),
        "hook conflict: {} is not a regular file",
        path.display()
    );
    Ok(Some(FileState {
        text: crate::config::read_bounded(path, crate::limits::MAX_METADATA_BYTES)?,
        mode: mode(metadata.permissions()),
    }))
}

fn block(owner: &str, event: &str) -> String {
    // Git events are fixed ASCII names, never shell source.
    format!(
        "{BEGIN}\n# Please Behave repository: {owner}\nplease run --at '{event}' --git-hook '{event}' --git-hook-path \"$0\" -- \"$@\" || exit \"$?\"\n{END}\n"
    )
}

fn desired(
    before: Option<&FileState>,
    owner: &str,
    event: &str,
    required: bool,
) -> Result<Option<FileState>> {
    let Some(before) = before else {
        return Ok(required.then(|| FileState {
            text: format!("{HEADER}{}", block(owner, event)),
            mode: 0o755,
        }));
    };
    let text = &before.text;
    let begins: Vec<_> = text.match_indices(BEGIN).collect();
    let ends: Vec<_> = text.match_indices(END).collect();
    // Any Please Behave ownership claim must use exactly one well-formed section.
    let managed = !begins.is_empty()
        || !ends.is_empty()
        || text.contains("Please Behave repository:")
        || text.contains("Please Behave hook wrapper");
    if !managed && !required {
        return Ok(Some(before.clone()));
    }
    let shebang = "#!/bin/sh\n";
    ensure!(
        text.starts_with(shebang),
        "hook conflict: safe composition requires an LF-terminated #!/bin/sh shebang"
    );
    let (start, end) = if managed {
        ensure!(
            begins.len() == 1 && ends.len() == 1,
            "hook conflict: malformed or duplicate ownership markers"
        );
        let start = begins[0].0;
        let end = ends[0].0 + END.len();
        ensure!(
            start > 0
                && text.as_bytes()[start - 1] == b'\n'
                && start < end
                && text.as_bytes().get(end) == Some(&b'\n'),
            "hook conflict: malformed ownership markers"
        );
        let section = &text[start..end + 1];
        let owner_line = format!("# Please Behave repository: {owner}\n");
        ensure!(
            section
                .lines()
                .filter(|line| line.starts_with("# Please Behave repository:"))
                .count()
                == 1
                && section.starts_with(&format!("{BEGIN}\n{owner_line}")),
            "hook conflict: missing or foreign repository ownership; inspect the hook manually"
        );
        ensure!(
            text.matches("# Please Behave repository:").count() == 1,
            "hook conflict: conflicting ownership claims"
        );
        // Only the insertion point we own is composable without interpreting shell.
        ensure!(
            start == shebang.len() || (text.starts_with(HEADER) && start == HEADER.len()),
            "hook conflict: managed section must immediately follow the shebang or owned header"
        );
        (start, end + 1)
    } else {
        ensure!(
            before.mode & 0o111 != 0,
            "hook conflict: unmanaged hook is not executable; enable it manually before composition"
        );
        (shebang.len(), shebang.len())
    };
    let replacement = if required {
        block(owner, event)
    } else {
        String::new()
    };
    let whole = text.starts_with(HEADER) && start == HEADER.len() && end == text.len();
    if whole && !required {
        return Ok(None);
    }
    let result = format!("{}{}{}", &text[..start], replacement, &text[end..]);
    ensure!(
        result.len() as u64 <= crate::limits::MAX_METADATA_BYTES,
        "composed hook exceeds metadata byte limit"
    );
    Ok(Some(FileState {
        text: result,
        mode: if required {
            before.mode | 0o100
        } else {
            before.mode
        },
    }))
}

/// Effective assignments already include Rule Set inheritance and imported Rules.
fn hook_required(project: &Project, event: &str) -> bool {
    project
        .effective
        .rules
        .values()
        .any(|rule| rule.execution_points.contains(event))
}

pub fn inspect(project: &Project) -> Result<Plan> {
    let (directory, owner) = location(project)?;
    let mut changes = Vec::new();
    for event in GIT_EVENTS {
        let path = directory.join(event);
        let before = read(&path)?;
        let after = desired(before.as_ref(), &owner, event, hook_required(project, event))
            .with_context(|| format!("inspect hook {}; no changes applied; resolve conflict manually, then run please sync", path.display()))?;
        if before != after {
            changes.push(Change {
                path,
                before,
                after,
            });
        }
    }
    Ok(Plan {
        directory,
        owner,
        changes,
    })
}

fn stage(directory: &Path, state: &FileState) -> Result<tempfile::NamedTempFile> {
    let mut file = tempfile::Builder::new()
        .prefix(".please-hook-")
        .tempfile_in(directory)?;
    file.write_all(state.text.as_bytes())?;
    file.as_file().set_permissions(permissions(state.mode))?;
    file.as_file().sync_all()?;
    Ok(file)
}

/// Apply only a still-current approved plan. Each replacement is atomic, the set is not.
pub fn apply(project: &Project, plan: &Plan) -> Result<()> {
    apply_operations(project, plan, &mut |_| Ok(()))
}

fn apply_operations(
    project: &Project,
    plan: &Plan,
    before_write: &mut dyn FnMut(usize) -> Result<()>,
) -> Result<()> {
    ensure!(
        inspect(project)? == *plan,
        "hook state changed after planning; no changes applied; rerun please sync"
    );
    if plan.synchronized() {
        return Ok(());
    }
    let mut directories = Vec::new();
    let mut ancestor = plan.directory.as_path();
    while !ancestor.try_exists()? {
        directories.push(ancestor.to_path_buf());
        ancestor = ancestor
            .parent()
            .context("hook directory has no existing ancestor")?;
    }
    let mut applied = 0;
    let mut created_directories = Vec::new();
    let result = (|| -> Result<()> {
        for directory in directories.iter().rev() {
            fs::create_dir(directory)
                .with_context(|| format!("create hook directory {}", directory.display()))?;
            created_directories.push(directory.clone());
        }
        // Prepare every replacement before touching any hook. Tempfiles clean up on errors.
        let staged: Vec<_> = plan
            .changes
            .iter()
            .map(|change| {
                change
                    .after
                    .as_ref()
                    .map(|state| {
                        stage(&plan.directory, state)
                            .with_context(|| format!("stage hook {}", change.path.display()))
                    })
                    .transpose()
            })
            .collect::<Result<_>>()?;
        ensure!(
            inspect(project)? == *plan,
            "hook state changed while staging; rerun please sync"
        );
        for (change, file) in plan.changes.iter().zip(staged) {
            before_write(applied)
                .with_context(|| format!("apply hook {}", change.path.display()))?;
            ensure!(
                read(&change.path)? == change.before,
                "hook changed concurrently: {}",
                change.path.display()
            );
            if let Some(file) = file {
                file.persist(&change.path)
                    .map_err(|error| error.error)
                    .with_context(|| format!("replace hook {}", change.path.display()))?;
            } else {
                fs::remove_file(&change.path)
                    .with_context(|| format!("remove hook {}", change.path.display()))?;
            }
            applied += 1;
        }
        fs::File::open(&plan.directory)?.sync_all()?;
        ensure!(inspect(project)?.synchronized(), "hook verification failed");
        Ok(())
    })();
    if let Err(error) = result {
        let mut failures = Vec::new();
        for change in plan.changes[..applied].iter().rev() {
            let rollback = (|| -> Result<()> {
                ensure!(
                    read(&change.path)? == change.after,
                    "concurrent edit; preserve file"
                );
                if let Some(before) = &change.before {
                    stage(&plan.directory, before)?
                        .persist(&change.path)
                        .map_err(|error| error.error)?;
                } else {
                    fs::remove_file(&change.path)?;
                }
                Ok(())
            })();
            if let Err(error) = rollback {
                failures.push(format!("{}: {error:#}", change.path.display()));
            }
        }
        for directory in created_directories.iter().rev() {
            let _ = fs::remove_dir(directory);
        }
        bail!(
            "reconciliation failed in {}: {error:#}; {applied} earlier operations applied; rollback {}; inspect hooks and rerun please sync",
            plan.directory.display(),
            if failures.is_empty() {
                "completed".into()
            } else {
                failures.join("; ")
            }
        );
    }
    Ok(())
}

/// Shared lifecycle. Approval is an injected boundary for terminals and offline tests.
pub fn reconcile(
    project: &Project,
    approve: &mut dyn FnMut(&Plan) -> Result<bool>,
) -> Result<bool> {
    let plan = inspect(project)?;
    if !plan.synchronized() {
        ensure!(
            approve(&plan)?,
            "reconciliation declined; no changes applied; run please sync when ready"
        );
        apply(project, &plan)?;
    }
    Ok(!plan.synchronized())
}

fn approve_plan(plan: &Plan, auto_approve: bool) -> Result<bool> {
    eprint!(
        "Planned changes:\n{}",
        crate::terminal::clean(&plan.display())
    );
    if auto_approve {
        return Ok(true);
    }
    ensure!(
        std::io::stdin().is_terminal() && std::io::stderr().is_terminal(),
        "reconciliation requires confirmation in a terminal; run please sync interactively; no changes applied"
    );
    eprint!("Apply these changes? [y/N] ");
    std::io::stderr().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    Ok(answer.trim().eq_ignore_ascii_case("y") || answer.trim().eq_ignore_ascii_case("yes"))
}

pub fn prepare(
    project: &Project,
    invocation: Option<HookInvocation>,
    point: Option<&str>,
    auto_approve: bool,
) -> Result<(Option<HookContext>, bool)> {
    let (_, owner) = repository_identity(project)?;
    ensure!(
        std::env::var_os(GUARD).as_deref() != Some(std::ffi::OsStr::new(&owner)),
        "recursive Please Behave hook execution blocked; run please sync manually outside the hook"
    );
    // Direct explicit points need recursion protection, but no hook state.
    if invocation.is_none() && point.is_some_and(|point| !GIT_EVENTS.contains(&point)) {
        return Ok((None, false));
    }
    if let Some(invocation) = invocation {
        let plan = inspect(project)?;
        ensure!(
            GIT_EVENTS.contains(&invocation.event.as_str()),
            "unsupported hook invocation context"
        );
        ensure!(
            point == Some(invocation.event.as_str()) && hook_required(project, &invocation.event),
            "hook invocation point must match its Git event and require enabled Rules; run please sync manually"
        );
        let path = if invocation.path.is_absolute() {
            invocation.path.clone()
        } else {
            project.root.join(&invocation.path)
        };
        safe_path(&path)?;
        ensure!(
            path.canonicalize()? == plan.directory.join(&invocation.event).canonicalize()?,
            "hook invocation path does not match this repository"
        );
        ensure!(
            plan.synchronized(),
            "Git hooks require synchronization; run please sync manually outside the hook; no checks executed"
        );
        Ok((Some(HookContext { invocation, owner }), false))
    } else {
        let changed = reconcile(project, &mut |plan| approve_plan(plan, auto_approve))?;
        Ok((None, changed))
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn project() -> tempfile::TempDir {
        let root = tempfile::tempdir().unwrap();
        assert!(
            Command::new("git")
                .args(["init", "--quiet"])
                .current_dir(root.path())
                .status()
                .unwrap()
                .success()
        );
        crate::config::init(root.path()).unwrap();
        fs::write(
            root.path().join(".please/config.toml"),
            "project = 'My_App'\nmax_display_files = 5\n",
        )
        .unwrap();
        let path = root.path().join(".please/checks/always-ok/rule.toml");
        fs::write(
            &path,
            fs::read_to_string(&path)
                .unwrap()
                .replace("[\"pre-push\"]", "['pre-commit', 'pre-push']"),
        )
        .unwrap();
        root
    }

    #[test]
    fn partial_write_failure_rolls_back_and_reports_operation() {
        let root = project();
        let project = crate::config::load(root.path()).unwrap();
        let plan = inspect(&project).unwrap();
        let error = apply_operations(&project, &plan, &mut |index| {
            ensure!(index == 0, "injected replacement failure");
            Ok(())
        })
        .unwrap_err();
        let diagnostic = format!("{error:#}");
        assert!(
            diagnostic.contains("pre-push")
                && diagnostic.contains("1 earlier operations applied; rollback completed"),
            "{diagnostic}"
        );
        assert!(!plan.directory.join("pre-commit").exists());
        assert!(!plan.directory.join("pre-push").exists());
        assert!(!fs::read_dir(&plan.directory).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".please-hook-")
        }));
        apply(&project, &inspect(&project).unwrap()).unwrap();
        assert!(inspect(&project).unwrap().synchronized());
    }

    #[test]
    fn rollback_does_not_overwrite_concurrent_custom_edits() {
        let root = project();
        let project = crate::config::load(root.path()).unwrap();
        let plan = inspect(&project).unwrap();
        let path = plan.directory.join("pre-commit");
        let error = apply_operations(&project, &plan, &mut |index| {
            if index == 1 {
                fs::write(&path, "#!/bin/sh\n# concurrent edit\n")?;
                bail!("injected failure");
            }
            Ok(())
        })
        .unwrap_err();
        assert!(format!("{error:#}").contains("concurrent edit; preserve file"));
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "#!/bin/sh\n# concurrent edit\n"
        );
    }

    #[test]
    fn filesystem_permission_failure_applies_no_hooks() {
        let root = project();
        let project = crate::config::load(root.path()).unwrap();
        let plan = inspect(&project).unwrap();
        fs::set_permissions(&plan.directory, permissions(0o555)).unwrap();
        let result = apply(&project, &plan);
        fs::set_permissions(&plan.directory, permissions(0o755)).unwrap();
        // Root can bypass directory mode; CI normally runs unprivileged.
        if unsafe { libc::geteuid() } != 0 {
            let diagnostic = format!("{:#}", result.unwrap_err());
            assert!(
                diagnostic.contains("stage hook")
                    && diagnostic.contains("0 earlier operations applied")
            );
            assert!(!plan.directory.join("pre-commit").exists());
            assert!(!plan.directory.join("pre-push").exists());
        }
    }
}
