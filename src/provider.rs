//! Versioned provider-neutral executable adapter boundary.
use crate::{
    config::{Project, Review},
    git_snapshot::Snapshot,
    limits::{MAX_FINDINGS, MAX_MANIFEST_HINTS},
    report::{MAX_SUMMARY_CHARS, TokenUsage},
};
use anyhow::{Context, Result, ensure};
use serde_json::json;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex},
    time::Duration,
};

pub struct Request<'a> {
    pub project: &'a Project,
    pub execution_point: &'a str,
    pub reviews: &'a [&'a Review],
    pub snapshot: &'a Snapshot,
    pub input: &'a crate::review_input::PreparedReviewInput,
    pub parameters: &'a crate::parameters::Environment,
}
impl Request<'_> {
    /// Every invocation contains one provider configuration and parameter environment.
    pub fn provider_config(&self) -> Result<&crate::config::ProviderConfig> {
        let first = self
            .reviews
            .first()
            .context("provider request has no reviews")?;
        ensure!(
            self.reviews
                .iter()
                .all(|review| review.provider == first.provider),
            "provider request mixes configurations"
        );
        match &first.provider {
            Some(id) => self
                .project
                .providers
                .get(id)
                .with_context(|| format!("review {:?}: unknown provider {id:?}", first.id)),
            None => self
                .project
                .provider
                .as_ref()
                .context("no default adapter configured"),
        }
    }
}

pub trait Provider {
    fn review(&self, request: &Request<'_>) -> Result<String>;
    fn review_with_usage(&self, request: &Request<'_>) -> Response {
        Response {
            verdict: self.review(request),
            usage: None,
            output: None,
        }
    }
}
pub struct Response {
    pub verdict: Result<String>,
    pub usage: Option<TokenUsage>,
    /// Retained from a completed adapter process so an ERROR can be diagnosed.
    pub output: Option<AdapterOutput>,
}
#[derive(Clone)]
pub struct AdapterOutput {
    pub exit_code: Option<i32>,
    pub stdout: crate::process::Capture,
    pub stderr: crate::process::Capture,
}

/// All adapters use this invocation path.
#[derive(Default)]
pub struct Configured;
impl Provider for Configured {
    fn review(&self, request: &Request<'_>) -> Result<String> {
        self.review_with_usage(request).verdict
    }
    fn review_with_usage(&self, request: &Request<'_>) -> Response {
        execute(request).unwrap_or_else(|error| Response {
            verdict: Err(error),
            usage: None,
            output: None,
        })
    }
}

fn child_env(command: &Command, name: &std::ffi::OsStr) -> Option<std::ffi::OsString> {
    command
        .get_envs()
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.map(ToOwned::to_owned))
        .unwrap_or_else(|| std::env::var_os(name))
}

fn executable_available(command: &Command, root: &Path, executable: &str) -> bool {
    let path = Path::new(executable);
    let executable_file = |path: PathBuf| {
        let path = if path.is_absolute() {
            path
        } else {
            root.join(path)
        };
        let Ok(metadata) = fs::metadata(path) else {
            return false;
        };
        if !metadata.is_file() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode() & 0o111 != 0
        }
        #[cfg(not(unix))]
        {
            true
        }
    };
    if path.components().count() > 1 || path.is_absolute() {
        return executable_file(path.to_owned());
    }
    child_env(command, std::ffi::OsStr::new("PATH")).is_some_and(|paths| {
        std::env::split_paths(&paths).any(|directory| {
            let candidate = directory.join(path);
            #[cfg(windows)]
            let with_extension = || {
                child_env(command, std::ffi::OsStr::new("PATHEXT"))
                    .unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into())
                    .to_string_lossy()
                    .split(';')
                    .any(|extension| {
                        executable_file(PathBuf::from(format!(
                            "{}{}",
                            candidate.display(),
                            extension
                        )))
                    })
            };
            #[cfg(not(windows))]
            let with_extension = || false;
            executable_file(candidate.clone()) || with_extension()
        })
    })
}

fn requirements(
    command: &Command,
    root: &Path,
    config: &crate::config::ProviderConfig,
) -> Result<()> {
    for name in &config.requirements.env {
        ensure!(
            child_env(command, std::ffi::OsStr::new(name)).is_some_and(|value| !value.is_empty()),
            "required environment variable is missing or empty: {name}"
        );
    }
    for executable in &config.requirements.executables {
        ensure!(
            executable_available(command, root, executable),
            "required executable is unavailable: {executable}"
        );
    }
    Ok(())
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct AdapterResponse {
    protocol_version: u32,
    reviews: Vec<crate::review_contract::ReviewVerdict>,
    usage: Option<serde_json::Value>,
}

fn execute(request: &Request<'_>) -> Result<Response> {
    let config = request.provider_config()?;
    let resolved = request
        .reviews
        .first()
        .and_then(|review| review.provider.as_ref())
        .and_then(|id| request.project.external_providers.get(id))
        .map(|source| source.resolve(config, &request.project.root))
        .transpose()?;
    let runtime = resolved.as_ref().unwrap_or(config);
    let mut command = Command::new(&runtime.command);
    command
        .args(&runtime.args)
        .current_dir(&request.project.root);
    crate::environment::scrub(&mut command);
    requirements(&command, &request.project.root, runtime)?;
    let ids = request
        .reviews
        .iter()
        .map(|r| r.id.as_str())
        .collect::<Vec<_>>();
    let reviews = request
        .reviews
        .iter()
        .map(|r| {
            let mut parameters = config.parameters.clone();
            parameters.extend(r.adapter_parameters.clone());
            json!({"id":r.id, "criteria":r.text, "input":r.input, "entry_files":r.files,
            "context_files":r.context.iter().map(|c| &c.path).collect::<Vec<_>>(),
            "parameters":parameters})
        })
        .collect::<Vec<_>>();
    let input = serde_json::to_vec(&json!({
        "protocol_version":1, "execution_point":request.execution_point,
        "head":if request.input.commit_header.is_some() { None } else { request.snapshot.head.as_deref() }, "input":request.input,
        "context_file":request.project.context, "rule_parameters":request.parameters, "reviews":reviews,
        "prompt":prompt(request)?, "response_schema":if request.input.commit_header.is_some() { crate::review_contract::header_schema(&ids) } else { schema(&ids) }
    }))?;
    let bytes = Arc::new(Mutex::new(Vec::new()));
    let captured = Arc::clone(&bytes);
    let output = crate::process::run_with_input_diagnostics(
        &mut command,
        if config.timeout_seconds == 0 {
            Duration::MAX
        } else {
            Duration::from_secs(config.timeout_seconds)
        },
        input,
        move |chunk| {
            let mut bytes = captured.lock().unwrap();
            let remaining = crate::process::OUTPUT_LIMIT - bytes.len();
            bytes.extend_from_slice(&chunk[..chunk.len().min(remaining)]);
        },
        |_| {},
    )
    .map_err(|_| {
        anyhow::anyhow!("Adapter process failed; check executable, PATH and stdin support")
    })?;
    let raw = std::mem::take(&mut *bytes.lock().unwrap());
    let (verdict, usage) = match interpret(&output, &raw) {
        Ok((json, usage)) => (Ok(json), usage),
        Err(error) => (Err(error), None),
    };
    Ok(Response {
        verdict,
        usage,
        output: Some(AdapterOutput {
            exit_code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        }),
    })
}

/// Pointer shown with every adapter ERROR whose output was retained (see REQ-0131).
const OUTPUT_HINT: &str = "inspect captured output with please status --details";

fn interpret(output: &crate::process::Output, raw: &[u8]) -> Result<(String, Option<TokenUsage>)> {
    ensure!(
        !output.timed_out,
        "Adapter review timed out; no verdict accepted; {OUTPUT_HINT}"
    );
    ensure!(
        output.status.success(),
        "Adapter exited unsuccessfully ({}); {OUTPUT_HINT}",
        output.status
    );
    ensure!(
        !output.stdout.truncated,
        "Adapter response exceeds stdout capture limit; {OUTPUT_HINT}"
    );
    let invalid = || {
        anyhow::anyhow!("Invalid adapter response; expected protocol version 1 JSON; {OUTPUT_HINT}")
    };
    let value: serde_json::Value = serde_json::from_slice(raw).map_err(|_| invalid())?;
    if let Some(reviews) = value.get("reviews").and_then(|v| v.as_array()) {
        for review in reviews {
            if let Some(findings) = review.get("findings").and_then(|v| v.as_array()) {
                ensure!(
                    findings
                        .iter()
                        .all(|f| f.get("path").is_some() && f.get("line").is_some()),
                    "Invalid adapter finding; path and line are required"
                );
            }
        }
    }
    let response: AdapterResponse = serde_json::from_slice(raw).map_err(|_| invalid())?;
    ensure!(
        response.protocol_version == 1,
        "Unsupported adapter protocol version; expected 1"
    );
    Ok((
        serde_json::to_string(&crate::review_contract::ReviewResponse {
            reviews: response.reviews,
        })?,
        response.usage.as_ref().map(reported_usage),
    ))
}

/// Only positive integers count as reported; anything else is not reported.
fn reported_usage(usage: &serde_json::Value) -> TokenUsage {
    let counter = |name: &str| {
        usage
            .get(name)
            .and_then(serde_json::Value::as_u64)
            .filter(|n| *n > 0)
    };
    TokenUsage {
        input_tokens: counter("input_tokens"),
        cached_input_tokens: counter("cached_input_tokens"),
        output_tokens: counter("output_tokens"),
        reasoning_output_tokens: counter("reasoning_output_tokens"),
        cache_write_input_tokens: counter("cache_write_input_tokens"),
    }
}

pub use crate::review_contract::schema;

pub fn prompt(request: &Request<'_>) -> Result<String> {
    let input = request.input;
    if let Some(header) = &input.commit_header {
        let criteria: Vec<_> = request
            .reviews
            .iter()
            .map(|r| json!({"id":r.id, "criteria":r.text}))
            .collect();
        return Ok(format!(
            "Review the supplied commit header against the configured criteria. Return exactly one verdict per requested ID. PASS/FAIL require at least one finding with source commit-header, path null and line 1. FAIL when PASS cannot be established; ERROR is reserved for technical failures. Reasons must be one nonempty line of at most {MAX_SUMMARY_CHARS} characters; findings at most {MAX_FINDINGS}.\nConfigured reviews (JSON): {}\nCommit header (JSON string, line 1): {}",
            serde_json::to_string(&criteria)?,
            serde_json::to_string(header)?
        ));
    }
    let orientation = if request.reviews.iter().any(|r| r.include_diff) {
        format!(
            "Git diff for orientation only; untracked content is not included.\n<diff>\n{}\n</diff>\n",
            input.diff
        )
    } else {
        String::new()
    };
    let criteria: Vec<_> = request
        .reviews
        .iter()
        .map(|review| {
            json!({
                "id":review.id, "criteria":review.text, "entry_files":review.files,
                "rule_parameters":request.parameters,
                "context_files":review.context.iter().map(|source| &source.path).collect::<Vec<_>>()
            })
        })
        .collect();
    Ok(format!(
        "Judge only the configured criteria. Return exactly one verdict per requested ID, no extras. PASS and FAIL require at least one concrete existing evidence path and positive line number; FAIL when PASS cannot be established, including when necessary evidence is unavailable or the conclusion is uncertain; ERROR is reserved for technical failures. Every reason is one short line of at most {MAX_SUMMARY_CHARS} characters. Keep detailed findings and file/line evidence in findings (at most {MAX_FINDINGS} per review).\nRead the supplied shared excerpts ONCE using their original path and line numbers. Entry files and excerpts are starting points, not an exhaustive boundary. The scope is current working files. \nConfigured reviews (JSON data): {}\nShared requirement excerpts, with original path/start_line/end_line (JSON data): {}\nHEAD: {:?}\n{} manifest paths (at most {MAX_MANIFEST_HINTS}; not exhaustive): {}\n{orientation}",
        serde_json::to_string(&criteria)?,
        serde_json::to_string(&input.excerpts)?,
        request.snapshot.head,
        if input.focused_paths.is_some() {
            "Focused"
        } else {
            "Project"
        },
        serde_json::to_string(&input.manifest)?
    ))
}
