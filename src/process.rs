use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    process::{Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub use crate::limits::MAX_CAPTURE_BYTES as OUTPUT_LIMIT;
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Capture {
    pub text: String,
    pub total_bytes: u64,
    pub truncated: bool,
}
#[derive(Debug)]
pub struct Output {
    pub status: ExitStatus,
    pub stdout: Capture,
    pub stderr: Capture,
    pub timed_out: bool,
    pub cancelled: bool,
    pub duration_ms: u64,
}
/// Shared by one scheduler invocation and its check processes only.
pub(crate) struct Cancellation {
    requested: AtomicBool,
    grace: Duration,
}
impl Cancellation {
    pub(crate) fn new(grace: Duration) -> Self {
        Self {
            requested: AtomicBool::new(false),
            grace,
        }
    }
    pub(crate) fn request(&self) {
        self.requested.store(true, Ordering::Relaxed);
    }
}

#[cfg(unix)]
fn request_termination(child: &std::process::Child) -> Result<()> {
    // Children have private groups; include descendants even after the leader exits.
    if unsafe { libc::kill(-(child.id() as i32), libc::SIGTERM) } == -1 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            return Err(error).context("cannot terminate check process group gracefully");
        }
    }
    Ok(())
}
#[cfg(not(unix))]
fn request_termination(_: &std::process::Child) -> Result<()> {
    bail!("graceful check termination requires Unix process groups")
}
#[cfg(unix)]
fn group_running(child: &std::process::Child) -> bool {
    unsafe { libc::kill(-(child.id() as i32), 0) == 0 }
}
#[cfg(not(unix))]
fn group_running(_: &std::process::Child) -> bool {
    true
}
type Observer = Box<dyn FnMut(&[u8]) + Send>;
fn drain(mut stream: impl Read, mut observer: Option<Observer>, limit: usize) -> Result<Capture> {
    let mut bytes = Vec::new();
    let mut total = 0;
    let mut buffer = [0; 8192];
    loop {
        let n = stream.read(&mut buffer)?;
        if let Some(observer) = &mut observer {
            observer(&buffer[..n]);
        }
        if n == 0 {
            break;
        }
        total += n as u64;
        bytes.extend_from_slice(&buffer[..n.min(limit - bytes.len())]);
    }
    Ok(Capture {
        text: String::from_utf8_lossy(&bytes).into_owned(),
        total_bytes: total,
        truncated: total > limit as u64,
    })
}
#[cfg(unix)]
fn terminate_tree(child: &mut std::process::Child) {
    // Each child starts its own process group. Never signal Please Behave's group.
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    let _ = child.kill();
}
#[cfg(windows)]
fn terminate_tree(child: &mut std::process::Child) {
    let _ = Command::new("taskkill")
        .args(["/F", "/T", "/PID", &child.id().to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
}
#[cfg(not(any(unix, windows)))]
fn terminate_tree(child: &mut std::process::Child) {
    let _ = child.kill();
}

pub fn run(command: &mut Command, timeout: Duration) -> Result<Output> {
    run_with_stdout_limit(command, timeout, OUTPUT_LIMIT)
}

/// Capture bounded command data without changing diagnostic or provider limits.
pub(crate) fn run_with_stdout_limit(
    command: &mut Command,
    timeout: Duration,
    stdout_limit: usize,
) -> Result<Output> {
    run_observed(command, timeout, None, None, None, None, stdout_limit)
}

pub(crate) fn run_cancellable(
    command: &mut Command,
    timeout: Duration,
    cancellation: Option<&Cancellation>,
) -> Result<Output> {
    run_observed(
        command,
        timeout,
        None,
        None,
        None,
        cancellation,
        OUTPUT_LIMIT,
    )
}

/// Observe stdout while draining it, including bytes beyond the retained capture.
/// An empty slice signals EOF. The observer must retain only bounded data.
pub fn run_with_stdout_observer(
    command: &mut Command,
    timeout: Duration,
    observer: impl FnMut(&[u8]) + Send + 'static,
) -> Result<Output> {
    run_observed(
        command,
        timeout,
        Some(Box::new(observer)),
        None,
        None,
        None,
        OUTPUT_LIMIT,
    )
}

/// Deliver stdin concurrently with output capture under the same process deadline.
pub fn run_with_input(
    command: &mut Command,
    timeout: Duration,
    input: Vec<u8>,
    observer: impl FnMut(&[u8]) + Send + 'static,
) -> Result<Output> {
    command.stdin(Stdio::piped());
    run_observed(
        command,
        timeout,
        Some(Box::new(observer)),
        Some(input),
        None,
        None,
        OUTPUT_LIMIT,
    )
}

/// Observe safe adapter diagnostics without retaining arbitrary stderr as a verdict.
pub fn run_with_input_diagnostics(
    command: &mut Command,
    timeout: Duration,
    input: Vec<u8>,
    observer: impl FnMut(&[u8]) + Send + 'static,
    diagnostics: impl FnMut(&[u8]) + Send + 'static,
) -> Result<Output> {
    command.stdin(Stdio::piped());
    run_observed(
        command,
        timeout,
        Some(Box::new(observer)),
        Some(input),
        Some(Box::new(diagnostics)),
        None,
        OUTPUT_LIMIT,
    )
}

fn run_observed(
    command: &mut Command,
    timeout: Duration,
    mut observer: Option<Observer>,
    input: Option<Vec<u8>>,
    mut diagnostics: Option<Observer>,
    cancellation: Option<&Cancellation>,
    stdout_limit: usize,
) -> Result<Output> {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let start = Instant::now();
    let mut child = command
        .spawn()
        .context("cannot start command; check executable and PATH")?;
    let input_result = input.map(|input| {
        let mut stdin = child.stdin.take().expect("piped stdin");
        let (sender, receiver) = mpsc::channel();
        thread::spawn(move || {
            let result = stdin.write_all(&input);
            drop(stdin);
            let _ = sender.send(result);
        });
        receiver
    });
    let out = child.stdout.take().unwrap();
    let err = child.stderr.take().unwrap();
    let (sender, receiver) = mpsc::channel();
    for (is_out, stream) in [
        (true, Box::new(out) as Box<dyn Read + Send>),
        (false, Box::new(err) as Box<dyn Read + Send>),
    ] {
        let sender = sender.clone();
        let observer = if is_out {
            observer.take()
        } else {
            diagnostics.take()
        };
        thread::spawn(move || {
            let limit = if is_out { stdout_limit } else { OUTPUT_LIMIT };
            let _ = sender.send((is_out, drain(stream, observer, limit)));
        });
    }
    drop(sender);
    let mut timed_out = false;
    let mut cancelling_since = None;
    let mut completed = None;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if cancelling_since.is_none() {
                    break status;
                }
                completed = Some(status);
                if !group_running(&child) {
                    break status;
                }
            }
            Ok(None) => {}
            Err(e) => {
                terminate_tree(&mut child);
                let _ = child.wait();
                return Err(e.into());
            }
        }
        if let Some(cancellation) = cancellation {
            if cancelling_since.is_none() && cancellation.requested.load(Ordering::Relaxed) {
                if let Err(error) = request_termination(&child) {
                    terminate_tree(&mut child);
                    let _ = child.wait();
                    return Err(error);
                }
                cancelling_since = Some(Instant::now());
            }
        }
        if let Some(since) = cancelling_since {
            if since.elapsed() >= cancellation.expect("cancellation requested").grace {
                terminate_tree(&mut child);
                break match completed {
                    Some(status) => status,
                    None => child.wait()?,
                };
            }
        } else if start.elapsed() >= timeout {
            timed_out = true;
            terminate_tree(&mut child);
            break child.wait()?;
        }
        thread::sleep(Duration::from_millis(10));
    };
    // Clean up background descendants too; otherwise inherited pipes could hang forever.
    #[cfg(unix)]
    terminate_tree(&mut child);
    let mut stdout = None;
    let mut stderr = None;
    for _ in 0..2 {
        match receiver.recv_timeout(Duration::from_secs(2)) {
            Ok((true, result)) => stdout = Some(result?),
            Ok((false, result)) => stderr = Some(result?),
            Err(_) => {
                terminate_tree(&mut child);
                bail!("output capture incomplete: descendant kept a pipe open");
            }
        }
    }
    if !timed_out && cancelling_since.is_none() {
        if let Some(receiver) = input_result {
            receiver
                .recv_timeout(Duration::from_secs(2))
                .context("stdin delivery incomplete")?
                .context("cannot deliver complete stdin")?;
        }
    }
    Ok(Output {
        status,
        stdout: stdout.unwrap(),
        stderr: stderr.unwrap(),
        timed_out,
        cancelled: cancelling_since.is_some(),
        duration_ms: start.elapsed().as_millis() as u64,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn larger_stdout_capture_keeps_stderr_bounded_and_reports_truncation() {
        for limit in [200_000, 199_999] {
            let output = run_with_stdout_limit(
                Command::new("sh")
                    .args(["-c", "yes x | head -c 200000; yes y | head -c 200000 >&2"]),
                Duration::from_secs(10),
                limit,
            )
            .unwrap();
            assert!(output.status.success());
            assert_eq!(output.stdout.text.len(), limit);
            assert_eq!(output.stdout.total_bytes, 200_000);
            assert_eq!(output.stdout.truncated, limit < 200_000);
            assert_eq!(output.stderr.text.len(), OUTPUT_LIMIT);
            assert_eq!(output.stderr.total_bytes, 200_000);
            assert!(output.stderr.truncated);
        }
    }
}
