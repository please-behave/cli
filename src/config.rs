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
    #[serde(default = "default_display")]
    pub max_display_files: usize,
    #[serde(default)]
    pub execution_points: ExecutionPoints,
    #[serde(default)]
    pub imports: Vec<crate::external::Import>,
    #[serde(default)]
    pub provider_imports: Vec<crate::external::ProviderImport>,
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
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
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
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub blocking: bool,
    pub command: crate::check_command::Command,
    pub source_root: PathBuf,
    pub rule_directory: PathBuf,
    pub timeout_seconds: u64,
    pub run_at: Vec<String>,
    pub parameters: crate::parameters::Binding,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ReviewInput {
    #[default]
    Worktree,
    CommitHeader,
}
#[derive(Debug, Clone, Serialize)]
pub struct Review {
    pub input: ReviewInput,
    pub adapter_parameters: BTreeMap<String, serde_json::Value>,
    pub include_diff: bool,
    pub trigger: Option<ReviewTrigger>,
    pub provider: Option<String>,
    pub id: String,
    pub name: String,
    pub description: String,
    pub enabled: bool,
    pub blocking: bool,
    pub files: Vec<String>,
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
    pub name: Option<String>,
    pub description: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub blocking: bool,
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
    #[serde(default)]
    pub input: ReviewInput,
    #[serde(
        default,
        deserialize_with = "adapter_parameters",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub parameters: BTreeMap<String, serde_json::Value>,
    #[serde(default = "default_true")]
    pub include_diff: bool,
    pub trigger: Option<ReviewTrigger>,
    pub provider: Option<String>,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
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
    pub provider: Option<ProviderConfig>,
    pub providers: BTreeMap<String, ProviderConfig>,
    pub external_providers: BTreeMap<String, crate::external::ProviderSource>,
    pub checks: Vec<Check>,
    pub reviews: Vec<Review>,
    pub rule_sets: Vec<RuleSet>,
    pub effective: crate::rules::EffectiveConfig,
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
fn nonempty(value: &str, name: &str) -> Result<()> {
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
    let provider = match root.join(self::path("provider.toml")).symlink_metadata() {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(e.into()),
        Ok(_) => Some(crate::discovery::document(
            &root,
            &self::path("provider.toml"),
        )?),
    };
    if let Some(provider) = &provider {
        validate_provider(provider).context("provider.toml")?;
    }
    let mut providers = BTreeMap::new();
    for (path, id, config) in loaded.providers {
        let mut ids = BTreeSet::new();
        identity(&id, "provider configuration", &mut ids)
            .and_then(|()| validate_provider(&config))
            .with_context(|| format!("invalid provider {id:?} ({})", path.display()))?;
        ensure!(
            providers.insert(id.clone(), config).is_none(),
            "duplicate provider identifier {id:?}"
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
            if review.input == ReviewInput::CommitHeader {
                ensure!(review.files.is_empty() && review.context.is_empty() && review.trigger.is_none(),
                    "commit-header reviews cannot select files, context or file-change triggers");
            }
            if let Some(trigger) = &review.trigger {
                crate::review_trigger::validate(trigger)?;
            }
            ensure!(
                review.files.len() <= MAX_ENTRY_FILES
                    && review.context.len() <= MAX_CONTEXT_ENTRIES,
                "review permits at most {MAX_ENTRY_FILES} files and {MAX_CONTEXT_ENTRIES} context entries"
            );
            let mut files = BTreeSet::new();
            for file in &mut review.files {
                let full = inside(&root, Path::new(file))?;
                ensure!(full.is_file(), "review file must be a regular file: {file}");
                *file = full
                    .strip_prefix(&root)?
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                ensure!(files.insert(file.clone()), "duplicate review file: {file}");
            }
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
                "total review instructions exceed {} bytes", MAX_TOTAL_INSTRUCTION_BYTES
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
    let rule_sets = loaded.rule_sets.into_iter().map(|(_, set)| set).collect();
    Ok(Project {
        root,
        config,
        provider,
        providers,
        external_providers: BTreeMap::new(),
        checks,
        reviews,
        rule_sets,
        effective,
        context: None,
    })
}

/// Shared validation for legacy and named configurations.
pub(crate) fn validate_provider(provider: &ProviderConfig) -> Result<()> {
    nonempty(&provider.command, "adapter command")?;
    ensure!(
        provider.args.iter().all(|arg| !arg.contains('\0')),
        "adapter args must contain no NUL"
    );
    for name in &provider.requirements.env {
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
    for executable in &provider.requirements.executables {
        ensure!(
            !executable.trim().is_empty() && !executable.contains('\0'),
            "invalid required executable"
        );
    }
    if provider.timeout_seconds != 0 {
        timeout(provider.timeout_seconds)?;
    }

    Ok(())
}

impl Project {
    pub(crate) fn resolve_providers(&self) -> Result<()> {
        for review in &self.reviews {
            if let Some(id) = &review.provider {
                ensure!(
                    self.providers.contains_key(id),
                    "review {:?}: unknown provider {id:?}",
                    review.id
                );
            }
        }
        Ok(())
    }

    pub fn review_provider(&self, review: &Review) -> Option<&ProviderConfig> {
        match &review.provider {
            Some(id) => self.providers.get(id),
            None => self.provider.as_ref(),
        }
    }
}
