//! RPT-001: readable names, interactive progress and concise execution output.
mod support;
use clap::Parser;
use please::{
    Progress,
    cli::{Cli, ProgressMode},
    config,
    provider::{Provider, Request},
    report::{self, Status},
    terminal,
};
use std::fs;
use tempfile::TempDir;

struct Never;
impl Provider for Never {
    fn review(&self, _: &Request<'_>) -> anyhow::Result<String> {
        panic!("no provider expected")
    }
}
fn project() -> TempDir {
    let root = tempfile::tempdir().unwrap();
    assert!(
        support::command("git")
            .args(["init", "--quiet"])
            .current_dir(root.path())
            .status()
            .unwrap()
            .success()
    );
    please::init::init(root.path()).unwrap();
    support::configure_adapter(root.path());
    support::write(root.path(), ".gitignore", ".please/reports/\n");
    support::check(root.path(), "first", &["check"], "exit 0");
    support::check(root.path(), "second", &["check"], "exit 0");
    root
}
fn named(root: &TempDir, id: &str, name: &str) {
    let path = root.path().join(format!(".please/checks/{id}/rule.toml"));
    let text = fs::read_to_string(&path).unwrap();
    fs::write(
        path,
        format!("name = {}\n{text}", serde_json::to_string(name).unwrap()),
    )
    .unwrap();
}
#[test]
fn names_are_saved_without_changing_identity_and_disambiguated_consistently() {
    let root = project();
    named(&root, "first", "Validation");
    named(&root, "second", "Validation");
    let report = please::verify_project_at(
        config::load(root.path()).unwrap(),
        &Never,
        please::VerificationMode::AllRules,
        Some("check"),
        &mut |_| {},
    )
    .unwrap();
    let names = report.plan.display_names();
    assert_eq!(names["first"], "Validation [local]");
    assert_eq!(names["second"], "Validation [local] #2");
    assert_eq!(report.plan.required_ids, ["first", "second"]);
    assert_eq!(
        report
            .results
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
    let saved = report::read(root.path()).unwrap();
    assert_eq!(saved.plan.display_names(), names);
    assert!(terminal::render(&saved, false).contains("Validation [local] #2"));
    assert!(terminal::render_details(&saved, false).contains("first · PASS"));
    // Existing reports without optional display metadata remain readable.
    let path = root.path().join(".please/reports/last.json");
    let mut old: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    for rule in old["plan"]["scheduled"].as_array_mut().unwrap() {
        rule.as_object_mut().unwrap().remove("name");
    }
    fs::write(path, serde_json::to_vec(&old).unwrap()).unwrap();
    assert_eq!(
        report::read(root.path()).unwrap().plan.display_names()["first"],
        "first"
    );
}
#[test]
fn invalid_names_are_rejected_including_disabled_rules() {
    for name in [
        "".into(),
        " leading".into(),
        "trailing ".into(),
        "two\nlines".into(),
        "escape\u{1b}".into(),
        "x".repeat(81),
    ] {
        let root = project();
        named(&root, "first", &name);
        let path = root.path().join(".please/checks/first/rule.toml");
        fs::write(
            &path,
            fs::read_to_string(&path)
                .unwrap()
                .replace("enabled = true", "enabled = false"),
        )
        .unwrap();
        assert!(config::load(root.path()).is_err(), "accepted {name:?}");
    }
}
#[test]
fn progress_modes_are_explicit_and_auto_respects_the_output_environment() {
    for mode in ["auto", "plain", "tty"] {
        assert!(Cli::try_parse_from(["please", "run", "--progress", mode]).is_ok());
    }
    assert!(Cli::try_parse_from(["please", "run", "--progress=unknown"]).is_err());
    assert!(Cli::try_parse_from(["please", "init", "--progress=plain"]).is_err());
    assert!(terminal::progress_redraw(
        ProgressMode::Auto,
        true,
        false,
        false
    ));
    for (tty, ci, dumb) in [
        (false, false, false),
        (true, true, false),
        (true, false, true),
    ] {
        assert!(!terminal::progress_redraw(
            ProgressMode::Auto,
            tty,
            ci,
            dumb
        ));
    }
    assert!(!terminal::progress_redraw(
        ProgressMode::Plain,
        true,
        false,
        false
    ));
    assert!(!terminal::progress_redraw(
        ProgressMode::Tty,
        false,
        true,
        true
    ));
}
#[test]
fn concise_cli_keeps_diagnostics_and_avoids_duplicate_results() {
    let root = project();
    named(&root, "first", "First check");
    for mode in ["auto", "plain", "tty"] {
        let output = support::command(env!("CARGO_BIN_EXE_please"))
            .args(["run", "--at", "check", "--progress", mode])
            .current_dir(root.path())
            .env("NO_COLOR", "1")
            .env("CI", "true")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let progress = String::from_utf8(output.stderr).unwrap();
        let summary = String::from_utf8(output.stdout).unwrap();
        assert!(!progress.contains('\u{1b}') && !progress.contains('\r'));
        assert!(
            progress.contains("RUNNING")
                && progress.contains("First check")
                && progress.contains("Test check")
        );
        assert_eq!(progress.matches("PASS").count(), 2);
        assert_eq!(summary.lines().count(), 1);
        assert!(summary.starts_with("PASS · 2 rules ·"));
        for unwanted in [
            "Command exited",
            "exit status",
            "Saving report",
            "Working files",
            "TYPE",
            "SUMMARY",
            "Report:",
            "no provider call",
        ] {
            assert!(
                !format!("{progress}{summary}").contains(unwanted),
                "found {unwanted}"
            );
        }
    }
    support::check(root.path(), "second", &["check"], "exit 2");
    let output = support::command(env!("CARGO_BIN_EXE_please"))
        .args(["run", "--at", "check"])
        .current_dir(root.path())
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("status 2"));
    assert!(String::from_utf8_lossy(&output.stdout).starts_with("ERROR · 2 rules · 1 errors ·"));
}
#[test]
fn terminal_rows_are_bounded_and_success_replaces_running_state() {
    let mut renderer = terminal::ProgressRenderer::new(true, false, 60, 24);
    let description = "Very long description ".repeat(20);
    let start = Progress::RuleStarted {
        number: 1,
        total: 1,
        kind: "check",
        id: "id",
        name: "Friendly name",
        description: &description,
    };
    let first = renderer.render(&start);
    let mut item = report::Item::simple(
        "id",
        "check",
        Status::Pass,
        "Command exited: exit status 0".into(),
    );
    item.duration_ms = 1234;
    let finished = renderer.render(&Progress::RuleFinished {
        number: 1,
        total: 1,
        item: &item,
        name: "Friendly name",
        description: &description,
    });
    assert!(first.contains("RUNNING"));
    assert!(finished.starts_with("\u{1b}[2A\r\u{1b}[2K"));
    assert!(finished.contains("PASS") && finished.contains("1.2 s"));
    assert!(!finished.contains("RUNNING") && !finished.contains("Command exited"));
    for text in [&first, &finished] {
        let row = text.split("\u{1b}[2K").last().unwrap().trim_end();
        assert!(
            row.chars()
                .map(|c| if c.is_ascii() { 1 } else { 2 })
                .sum::<usize>()
                < 60
        );
    }
    assert!(
        renderer
            .render(&Progress::Phase("Saving report"))
            .is_empty()
    );
}
#[test]
fn empty_execution_and_integrity_failures_do_not_claim_rule_success() {
    let root = project();
    let mut report = please::verify_project_at(
        config::load(root.path()).unwrap(),
        &Never,
        please::VerificationMode::AllRules,
        Some("commit-msg"),
        &mut |_| {},
    )
    .unwrap();
    assert!(terminal::render_outcome(&report, 5, false).contains("No rules executed"));
    report.results.push(report::Item::simple(
        "please-tree-stability",
        "integrity",
        Status::Error,
        "Working files changed".into(),
    ));
    report.overall = Status::Error;
    let text = terminal::render_outcome(&report, 5, false);
    assert!(text.contains("Working files changed") && text.starts_with("ERROR"));
}

#[cfg(unix)]
fn terminal_run(github_actions: bool) -> (String, String) {
    use std::{io::Read, os::fd::FromRawFd, process::Stdio};
    let root = project();
    named(&root, "first", "First check");
    let mut master = 0;
    let mut slave = 0;
    let mut size = libc::winsize {
        ws_row: 24,
        ws_col: 100,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // The slave is the child's stderr terminal; reading the master observes the UI.
    assert_eq!(
        unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                // libc declares this pointer mutable on macOS and const on Linux.
                &raw mut size,
            )
        },
        0
    );
    let mut reader = unsafe { fs::File::from_raw_fd(master) };
    let writer = unsafe { fs::File::from_raw_fd(slave) };
    let mut command = support::command(env!("CARGO_BIN_EXE_please"));
    command.args(["run", "--at", "check"]);
    if github_actions {
        command.env("GITHUB_ACTIONS", "true").arg("--progress=tty");
    } else {
        command.env_remove("GITHUB_ACTIONS");
    }
    let child = command
        .current_dir(root.path())
        .env_remove("CI")
        .env("TERM", "xterm")
        .env("NO_COLOR", "1")
        .stdout(Stdio::piped())
        .stderr(writer)
        .spawn()
        .unwrap();
    // Command retains its configured Stdio handles after spawn. Close the
    // parent's slave descriptor so the reader can observe EOF when the child exits.
    drop(command);
    // Drain while the slave is open: macOS can discard unread terminal output
    // when its final slave descriptor closes.
    let reading = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => bytes.extend_from_slice(&buffer[..n]),
                Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
                Err(error) => panic!("PTY read failed: {error}"),
            }
        }
        bytes
    });
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success());
    let bytes = reading.join().unwrap();
    let raw = String::from_utf8(bytes).unwrap();
    (raw, String::from_utf8(output.stdout).unwrap())
}

#[cfg(unix)]
#[test]
fn github_actions_terminal_stays_append_only_with_explicit_tty() {
    let (raw, summary) = terminal_run(true);
    // A PTY translates ordinary newlines to CRLF; only escape sequences
    // or bare carriage returns would indicate interactive output here.
    assert!(!raw.contains('\u{1b}'), "{raw:?}");
    assert!(!raw.replace("\r\n", "\n").contains('\r'), "{raw:?}");
    assert!(raw.contains("RUNNING") && raw.matches("PASS").count() == 2);
    assert!(summary.starts_with("PASS · 2 rules"));
}

#[cfg(unix)]
#[test]
fn real_terminal_finishes_with_one_visible_row_per_check() {
    let (raw, summary) = terminal_run(false);
    assert!(raw.contains("\u{1b}[2A"), "{raw:?}");
    // Interpret the small cursor-control vocabulary used by progress, independently
    // of the renderer: the final screen contains two completed rows, no starts.
    let mut rows = vec![String::new()];
    let mut row = 0usize;
    let chars: Vec<_> = raw.chars().collect();
    let mut cursor = 0;
    while cursor < chars.len() {
        match chars[cursor] {
            '\u{1b}' => {
                assert_eq!(chars[cursor + 1], '[');
                cursor += 2;
                let mut digits = String::new();
                while chars[cursor].is_ascii_digit() {
                    digits.push(chars[cursor]);
                    cursor += 1;
                }
                match chars[cursor] {
                    'A' => row = row.saturating_sub(digits.parse().unwrap()),
                    'K' => rows[row].clear(),
                    other => panic!("unexpected cursor control: {other}"),
                }
            }
            '\n' => {
                row += 1;
                if row == rows.len() {
                    rows.push(String::new());
                }
            }
            '\r' => {}
            c => rows[row].push(c),
        }
        cursor += 1;
    }
    let final_rows: Vec<_> = rows.iter().filter(|line| !line.is_empty()).collect();
    assert_eq!(final_rows.len(), 3, "{raw:?}");
    assert!(
        final_rows
            .iter()
            .skip(1)
            .all(|line| line.contains("PASS") && !line.contains("RUNNING"))
    );
    assert!(final_rows[1].contains("First check"));
    assert!(summary.starts_with("PASS · 2 rules"));
}

#[test]
fn large_review_batches_use_bounded_output_without_rewriting_scrollback() {
    let mut renderer = terminal::ProgressRenderer::new(true, false, 80, 10);
    let batch = renderer.render(&Progress::ReviewBatch {
        count: 20,
        calls: 1,
        timeout_seconds: 180,
    });
    assert_eq!(batch, "Reviews: 20 · 1 provider call\n");
    assert_eq!(
        terminal::render_progress(
            &Progress::ReviewBatch {
                count: 20,
                calls: 2,
                timeout_seconds: 180,
            },
            false,
        ),
        "Reviews: 20 · 2 provider calls\n"
    );
    let row = renderer.render(&Progress::RuleStarted {
        number: 1,
        total: 20,
        kind: "review",
        id: "review",
        name: "Review",
        description: "Inspect evidence",
    });
    assert!(row.contains("RUNNING") && row.lines().count() <= 8);
}

#[test]
fn elapsed_ticks_waiting_states_and_short_boards_are_bounded() {
    let now = std::time::Instant::now();
    let mut renderer = terminal::ProgressRenderer::new(true, false, 80, 6);
    for number in 1..=12 {
        renderer.render_at(
            &Progress::RuleQueued {
                number,
                total: 12,
                name: "Queued check",
                description: "Inspect files",
                resource: None,
            },
            now,
        );
    }
    renderer.render_at(
        &Progress::RuleStarted {
            number: 1,
            total: 12,
            kind: "check",
            id: "first",
            name: "First",
            description: "Inspect files",
        },
        now,
    );
    renderer.render_at(
        &Progress::RuleQueued {
            number: 2,
            total: 12,
            name: "Second",
            description: "Inspect files",
            resource: Some("shared"),
        },
        now,
    );
    let frame = renderer.render_at(&Progress::Tick, now + std::time::Duration::from_secs(2));
    assert!(frame.contains("0/12 completed"));
    assert!(
        frame.contains("2.0 s") && frame.contains("WAITING"),
        "{frame}"
    );
    assert!(frame.lines().count() <= 4);
    assert!(
        renderer
            .render_at(
                &Progress::Tick,
                now + std::time::Duration::from_millis(2050)
            )
            .is_empty()
    );
    assert!(terminal::render_progress(&Progress::Tick, false).is_empty());
    let mut narrow = terminal::ProgressRenderer::new(true, false, 20, 2);
    let plain = narrow.render(&Progress::RuleQueued {
        number: 1,
        total: 1,
        name: "Check",
        description: "Files",
        resource: Some("shared"),
    });
    assert!(plain.contains("shared") && !plain.contains('\u{1b}'));
}

#[test]
fn start_numbers_survive_out_of_order_completion_and_unstarted_results() {
    for redraw in [false, true] {
        let mut renderer = terminal::ProgressRenderer::new(redraw, false, 120, 24);
        let now = std::time::Instant::now();
        for number in 1..=4 {
            let text = renderer.render_at(
                &Progress::RuleQueued {
                    number,
                    total: 4,
                    name: "Pending",
                    description: "Files",
                    resource: None,
                },
                now,
            );
            assert!(!text.contains("[1/4]"));
        }
        for (number, name) in [(3, "Third"), (1, "First")] {
            let text = renderer.render_at(
                &Progress::RuleStarted {
                    number,
                    total: 4,
                    kind: "check",
                    id: name,
                    name,
                    description: "Files",
                },
                now,
            );
            if !redraw {
                let expected = if number == 3 { 1 } else { 2 };
                assert!(
                    text.starts_with(&format!("[{expected}/4] RUNNING")),
                    "{text}"
                );
            }
        }
        let frame = renderer.render_at(&Progress::Tick, now + std::time::Duration::from_secs(1));
        if redraw {
            assert!(frame.contains("0/4 completed · 2 running"), "{frame}");
            assert!(frame.find("[1/4] RUNNING").unwrap() < frame.find("[2/4] RUNNING").unwrap());
            assert!(frame.find("Third").unwrap() < frame.find("First").unwrap());
            assert!(frame.find("First").unwrap() < frame.find("PENDING").unwrap());
        }
        let pass = report::Item::simple("first", "check", Status::Pass, String::new());
        let text = renderer.render_at(
            &Progress::RuleFinished {
                number: 1,
                total: 4,
                item: &pass,
                name: "First",
                description: "Files",
            },
            now + std::time::Duration::from_secs(2),
        );
        assert!(text.contains("[2/4] PASS"), "{text}");
        assert!(text.contains("1/4 completed · 1 running"), "{text}");
        let skipped = report::Item::simple(
            "second",
            "check",
            Status::NotRun,
            "Dependency failed".into(),
        );
        let text = renderer.render_at(
            &Progress::RuleFinished {
                number: 2,
                total: 4,
                item: &skipped,
                name: "Second",
                description: "Files",
            },
            now + std::time::Duration::from_secs(3),
        );
        assert!(text.contains("NOT_RUN"), "{text}");
        assert!(!text.contains("[3/4]"), "{text}");
        let text = renderer.render_at(
            &Progress::RuleQueued {
                number: 4,
                total: 4,
                name: "Fourth",
                description: "Files",
                resource: Some("shared"),
            },
            now + std::time::Duration::from_secs(4),
        );
        assert!(
            text.contains("WAITING") && !text.contains("[4/4]"),
            "{text}"
        );
        let text = renderer.render_at(
            &Progress::RuleFinished {
                number: 3,
                total: 4,
                item: &pass,
                name: "Third",
                description: "Files",
            },
            now + std::time::Duration::from_secs(5),
        );
        assert!(text.contains("[1/4] PASS"), "{text}");
        assert!(text.contains("3/4 completed · 0 running"), "{text}");
        if redraw {
            assert!(text.find("Third").unwrap() < text.find("First").unwrap());
        }
    }
}

#[test]
fn cli_numbers_history_order_but_preserves_saved_order() {
    let root = project();
    fs::write(
        root.path().join(".git/please-scheduling.json"),
        r#"{"estimates":{"check":{"first":1,"second":10000}}}"#,
    )
    .unwrap();
    let output = support::command(env!("CARGO_BIN_EXE_please"))
        .args([
            "run",
            "--at",
            "check",
            "--max-parallel",
            "1",
            "--progress=plain",
        ])
        .current_dir(root.path())
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = String::from_utf8(output.stderr).unwrap();
    assert!(text.contains("[1/2] RUNNING · second"), "{text}");
    assert!(text.contains("[1/2] PASS    · second"), "{text}");
    assert!(text.contains("[2/2] RUNNING · first"), "{text}");
    let saved = report::read(root.path()).unwrap();
    assert_eq!(
        saved
            .results
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
    assert_eq!(
        saved
            .plan
            .scheduled
            .iter()
            .map(|r| r.id.as_str())
            .collect::<Vec<_>>(),
        ["first", "second"]
    );
}
