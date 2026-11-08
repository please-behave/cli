//! CFG-001: the public reference and executable examples must follow Serde's contract.
mod support;
use please::{
    config, planner,
    provider::{Provider, Request},
    report::Status,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{collections::BTreeMap, fs, process::Command};
use tempfile::TempDir;

const README: &str = include_str!("../README.md");
fn section<'a>(text: &'a str, title: &str) -> &'a str {
    text.split_once(&format!("### {title}\n"))
        .unwrap()
        .1
        .split("\n### ")
        .next()
        .unwrap()
}
fn rows(text: &str) -> BTreeMap<String, String> {
    text.split("\n\n")
        .filter(|paragraph| paragraph.trim_start().starts_with("| Field |"))
        .flat_map(str::lines)
        .filter_map(|line| {
            let cells: Vec<_> = line.split('|').map(str::trim).collect();
            let key = cells.get(1)?.strip_prefix('`')?.strip_suffix('`')?;
            Some((key.to_owned(), cells.get(2)?.to_string()))
        })
        .collect()
}
fn field_paths(value: &Value, parent: Vec<String>, paths: &mut Vec<Vec<String>>) {
    for (key, child) in value.as_object().unwrap() {
        let mut path = parent.clone();
        path.push(key.clone());
        paths.push(path.clone());
        if child.is_object() {
            field_paths(child, path, paths);
        }
    }
}
fn contract<T: Serialize + DeserializeOwned>(sample: &T, text: &str, prefix: &str) {
    let value = serde_json::to_value(sample).unwrap();
    let documented = rows(text);
    let fields = value.as_object().unwrap();
    for key in documented.keys().filter_map(|key| key.strip_prefix(prefix)) {
        if !key.contains('.') {
            assert!(
                fields.contains_key(key),
                "obsolete documented field: {prefix}{key}"
            );
        }
    }
    let mut paths = Vec::new();
    field_paths(&value, vec![], &mut paths);
    for path in paths {
        let key = path.last().unwrap();
        let label = format!("{prefix}{}", path.join("."));
        let pointer = format!("/{}", path.join("/"));
        let parent = if path.len() == 1 {
            String::new()
        } else {
            format!("/{}", path[..path.len() - 1].join("/"))
        };
        let cell = documented
            .get(&label)
            .unwrap_or_else(|| panic!("undocumented configuration field: {label}"));
        let mut omitted = value.clone();
        omitted
            .pointer_mut(&parent)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove(key);
        match serde_json::from_value::<T>(omitted) {
            Err(_) => assert!(cell.starts_with("Required"), "required field {label}"),
            Ok(parsed) => {
                assert!(cell.starts_with("Optional"), "optional field {label}");
                let defaults = serde_json::to_value(parsed).unwrap();
                let empty = Value::Array(vec![]);
                let default = defaults.pointer(&pointer).unwrap_or_else(|| {
                    assert!(matches!(key.as_str(), "files" | "context"));
                    &empty
                });
                let expected = match (key.as_str(), default) {
                    ("model" | "reasoning_effort", Value::Null) => Some("Codex default".into()),
                    (_, Value::Object(_)) => None, // Child fields are visited recursively.
                    _ => Some(default.to_string()),
                };
                if let Some(expected) = expected {
                    let expected = if expected == "Codex default" {
                        expected
                    } else {
                        format!("`{expected}`")
                    };
                    assert!(
                        cell.contains(&expected),
                        "wrong default for {label}: expected {expected}, found {cell}"
                    );
                }
            }
        }
    }
}
fn project() -> TempDir {
    let root = tempfile::tempdir().unwrap();
    assert!(
        Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(root.path())
            .status()
            .unwrap()
            .success()
    );
    config::init(root.path()).unwrap();
    root
}
#[test]
fn every_configuration_field_required_marker_and_default_is_documented() {
    let root = project();
    let p = config::load(root.path()).unwrap();
    contract(&p.config, section(README, "Project settings"), "");
    contract(
        &please::external::Import {
            git: "https://example.invalid/rules.git".into(),
            path: "repository".into(),
            reference: "main".into(),
            execution_points: Default::default(),
            rule_parameters: Default::default(),
        },
        section(README, "Import rule sets from Git"),
        "",
    );
    let rule = config::RuleDefinition {
        id: "example".into(),
        name: Some("Example".into()),
        description: "Example".into(),
        enabled: true,
        blocking: true,
        run_at: vec!["pre-push".into()],
        parameters: Default::default(),
        check: Some(config::CheckDefinition {
            resource: None,
            entrypoint: Some("index.sh".into()),
            command: None,
            interpreter: None,
            timeout_seconds: 30,
        }),
        review: Some(config::ReviewDefinition {
            include_diff: true,
            trigger: Some(config::ReviewTrigger {
                changed_files: vec!["docs/**".into()],
            }),
            provider: None,
            prompt: "prompt.md".into(),
            files: vec!["README.md".into()],
            context: vec![config::ReviewContext {
                path: "README.md".into(),
                headings: vec![],
            }],
        }),
    };
    let shared = format!(
        "{}\n{}\n{}",
        section(README, "Fields shared by checks and reviews"),
        section(README, "Add a check"),
        section(README, "Add a review")
    );
    contract(&rule, &shared, "");
    contract(
        &please::parameters::Declaration {
            kind: "file".into(),
            required: true,
            description: Some("Allowed scopes".into()),
            value: Some(toml::Value::String(".commit-scopes".into())),
        },
        section(README, "Rule parameters"),
        "parameters.<name>.",
    );
    contract(
        &p.provider,
        section(README, "Configure the AI provider"),
        "",
    );
    contract(
        &p.provider.review_profile,
        section(README, "Configure the AI provider"),
        "review_profile.",
    );
    contract(
        &config::ReviewContext {
            path: "README.md".into(),
            headings: vec![],
        },
        section(README, "Add a review"),
        "review.context.",
    );
    contract(
        &config::RuleSet {
            id: "quality".into(),
            description: "Reusable rules".into(),
            rules: vec![],
            run_at: vec![],
        },
        section(README, "Group rules into rule sets"),
        "",
    );
}
#[test]
fn documentation_contract_detects_missing_fields_and_wrong_defaults() {
    let root = project();
    let p = config::load(root.path()).unwrap();
    let original = section(README, "Configure the AI provider");
    for bad in [
        original
            .lines()
            .filter(|line| !line.starts_with("| `model` |"))
            .collect::<Vec<_>>()
            .join("\n"),
        original.replace("Optional; `[]`", "Optional; `[\"all\"]`"),
        original.replace("Optional; `2000`", "Optional; `20000`"),
        original.replace(
            "| `review_profile.tool_output_token_limit` |",
            "| `removed` |",
        ),
    ] {
        assert!(std::panic::catch_unwind(|| contract(&p.provider, &bad, "")).is_err());
    }
}
fn block<'a>(text: &'a str, kind: &str) -> &'a str {
    text.split_once(&format!("```{kind}\n"))
        .unwrap()
        .1
        .split_once("\n```")
        .unwrap()
        .0
}
struct Never;
impl Provider for Never {
    fn review(&self, _: &Request<'_>) -> anyhow::Result<String> {
        panic!("documentation tests must not call a provider")
    }
}
#[test]
fn readme_examples_validate_and_plan_without_ai_or_check_execution() {
    let root = project();
    assert_eq!(
        please::verify(root.path(), &Never).unwrap().overall,
        Status::Pass
    );
    let base = root.path().join(".please");
    fs::write(
        base.join("config.toml"),
        block(section(README, "Project settings"), "toml"),
    )
    .unwrap();
    for (kind, name, title) in [
        ("checks", "readme", "Add a check"),
        ("reviews", "architecture", "Add a review"),
    ] {
        let dir = base.join(kind).join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("rule.toml"), block(section(README, title), "toml")).unwrap();
        fs::write(
            dir.join(if kind == "checks" {
                "index.sh"
            } else {
                "prompt.md"
            }),
            block(
                section(README, title),
                if kind == "checks" { "sh" } else { "markdown" },
            ),
        )
        .unwrap();
    }
    fs::create_dir_all(base.join("rule-sets")).unwrap();
    fs::write(
        base.join("rule-sets/repository.toml"),
        block(section(README, "Group rules into rule sets"), "toml"),
    )
    .unwrap();
    let p = config::load(root.path()).unwrap();
    let plan = planner::build(&p, Some("check"), false).unwrap().plan;
    assert!(plan.executable());
    assert_eq!(plan.scheduled.len(), 2);
}

#[test]
fn documented_provider_scaffold_matches_the_implementation() {
    let example: config::ProviderConfig =
        toml::from_str(block(section(README, "Configure the AI provider"), "toml")).unwrap();
    assert_eq!(
        serde_json::to_value(example).unwrap(),
        serde_json::to_value(config::ProviderConfig::scaffold()).unwrap()
    );
}
