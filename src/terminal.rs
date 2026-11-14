use crate::{
    git_snapshot::Changes,
    report::{Report, Status},
};
use std::io::IsTerminal;

#[derive(Clone, Copy)]
pub enum Tone {
    Success,
    Failure,
    Warning,
    Accent,
    Muted,
    Heading,
}
pub fn paint(text: &str, tone: Tone, color: bool) -> String {
    if !color {
        return text.to_owned();
    }
    let code = match tone {
        Tone::Success => 32,
        Tone::Failure => 31,
        Tone::Warning => 33,
        Tone::Accent => 36,
        Tone::Muted => 90,
        Tone::Heading => 1,
    };
    format!("\x1b[{code}m{text}\x1b[0m")
}
pub fn error_color(no_color: bool) -> bool {
    !no_color && std::env::var_os("NO_COLOR").is_none() && std::io::stderr().is_terminal()
}
pub fn render_help(text: &str, color: bool) -> String {
    if !color {
        return text.to_owned();
    }
    let banner = include_str!("banner.txt").trim_end();
    let mut output = text.replacen(banner, &paint(banner, Tone::Accent, true), 1);
    for heading in ["Commands:", "Options:", "Usage:"] {
        output = output.replace(heading, &paint(heading, Tone::Heading, true));
    }
    for phrase in ["init", "sync", "run", "status", "help"] {
        output = output.replace(
            &format!("  {phrase}"),
            &format!("  {}", paint(phrase, Tone::Accent, true)),
        );
    }
    for flag in [
        "--no-color",
        "--help",
        "--version",
        "--checks-only",
        "-h,",
        "-V,",
    ] {
        output = output.replace(flag, &paint(flag, Tone::Accent, true));
    }
    output
}
pub fn use_color(no_color: bool) -> bool {
    !no_color && std::env::var_os("NO_COLOR").is_none() && std::io::stdout().is_terminal()
}
// Repository and provider strings cannot inject terminal escapes or forge table rows.
pub fn clean(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}
fn status(status: Status, color: bool) -> String {
    let tone = match status {
        Status::Pass => Tone::Success,
        Status::Fail => Tone::Failure,
        Status::Error => Tone::Warning,
        Status::NotRun | Status::Cancelled => Tone::Muted,
    };
    paint(status.label(), tone, color)
}
fn short_summary(reason: &str) -> String {
    let text = clean(reason)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if text.chars().count() <= crate::report::MAX_SUMMARY_CHARS {
        return text;
    }
    text.chars()
        .take(crate::report::MAX_SUMMARY_CHARS - 1)
        .chain(std::iter::once('…'))
        .collect()
}
/// Select redraw independently of color. Redirected logs never need cursor controls.
pub fn progress_redraw(
    mode: crate::cli::ProgressMode,
    terminal: bool,
    ci: bool,
    dumb: bool,
) -> bool {
    match mode {
        crate::cli::ProgressMode::Auto | crate::cli::ProgressMode::Tty => terminal && !ci && !dumb,
        crate::cli::ProgressMode::Plain => false,
    }
}

pub fn terminal_dimensions() -> (usize, usize) {
    #[cfg(unix)]
    {
        let mut size: libc::winsize = unsafe { std::mem::zeroed() };
        // ioctl writes the winsize structure for stderr's terminal, when present.
        if unsafe { libc::ioctl(libc::STDERR_FILENO, libc::TIOCGWINSZ, &mut size) } == 0
            && size.ws_col > 0
        {
            return (usize::from(size.ws_col), usize::from(size.ws_row).max(1));
        }
    }
    let dimension = |key, default| {
        std::env::var(key)
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(default)
    };
    (dimension("COLUMNS", 80), dimension("LINES", 24))
}

fn duration(milliseconds: u64) -> String {
    format!("{:.1} s", milliseconds as f64 / 1000.0)
}

// Conservatively reserve two columns for non-ASCII characters, including wide
// Unicode names. This can shorten a row early, but never lets it wrap.
fn display_width(text: &str) -> usize {
    text.chars().map(|c| if c.is_ascii() { 1 } else { 2 }).sum()
}
fn fit(text: &str, width: usize) -> String {
    let text = clean(text);
    let columns = |c: char| if c.is_ascii() { 1 } else { 2 };
    if text.chars().map(columns).sum::<usize>() <= width {
        return text;
    }
    let mut result = String::new();
    let mut used = 0;
    for c in text.chars() {
        if used + columns(c) + 2 > width {
            break;
        }
        result.push(c);
        used += columns(c);
    }
    if width >= 2 {
        result.push('…');
    }
    result
}

fn progress_row(
    index: (Option<usize>, usize),
    state: (&str, Tone),
    label: (&str, &str),
    elapsed: Option<u64>,
    color: bool,
    width: Option<usize>,
) -> String {
    let (number, total) = index;
    let position = number
        .map(|n| format!("[{n}/{total}] "))
        .unwrap_or_default();
    let (state, tone) = state;
    let (name, description) = label;
    let prefix = format!("{position}{state:<7} · ");
    let suffix = elapsed
        .map(|ms| format!(" · {}", duration(ms)))
        .unwrap_or_default();
    let label = format!("{} — {}", clean(name), short_summary(description));
    let label = width
        .map(|w| {
            fit(
                &label,
                w.saturating_sub(display_width(&prefix) + display_width(&suffix) + 1),
            )
        })
        .unwrap_or(label);
    format!(
        "{position}{}{} · {label}{suffix}\n",
        paint(state, tone, color),
        " ".repeat(7usize.saturating_sub(state.chars().count()))
    )
}

fn progress_event(
    event: &crate::Progress<'_>,
    color: bool,
    width: Option<usize>,
    display_number: Option<usize>,
) -> String {
    use crate::Progress;
    match event {
        Progress::Plan { .. } | Progress::Phase(_) | Progress::Tick => String::new(),
        Progress::RuleQueued {
            number: _,
            total,
            name,
            description,
            resource,
        } => {
            // Capacity-only queueing is useful on the board, but noisy in logs.
            if let Some(resource) = resource {
                progress_row(
                    (display_number, *total),
                    ("WAITING", Tone::Muted),
                    (name, &format!("resource {}", clean(resource))),
                    None,
                    color,
                    width,
                )
            } else if width.is_some() {
                progress_row(
                    (display_number, *total),
                    ("PENDING", Tone::Muted),
                    (name, description),
                    None,
                    color,
                    width,
                )
            } else {
                String::new()
            }
        }
        Progress::RuleStarted {
            number: _,
            total,
            name,
            description,
            ..
        } => progress_row(
            (display_number, *total),
            ("RUNNING", Tone::Accent),
            (name, description),
            None,
            color,
            width,
        ),
        Progress::RuleFinished {
            number: _,
            total,
            item,
            name,
            description,
        } => {
            let tone = match item.status {
                Status::Pass => Tone::Success,
                Status::Fail => Tone::Failure,
                Status::Error => Tone::Warning,
                Status::NotRun | Status::Cancelled => Tone::Muted,
            };
            let mut row = progress_row(
                (display_number, *total),
                (item.status.label(), tone),
                (name, description),
                Some(item.duration_ms),
                color,
                width,
            );
            if item.status != Status::Pass {
                let reason = format!("  {}", short_summary(&item_reason(item)));
                row.push_str(
                    &width
                        .map(|w| fit(&reason, w.saturating_sub(1)))
                        .unwrap_or(reason),
                );
                row.push('\n');
                for finding in &item.findings {
                    let line = format!(
                        "  {}{}",
                        short_summary(&finding.message),
                        if finding.source.is_some() {
                            " [commit-header:1]".into()
                        } else {
                            finding
                                .path
                                .as_ref()
                                .map(|path| {
                                    format!(" [{}:{}]", clean(path), finding.line.unwrap_or(0))
                                })
                                .unwrap_or_default()
                        }
                    );
                    row.push_str(
                        &width
                            .map(|w| fit(&line, w.saturating_sub(1)))
                            .unwrap_or(line),
                    );
                    row.push('\n');
                }
            }
            row
        }
        Progress::ReviewBatch { count, calls, .. } => format!(
            "Reviews: {count} · {calls} {}\n",
            if *calls == 1 {
                "provider call"
            } else {
                "provider calls"
            }
        ),
    }
}

/// Render one event using its supplied position. Use ProgressRenderer for an
/// execution stream with start-order numbering and completion counts.
pub fn render_progress(event: &crate::Progress<'_>, color: bool) -> String {
    let number = match event {
        crate::Progress::RuleStarted { number, .. } => Some(*number),
        crate::Progress::RuleFinished { number, item, .. } if item.status != Status::NotRun => {
            Some(*number)
        }
        _ => None,
    };
    progress_event(event, color, None, number)
}

/// Stateful terminal output replaces running rows, including shared review batches.
/// Plain mode is append-only and uses the same names and messages.
struct ProgressRow {
    text: String,
    started: Option<std::time::Instant>,
    finished: bool,
    waiting: bool,
    failed: bool,
}

pub struct ProgressRenderer {
    redraw: bool,
    color: bool,
    width: usize,
    rows: std::collections::BTreeMap<usize, ProgressRow>,
    start_numbers: std::collections::BTreeMap<usize, usize>,
    visible: usize,
    height: usize,
    total: usize,
    last_draw: Option<std::time::Instant>,
}
impl ProgressRenderer {
    pub fn new(redraw: bool, color: bool, width: usize, height: usize) -> Self {
        Self {
            redraw: redraw && width >= 40 && height >= 4,
            color,
            width,
            rows: Default::default(),
            start_numbers: Default::default(),
            visible: 0,
            height,
            total: 0,
            last_draw: None,
        }
    }
    pub fn render(&mut self, event: &crate::Progress<'_>) -> String {
        self.render_at(event, std::time::Instant::now())
    }
    /// Supply the clock explicitly for deterministic elapsed-time rendering.
    pub fn render_at(&mut self, event: &crate::Progress<'_>, now: std::time::Instant) -> String {
        use crate::Progress;
        let (number, running, finished, failed) = match event {
            Progress::Plan { total } => {
                self.total = *total;
                return String::new();
            }
            Progress::RuleQueued { number, total, .. } => {
                self.total = *total;
                (*number, false, false, false)
            }
            Progress::RuleStarted { number, total, .. } => {
                self.total = *total;
                (*number, true, false, false)
            }
            Progress::RuleFinished {
                number,
                total,
                item,
                ..
            } => {
                self.total = *total;
                (
                    *number,
                    false,
                    true,
                    matches!(item.status, Status::Fail | Status::Error),
                )
            }
            Progress::Tick => (0, false, false, false),
            Progress::ReviewBatch { .. } => {
                // End the current board before the provider-call announcement.
                let mut text = String::new();
                if self.visible > 0 {
                    text.push_str(&format!("\x1b[{}A", self.visible));
                    for _ in 0..self.visible {
                        text.push_str("\r\x1b[2K\n");
                    }
                    text.push_str(&format!("\x1b[{}A", self.visible));
                }
                self.visible = 0;
                self.last_draw = None;
                text.push_str(&render_progress(event, self.color));
                return text;
            }
            Progress::Phase(_) => return String::new(),
        };
        if running && !self.start_numbers.contains_key(&number) {
            self.start_numbers
                .insert(number, self.start_numbers.len() + 1);
        }
        let display_number = self.start_numbers.get(&number).copied();
        if !self.redraw && number == 0 {
            return render_progress(event, self.color);
        }
        if number != 0 {
            let width = if running {
                self.width.saturating_sub(12)
            } else {
                self.width
            };
            self.rows.insert(
                number,
                ProgressRow {
                    text: match event {
                        Progress::RuleFinished {
                            total, item, name, ..
                        } if item.status != Status::Pass => progress_row(
                            (display_number, *total),
                            (
                                item.status.label(),
                                if failed { Tone::Failure } else { Tone::Muted },
                            ),
                            (name, &item_reason(item)),
                            Some(item.duration_ms),
                            self.color,
                            Some(width),
                        ),
                        _ => progress_event(event, self.color, Some(width), display_number),
                    },
                    started: running.then_some(now),
                    finished,
                    failed,
                    waiting: matches!(
                        event,
                        Progress::RuleQueued {
                            resource: Some(_),
                            ..
                        }
                    ),
                },
            );
        }
        let completed = self.rows.values().filter(|r| r.finished).count();
        let running_count = self.rows.values().filter(|r| r.started.is_some()).count();
        if !self.redraw {
            let mut text = progress_event(event, self.color, None, display_number);
            if finished {
                text.push_str(&format!(
                    "Progress: {completed}/{} completed · {running_count} running\n",
                    self.total
                ));
            }
            return text;
        }
        // Coalesce bursts of queue/start/completion events, but always publish
        // the final state. The coordinator sends ticks even without completions.
        if completed != self.total
            && self.last_draw.is_some_and(|last| {
                now.saturating_duration_since(last) < std::time::Duration::from_millis(100)
            })
        {
            return String::new();
        }
        self.last_draw = Some(now);
        let capacity = self.height.saturating_sub(3);
        let mut selected: Vec<_> = self.rows.iter().collect();
        // Prefer running and queued rows, then failures and recent completions.
        selected.sort_by_key(|(number, row)| {
            (
                if row.started.is_some() {
                    0
                } else if row.waiting {
                    1
                } else if !row.finished {
                    2
                } else if row.failed {
                    3
                } else {
                    4
                },
                std::cmp::Reverse(**number),
            )
        });
        selected.truncate(capacity);
        selected.sort_by_key(|(number, _)| {
            (
                self.start_numbers
                    .get(number)
                    .copied()
                    .unwrap_or(usize::MAX),
                **number,
            )
        });
        let hidden = self.rows.len().saturating_sub(selected.len());
        let failures = self.rows.values().filter(|r| r.failed).count();
        let summary = format!(
            "Progress: {completed}/{} completed · {running_count} running · {failures} failed{}",
            self.total,
            if hidden > 0 {
                format!(" · {hidden} hidden; please status --details")
            } else {
                String::new()
            }
        );
        let mut lines = vec![fit(&summary, self.width.saturating_sub(1))];
        for (_, row) in selected {
            let mut text = row.text.lines().next().unwrap_or_default().to_owned();
            if let Some(started) = row.started {
                text.push_str(&format!(
                    " · {}",
                    duration(now.saturating_duration_since(started).as_millis() as u64)
                ));
            }
            lines.push(text);
        }
        let mut text = if self.visible > 0 {
            format!("\x1b[{}A", self.visible)
        } else {
            String::new()
        };
        // Clear stale rows when the visible window shrinks.
        let count = lines.len().max(self.visible);
        for index in 0..count {
            text.push_str("\r\x1b[2K");
            if let Some(line) = lines.get(index) {
                text.push_str(line);
            }
            text.push('\n');
        }
        self.visible = count;
        text
    }
}

/// Execution already printed its results. Only new integrity failures and the
/// final aggregate belong here; saved status has its own result listing.
pub fn render_outcome(report: &Report, elapsed_ms: u64, color: bool) -> String {
    let mut text = String::new();
    for item in report
        .results
        .iter()
        .filter(|item| item.kind == "integrity")
    {
        text.push_str(&format!(
            "{} · {}\n",
            status(item.status, color),
            clean(&item.reason)
        ));
    }
    let count = report.plan.scheduled.len();
    if count == 0 {
        text.push_str(&format!(
            "{} · No rules executed · {}\n",
            status(report.overall, color),
            duration(elapsed_ms)
        ));
    } else {
        let failed = report
            .results
            .iter()
            .filter(|r| r.status == Status::Fail)
            .count();
        let errors = report
            .results
            .iter()
            .filter(|r| r.status == Status::Error)
            .count();
        let skipped = report
            .results
            .iter()
            .filter(|r| r.status == Status::NotRun)
            .count();
        let cancelled = report
            .results
            .iter()
            .filter(|r| r.status == Status::Cancelled)
            .count();
        let mut summary = format!("{count} rules");
        if failed > 0 {
            summary.push_str(&format!(" · {failed} failed"));
        }
        if errors > 0 {
            summary.push_str(&format!(" · {errors} errors"));
        }
        if cancelled > 0 {
            summary.push_str(&format!(" · {cancelled} cancelled"));
        }
        if skipped > 0 {
            summary.push_str(&format!(" · {skipped} not executed"));
        }
        text.push_str(&format!(
            "{} · {summary} · {}\n",
            status(report.overall, color),
            duration(elapsed_ms)
        ));
    }
    text
}

pub fn render_plan(plan: &crate::planner::Plan, color: bool) -> String {
    let mut text = format!(
        "{} · {}\nExecution Point: {} ({}) · {} · Input: {}\nPlan-ID: {}\nHEAD: {}\nSnapshot: {} · {}\n",
        paint("Please Behave execution plan", Tone::Heading, color),
        paint(
            &plan.status.to_uppercase(),
            if plan.executable() {
                Tone::Accent
            } else {
                Tone::Warning
            },
            color
        ),
        clean(&plan.at),
        "execution point",
        "Execution point",
        clean(&plan.input.kind),
        plan.plan_id,
        plan.input.head.as_deref().unwrap_or("unborn"),
        plan.input.fingerprint,
        if plan.input.complete {
            "complete"
        } else {
            "UNKNOWN"
        }
    );
    if let Some(base) = &plan.change_base {
        text.push_str(&format!("Change base: {}\n", clean(base)));
    }
    if plan.checks_only {
        text.push_str("Mode: Checks only · no provider calls\n");
    }
    let names = plan.display_names();
    for (index, rule) in plan.scheduled.iter().enumerate() {
        text.push_str(&format!(
            "[{}] {} {} · {}\n    Selection: {}\n",
            index + 1,
            rule.kind,
            clean(&names[&rule.id]),
            clean(&rule.description),
            clean(&rule.selected_by.join(", "))
        ));
        if !rule.depends_on.is_empty() {
            text.push_str(&format!(
                "    Depends on: {}\n",
                clean(&rule.depends_on.join(", "))
            ));
        }
    }
    if plan.scheduled.is_empty() {
        text.push_str("No active rules selected; no rule PASS established.\n");
    }
    for omitted in &plan.omitted {
        text.push_str(&format!(
            "Omitted: {} {} · {}\n",
            omitted.rule.kind,
            clean(&omitted.rule.id),
            omitted.reason
        ));
    }
    if !plan.required_ids.is_empty() {
        text.push_str(&format!(
            "Required rules: {}\n",
            clean(&plan.required_ids.join(", "))
        ));
    }
    if let Some(header) = &plan.input.commit_header {
        text.push_str(&format!(
            "Commit header (saved invocation input): {}\n",
            clean(header)
        ));
    }
    if let Some(config) = &plan.provider {
        text.push_str(&format!(
            "Default adapter: {} · Timeout: {} s\n",
            clean(&config.command),
            config.timeout_seconds
        ));
    }
    text.push_str(&format!("Adapter availability not checked · Planned model calls: {}\nContext: {} / {} bytes · Diff: {} bytes\n",
        plan.planned_model_calls, plan.context.selected_bytes, plan.context.byte_limit, plan.context.orientation_diff_bytes));
    for rule in &plan.scheduled {
        if let Some(origin) = &rule.provider_origin {
            text.push_str(&format!(
                "Provider source: {} · {} @ {}\n",
                clean(rule.provider.as_deref().unwrap_or_default()),
                clean(&origin.path),
                clean(&origin.commit)
            ));
        }
        if let (Some(id), Some(config)) = (&rule.provider, &rule.provider_config) {
            text.push_str(&format!(
                "Review {} · Provider configuration: {} · Adapter: {} · Timeout: {} s\n",
                clean(&names[&rule.id]),
                clean(id),
                clean(&config.command),
                config.timeout_seconds
            ));
        }
    }
    for path in &plan.context.entry_files {
        text.push_str(&format!("Entry file: {}\n", clean(path)));
    }
    text.push_str(&format!(
        "Review criteria: {} / {} bytes · Orientation: at most {} manifest paths\n",
        plan.context.instruction_bytes,
        plan.context.instruction_byte_limit,
        plan.context.manifest_hint_limit
    ));
    for selection in &plan.context.selections {
        text.push_str(&format!(
            "Requirements: {} · {}\n",
            clean(&selection.path),
            if selection.headings.is_empty() {
                "entire file".into()
            } else {
                clean(&selection.headings.join(", "))
            }
        ));
    }
    for section in &plan.context.sections {
        text.push_str(&format!(
            "Context section: {}:{}–{} · {} bytes\n",
            clean(&section.path),
            section.start_line,
            section.end_line,
            section.bytes
        ));
    }
    for reason in plan.blocking_reasons.iter().chain(&plan.input.limitations) {
        text.push_str(&format!(
            "Blocked: {}\n",
            paint(&clean(reason), Tone::Warning, color)
        ));
    }
    text.push_str("Preview: no rules executed, no tokens consumed, no report overwritten.\n");
    text
}

pub fn render(report: &Report, color: bool) -> String {
    let mut text = format!(
        "{} · {} · {}\n",
        paint("Please Behave", Tone::Accent, color),
        clean(&report.project),
        paint(&clean(&report.timestamp), Tone::Muted, color)
    );
    {
        let plan = &report.plan;
        text.push_str(&format!(
            "Execution Point: {} · Input: {} · {}\n",
            clean(&plan.at),
            clean(&plan.input.kind),
            "Execution point"
        ));
        if plan.scheduled.is_empty() {
            text.push_str("No active rules selected; no rule PASS established.\n");
        }
    }
    for omission in &report.plan.omitted {
        text.push_str(&format!(
            "Omitted: {} · {}\n",
            clean(omission.rule.display_name()),
            clean(&omission.reason)
        ));
    }
    if report.checks_only {
        text.push_str(&paint(
            "Mode: Checks only · Reviews not run · no provider calls\n",
            Tone::Accent,
            color,
        ));
        if !report.results.iter().any(|item| item.kind == "check") {
            text.push_str("No active checks configured.\n");
        }
    }
    if let Some(header) = &report.plan.input.commit_header {
        text.push_str(&format!(
            "Commit header (saved invocation input): {}\n",
            clean(header)
        ));
    }
    text.push_str(&format!(
        "\n{:<9} {:<24} {:<9} {:<9} SUMMARY\n",
        "TYPE", "NAME", "STATUS", "TIME"
    ));
    let names = report.plan.display_names();
    for item in &report.results {
        text.push_str(&format!(
            "{:<9} {:<24} {}{} {:<9} {}\n",
            clean(&item.kind),
            clean(names.get(&item.id).unwrap_or(&item.id)),
            status(item.status, color),
            " ".repeat(9 - item.status.label().len()),
            format!("{} ms", item.duration_ms),
            if item.status == Status::Pass && item.kind == "check" {
                report
                    .plan
                    .scheduled
                    .iter()
                    .find(|r| r.id == item.id)
                    .map(|r| short_summary(&r.description))
                    .unwrap_or_default()
            } else {
                short_summary(&item_reason(item))
            }
        ));
    }
    text.push_str(&format!(
        "\nOverall: {}\nReport: .please/reports/last.json\nDetails: please status --details\n",
        status(report.overall, color)
    ));
    if let Some(usage) = report.provider_usage {
        text.push_str(&format!(
            "Adapter tokens: {} total · {} input ({} cached) · {} output\n",
            count(usage.total()),
            count(usage.input_tokens),
            count(usage.cached_input_tokens),
            count(usage.output_tokens)
        ));
    } else if report
        .results
        .iter()
        .any(|item| item.kind == "review" && item.status != Status::NotRun)
    {
        text.push_str("Adapter tokens: unavailable\n");
    }
    if !report.snapshot.complete {
        text.push_str("Snapshot: UNKNOWN (details in report)\n");
    }
    text
}
fn count(value: Option<u64>) -> String {
    value.map_or_else(|| "unavailable".into(), |n| n.to_string())
}
/// Render the saved report only; this never executes commands or invokes a provider.
pub fn render_details(report: &Report, color: bool) -> String {
    let mut text = render(report, color);
    {
        let plan = &report.plan;
        text.push_str(&format!("Plan-ID: {}\n", clean(&plan.plan_id)));
        if !plan.required_ids.is_empty() {
            text.push_str(&format!(
                "Required rules: {}\n",
                clean(&plan.required_ids.join(", "))
            ));
        }
        for omitted in &plan.omitted {
            text.push_str(&format!(
                "Omitted: {} · {}\n",
                clean(&omitted.rule.id),
                clean(&omitted.reason)
            ));
        }
    }
    for rule in &report.plan.scheduled {
        if let Some(origin) = &rule.provider_origin {
            text.push_str(&format!(
                "Provider source: {} · {} @ {}\n",
                clean(rule.provider.as_deref().unwrap_or_default()),
                clean(&origin.path),
                clean(&origin.commit)
            ));
        }
        if let Some(external) = rule
            .origin
            .as_ref()
            .and_then(|origin| origin.external.as_ref())
        {
            text.push_str(&format!(
                "Source: {} · {} @ {}\n",
                clean(&rule.id),
                clean(&external.path),
                clean(&external.commit)
            ));
        }
    }
    if let Some(usage) = report.provider_usage {
        text.push_str(&format!(
            "Reasoning tokens (included in output): {} · Cache write tokens (included in input): {}\n",
            count(usage.reasoning_output_tokens),
            count(usage.cache_write_input_tokens)
        ));
        text.push_str("Usage reported by adapters for the entire review call; not a billing or quota statement.\n");
    }
    text.push_str(&format!(
        "\n{}\nRepository: {}\nProvider: {}\nHEAD: {}\nSnapshot: {}\n{}\n",
        paint("Details", Tone::Heading, color),
        clean(&report.repository),
        clean(&report.provider),
        clean(report.snapshot.head.as_deref().unwrap_or("unborn")),
        clean(&report.snapshot.fingerprint),
        clean(&report.git_summary)
    ));
    for item in &report.results {
        text.push_str(&format!(
            "\n{} · {}\n{}\n",
            paint(&clean(&item.id), Tone::Accent, color),
            status(item.status, color),
            clean(&item_reason(item))
        ));
        for finding in &item.findings {
            let reference = if finding.source.is_some() {
                " [commit-header:1]".into()
            } else {
                finding
                    .path
                    .as_ref()
                    .map(|path| {
                        format!(
                            " [{}:{}]",
                            clean(path),
                            finding
                                .line
                                .map(|line| line.to_string())
                                .unwrap_or_else(|| "?".into())
                        )
                    })
                    .unwrap_or_default()
            };
            text.push_str(&format!("  - {}{}\n", clean(&finding.message), reference));
        }
        for (name, capture) in [("stdout", &item.stdout), ("stderr", &item.stderr)] {
            if let Some(capture) = capture {
                if capture.total_bytes == 0 {
                    continue;
                }
                text.push_str(&format!(
                    "  {name}: {} bytes{}\n",
                    capture.total_bytes,
                    if capture.truncated {
                        " (truncated)"
                    } else {
                        ""
                    }
                ));
                for line in capture.text.lines() {
                    text.push_str(&format!("    {}\n", clean(line)));
                }
            }
        }
    }
    for limitation in &report.snapshot.limitations {
        text.push_str(&format!("Snapshot limitation: {}\n", clean(limitation)));
    }
    text
}
pub fn render_changes(changes: &Changes, color: bool) -> String {
    let count = changes
        .total_files
        .map(|n| n.to_string())
        .unwrap_or_else(|| "unavailable (partial snapshot)".into());
    let mut text = format!(
        "\n{} · changed files since verification: {count}\n",
        paint(
            &changes.state,
            if changes.state == "CURRENT" {
                Tone::Success
            } else {
                Tone::Warning
            },
            color
        )
    );
    if changes.head_changed {
        text.push_str("  Git HEAD changed since verification\n");
    }
    for path in &changes.sample_paths {
        text.push_str(&format!("  {} (line counts unavailable)\n", clean(path)));
    }
    text.push_str("Insertions/deletions since verification: unavailable (hash-only baseline)\n");
    text
}

fn item_reason(item: &crate::report::Item) -> String {
    if item.status == crate::report::Status::Fail && !item.blocking {
        format!("Non-blocking: {}", item.reason)
    } else {
        item.reason.clone()
    }
}
