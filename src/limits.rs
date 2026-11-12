//! Shared public-contract bounds. Keep README tables and boundary tests in sync.
pub const DEFAULT_MAX_PARALLEL: usize = 4;
pub const DEFAULT_FAIL_FAST_TERMINATION_GRACE_SECONDS: u64 = 5;
pub const CONFIG_DIRECTORY: &str = ".please";
pub const MAX_METADATA_BYTES: u64 = 64 * 1024;
pub const MAX_INSTRUCTION_BYTES: u64 = 32 * 1024;
pub const MAX_TOTAL_INSTRUCTION_BYTES: usize = 256 * 1024;
pub const MAX_TIMEOUT_SECONDS: u64 = 3600;
pub const MAX_DISPLAY_FILES: usize = 5;
pub const MAX_RULES: usize = 256;
pub const MAX_RULE_NAME_CHARS: usize = 80;
pub const MAX_ID_BYTES: usize = 80;
pub const MAX_ENTRY_FILES: usize = 64;
pub const MAX_TRIGGER_PATTERNS: usize = 64;
pub const MAX_TRIGGER_PATTERN_BYTES: usize = 1024;
pub const MAX_CONTEXT_ENTRIES: usize = 32;
pub const MAX_CONTEXT_FILES: usize = 32;
pub const MAX_CONTEXT_HEADINGS: usize = 32;
pub const MAX_CONTEXT_FILE_BYTES: u64 = 1024 * 1024;
pub const MAX_CONTEXT_BYTES: usize = 64 * 1024;
pub const MAX_MANIFEST_HINTS: usize = 200;
pub const MAX_DIFF_BYTES: usize = 32 * 1024;
pub const DEFAULT_CONTEXT_RETAIN_RUNS: usize = 100;
pub const MAX_CONTEXT_RETAIN_RUNS: usize = 10_000;
pub const DEFAULT_CONTEXT_SECTION_BYTES: u64 = 10 * 1024 * 1024;
pub const MIN_CONTEXT_SECTION_BYTES: u64 = 1024;
pub const MAX_CONTEXT_SECTION_BYTES: u64 = 1024 * 1024 * 1024;
pub const MAX_SNAPSHOT_FILE_BYTES: u64 = 8 * 1024 * 1024;
pub const MAX_SNAPSHOT_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_SNAPSHOT_FILES: usize = 100_000;
pub const MAX_CONFIG_DEPTH: usize = 16;
pub const MAX_REPORT_BYTES: u64 = 128 * 1024 * 1024;
pub const MAX_PROVIDER_RESPONSE_BYTES: u64 = 1024 * 1024;
pub const MAX_CAPTURE_BYTES: usize = 64 * 1024;
pub const MAX_SUMMARY_CHARS: usize = 120;
pub const MAX_FINDINGS: usize = 20;
pub const MAX_FINDING_MESSAGE_BYTES: usize = 2000;

// External Git source retrieval and immutable-cache bounds.
pub const MAX_EXTERNAL_REFERENCE_BYTES: usize = 1024;
pub const MAX_EXTERNAL_CACHE_DEPTH: usize = 128;
pub const EXTERNAL_FETCH_TIMEOUT_SECONDS: u64 = 60;
pub const EXTERNAL_GIT_TIMEOUT_SECONDS: u64 = 30;
pub const EXTERNAL_LOCK_TIMEOUT_SECONDS: u64 = 30;

pub const MAX_COMMIT_HEADER_BYTES: usize = 4096;
