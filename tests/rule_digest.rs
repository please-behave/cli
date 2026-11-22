// REQ-0352 REQ-0353
#[path = "../src/rule_digest/canonical.rs"]
mod canonical;
mod support;

use please::rule_digest::{RuleSource, from_directory};
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::Path;
use support::{command, write};

const REMOTE: &str = "ssh://git@GitHub.COM:443/example/catalog.git";
const REVIEW: &str = ".please/reviews/summary";

#[derive(Deserialize)]
struct Vector {
    name: String,
    input: Value,
    digest: String,
}

fn vectors() -> Vec<Vector> {
    // Unmodified from please-behave/ledger at cc98b9c27e104f55c5d937f2b2f8b01bf90659bc,
    // why/contracts/rule-digest/vectors.json (ledger CON-0005).
    serde_json::from_str(include_str!("fixtures/ledger-rule-digest-vectors.json")).unwrap()
}

fn write_definition(root: &Path, path: &str, input: &Value) {
    let mut definition = toml::Table::new();
    for key in ["id", "version", "name", "description", "blocking"] {
        if let Some(value) = input.get(key) {
            let value = match value {
                Value::String(text) => toml::Value::String(text.clone()),
                Value::Bool(flag) => toml::Value::Boolean(*flag),
                _ => panic!("invalid test input for {key}"),
            };
            definition.insert(key.into(), value);
        }
    }
    definition
        .entry("description")
        .or_insert_with(|| toml::Value::String("Check formatting".into()));
    let (kind, implementation) = match input["kind"].as_str().unwrap() {
        "review" => ("review", ("prompt", "prompt.md")),
        "check" => ("check", ("command", "never-run")),
        kind => panic!("unknown kind {kind}"),
    };
    definition.insert(
        kind.into(),
        toml::Value::Table(toml::Table::from_iter([(
            implementation.0.to_owned(),
            toml::Value::String(implementation.1.to_owned()),
        )])),
    );
    write(root, &format!("{path}/rule.toml"), &definition.to_string());
}

fn write_rule(root: &Path, path: &str, input: &Value) {
    write_definition(root, path, input);
    if let Some(prompt) = input["prompt"].as_str() {
        write(root, &format!("{path}/prompt.md"), prompt);
    }
    if let Some(knowledge) = input.get("knowledge") {
        // JSON flow syntax is valid YAML and preserves escaped control characters.
        write(
            root,
            &format!("{path}/rule.yaml"),
            &format!("{knowledge:#}\n"),
        );
    }
}

fn imported(root: &Path, path: &str, git: &str) -> anyhow::Result<Option<String>> {
    from_directory(RuleSource::Imported { git }, root, Path::new(path))
}

#[test]
fn matches_the_ledger_vectors() {
    for vector in vectors() {
        assert_eq!(
            canonical::digest(vector.input.clone()).unwrap(),
            vector.digest,
            "{}",
            vector.name
        );
        let root = tempfile::tempdir().unwrap();
        let path = vector.input["path"].as_str().unwrap();
        write_rule(root.path(), path, &vector.input);
        let actual = imported(root.path(), path, REMOTE).unwrap();
        // The ledger's check vector has no knowledge and is ineligible in the CLI.
        let expected = vector.input.get("knowledge").map(|_| vector.digest);
        assert_eq!(actual, expected, "{}", vector.name);
    }
}

#[test]
fn local_and_imported_checks_use_the_same_source_identity_and_repository_relative_path() {
    let root = tempfile::tempdir().unwrap();
    for args in [
        ["init", "-q", "."].as_slice(),
        &["remote", "add", "origin", REMOTE],
    ] {
        assert!(
            command("git")
                .args(args)
                .current_dir(root.path())
                .status()
                .unwrap()
                .success()
        );
    }
    let mut input = vectors().remove(0).input;
    input["description"] = json!("Check formatting");
    input["knowledge"] = json!({"purpose": "Keep formatting consistent."});
    let path = input["path"].as_str().unwrap();
    write_rule(root.path(), path, &input);

    let expected = Some(canonical::digest(input.clone()).unwrap());
    assert_eq!(
        from_directory(RuleSource::Local, root.path(), Path::new(path)).unwrap(),
        expected
    );
    assert_eq!(
        imported(
            root.path(),
            ".please//checks/./format",
            "https://github.com/example/catalog/",
        )
        .unwrap(),
        expected
    );
}

#[test]
fn knowledge_uses_parsed_yaml_without_comments_formatting_or_line_endings() {
    let root = tempfile::tempdir().unwrap();
    let mut input = vectors().remove(1).input;
    input["knowledge"] = json!({"items": ["one\ntwo\n", true], "title": "Example"});
    write_rule(root.path(), REVIEW, &input);
    let expected = imported(root.path(), REVIEW, REMOTE).unwrap();
    for text in [
        "# Comment\ntitle: 'Example'\nitems:\n  - |\n    one\n    two\n  - !!bool \"true\"\n",
        "items: [\"one\\ntwo\\n\", true]\r\ntitle: Example # comment\r\n",
    ] {
        write(root.path(), &format!("{REVIEW}/rule.yaml"), text);
        assert_eq!(imported(root.path(), REVIEW, REMOTE).unwrap(), expected);
    }
}

#[test]
fn every_covered_field_changes_the_digest() {
    let root = tempfile::tempdir().unwrap();
    let input = vectors().remove(1).input;
    write_rule(root.path(), REVIEW, &input);
    let original = imported(root.path(), REVIEW, REMOTE).unwrap();

    for (field, replacement) in [
        ("id", json!("other")),
        ("version", json!("2.11")),
        ("name", json!("Other name")),
        ("description", json!("Other description")),
        ("blocking", json!(true)),
        ("prompt", json!("Other prompt.")),
        ("knowledge", json!({"extension": false})),
        ("kind", json!("check")),
    ] {
        let mut changed = input.clone();
        changed[field] = replacement;
        if field == "kind" {
            changed.as_object_mut().unwrap().remove("blocking");
        }
        write_rule(root.path(), REVIEW, &changed);
        assert_ne!(
            imported(root.path(), REVIEW, REMOTE).unwrap(),
            original,
            "{field}"
        );
    }

    let other_directory = ".please/reviews/other";
    write_rule(root.path(), other_directory, &input);
    assert_ne!(
        imported(root.path(), other_directory, REMOTE).unwrap(),
        original
    );
    write_rule(root.path(), REVIEW, &input);
    assert_ne!(
        imported(root.path(), REVIEW, "git@github.com:example/other.git").unwrap(),
        original
    );
}

#[test]
fn review_uses_effective_blocking_and_ignores_execution_settings() {
    let root = tempfile::tempdir().unwrap();
    let mut input = vectors().remove(1).input;
    input["blocking"] = json!(true);
    write_rule(root.path(), REVIEW, &input);
    let expected = imported(root.path(), REVIEW, REMOTE).unwrap();

    input.as_object_mut().unwrap().remove("blocking");
    write_rule(root.path(), REVIEW, &input);
    let rule = format!("{REVIEW}/rule.toml");
    let text = std::fs::read_to_string(root.path().join(&rule)).unwrap();
    write(
        root.path(),
        &rule,
        &format!("enabled = false\nrun_at = ['pre-push']\n{text}"),
    );
    assert_eq!(imported(root.path(), REVIEW, REMOTE).unwrap(), expected);
}

#[test]
fn unversioned_rules_have_no_digest_and_invalid_knowledge_is_rejected() {
    let root = tempfile::tempdir().unwrap();
    let mut input = vectors().remove(1).input;
    let version = input.as_object_mut().unwrap().remove("version").unwrap();
    write_rule(root.path(), REVIEW, &input);
    assert_eq!(imported(root.path(), REVIEW, REMOTE).unwrap(), None);

    input["version"] = version;
    write_definition(root.path(), REVIEW, &input);
    for invalid in [
        "- item",
        "flag: 1",
        "flag: null",
        "true: value",
        "key: first\nkey: second",
        "\"\\r\\n\": first\n\"\\n\": second",
        "key: first\n---\nkey: second",
        "key: &key [*key]",
    ] {
        write(root.path(), &format!("{REVIEW}/rule.yaml"), invalid);
        assert!(imported(root.path(), REVIEW, REMOTE).is_err(), "{invalid}");
    }
}
