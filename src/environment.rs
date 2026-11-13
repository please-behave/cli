//! Environment contract for rule processes.
//!
//! Checks and review adapters receive no `PLEASE_*` variables except one reserved internal
//! recursion guard. Every other inherited `PLEASE_*` variable, including `PLEASE_CHANGE_BASE`, is
//! removed, so the environment of the harness never leaks into the code a rule runs. Inputs such as
//! the run context and parameters are passed explicitly as `@` arguments or request fields.
use std::process::Command;

/// Remove all inherited `PLEASE_*` variables; callers set the contract variables afterwards.
pub(crate) fn scrub(command: &mut Command) {
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("PLEASE_") {
            command.env_remove(name);
        }
    }
}
