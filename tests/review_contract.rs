mod support;
use please::{
    config, git_snapshot,
    limits::*,
    report::{Finding, Status},
    review_contract::{self, ReviewResponse, ReviewVerdict},
    reviews,
};
use serde_json::{Value, json};
use std::collections::BTreeSet;

fn object_contract(schema: &Value, wire: &Value) {
    let keys: BTreeSet<_> = wire
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(
        keys,
        schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect()
    );
    assert_eq!(
        keys,
        schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect()
    );
    assert_eq!(schema["additionalProperties"], false);
}
#[test]
fn schema_and_wire_types_cover_the_same_fields_and_verdicts() {
    let schema = review_contract::schema(&["one", "two"]);
    let wire = serde_json::to_value(ReviewResponse {
        reviews: vec![ReviewVerdict {
            id: "one".into(),
            status: Status::Pass,
            reason: "Evidence inspected".into(),
            findings: vec![Finding {
                source: None,
                message: "Found it".into(),
                path: None,
                line: None,
            }],
        }],
    })
    .unwrap();
    object_contract(&schema, &wire);
    let verdict = &schema["properties"]["reviews"]["items"];
    object_contract(verdict, &wire["reviews"][0]);
    object_contract(
        &verdict["properties"]["findings"]["items"],
        &wire["reviews"][0]["findings"][0],
    );
    assert_eq!(schema["properties"]["reviews"]["minItems"], 2);
    assert_eq!(schema["properties"]["reviews"]["maxItems"], 2);
    assert_eq!(verdict["properties"]["id"]["enum"], json!(["one", "two"]));
    assert_eq!(
        verdict["properties"]["status"]["enum"],
        json!(["PASS", "FAIL", "ERROR"])
    );
    assert_eq!(
        verdict["properties"]["reason"]["maxLength"],
        MAX_SUMMARY_CHARS
    );
    assert_eq!(verdict["properties"]["findings"]["maxItems"], MAX_FINDINGS);
    assert_eq!(
        verdict["properties"]["findings"]["items"]["properties"]["message"]["maxLength"],
        MAX_FINDING_MESSAGE_BYTES
    );
}
#[test]
fn review_contract_enforces_boundaries_and_semantic_evidence_locally() {
    let root = tempfile::tempdir().unwrap();
    assert!(
        support::command("git")
            .args(["init", "--quiet"])
            .current_dir(root.path())
            .status()
            .unwrap()
            .success()
    );
    config::init(root.path()).unwrap();
    support::configure_adapter(root.path());
    let project = config::load(root.path()).unwrap();
    let snapshot = git_snapshot::capture(root.path()).unwrap();
    let active = [&project.reviews[0]];
    let valid = json!({"reviews":[{"id":active[0].id, "status":"PASS", "reason":"é".repeat(MAX_SUMMARY_CHARS), "findings":[{"message":"é".repeat(MAX_FINDING_MESSAGE_BYTES / 2), "path":".please/README.md", "line":1}]}]});
    assert!(reviews::parse(&project, &active, &snapshot, &valid.to_string()).is_ok());
    for case in [
        "long-summary",
        "multiline",
        "blank-summary",
        "too-many-findings",
        "long-byte-message",
        "blank-message",
        "zero-line",
        "missing-path",
        "header-evidence",
        "extra-id",
        "duplicate-id",
        "not-run",
    ] {
        let mut value = valid.clone();
        let verdict = &mut value["reviews"][0];
        match case {
            "long-summary" => verdict["reason"] = "x".repeat(MAX_SUMMARY_CHARS + 1).into(),
            "multiline" => verdict["reason"] = "not\na line".into(),
            "blank-summary" => verdict["reason"] = "  ".into(),
            "too-many-findings" => {
                verdict["findings"] = vec![verdict["findings"][0].clone(); MAX_FINDINGS + 1].into()
            }
            "long-byte-message" => {
                verdict["findings"][0]["message"] =
                    "é".repeat(MAX_FINDING_MESSAGE_BYTES / 2 + 1).into()
            }
            "blank-message" => verdict["findings"][0]["message"] = "  ".into(),
            "zero-line" => verdict["findings"][0]["line"] = 0.into(),
            "missing-path" => verdict["findings"][0]["path"] = "missing.rs".into(),
            "header-evidence" => {
                verdict["findings"][0]["source"] = "commit-header".into();
                verdict["findings"][0]["path"] = Value::Null;
            }
            "extra-id" => verdict["id"] = "not-requested".into(),
            "duplicate-id" => value["reviews"]
                .as_array_mut()
                .unwrap()
                .push(valid["reviews"][0].clone()),
            "not-run" => verdict["status"] = "NOT_RUN".into(),
            _ => unreachable!(),
        }
        assert!(
            reviews::parse(&project, &active, &snapshot, &value.to_string()).is_err(),
            "accepted {case}"
        );
    }
}
