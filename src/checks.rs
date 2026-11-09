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

/// Events stay on the coordinator thread; workers only return their result.
pub(crate) enum Event<'a> {
    Queued(usize, Option<&'a str>),
    Started(usize),
    Finished(usize, &'a Item),
    Tick,
}

pub(crate) fn schedule(
    checks: &[&Check],
    max_parallel: std::num::NonZeroUsize,
    fail_fast: bool,
    execute: impl Fn(&Check) -> Item + Sync,
    progress: &mut dyn FnMut(Event<'_>),
) -> Vec<Item> {
    use std::{
        collections::{BTreeMap, BTreeSet},
        sync::mpsc,
        thread,
    };
    // Notify even during unwinding. The coordinator joins every worker, so a
    // panic becomes a per-check ERROR rather than propagating out of the scope.
    struct Done(mpsc::Sender<usize>, usize);
    impl Drop for Done {
        fn drop(&mut self) {
            let _ = self.0.send(self.1);
        }
    }
    let error = |index: usize, reason: String, elapsed: Duration| {
        let mut item = Item::simple(&checks[index].id, "check", Status::Error, reason);
        item.blocking = checks[index].blocking;
        item.duration_ms = elapsed.as_millis() as u64;
        item
    };
    thread::scope(|scope| {
        let (sender, receiver) = mpsc::channel();
        let mut pending: BTreeSet<_> = (0..checks.len()).collect();
        let mut running = BTreeMap::new();
        let mut resources = BTreeSet::new();
        let mut waiting = BTreeSet::new();
        let mut results: Vec<Option<Item>> = (0..checks.len()).map(|_| None).collect();
        for index in &pending {
            progress(Event::Queued(*index, None));
        }
        let mut stopped = false;
        while !pending.is_empty() || !running.is_empty() {
            if stopped {
                for index in std::mem::take(&mut pending) {
                    let mut item = Item::simple(
                        &checks[index].id,
                        "check",
                        Status::NotRun,
                        "Fail-fast triggered; no command started".into(),
                    );
                    item.blocking = checks[index].blocking;
                    progress(Event::Finished(index, &item));
                    results[index] = Some(item);
                }
            }
            // A blocked resource never claims a slot; scan past it.
            while !stopped && running.len() < max_parallel.get() {
                let Some(index) = pending.iter().copied().find(|&index| {
                    checks[index]
                        .resource
                        .as_deref()
                        .is_none_or(|r| !resources.contains(r))
                }) else {
                    break;
                };
                pending.remove(&index);
                waiting.remove(&index);
                let started = Instant::now();
                let done = Done(sender.clone(), index);
                let execute = &execute;
                progress(Event::Started(index));
                match thread::Builder::new().spawn_scoped(scope, move || {
                    let _done = done;
                    execute(checks[index])
                }) {
                    Ok(handle) => {
                        if let Some(resource) = &checks[index].resource {
                            resources.insert(resource.as_str());
                        }
                        running.insert(index, (handle, started));
                    }
                    Err(cause) => {
                        let item = error(
                            index,
                            format!("Cannot start check worker: {cause}"),
                            started.elapsed(),
                        );
                        stopped |= fail_fast && crate::report::prevents_success(&item);
                        progress(Event::Finished(index, &item));
                        results[index] = Some(item);
                    }
                }
            }
            for &index in &pending {
                let resource = checks[index]
                    .resource
                    .as_deref()
                    .filter(|r| resources.contains(r));
                if resource.is_some() && waiting.insert(index) {
                    progress(Event::Queued(index, resource));
                } else if resource.is_none() && waiting.remove(&index) {
                    progress(Event::Queued(index, None));
                }
            }
            if running.is_empty() {
                continue;
            }
            match receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(index) => {
                    // Failed thread creation can also drop its notification.
                    let Some((handle, started)) = running.remove(&index) else {
                        continue;
                    };
                    let item = handle.join().unwrap_or_else(|_| {
                        error(
                            index,
                            "Check worker terminated unexpectedly".into(),
                            started.elapsed(),
                        )
                    });
                    if let Some(resource) = &checks[index].resource {
                        resources.remove(resource.as_str());
                    }
                    stopped |= fail_fast && crate::report::prevents_success(&item);
                    progress(Event::Finished(index, &item));
                    results[index] = Some(item);
                }
                Err(mpsc::RecvTimeoutError::Timeout) => progress(Event::Tick),
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    unreachable!("coordinator retains a sender")
                }
            }
        }
        results.into_iter().map(Option::unwrap).collect()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        collections::BTreeMap,
        path::PathBuf,
        sync::{Mutex, mpsc},
        thread,
    };

    fn check(id: &str, resource: Option<&str>) -> Check {
        Check {
            id: id.into(),
            name: id.into(),
            description: id.into(),
            enabled: true,
            blocking: true,
            resource: resource.map(str::to_owned),
            entrypoint: None,
            command: None,
            source_root: PathBuf::new(),
            rule_directory: PathBuf::new(),
            interpreter: None,
            timeout_seconds: 1,
            run_at: vec![],
            parameters: crate::parameters::Binding::local(BTreeMap::new()),
        }
    }

    #[test]
    fn scheduler_bounds_workers_skips_waiters_and_preserves_order() {
        let checks = [
            check("a", Some("shared")),
            check("b", Some("shared")),
            check("c", Some("Shared")),
            check("d", None),
        ];
        let checks: Vec<_> = checks.iter().collect();
        let (started, starts) = mpsc::channel();
        let channels: Vec<_> = (0..4).map(|_| mpsc::channel()).collect();
        let releases: Vec<_> = channels.iter().map(|(s, _)| s.clone()).collect();
        let waits: Vec<_> = channels.into_iter().map(|(_, r)| Mutex::new(r)).collect();
        thread::scope(|scope| {
            let handle = scope.spawn(|| {
                schedule(
                    &checks,
                    2.try_into().unwrap(),
                    false,
                    |check| {
                        let index = usize::from(check.id.as_bytes()[0] - b'a');
                        started.send(index).unwrap();
                        waits[index]
                            .lock()
                            .unwrap()
                            .recv_timeout(Duration::from_secs(5))
                            .unwrap();
                        Item::simple(&check.id, "check", Status::Pass, String::new())
                    },
                    &mut |_| {},
                )
            });
            let mut first = [
                starts.recv_timeout(Duration::from_secs(5)).unwrap(),
                starts.recv_timeout(Duration::from_secs(5)).unwrap(),
            ];
            first.sort();
            assert_eq!(first, [0, 2]); // b cannot consume c's execution slot.
            assert!(starts.try_recv().is_err());
            releases[2].send(()).unwrap();
            assert_eq!(starts.recv_timeout(Duration::from_secs(5)).unwrap(), 3);
            releases[0].send(()).unwrap();
            assert_eq!(starts.recv_timeout(Duration::from_secs(5)).unwrap(), 1);
            releases[1].send(()).unwrap();
            releases[3].send(()).unwrap();
            let results = handle.join().unwrap();
            assert_eq!(
                results.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
                ["a", "b", "c", "d"]
            );
        });
    }

    #[test]
    fn worker_panics_release_resources_and_do_not_stop_checks() {
        let checks = [check("a", Some("shared")), check("b", Some("shared"))];
        let checks: Vec<_> = checks.iter().collect();
        let results = schedule(
            &checks,
            4.try_into().unwrap(),
            false,
            |check| {
                assert_ne!(check.id, "a", "simulated worker failure");
                Item::simple(&check.id, "check", Status::Pass, String::new())
            },
            &mut |_| {},
        );
        assert_eq!(results[0].status, Status::Error);
        assert!(results[0].reason.contains("worker terminated"));
        assert_eq!(results[1].status, Status::Pass);
    }
    #[test]
    fn fail_fast_drains_running_workers_without_starting_pending_checks() {
        for second_status in [Status::Pass, Status::Fail] {
            let checks = [
                check("a", Some("shared")),
                check("b", None),
                check("c", Some("shared")),
                check("d", None),
            ];
            let checks: Vec<_> = checks.iter().collect();
            let (started, starts) = mpsc::channel();
            let (finished, finishes) = mpsc::channel();
            let channels: Vec<_> = (0..2).map(|_| mpsc::channel()).collect();
            let releases: Vec<_> = channels.iter().map(|(s, _)| s.clone()).collect();
            let waits: Vec<_> = channels.into_iter().map(|(_, r)| Mutex::new(r)).collect();
            thread::scope(|scope| {
                let handle = scope.spawn(|| {
                    schedule(
                        &checks,
                        2.try_into().unwrap(),
                        true,
                        |check| {
                            let index = usize::from(check.id.as_bytes()[0] - b'a');
                            started.send(index).unwrap();
                            waits[index]
                                .lock()
                                .unwrap()
                                .recv_timeout(Duration::from_secs(5))
                                .unwrap();
                            Item::simple(
                                &check.id,
                                "check",
                                if index == 0 {
                                    Status::Fail
                                } else {
                                    second_status
                                },
                                "completed".into(),
                            )
                        },
                        &mut |event| {
                            if let Event::Finished(index, _) = event {
                                finished.send(index).unwrap();
                            }
                        },
                    )
                });
                let mut first = [
                    starts.recv_timeout(Duration::from_secs(5)).unwrap(),
                    starts.recv_timeout(Duration::from_secs(5)).unwrap(),
                ];
                first.sort();
                assert_eq!(first, [0, 1]);
                releases[0].send(()).unwrap();
                assert_eq!(finishes.recv_timeout(Duration::from_secs(5)).unwrap(), 0);
                // Observe the stopped pending work before allowing b to finish.
                assert_eq!(finishes.recv_timeout(Duration::from_secs(5)).unwrap(), 2);
                assert_eq!(finishes.recv_timeout(Duration::from_secs(5)).unwrap(), 3);
                assert!(starts.try_recv().is_err());
                releases[1].send(()).unwrap();
                let results = handle.join().unwrap();
                assert_eq!(
                    results.iter().map(|r| r.status).collect::<Vec<_>>(),
                    [Status::Fail, second_status, Status::NotRun, Status::NotRun]
                );
                assert!(results[2].reason.contains("Fail-fast"));
                assert!(starts.try_recv().is_err());
            });
        }
    }

    #[test]
    fn fail_fast_worker_error_stops_even_an_advisory_check() {
        let mut checks = [check("a", None), check("b", None)];
        checks[0].blocking = false;
        let checks: Vec<_> = checks.iter().collect();
        let results = schedule(
            &checks,
            1.try_into().unwrap(),
            true,
            |check| {
                assert_ne!(check.id, "a", "simulated worker failure");
                Item::simple(&check.id, "check", Status::Pass, String::new())
            },
            &mut |_| {},
        );
        assert_eq!(results[0].status, Status::Error);
        assert_eq!(results[1].status, Status::NotRun);
        assert!(!results[0].blocking);
    }
}
