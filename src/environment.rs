//! Environment contract for rule processes.
//!
//! Checks and review adapters receive exactly these `PLEASE_*` variables, set by Please Behave:
//! `PLEASE_CONTEXT`, `PLEASE_EXECUTION_POINT`, `PLEASE_RULE_DIRECTORY` (checks only),
//! `PLEASE_PARAM_<NAME>` for declared parameters and the reserved recursion guard. Every other
//! inherited `PLEASE_*` variable, including `PLEASE_CHANGE_BASE`, is removed first, so the
//! environment of the harness never leaks into the code a rule runs.
use std::process::Command;

/// Remove all inherited `PLEASE_*` variables; callers set the contract variables afterwards.
pub(crate) fn scrub(command: &mut Command) {
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("PLEASE_") {
            command.env_remove(name);
        }
    }
}
