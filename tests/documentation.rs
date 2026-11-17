//! The public reference and executable examples must follow Serde's contract.
mod support;
use please::{
    config, planner,
    provider::{Provider, Request},
    report::Status,
};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use std::{collections::BTreeMap, fs};
use tempfile::TempDir;

const CONFIGURATION: &str = include_str!("../docs/configuration.md");
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
                let empty = if key == "parameters" {
                    serde_json::json!({})
                } else if key == "depends_on" {
                    Value::Null
                } else if key == "blocking" {
                    Value::Bool(true)
                } else {
                    Value::Array(vec![])
                };
                let default = defaults.pointer(&pointer).unwrap_or_else(|| {
                    assert!(matches!(
                        key.as_str(),
                        "files" | "context" | "parameters" | "depends_on" | "blocking"
                    ));
                    &empty
                });
                let expected = match (key.as_str(), default) {
                    (_, Value::Object(_)) => None, // Child fields are visited recursively.
                    _ => Some(default.to_string()),
                };
                if let Some(expected) = expected {
                    let expected = format!("`{expected}`");
                    assert!(
                        cell.contains(&expected),
                        "wrong default for {label}: expected {expected}, found {cell}"
                    );
                }
            }
        }
    }
}
fn project_schema() -> Value {
    serde_json::from_str(include_str!(
        "../knowledge/contracts/configuration/project.schema.json"
    ))
    .unwrap()
}

fn project_contract(schema: &Value, config: &config::Config) {
    let sample = serde_json::to_value(config).unwrap();
    jsonschema::validator_for(schema)
        .unwrap()
        .validate(&sample)
        .unwrap();
    let mut paths = Vec::new();
    field_paths(&sample, vec![], &mut paths);
    // Each object has exactly the documented keys, including optional defaulted fields.
    for path in std::iter::once(vec![]).chain(paths.iter().cloned()) {
        let pointer = if path.is_empty() {
            String::new()
        } else {
            format!("/{}", path.join("/"))
        };
        let value = sample.pointer(&pointer).unwrap();
        let declaration = path
            .iter()
            .fold(schema, |node, key| &node["properties"][key]);
        if let Some(object) = value.as_object() {
            let properties = declaration["properties"].as_object().unwrap();
            assert!(properties.keys().eq(object.keys()), "{pointer}");
        }
    }
    for path in paths {
        let key = path.last().unwrap();
        let pointer = format!("/{}", path.join("/"));
        let parent = if path.len() == 1 {
            String::new()
        } else {
            format!("/{}", path[..path.len() - 1].join("/"))
        };
        let parent_schema = path[..path.len() - 1]
            .iter()
            .fold(schema, |node, key| &node["properties"][key]);
        let required = parent_schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == key);
        let declaration = &parent_schema["properties"][key];
        let mut omitted = sample.clone();
        omitted
            .pointer_mut(&parent)
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove(key);
        match serde_json::from_value::<config::Config>(omitted) {
            Err(_) => assert!(required, "required field {pointer}"),
            Ok(parsed) => {
                assert!(!required, "optional field {pointer}");
                let restored = serde_json::to_value(parsed).unwrap();
                let actual = restored.pointer(&pointer).unwrap();
                if actual.is_object() {
                    assert_eq!(declaration["default"], serde_json::json!({}), "{pointer}");
                } else {
                    assert_eq!(&declaration["default"], actual, "default for {pointer}");
                }
            }
        }
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
    support::commit_all(root.path());
    support::use_head_as_change_base();
    root
}
#[test]
fn every_configuration_field_required_marker_and_default_is_documented() {
    // REQ-0327, REQ-0328: documented defaults, fields and required markers match the configuration types.
    let root = project();
    let p = config::load(root.path()).unwrap();
    project_contract(&project_schema(), &p.config);
    contract(
        &please::external::Import {
            git: "https://example.invalid/rules.git".into(),
            path: "repository".into(),
            reference: "main".into(),
            execution_points: Default::default(),
            rule_parameters: Default::default(),
        },
        section(CONFIGURATION, "Import rule sets from Git"),
        "",
    );
    contract(
        &please::external::ProviderImport {
            id: "quality".into(),
            git: "https://example.invalid/providers.git".into(),
            path: "providers/quality".into(),
            reference: "main".into(),
        },
        section(CONFIGURATION, "Import providers from Git"),
        "",
    );
    let rule = config::RuleDefinition {
        id: "example".into(),
        depends_on: Some(vec![]),
        name: Some("Example".into()),
        description: "Example".into(),
        enabled: true,
        blocking: Some(true),
        run_at: vec!["pre-push".into()],
        parameters: Default::default(),
        check: Some(config::CheckDefinition {
            resource: None,
            command: please::check_command::Command::new("sh @rule/index.sh").unwrap(),
            timeout_seconds: 30,
        }),
        review: Some(config::ReviewDefinition {
            parameters: Default::default(),
            include_content: true,
            include_change_set: false,
            trigger: Some(config::ReviewTrigger {
                changed_files: vec!["docs/**".into()],
            }),
            provider: None,
            prompt: "prompt.md".into(),
            context: vec![config::ReviewContext {
                path: "README.md".into(),
                headings: vec![],
            }],
        }),
    };
    let shared = format!(
        "{}\n{}\n{}",
        section(CONFIGURATION, "Fields shared by checks and reviews"),
        section(CONFIGURATION, "Add a check"),
        section(CONFIGURATION, "Add a review")
    );
    contract(&rule, &shared, "");
    contract(
        &please::parameters::Declaration {
            kind: "file".into(),
            required: true,
            description: Some("Allowed scopes".into()),
            value: Some(toml::Value::String(".commit-scopes".into())),
        },
        section(CONFIGURATION, "Rule parameters"),
        "parameters.<name>.",
    );
    let adapter: config::ProviderConfig = toml::from_str(block(
        section(CONFIGURATION, "Configure the AI provider"),
        "toml",
    ))
    .unwrap();
    contract(
        &adapter,
        section(CONFIGURATION, "Configure the AI provider"),
        "",
    );
    contract(
        &config::ReviewContext {
            path: "README.md".into(),
            headings: vec![],
        },
        section(CONFIGURATION, "Add a review"),
        "review.context.",
    );
    contract(
        &config::RuleSet {
            id: "quality".into(),
            description: "Reusable rules".into(),
            rules: vec![],
            run_at: vec![],
        },
        section(CONFIGURATION, "Group rules into rule sets"),
        "",
    );
}
#[test]
fn project_schema_and_loader_agree_on_types_bounds_and_unknown_fields() {
    let root = project();
    let schema = project_schema();
    let validator = jsonschema::validator_for(&schema).unwrap();
    for (text, valid) in [
        ("project = 'Example'", true),
        ("project = 'Example'\n[execution]\nmax_parallel = 1", true),
        (
            "project = 'Example'\n[execution]\nfail_fast_termination_grace_seconds = 3600",
            true,
        ),
        (
            "project = 'Example'\n[context]\nretain_runs = 1\nmax_section_bytes = 1024",
            true,
        ),
        (
            "project = 'Example'\n[execution_points]\ncustom = ['release']",
            true,
        ),
        (
            "project = 'Example'\n[execution]\n[context]\n[execution_points]",
            true,
        ),
        ("project = 'Example'\nversion = 0", false),
        ("project = 'Example'\nschema_version = 0", false),
        ("project = 'Example'\nunknown = true", false),
        ("project = '   '", false),
        ("max_display_files = 5", false),
        ("project = 'Example'\nmax_display_files = 0", false),
        ("project = 'Example'\nmax_display_files = 6", false),
        ("project = 'Example'\n[execution]\nmax_parallel = 0", false),
        (
            "project = 'Example'\n[execution]\nmax_parallel = 1.5",
            false,
        ),
        ("project = 'Example'\n[execution]\nfail_fast = true", false),
        (
            "project = 'Example'\n[execution]\nfail_fast_termination_grace_seconds = 0",
            false,
        ),
        (
            "project = 'Example'\n[execution]\nfail_fast_termination_grace_seconds = 3601",
            false,
        ),
        ("project = 'Example'\n[context]\nretain_runs = 10001", false),
        (
            "project = 'Example'\n[context]\nmax_section_bytes = 1023",
            false,
        ),
        (
            "project = 'Example'\n[execution_points]\ncustom = ['check']",
            false,
        ),
        (
            "project = 'Example'\n[execution_points]\ncustom = ['Release']",
            false,
        ),
        (
            "project = 'Example'\n[execution_points]\ncustom = ['release', 'release']",
            false,
        ),
    ] {
        let raw: toml::Value = toml::from_str(text).unwrap();
        let json = serde_json::to_value(raw).unwrap();
        assert_eq!(validator.is_valid(&json), valid, "schema: {text}");
        support::write(root.path(), ".please/config.toml", text);
        assert_eq!(config::load(root.path()).is_ok(), valid, "loader: {text}");
    }
}

#[test]
fn documentation_contract_detects_missing_fields_and_wrong_defaults() {
    let root = project();
    let _p = config::load(root.path()).unwrap();
    let adapter: config::ProviderConfig = toml::from_str(block(
        section(CONFIGURATION, "Configure the AI provider"),
        "toml",
    ))
    .unwrap();
    let original = section(CONFIGURATION, "Configure the AI provider");
    for bad in [
        original
            .lines()
            .filter(|line| !line.starts_with("| `command` |"))
            .collect::<Vec<_>>()
            .join("\n"),
        original.replace("Optional; `[]`", "Optional; `[\"all\"]`"),
        original.replace("Required | Executable", "Optional | Executable"),
        original.replace("| `command` |", "| `removed` |"),
    ] {
        assert!(std::panic::catch_unwind(|| contract(&adapter, &bad, "")).is_err());
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
    // REQ-0329: configuration examples load and produce an executable plan without a provider call.
    let root = project();
    assert_eq!(
        please::verify(root.path(), &Never).unwrap().overall,
        Status::Pass
    );
    let base = root.path().join(".please");
    fs::write(
        base.join("config.toml"),
        block(section(CONFIGURATION, "Project settings"), "toml"),
    )
    .unwrap();
    for (kind, name, title) in [
        ("checks", "readme", "Add a check"),
        ("reviews", "architecture", "Add a review"),
    ] {
        let dir = base.join(kind).join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("rule.toml"),
            block(section(CONFIGURATION, title), "toml"),
        )
        .unwrap();
        fs::write(
            dir.join(if kind == "checks" {
                "index.sh"
            } else {
                "prompt.md"
            }),
            block(
                section(CONFIGURATION, title),
                if kind == "checks" { "sh" } else { "markdown" },
            ),
        )
        .unwrap();
    }
    fs::create_dir_all(base.join("rule-sets")).unwrap();
    fs::write(
        base.join("rule-sets/repository.toml"),
        block(section(CONFIGURATION, "Group rules into rule sets"), "toml"),
    )
    .unwrap();
    support::write(
        root.path(),
        ".please/provider.toml",
        block(section(CONFIGURATION, "Configure the AI provider"), "toml"),
    );
    let p = config::load(root.path()).unwrap();
    let plan = planner::build(&p, Some("check"), false).unwrap().plan;
    assert!(plan.executable());
    assert_eq!(plan.scheduled.len(), 2);
}

#[test]
fn documented_adapter_configuration_is_valid() {
    let example: config::ProviderConfig = toml::from_str(block(
        section(CONFIGURATION, "Configure the AI provider"),
        "toml",
    ))
    .unwrap();
    assert_eq!(example.command, "python3");
    assert_eq!(example.timeout_seconds, 180);
}
