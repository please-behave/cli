//! Configuration types and loaders follow the JSON schema contracts.
mod support;
use please::config;
use serde_json::Value;
use std::fs;
use tempfile::TempDir;

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

fn project_schema() -> Value {
    serde_json::from_str(include_str!(
        "../why/contracts/configuration/project.schema.json"
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
                let Some(actual) = restored.pointer(&pointer) else {
                    assert!(
                        declaration.get("default").is_none(),
                        "default for {pointer}"
                    );
                    continue;
                };
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
fn initialized_configuration_matches_project_and_rule_schemas() {
    // REQ-0327: the initialized configuration follows the configuration contracts.
    let root = project();
    let mut config = config::load(root.path()).unwrap().config;
    assert!(config.publish.is_none());
    let schema = project_schema();
    config.publish = Some(config::PublishSettings {
        auth: config::PublishAuth::ApiKey,
        key_env: Some("PLEASE_GATEWAY_KEY".into()),
        repository: Some("example.invalid/owner/repo".into()),
    });
    project_contract(&schema, &config);
    config.publish = Some(config::PublishSettings {
        auth: config::PublishAuth::GithubOidc,
        key_env: None,
        repository: None,
    });
    jsonschema::validator_for(&schema)
        .unwrap()
        .validate(&serde_json::to_value(&config).unwrap())
        .unwrap();
    for rule in [
        "checks/always-ok/rule.toml",
        "checks/deliberately-failing/rule.toml",
        "reviews/product-consistency/rule.toml",
    ] {
        support::assert_rule_contract(
            &fs::read_to_string(root.path().join(".please").join(rule)).unwrap(),
        );
    }
}

#[test]
fn context_retention_default_matches_the_project_schema() {
    let schema = project_schema();
    assert_eq!(
        schema["properties"]["context"]["properties"]["retain_runs"]["default"],
        10
    );
    for text in ["project = 'Example'", "project = 'Example'\n[context]"] {
        let config: config::Config = toml::from_str(text).unwrap();
        assert_eq!(config.context.retain_runs, 10);
    }
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
        ("project = 'Example'\n[publish]", false),
        ("project = 'Example'\n[publish]\nauth = 'api-key'", false),
        ("project = 'Example'\n[publish]\nauth = 'github-oidc'", true),
        (
            "project = 'Example'\n[publish]\nauth = 'unknown'\nkey_env = 'KEY'",
            false,
        ),
        (
            "project = 'Example'\n[publish]\nkey_env = 'PLEASE_GATEWAY_KEY'",
            true,
        ),
        (
            "project = 'Example'\n[publish]\ngateway_url = 'https://gateway.example.invalid'\nkey_env = 'PLEASE_GATEWAY_KEY'",
            false,
        ),
        (
            "project = 'Example'\n[publish]\nkey_env = 'KEY=VALUE'",
            false,
        ),
        (
            "project = 'Example'\n[publish]\nkey_env = 'KEY'\nrepository = ' padded '",
            false,
        ),
        (
            "project = 'Example'\n[publish]\nkey_env = 'KEY'\napi_key = 'fixture'",
            false,
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
fn rule_schema_preserves_common_structure_and_implementation_exclusivity() {
    let root = project();
    for (text, schema_valid, loader_valid) in [
        (
            "id='contract'\ndescription='Check'\nrun_at=['check']\n[check]\ncommand='true'",
            true,
            true,
        ),
        (
            "id='contract'\nname='A readable label'\ndescription='Check'\nenabled=false\n[check]\ncommand='true'",
            true,
            true,
        ),
        ("id='contract'\ndescription='Check'", false, false),
        (
            "id='contract'\ndescription='Check'\nenabled=false\n[check]",
            false,
            false,
        ),
        (
            "id='contract'\ndescription='Check'\nrun_at=['check']\nblocking=true\n[check]\ncommand='true'",
            false,
            false,
        ),
        (
            "id='contract'\ndescription='Check'\nrun_at=['check']\n[check]\ncommand='true'\n[review]\nprompt='absent.md'",
            false,
            false,
        ),
        (
            "id='contract'\ndescription='Check'\nrun_at=['check']\nunknown=true\n[check]\ncommand='true'",
            false,
            false,
        ),
        (
            "id='please-reserved'\ndescription='Check'\nrun_at=['check']\n[check]\ncommand='true'",
            false,
            false,
        ),
        (
            "id='contract'\nname=' padded '\ndescription='Check'\nrun_at=['check']\n[check]\ncommand='true'",
            false,
            false,
        ),
        (
            "id='contract'\ndescription='Check'\nrun_at=['check','check']\n[check]\ncommand='true'",
            false,
            false,
        ),
        // A known-point lookup needs the consuming configuration, beyond the structure schema.
        (
            "id='contract'\ndescription='Check'\nrun_at=['unknown']\n[check]\ncommand='true'",
            true,
            false,
        ),
    ] {
        let parsed: toml::Value = toml::from_str(text).unwrap();
        let value = serde_json::to_value(parsed).unwrap();
        assert_eq!(
            support::rule_contract_validator().is_valid(&value),
            schema_valid,
            "{text}"
        );
        support::write(root.path(), ".please/checks/contract/rule.toml", text);
        assert_eq!(config::load(root.path()).is_ok(), loader_valid, "{text}");
    }
    fs::remove_dir_all(root.path().join(".please/checks/contract")).unwrap();
    let text = "id='contract'\ndescription='Review'\nrun_at=['check']\ndepends_on=[]\n[review]\nprompt='absent.md'";
    let value: toml::Value = toml::from_str(text).unwrap();
    assert!(!support::rule_contract_validator().is_valid(&serde_json::to_value(value).unwrap()));
    support::write(root.path(), ".please/reviews/contract/rule.toml", text);
    assert!(config::load(root.path()).is_err());
}

#[test]
fn rule_version_contract_matches_local_loading() {
    // REQ-0351: optional versions are retained, and invalid values identify the rule.
    let root = project();
    let validator = support::rule_contract_validator();
    let path = ".please/checks/source-directory/rule.toml";
    let max_length = format!("'{}.0'", "9".repeat(62));
    let too_long = format!("'{}.0'", "9".repeat(63));
    for (literal, valid) in [
        (None, true),
        (Some("'1.0'"), true),
        (Some("'1.10'"), true),
        (Some("'0.3'"), true),
        (Some(&max_length), true),
        (Some("'1'"), false),
        (Some("'1.0.0'"), false),
        (Some("'01.1'"), false),
        (Some("'1.01'"), false),
        (Some("1.10"), false),
        (Some("''"), false),
        (Some(r#""1.0\n""#), false),
        (Some("'1.٠'"), false),
        (Some(&too_long), false),
    ] {
        let mut text =
            "id = 'versioned-check'\ndescription = 'Check'\nenabled = false\n".to_owned();
        if let Some(literal) = literal {
            text.push_str(&format!("version = {literal}\n"));
        }
        text.push_str("[check]\ncommand = 'true'\n");
        let parsed: toml::Value = toml::from_str(&text).unwrap();
        let expected = parsed.get("version").and_then(toml::Value::as_str);
        assert_eq!(
            validator.is_valid(&serde_json::to_value(&parsed).unwrap()),
            valid,
            "schema: {literal:?}"
        );
        support::write(root.path(), path, &text);
        let loaded = config::load(root.path());
        if valid {
            let project = loaded.unwrap();
            let check = project
                .checks
                .iter()
                .find(|check| check.id == "versioned-check")
                .unwrap();
            assert_eq!(check.version.as_deref(), expected);
        } else {
            let error = format!("{:#}", loaded.unwrap_err());
            assert!(error.contains(path), "{error}");
            assert!(error.contains("rule \"versioned-check\""), "{error}");
            assert!(error.contains("version must be a string"), "{error}");
        }
    }
}

#[test]
fn parameter_schema_keeps_declaration_loading_and_resolution_separate() {
    let root = project();
    support::write(root.path(), "scopes.txt", "cli\n");
    let mut active: Value = serde_json::from_str(include_str!(
        "../why/contracts/configuration/parameters.schema.json"
    ))
    .unwrap();
    let declaration = &active["$defs"]["declaration"];
    let minimal = serde_json::json!({"type": "file"});
    let validator = jsonschema::validator_for(declaration).unwrap();
    assert!(validator.is_valid(&minimal));
    let parsed: please::parameters::Declaration = serde_json::from_value(minimal).unwrap();
    let defaults = serde_json::to_value(parsed).unwrap();
    for field in ["required", "description", "value"] {
        assert_eq!(
            defaults[field], declaration["properties"][field]["default"],
            "{field}"
        );
    }
    active["$ref"] = serde_json::json!("#/$defs/activeDeclarations");
    let validator = jsonschema::validator_for(&active).unwrap();
    for (declaration, valid) in [
        ("[scope]\ntype='string'\nvalue='text with spaces'", true),
        ("[scope]\ntype='file'\nvalue='scopes.txt'", true),
        ("[scope]\ntype='string'", true),
        ("[scope]\ntype='number'", false),
        ("['bad-name']\ntype='string'", false),
        ("[scope]\ntype='string'\nvalue=3", false),
    ] {
        let values: toml::Value = toml::from_str(declaration).unwrap();
        assert_eq!(
            validator.is_valid(&serde_json::to_value(values).unwrap()),
            valid,
            "{declaration}"
        );
        let rule = format!(
            "id='contract'\ndescription='Parameters'\nrun_at=['check']\n[check]\ncommand='true'\n{}",
            declaration.replace('[', "[parameters.")
        );
        support::assert_rule_contract(&rule);
        support::write(root.path(), ".please/checks/contract/rule.toml", &rule);
        let project = config::load(root.path()).unwrap();
        let check = project
            .checks
            .iter()
            .find(|rule| rule.id == "contract")
            .unwrap();
        assert_eq!(
            check.parameters.resolve(&project.root, &check.id).is_ok(),
            valid,
            "{declaration}"
        );
    }
}
