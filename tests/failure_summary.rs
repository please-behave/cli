//! REQ-0133: the run closes with a bounded failure summary.
mod support;
use please::{
    process::Capture,
    report::{self, Finding, Item, Status},
    terminal,
};

struct Never;
impl please::provider::Provider for Never {
    fn review(&self, _: &please::provider::Request<'_>) -> anyhow::Result<String> {
        panic!("no provider expected")
    }
}

fn base() -> report::Report {
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
    please::verify_project_at(
        please::config::load(root.path()).unwrap(),
        &Never,
        please::VerificationMode::AllRules,
        Some("commit-msg"),
        &mut |_| {},
    )
    .unwrap()
}
fn capture(text: &str) -> Option<Capture> {
    Some(Capture {
        text: text.into(),
        total_bytes: text.len() as u64,
        truncated: false,
    })
}
fn item(id: &str, kind: &str, status: Status, reason: &str) -> Item {
    Item::simple(id, kind, status, reason.into())
}
fn outcome(items: Vec<Item>) -> String {
    let mut report = base();
    report.overall = report::overall(&items);
    report.results = items;
    terminal::render_outcome(&report, 5, false)
}
fn summary(text: &str) -> &str {
    &text[text.find("prevented success").unwrap()..]
}

#[test]
fn a_failing_check_shows_the_last_error_lines_and_the_pointer() {
    let mut check = item("lint", "check", Status::Fail, "Exited with status 1");
    check.stderr = capture("one\ntwo\n\nthree\nfour\n\n");
    check.stdout = capture("ignored");
    let text = outcome(vec![check]);
    assert!(text.starts_with("FAIL"));
    assert!(
        summary(&text).starts_with(
            "prevented success · first: lint · FAIL\n  two\n  three\n  four\nComplete output: please status --details\n"
        ),
        "{text}"
    );
    assert!(text.contains("\n1 rule prevented"));
}

#[test]
fn standard_output_and_then_the_reason_are_the_fallbacks() {
    let mut check = item("lint", "check", Status::Fail, "Exited with status 1");
    check.stdout = capture("from stdout\n");
    assert!(summary(&outcome(vec![check])).contains("\n  from stdout\n"));
    let check = item("lint", "check", Status::Error, "Timed out after 10 s");
    assert!(summary(&outcome(vec![check])).contains("\n  Timed out after 10 s\n"));
}

#[test]
// REQ-0269: failed reviews show reasons and findings without raw adapter noise.
fn a_review_fail_shows_its_reason_and_findings_not_its_output() {
    let mut review = item(
        "consistency",
        "review",
        Status::Fail,
        "Docs contradict code",
    );
    review.findings = vec![
        Finding {
            message: "Stale flag".into(),
            path: Some("docs/usage.md".into()),
            line: Some(7),
        },
        Finding {
            message: "Second".into(),
            path: None,
            line: None,
        },
        Finding {
            message: "Cut off".into(),
            path: None,
            line: None,
        },
    ];
    review.stderr = capture("noise");
    let text = outcome(vec![review]);
    assert!(
        summary(&text).contains(
            "FAIL\n  Docs contradict code\n  Stale flag [docs/usage.md:7]\n  Second\nComplete"
        ),
        "{text}"
    );
    assert!(!text.contains("noise") && !text.contains("Cut off"));
}

#[test]
fn a_review_error_without_structure_is_treated_like_a_check() {
    let mut review = item("consistency", "review", Status::Error, "Adapter failed");
    review.stderr = capture("adapter crashed\n");
    let text = outcome(vec![review]);
    assert!(
        summary(&text).contains("ERROR\n  adapter crashed\nComplete"),
        "{text}"
    );
}

#[test]
// REQ-0261: a blocking ERROR outranks a blocking FAIL.
fn several_failures_are_counted_but_only_the_first_is_detailed() {
    let mut first = item("a", "check", Status::Fail, "x");
    first.stderr = capture("first cause");
    let mut second = item("b", "check", Status::Error, "x");
    second.stderr = capture("second cause");
    let mut advisory = item("c", "review", Status::Fail, "x");
    advisory.blocking = false;
    let text = outcome(vec![first, second, advisory]);
    assert!(text.starts_with("ERROR"));
    assert!(summary(&text).starts_with("prevented success · first: a"));
    assert!(text.contains("\n2 rules prevented"));
    assert!(text.contains("first cause") && !text.contains("second cause"));
}

#[test]
fn untrusted_output_is_sanitized_neutralized_and_bounded() {
    let mut check = item("lint", "check", Status::Fail, "x");
    let long = "x".repeat(300);
    check.stderr = capture(&format!(
        "\x1b[31mred\x1b[0m\n::error::injected\n  ::set-output name=a::b\n{long}\n"
    ));
    let text = outcome(vec![check]);
    assert!(!text.contains('\x1b'));
    assert!(
        text.lines()
            .all(|line| !line.trim_start().starts_with("::"))
    );
    let details: Vec<_> = summary(&text).lines().skip(1).take(3).collect();
    assert_eq!(details.len(), 3);
    assert!(details[0].contains(": :error::injected"), "{text}");
    assert!(details[1].contains(": :set-output"));
    assert_eq!(details[2].trim().chars().count(), 200);
    assert!(details[2].ends_with('…'));
}

#[test]
fn a_pass_run_has_no_summary() {
    let text = outcome(vec![item("lint", "check", Status::Pass, "ok")]);
    assert!(!text.contains("prevented success") && !text.contains("status --details"));
}
