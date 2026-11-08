use crate::{
    config::{Check, Project},
    process,
    report::{Item, Status},
};
use std::{
    process::{Command, Stdio},
    time::{Duration, Instant},
};
pub fn execute(project: &Project, check: &Check, execution_point: &str) -> Item {
    execute_with_context(project, check, execution_point, None)
}

pub fn execute_with_context(
    project: &Project,
    check: &Check,
    execution_point: &str,
    hook: Option<&crate::reconciliation::HookContext>,
) -> Item {
    let start = Instant::now();
    let mut item = Item::simple(&check.id, "check", Status::Error, String::new());
    item.blocking = check.blocking;
    let prepared = (|| -> anyhow::Result<_> {
        let environment = check.parameters.resolve(&project.root, &check.id)?;
        let directory = crate::config::source_path(
            &check.source_root,
            check.rule_directory.strip_prefix(&check.source_root)?,
        )?;
        anyhow::ensure!(directory.is_dir(), "Rule directory must be a directory");
        let mut command = match (&check.entrypoint, &check.command) {
            (Some(path), None) => {
                let entrypoint = crate::config::source_path(
                    &check.source_root,
                    path.strip_prefix(&check.source_root)?,
                )
                .map_err(|error| {
                    anyhow::anyhow!(
                        "Rule {}: missing or unsafe check entrypoint {}: {error:#}",
                        check.id,
                        path.display()
                    )
                })?;
                anyhow::ensure!(
                    entrypoint.is_file(),
                    "Rule {}: check entrypoint must be a regular file",
                    check.id
                );
                if let Some(interpreter) = &check.interpreter {
                    let mut command = Command::new(interpreter);
                    command.arg(entrypoint);
                    command
                } else {
                    Command::new(entrypoint)
                }
            }
            (None, Some(arguments)) if !arguments.is_empty() => {
                let arguments =
                    arguments.resolve(&project.root, &check.source_root, &check.rule_directory)?;
                let mut command = Command::new(&arguments[0]);
                command.args(&arguments[1..]);
                command
            }
            _ => anyhow::bail!("Rule {}: expected one check implementation", check.id),
        };
        if hook.is_none() {
            crate::reconciliation::configure_guard(project, &mut command)?;
        }
        Ok((environment, command))
    })();
    let (environment, mut command) = match prepared {
        Ok(prepared) => prepared,
        Err(error) => {
            item.reason = format!("{error:#}");
            item.duration_ms = start.elapsed().as_millis() as u64;
            return item;
        }
    };
    crate::parameters::configure(&mut command, &environment);
    command.env("PLEASE_RULE_DIRECTORY", &check.rule_directory);
    command.env("PLEASE_EXECUTION_POINT", execution_point);
    command.current_dir(&project.root).stdin(Stdio::null());
    if let Some(hook) = hook {
        hook.configure(&mut command);
    }
    match process::run(&mut command, Duration::from_secs(check.timeout_seconds)) {
        Ok(output) => {
            item.status = if output.timed_out || matches!(output.status.code(), None | Some(2)) {
                Status::Error
            } else if output.status.success() {
                Status::Pass
            } else {
                Status::Fail
            };
            item.reason = if output.timed_out {
                "Command timed out".into()
            } else if output.status.code().is_none() {
                "Check terminated by signal; execution could not complete".into()
            } else if item.status == Status::Error {
                "Check reported an execution error (exit 2)".into()
            } else if !matches!(output.status.code(), Some(0 | 1)) {
                format!(
                    "Unexpected check exit code {}; treated as FAIL (expected 0, 1 or 2)",
                    output.status
                )
            } else {
                if output.status.success() {
                    String::new()
                } else {
                    "Check reported a violation; inspect captured output with please status --details".into()
                }
            };
            item.exit_code = output.status.code();
            item.stdout = Some(output.stdout);
            item.stderr = Some(output.stderr);
        }
        Err(error) => item.reason = format!("{error:#}"),
    }
    item.duration_ms = start.elapsed().as_millis() as u64;
    item
}
