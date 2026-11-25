use crate::limits::*;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Read,
    path::{Component, Path, PathBuf},
};

pub use crate::limits::{
    MAX_INSTRUCTION_BYTES as MAX_INSTRUCTIONS, MAX_METADATA_BYTES as MAX_METADATA,
};

pub fn path(relative: impl AsRef<Path>) -> PathBuf {
    // A trailing slash would make symlink_metadata follow a directory symlink.
    if relative.as_ref().as_os_str().is_empty() {
        PathBuf::from(CONFIG_DIRECTORY)
    } else {
        Path::new(CONFIG_DIRECTORY).join(relative)
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub project: String,
    #[serde(default)]
    pub execution: Execution,
    #[serde(default)]
    pub context: ContextSettings,
    #[serde(default)]
    pub report: ReportSettings,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publish: Option<PublishSettings>,
    #[serde(default = "default_display")]
    pub max_display_files: usize,
    #[serde(default)]
    pub execution_points: ExecutionPoints,
    #[serde(default)]
    pub imports: Vec<crate::external::Import>,
    #[serde(default)]
    pub adapter_imports: Vec<crate::external::AdapterImport>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Execution {
    pub max_parallel: std::num::NonZeroUsize,
    pub fail_fast_terminate_running_instances: bool,
    pub fail_fast_termination_grace_seconds: u64,
}
impl Default for Execution {
    fn default() -> Self {
        Self {
            max_parallel: std::num::NonZeroUsize::new(DEFAULT_MAX_PARALLEL).unwrap(),
            fail_fast_terminate_running_instances: false,
            fail_fast_termination_grace_seconds: DEFAULT_FAIL_FAST_TERMINATION_GRACE_SECONDS,
        }
    }
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ContextSettings {
    pub retain_runs: usize,
    pub max_section_bytes: u64,
}
impl Default for ContextSettings {
    fn default() -> Self {
        Self {
            retain_runs: crate::limits::DEFAULT_CONTEXT_RETAIN_RUNS,
            max_section_bytes: crate::limits::DEFAULT_CONTEXT_SECTION_BYTES,
        }
    }
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct ReportSettings {
    pub retain_runs: usize,
}
impl Default for ReportSettings {
    fn default() -> Self {
        Self {
            retain_runs: crate::limits::DEFAULT_REPORT_RETAIN_RUNS,
        }
    }
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PublishSettings {
    #[serde(default)]
    pub auth: PublishAuth,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_env: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum PublishAuth {
    #[default]
    ApiKey,
    GithubOidc,
}

impl PublishSettings {
    pub(crate) fn validate(&self) -> Result<()> {
        if let Some(key_env) = &self.key_env {
            let mut bytes = key_env.bytes();
            ensure!(
                bytes
                    .next()
                    .is_some_and(|first| first.is_ascii_alphabetic() || first == b'_')
                    && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_'),
                "publish.key_env must name an environment variable"
            );
        } else {
            ensure!(
                matches!(self.auth, PublishAuth::GithubOidc),
                "publish.key_env is required for api-key authentication"
            );
        }
        if let Some(repository) = &self.repository {
            ensure!(
                !repository.is_empty()
                    && repository.chars().count() <= 2048
                    && repository.trim() == repository
                    && !repository.chars().any(char::is_control),
                "publish.repository must be 1 to 2048 characters without surrounding whitespace or control characters"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AdapterProfile {
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub requirements: RuntimeRequirements,
    pub timeout_seconds: u64,
    #[serde(default, deserialize_with = "adapter_parameters")]
    pub parameters: BTreeMap<String, serde_json::Value>,
}
fn adapter_parameters<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<BTreeMap<String, serde_json::Value>, D::Error> {
    fn convert(value: toml::Value) -> Result<serde_json::Value> {
        Ok(match value {
            toml::Value::Float(n) => serde_json::Value::Number(
                serde_json::Number::from_f64(n)
                    .context("adapter parameters require finite numbers")?,
            ),
            toml::Value::Datetime(_) => {
                anyhow::bail!("adapter parameters do not support TOML dates; use a string")
            }
            toml::Value::Array(values) => {
                serde_json::Value::Array(values.into_iter().map(convert).collect::<Result<_>>()?)
            }
            toml::Value::Table(values) => serde_json::Value::Object(
                values
                    .into_iter()
                    .map(|(k, v)| Ok((k, convert(v)?)))
                    .collect::<Result<_>>()?,
            ),
            value => serde_json::to_value(value)?,
        })
    }
    let values = BTreeMap::<String, toml::Value>::deserialize(deserializer)?;
    values
        .into_iter()
        .map(|(k, v)| convert(v).map(|v| (k, v)).map_err(serde::de::Error::custom))
        .collect()
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct RuntimeRequirements {
    pub env: Vec<String>,
    pub executables: Vec<String>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewContext {
    pub path: String,
    #[serde(default)]
    pub headings: Vec<String>,
}
/// A loaded check retaining its source boundary and declaring directory for execution.
#[derive(Debug, Clone, Serialize)]
pub struct Check {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub depends_on: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub command: crate::check_command::Command,
    pub source_root: PathBuf,
    pub rule_directory: PathBuf,
    pub timeout_seconds: u64,
    pub run_at: Vec<String>,
    pub parameters: crate::parameters::Binding,
}
#[derive(Debug, Clone, Serialize)]
pub struct Review {
    pub adapter_parameters: BTreeMap<String, serde_json::Value>,
    pub include_content: bool,
    pub include_change_set: bool,
    pub trigger: Option<ReviewTrigger>,
    pub adapter: Option<String>,
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub blocking: bool,
    pub context: Vec<ReviewContext>,
    pub run_at: Vec<String>,
    pub text: String,
    pub parameters: crate::parameters::Binding,
    pub execution_error: Option<String>,
}
pub(crate) fn default_true() -> bool {
    true
}
fn default_display() -> usize {
    MAX_DISPLAY_FILES
}
fn default_timeout() -> u64 {
    30
}

/// The single rule document format used by local and imported rules.
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuleDefinition {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depends_on: Option<Vec<String>>,
    pub id: String,
    // Retain the TOML type so discovery can diagnose invalid values with the rule ID.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<toml::Value>,
    pub name: Option<String>,
    pub description: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Reviews only; defaults to true. A check must not declare it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocking: Option<bool>,
    #[serde(default)]
    pub run_at: Vec<String>,
    #[serde(default)]
    pub parameters: BTreeMap<String, crate::parameters::Declaration>,
    pub check: Option<CheckDefinition>,
    pub review: Option<ReviewDefinition>,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CheckDefinition {
    pub resource: Option<String>,
    #[serde(deserialize_with = "crate::check_command::deserialize")]
    pub command: crate::check_command::Command,
    #[serde(default = "default_timeout")]
    pub timeout_seconds: u64,
}
#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewDefinition {
    #[serde(
        default,
        deserialize_with = "adapter_parameters",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub parameters: BTreeMap<String, serde_json::Value>,
    #[serde(default = "default_true")]
    pub include_content: bool,
    #[serde(default)]
    pub include_change_set: bool,
    pub trigger: Option<ReviewTrigger>,
    pub adapter: Option<String>,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub context: Vec<ReviewContext>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ReviewTrigger {
    pub changed_files: Vec<String>,
}
#[derive(Debug)]
pub struct Project {
    pub root: PathBuf,
    pub config: Config,
    pub adapter: Option<AdapterProfile>,
    pub adapters: BTreeMap<String, AdapterProfile>,
    pub external_adapters: BTreeMap<String, crate::external::AdapterSource>,
    pub checks: Vec<Check>,
    pub reviews: Vec<Review>,
    pub rule_sets: Vec<RuleSet>,
    pub(crate) versioned_rule_sets: BTreeMap<crate::rules::RuleSetIdentity, BTreeSet<String>>,
    pub effective: crate::rules::EffectiveConfig,
    pub(crate) rule_metadata: BTreeMap<String, crate::rule_digest::RuleMetadata>,
    /// Absolute path of the current run's context file, set once per execution.
    pub context: Option<PathBuf>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionPoints {
    #[serde(default)]
    pub custom: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuleSet {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<toml::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub description: String,
    pub rules: Vec<RuleReference>,
    pub run_at: Vec<String>,
}

/// A scope-local rule ID (or an external Rule directory) with optional assignments.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum RuleReference {
    Id(String),
    Configured {
        id: String,
        run_at: Option<Vec<String>>,
    },
}
impl RuleReference {
    pub fn id(&self) -> &str {
        match self {
            Self::Id(id) | Self::Configured { id, .. } => id,
        }
    }
    pub(crate) fn id_mut(&mut self) -> &mut String {
        match self {
            Self::Id(id) | Self::Configured { id, .. } => id,
        }
    }
    pub(crate) fn run_at(&self) -> Option<&[String]> {
        match self {
            Self::Id(_) => None,
            Self::Configured { run_at, .. } => run_at.as_deref(),
        }
    }
}
impl From<String> for RuleReference {
    fn from(id: String) -> Self {
        Self::Id(id)
    }
}

pub fn relative(path: &Path) -> Result<()> {
    ensure!(
        !path.as_os_str().is_empty() && !path.is_absolute(),
        "path must be nonempty and relative: {}",
        path.display()
    );
    ensure!(
        path.components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir)),
        "path cannot traverse parents: {}",
        path.display()
    );
    Ok(())
}
/// Resolve a source resource without following any symlink component.
pub(crate) fn source_path(root: &Path, relative: &Path) -> Result<PathBuf> {
    self::relative(relative)?;
    let mut path = root.to_path_buf();
    ensure!(
        path.symlink_metadata()?.is_dir() && !path.symlink_metadata()?.file_type().is_symlink(),
        "source boundary must be a directory, not a symlink"
    );
    for part in relative.components() {
        if let Component::Normal(part) = part {
            path.push(part);
            let metadata = path.symlink_metadata().context("missing source path")?;
            ensure!(
                !metadata.file_type().is_symlink(),
                "source symlink traversal is unsupported"
            );
        }
    }
    ensure!(
        path.canonicalize()?.starts_with(root.canonicalize()?),
        "source escapes selected directory"
    );
    Ok(path)
}

pub fn inside(root: &Path, path: &Path) -> Result<PathBuf> {
    relative(path)?;
    let root = root.canonicalize()?;
    let p = root
        .join(path)
        .canonicalize()
        .with_context(|| format!("missing path {}", path.display()))?;
    ensure!(
        p.starts_with(root),
        "path escapes project: {}",
        path.display()
    );
    Ok(p)
}
pub fn read_bounded(path: &Path, limit: u64) -> Result<String> {
    ensure!(
        path.metadata()?.is_file(),
        "{} must be a regular file",
        path.display()
    );
    let mut bytes = Vec::new();
    fs::File::open(path)?
        .take(limit + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= limit,
        "{} exceeds {} bytes",
        path.display(),
        limit
    );
    String::from_utf8(bytes).context("file must be UTF-8")
}
fn timeout(value: u64) -> Result<()> {
    ensure!(
        (1..=MAX_TIMEOUT_SECONDS).contains(&value),
        "timeout_seconds must be 1..={MAX_TIMEOUT_SECONDS}"
    );
    Ok(())
}
pub(crate) fn nonempty(value: &str, name: &str) -> Result<()> {
    ensure!(
        !value.trim().is_empty() && !value.contains('\0'),
        "{name} must be nonempty and contain no NUL"
    );
    Ok(())
}
pub(crate) fn identity(id: &str, description: &str, ids: &mut BTreeSet<String>) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= MAX_ID_BYTES
            && id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_".contains(&c)),
        "invalid id {id:?}: use 1..{MAX_ID_BYTES} ASCII letters, digits, - or _"
    );
    ensure!(
        !id.starts_with("please-"),
        "IDs beginning please- are reserved"
    );
    ensure!(ids.insert(id.to_owned()), "duplicate id: {id}");
    nonempty(description, "description")
}
pub fn load(root: &Path) -> Result<Project> {
    crate::external::load(root, false, None)
}

/// Refresh external sources and publish resolution state only after full validation.
pub fn refresh(root: &Path) -> Result<(Project, bool)> {
    let mut changed = false;
    let project = crate::external::load_with_changes(root, true, None, &mut changed)?;
    Ok((project, changed))
}

pub(crate) fn validate(root: PathBuf, loaded: crate::discovery::Loaded) -> Result<Project> {
    let assignments: Vec<_> = loaded
        .checks
        .iter()
        .map(|(source, rule)| crate::rules::Assignment {
            id: rule.id.clone(),
            enabled: rule.enabled,
            run_at: rule.run_at.clone(),
            source: source.clone(),
        })
        .chain(
            loaded
                .reviews
                .iter()
                .map(|(source, rule)| crate::rules::Assignment {
                    id: rule.id.clone(),
                    enabled: rule.enabled,
                    run_at: rule.run_at.clone(),
                    source: source.clone(),
                }),
        )
        .collect();
    let config = loaded.config;
    if let Some(publish) = &config.publish {
        publish.validate().context(".please/config.toml")?;
    }
    ensure!(
        (1..=MAX_TIMEOUT_SECONDS).contains(&config.execution.fail_fast_termination_grace_seconds),
        ".please/config.toml: execution.fail_fast_termination_grace_seconds must be 1..={MAX_TIMEOUT_SECONDS}"
    );
    ensure!(
        cfg!(unix) || !config.execution.fail_fast_terminate_running_instances,
        ".please/config.toml: execution.fail_fast_terminate_running_instances requires Unix process groups"
    );
    ensure!(
        (1..=crate::limits::MAX_CONTEXT_RETAIN_RUNS).contains(&config.context.retain_runs),
        ".please/config.toml: context.retain_runs must be 1..={}",
        crate::limits::MAX_CONTEXT_RETAIN_RUNS
    );
    ensure!(
        (1..=crate::limits::MAX_REPORT_RETAIN_RUNS).contains(&config.report.retain_runs),
        ".please/config.toml: report.retain_runs must be 1..={}",
        crate::limits::MAX_REPORT_RETAIN_RUNS
    );
    ensure!(
        (crate::limits::MIN_CONTEXT_SECTION_BYTES..=crate::limits::MAX_CONTEXT_SECTION_BYTES)
            .contains(&config.context.max_section_bytes),
        ".please/config.toml: context.max_section_bytes must be {}..={}",
        crate::limits::MIN_CONTEXT_SECTION_BYTES,
        crate::limits::MAX_CONTEXT_SECTION_BYTES
    );
    nonempty(&config.project, "project").context(".please/config.toml")?;
    ensure!(
        (1..=MAX_DISPLAY_FILES).contains(&config.max_display_files),
        ".please/config.toml: max_display_files must be 1..={MAX_DISPLAY_FILES}"
    );
    let adapter = match root.join(self::path("adapter.toml")).symlink_metadata() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
        Ok(_) => Some(crate::discovery::document(
            &root,
            &self::path("adapter.toml"),
        )?),
    };
    if let Some(adapter) = &adapter {
        validate_adapter(adapter).context("adapter.toml")?;
    }
    let mut adapters = BTreeMap::new();
    for (path, id, config) in loaded.adapters {
        let mut ids = BTreeSet::new();
        identity(&id, "adapter profile", &mut ids)
            .and_then(|()| validate_adapter(&config))
            .with_context(|| format!("invalid adapter profile {id:?} ({})", path.display()))?;
        ensure!(
            adapters.insert(id.clone(), config).is_none(),
            "duplicate adapter profile identifier {id:?}"
        );
    }
    let mut ids = BTreeSet::new();
    let mut checks = Vec::new();
    for (path, check) in loaded.checks {
        (|| -> Result<()> {
            identity(&check.id, &check.description, &mut ids)?;
            timeout(check.timeout_seconds)?;
            for dependency in &check.depends_on {
                ensure!(!dependency.is_empty() && dependency.trim() == dependency && !dependency.chars().any(char::is_control), "depends_on: expected nonempty exact check identities without control characters or surrounding whitespace");
            }
            if let Some(resource) = &check.resource {
                ensure!(
                    !resource.is_empty()
                        && resource.trim() == resource
                        && !resource.chars().any(char::is_control),
                    "resource must be nonempty without control characters or surrounding whitespace"
                );
            }
            Ok(())
        })()
        .with_context(|| format!("invalid check '{}' ({})", check.id, path.display()))?;
        checks.push(check);
    }
    let mut reviews = Vec::new();
    let mut instruction_bytes = 0;
    for (path, mut review) in loaded.reviews {
        (|| -> Result<()> {
            identity(&review.id, &review.description, &mut ids)?;
            if let Some(trigger) = &review.trigger {
                crate::review_trigger::validate(trigger)?;
            }
            ensure!(
                review.context.len() <= MAX_CONTEXT_ENTRIES,
                "review permits at most {MAX_CONTEXT_ENTRIES} context entries"
            );
            for source in &mut review.context {
                let full = inside(&root, Path::new(&source.path))?;
                ensure!(
                    full.is_file(),
                    "context must be a regular file: {}",
                    source.path
                );
                source.path = full
                    .strip_prefix(&root)?
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                ensure!(
                    source.headings.len() <= MAX_CONTEXT_HEADINGS,
                    "at most {MAX_CONTEXT_HEADINGS} headings per context entry"
                );
                for heading in &source.headings {
                    nonempty(heading, "context heading")?;
                }
            }
            if review.execution_error.is_none() {
                nonempty(&review.text, "instructions")?;
            }
            instruction_bytes += review.text.len();
            ensure!(
                instruction_bytes <= MAX_TOTAL_INSTRUCTION_BYTES,
                "total review instructions exceed {} bytes",
                MAX_TOTAL_INSTRUCTION_BYTES
            );
            Ok(())
        })()
        .with_context(|| format!("invalid review '{}' ({})", review.id, path.display()))?;
        reviews.push(review);
    }
    checks.sort_by(|a, b| a.id.cmp(&b.id));
    reviews.sort_by(|a, b| a.id.cmp(&b.id));
    crate::review_context::collect(
        &root,
        &reviews
            .iter()
            .filter(|r| r.trigger.is_none())
            .collect::<Vec<_>>(),
    )?;
    let effective =
        crate::rules::resolve(&config.execution_points, &assignments, &loaded.rule_sets)?;
    let versioned_rule_sets =
        effective.versioned_rule_sets(&loaded.rule_sets, loaded.rule_source.as_deref())?;
    let rule_sets = loaded.rule_sets.into_iter().map(|(_, set)| set).collect();
    Ok(Project {
        root,
        config,
        adapter,
        adapters,
        external_adapters: BTreeMap::new(),
        checks,
        reviews,
        rule_sets,
        versioned_rule_sets,
        effective,
        rule_metadata: loaded.rule_metadata,
        context: None,
    })
}

/// Shared validation for legacy and named configurations.
pub(crate) fn validate_adapter(adapter: &AdapterProfile) -> Result<()> {
    nonempty(&adapter.command, "adapter command")?;
    ensure!(
        adapter.args.iter().all(|arg| !arg.contains('\0')),
        "adapter args must contain no NUL"
    );
    for name in &adapter.requirements.env {
        ensure!(
            !name.is_empty()
                && name
                    .bytes()
                    .enumerate()
                    .all(|(i, b)| b.is_ascii_alphabetic()
                        || b == b'_'
                        || (i > 0 && b.is_ascii_digit())),
            "invalid required environment variable name"
        );
    }
    for executable in &adapter.requirements.executables {
        ensure!(
            !executable.trim().is_empty() && !executable.contains('\0'),
            "invalid required executable"
        );
    }
    if adapter.timeout_seconds != 0 {
        timeout(adapter.timeout_seconds)?;
    }

    Ok(())
}

impl Project {
    pub(crate) fn resolve_adapters(&self) -> Result<()> {
        for review in &self.reviews {
            if let Some(id) = &review.adapter {
                ensure!(
                    self.adapters.contains_key(id),
                    "review {:?}: unknown adapter profile {id:?}",
                    review.id
                );
            }
        }
        Ok(())
    }

    pub fn review_adapter(&self, review: &Review) -> Option<&AdapterProfile> {
        match &review.adapter {
            Some(id) => self.adapters.get(id),
            None => self.adapter.as_ref(),
        }
    }
}
