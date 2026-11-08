//! Rule declarations and import-local values, resolved only for active execution.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
    process::Command,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Declaration {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub required: bool,
    pub description: Option<String>,
    pub value: Option<toml::Value>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Binding {
    pub declarations: BTreeMap<String, Declaration>,
    pub values: BTreeMap<String, toml::Value>,
    pub imported: bool,
}

pub type Environment = BTreeMap<String, String>;

impl Binding {
    pub(crate) fn local(declarations: BTreeMap<String, Declaration>) -> Self {
        Self {
            declarations,
            ..Self::default()
        }
    }
    pub(crate) fn bind_import(&mut self, values: BTreeMap<String, toml::Value>) {
        self.imported = true;
        self.values = values;
    }
    pub fn resolve(&self, root: &Path, rule: &str) -> Result<Environment> {
        let mut environment = Environment::new();
        let mut names = BTreeSet::new();
        for name in self.values.keys() {
            ensure!(
                self.declarations.contains_key(name),
                "Rule {rule}: parameter '{name}' is not declared"
            );
        }
        for (name, declaration) in &self.declarations {
            (|| -> Result<()> {
                ensure!(name.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
                    && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_'),
                    "use ASCII letters, digits and underscores, starting with a letter");
                let variable = format!("PLEASE_PARAM_{}", name.to_ascii_uppercase());
                ensure!(names.insert(variable.clone()), "environment variable name collides after uppercasing");
                ensure!(matches!(declaration.kind.as_str(), "string" | "file"),
                    "unsupported type {:?}; use string or file", declaration.kind);
                ensure!(!self.imported || declaration.value.is_none(),
                    "imported declarations must omit value; supply it in imports.rule_parameters");
                let value = if self.imported { self.values.get(name) } else { declaration.value.as_ref() };
                let Some(value) = value else {
                    ensure!(!declaration.required, "required value is missing");
                    return Ok(());
                };
                let value = value.as_str().context("value must be a string")?;
                ensure!(!value.contains('\0'), "value must not contain NUL");
                let value = if declaration.kind == "file" {
                    let path = crate::config::source_path(root, Path::new(value))
                        .context("file must exist within the consuming repository; absolute paths, parent traversal and symlinks are forbidden")?;
                    ensure!(path.is_file(), "file must be a regular file");
                    path.canonicalize()?.to_str().context("file path must be UTF-8")?.to_owned()
                } else { value.to_owned() };
                environment.insert(variable, value);
                Ok(())
            })().with_context(|| format!("Rule {rule}: parameter '{name}'"))?;
        }
        Ok(environment)
    }
}

/// Omitted or undeclared parameters must not be supplied by the parent process.
pub fn configure(command: &mut Command, environment: &Environment) {
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("PLEASE_PARAM_") {
            command.env_remove(name);
        }
    }
    command.envs(environment);
}
