mod support;
use please::{
    config, git_snapshot,
    limits::*,
    report::Finding,
    review_contract::{self, AdapterVerdict},
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
fn schema_and_wire_types_cover_the_same_fields() {
    let schema = review_contract::schema();
    let wire = serde_json::to_value(AdapterVerdict {
        reason: "Evidence inspected".into(),
        explanation: Some("Longer explanation".into()),
        findings: vec![Finding {
            message: "Found it".into(),
            path: None,
            line: None,
        }],
    })
    .unwrap();
    // The optional explanation is present on the wire and absent from the required keys.
    assert_eq!(
        wire.as_object().unwrap().keys().collect::<BTreeSet<_>>(),
        schema["properties"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<BTreeSet<_>>()
    );
    assert_eq!(schema["required"], json!(["reason", "findings"]));
    assert_eq!(schema["additionalProperties"], false);
    object_contract(
        &schema["properties"]["findings"]["items"],
        &wire["findings"][0],
    );
    let properties = &schema["properties"];
    assert!(properties.get("status").is_none() && properties.get("id").is_none());
    assert_eq!(properties["reason"]["maxLength"], MAX_SUMMARY_CHARS);
    assert_eq!(
        properties["explanation"]["maxLength"],
        MAX_EXPLANATION_BYTES
    );
    assert_eq!(properties["findings"]["maxItems"], MAX_FINDINGS);
    assert_eq!(
        properties["findings"]["items"]["properties"]["message"]["maxLength"],
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
    please::init::init(root.path()).unwrap();
    support::configure_adapter(root.path());
    let project = config::load(root.path()).unwrap();
    let snapshot = git_snapshot::capture(root.path()).unwrap();
    let active = [&project.reviews[0]];
    let valid = json!({"reviews":[{"id":active[0].id, "status":"PASS", "reason":"é".repeat(MAX_SUMMARY_CHARS), "explanation":"é".repeat(MAX_EXPLANATION_BYTES / 2), "findings":[{"message":"é".repeat(MAX_FINDING_MESSAGE_BYTES / 2), "path":".please/README.md", "line":1}]}]});
    assert!(reviews::parse(&project, &active, &snapshot, &valid.to_string()).is_ok());
    for case in [
        "long-summary",
        "multiline",
        "blank-summary",
        "blank-explanation",
        "long-explanation",
        "too-many-findings",
        "long-byte-message",
        "blank-message",
        "zero-line",
        "missing-path",
        "message-line-without-message",
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
            "blank-explanation" => verdict["explanation"] = "  ".into(),
            "long-explanation" => {
                verdict["explanation"] = "é".repeat(MAX_EXPLANATION_BYTES / 2 + 1).into()
            }
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
            "message-line-without-message" => verdict["findings"][0]["path"] = Value::Null,
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
