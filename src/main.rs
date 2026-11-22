use anyhow::{Context, Result};
use clap::{CommandFactory, Parser, error::ErrorKind};
use please::{
    adapter::Configured,
    cli::{Cli, Words},
    config, git_snapshot, report,
    terminal::{self, Tone},
};
use std::io::{IsTerminal, Write};
fn version_line() -> String {
    format!("Please Behave {}", env!("CARGO_PKG_VERSION"))
}
/// Announces the executing binary before any other `run` output. Stdout carries the line only when
/// the configuration is valid and stdout is not reserved for JSON; otherwise it precedes the error.
fn announce<T>(loaded: Result<T>, stdout: bool) -> Result<T> {
    if stdout && loaded.is_ok() {
        println!("{}", version_line());
    } else {
        eprintln!("{}", terminal::clean(&version_line()));
    }
    loaded
}
fn execute(cli: Cli) -> Result<i32> {
    let root = std::env::current_dir()?.canonicalize()?;
    let color = terminal::use_color(cli.no_color);
    match cli.command {
        Words::Init => {
            let created = please::init::init(&root)?;
            println!(
                "{}",
                terminal::paint(
                    &format!("Initialized .please/ ({created} files created)."),
                    Tone::Success,
                    color
                )
            );
            Ok(0)
        }
        Words::Sync { yes } => {
            please::reconciliation::ensure_not_recursive(&root)?;
            let (project, imports_changed) = config::refresh(&root)?;
            for diagnostic in &project.effective.diagnostics {
                eprintln!("Note: {}", terminal::clean(diagnostic));
            }
            let (_, hooks_changed) = please::reconciliation::prepare(&project, None, None, yes)?;
            println!(
                "{}",
                terminal::paint(
                    if imports_changed || hooks_changed {
                        "Synchronization completed successfully."
                    } else {
                        "Already synchronized."
                    },
                    Tone::Success,
                    color
                )
            );
            Ok(0)
        }
        Words::Trust {
            repository,
            remove,
            list,
        } => {
            if list {
                for repository in please::external::trust::list()? {
                    println!("{}", terminal::clean(&repository));
                }
                return Ok(0);
            }
            let selector = please::external::repository_selector(
                &repository.context("missing repository")?,
                &root,
            )?;
            let identity = please::external::trust::identity(&selector);
            let changed = if remove {
                please::external::trust::remove(&selector)?
            } else {
                please::external::trust::add(&selector)?
            };
            let message = match (remove, changed) {
                (false, true) => format!("Trusted {identity}."),
                (false, false) => format!("Already trusted {identity}."),
                (true, true) => format!("No longer trusting {identity}."),
                (true, false) => format!("Was not trusted: {identity}."),
            };
            println!(
                "{}",
                terminal::paint(&terminal::clean(&message), Tone::Success, color)
            );
            Ok(0)
        }
        Words::Run {
            progress,
            checks_only,
            max_parallel,
            fail_fast,
            at,
            plan,
            json,
            git_hook,
            git_hook_path,
            hook_arguments,
        } => {
            if plan {
                let prepared = announce(config::load(&root), !json).and_then(|project| {
                    please::planner::build(&project, at.as_deref(), checks_only)
                });
                return match prepared {
                    Ok(prepared) => {
                        if json {
                            println!("{}", serde_json::to_string_pretty(&prepared.plan)?);
                        } else {
                            print!("{}", terminal::render_plan(&prepared.plan, color));
                        }
                        Ok(if prepared.plan.executable() { 0 } else { 2 })
                    }
                    Err(error) if json => {
                        println!(
                            "{}",
                            serde_json::to_string_pretty(&serde_json::json!({
                                "plan_schema_version": please::planner::PLAN_SCHEMA_VERSION,
                                "status": "blocked",
                                "blocking_reasons": [format!("{error:#}")]
                            }))?
                        );
                        Ok(2)
                    }
                    Err(error) => Err(error),
                };
            }
            let progress_color = terminal::error_color(cli.no_color);
            let mut stderr = std::io::stderr().lock();
            let redraw = terminal::progress_redraw(
                progress,
                stderr.is_terminal(),
                matches!(std::env::var("CI").as_deref(), Ok("true" | "1"))
                    || matches!(std::env::var("GITHUB_ACTIONS").as_deref(), Ok("true")),
                std::env::var("TERM").is_ok_and(|term| term == "dumb"),
            );
            let (width, height) = terminal::terminal_dimensions();
            let mut renderer =
                terminal::ProgressRenderer::new(redraw, progress_color, width, height);
            let started = std::time::Instant::now();
            let mut project = announce(config::load(&root), true)?;
            if let Some(limit) = max_parallel {
                project.config.execution.max_parallel = limit;
            }
            for diagnostic in &project.effective.diagnostics {
                eprintln!("Note: {}", terminal::clean(diagnostic));
            }
            let invocation = git_hook.zip(git_hook_path).map(|(event, path)| {
                please::reconciliation::HookInvocation {
                    event,
                    path,
                    arguments: hook_arguments,
                }
            });
            let (hook_context, _) =
                please::reconciliation::prepare(&project, invocation, at.as_deref(), false)?;
            let no_rules = project.checks.is_empty() && project.reviews.is_empty();
            let report = please::verify_project_with_fail_fast(
                project,
                &Configured,
                if checks_only {
                    please::VerificationMode::ChecksOnly
                } else {
                    please::VerificationMode::AllRules
                },
                at.as_deref(),
                hook_context.as_ref(),
                fail_fast,
                &mut |event| {
                    // Progress must be visible before a slow command or adapter call finishes.
                    // Losing the progress stream must not prevent writing the report.
                    let _ = stderr.write_all(renderer.render(&event).as_bytes());
                    let _ = stderr.flush();
                },
            )?;
            if no_rules {
                println!(
                    "{}",
                    terminal::paint("No rules defined yet.", Tone::Warning, color)
                );
            }
            print!(
                "{}",
                terminal::render_outcome(&report, started.elapsed().as_millis() as u64, color)
            );
            Ok(report.overall.exit_code())
        }
        Words::Status {
            details,
            report: selected,
        } => {
            git_snapshot::require_root(&root)?;
            let saved = report::saved(&root)?;
            let Some(number) = selected
                .or(saved.last().copied())
                .filter(|number| saved.contains(number))
            else {
                eprintln!(
                    "{}",
                    terminal::paint(
                        &match selected {
                            Some(number) => format!("No saved report {number}. Run: please status"),
                            None => "No saved report. Run: please run".to_string(),
                        },
                        Tone::Warning,
                        terminal::error_color(cli.no_color)
                    )
                );
                return Ok(3);
            };
            let report = report::read_number(&root, number)
                .with_context(|| format!("cannot read report {number}; rerun please run"))?;
            print!(
                "{}",
                if details {
                    terminal::render_details(&report, color)
                } else {
                    terminal::render(&report, color)
                }
            );
            match git_snapshot::compare_current(&root, &report.snapshot, report.max_display_files) {
                Ok((changes, limitations)) => {
                    for reason in &limitations {
                        println!(
                            "{}",
                            terminal::paint(
                                &format!(
                                    "  Current snapshot limitation: {}",
                                    terminal::clean(reason)
                                ),
                                Tone::Warning,
                                color
                            )
                        );
                    }
                    print!("{}", terminal::render_changes(&changes, color));
                    Ok(match changes.state.as_str() {
                        "CURRENT" => report.overall.exit_code(),
                        "STALE" => 1,
                        _ => 2,
                    })
                }
                Err(error) => {
                    eprintln!(
                        "{}",
                        terminal::paint(
                            &terminal::clean(&format!(
                                "UNKNOWN: cannot inspect working files: {error:#}"
                            )),
                            Tone::Warning,
                            terminal::error_color(cli.no_color)
                        )
                    );
                    Ok(2)
                }
            }
        }
        Words::Publish { gateway_url } => {
            please::publish::send(&root, &gateway_url)?;
            println!(
                "{}",
                terminal::paint("Attestation accepted.", Tone::Success, color)
            );
            Ok(0)
        }
        Words::Help => {
            print!(
                "{}",
                terminal::render_help(&Cli::command().render_help().to_string(), color)
            );
            Ok(0)
        }
    }
}
fn main() {
    let args: Vec<_> = std::env::args_os().collect();
    let no_color = args.iter().any(|arg| arg == "--no-color");
    let code = match Cli::try_parse_from(&args) {
        Ok(cli) => match execute(cli) {
            Ok(code) => code,
            Err(error) => {
                eprintln!(
                    "{}",
                    terminal::paint(
                        &terminal::clean(&format!("Please Behave: ERROR: {error:#}")),
                        Tone::Warning,
                        terminal::error_color(no_color)
                    )
                );
                2
            }
        },
        Err(error) => match error.kind() {
            ErrorKind::DisplayHelp => {
                print!(
                    "{}",
                    terminal::render_help(&error.to_string(), terminal::use_color(no_color))
                );
                0
            }
            ErrorKind::DisplayVersion => {
                print!(
                    "{}",
                    terminal::paint(
                        &error.to_string(),
                        Tone::Accent,
                        terminal::use_color(no_color)
                    )
                );
                0
            }
            ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand if args.len() == 1 => {
                eprint!(
                    "{}",
                    terminal::render_help(&error.to_string(), terminal::error_color(no_color))
                );
                error.exit_code()
            }
            ErrorKind::InvalidSubcommand => {
                eprint!(
                    "{}",
                    terminal::render_help(
                        &Cli::command().render_help().to_string(),
                        terminal::error_color(no_color)
                    )
                );
                error.exit_code()
            }
            _ => {
                eprint!("{error}");
                error.exit_code()
            }
        },
    };
    std::process::exit(code);
}
