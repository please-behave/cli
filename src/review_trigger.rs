//! Restricted, portable path globs for review selection.
use crate::{
    config::ReviewTrigger,
    limits::{MAX_TRIGGER_PATTERN_BYTES, MAX_TRIGGER_PATTERNS},
};
use anyhow::{Result, ensure};

pub fn validate(trigger: &ReviewTrigger) -> Result<()> {
    ensure!(
        (1..=MAX_TRIGGER_PATTERNS).contains(&trigger.changed_files.len()),
        "changed_files requires 1..={MAX_TRIGGER_PATTERNS} patterns"
    );
    for pattern in &trigger.changed_files {
        ensure!(
            !pattern.is_empty()
                && pattern.len() <= MAX_TRIGGER_PATTERN_BYTES
                && !pattern
                    .chars()
                    .any(|c| c.is_control() || "\\:[]{}".contains(c))
                && pattern
                    .split('/')
                    .all(|part| !matches!(part, "" | "." | "..")
                        && (part == "**" || !part.contains("**"))),
            "invalid changed_files pattern: {pattern:?}"
        );
    }
    Ok(())
}

fn component(pattern: &str, path: &str) -> bool {
    let text: Vec<_> = path.chars().collect();
    let mut row = vec![false; text.len() + 1];
    row[0] = true;
    for token in pattern.chars() {
        let mut next = vec![false; row.len()];
        if token == '*' {
            next[0] = row[0];
        }
        for i in 0..text.len() {
            next[i + 1] = if token == '*' {
                row[i + 1] || next[i]
            } else {
                row[i] && (token == '?' || token == text[i])
            };
        }
        row = next;
    }
    row[text.len()]
}

pub fn matches(trigger: &ReviewTrigger, paths: &std::collections::BTreeSet<String>) -> bool {
    trigger.changed_files.iter().any(|pattern| {
        paths.iter().any(|path| {
            let parts: Vec<_> = path.split('/').collect();
            let mut row = vec![false; parts.len() + 1];
            row[0] = true;
            for token in pattern.split('/') {
                let mut next = vec![false; row.len()];
                if token == "**" {
                    next[0] = row[0];
                }
                for i in 0..parts.len() {
                    next[i + 1] = if token == "**" {
                        row[i + 1] || next[i]
                    } else {
                        row[i] && component(token, parts[i])
                    };
                }
                row = next;
            }
            row[parts.len()]
        })
    })
}
