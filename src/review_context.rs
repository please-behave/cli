//! Bounded, deduplicated requirements excerpts with original source line numbers.
use crate::config::{self, Review};
use anyhow::{Result, ensure};
use serde::Serialize;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub use crate::limits::MAX_CONTEXT_BYTES;
use crate::limits::{MAX_CONTEXT_FILE_BYTES as MAX_CONTEXT_FILE, MAX_CONTEXT_FILES};

#[derive(Debug, Serialize)]
pub struct Excerpt {
    pub path: String,
    pub start_line: usize,
    pub end_line: usize,
    pub text: String,
}

// ATX headings outside fenced/indented code. Exact titles avoid brittle line ranges.
fn headings(lines: &[&str]) -> Vec<(usize, usize, String)> {
    let mut result = Vec::new();
    let mut fence: Option<(u8, usize)> = None;
    for (index, line) in lines.iter().enumerate() {
        let content = line.trim_start_matches(' ');
        if line.len() - content.len() > 3 {
            continue;
        }
        let first = content.as_bytes().first().copied();
        if let Some((marker, count)) = fence {
            if first == Some(marker)
                && content.bytes().take_while(|c| *c == marker).count() >= count
                && content.trim_start_matches(marker as char).trim().is_empty()
            {
                fence = None;
            }
            continue;
        }
        if matches!(first, Some(b'`' | b'~')) {
            let marker = first.unwrap();
            let count = content.bytes().take_while(|c| *c == marker).count();
            if count >= 3 {
                fence = Some((marker, count));
                continue;
            }
        }
        let depth = content.bytes().take_while(|c| *c == b'#').count();
        if !(1..=6).contains(&depth) {
            continue;
        }
        let tail = &content[depth..];
        if !tail.is_empty() && !tail.starts_with([' ', '\t']) {
            continue;
        }
        let mut title = tail.trim();
        if title.ends_with('#') {
            let without = title.trim_end_matches('#');
            if without.ends_with([' ', '\t']) {
                title = without.trim_end();
            }
        }
        result.push((index, depth, title.to_owned()));
    }
    result
}

pub fn collect(root: &Path, reviews: &[&Review]) -> Result<Vec<Excerpt>> {
    // Repeated/overlapping selections are read once; still validate every named heading.
    let mut sources: BTreeMap<&str, (bool, BTreeSet<&str>)> = BTreeMap::new();
    for review in reviews {
        for source in &review.context {
            let selection = sources.entry(&source.path).or_default();
            selection.0 |= source.headings.is_empty();
            selection
                .1
                .extend(source.headings.iter().map(String::as_str));
        }
    }
    ensure!(
        sources.len() <= MAX_CONTEXT_FILES,
        "at most {MAX_CONTEXT_FILES} distinct review context files"
    );
    let mut excerpts = Vec::new();
    let mut total = 0;
    for (path, (whole_file, requested)) in sources {
        let text = config::read_bounded(&config::inside(root, Path::new(path))?, MAX_CONTEXT_FILE)?;
        let lines: Vec<_> = text.lines().collect();
        ensure!(!lines.is_empty(), "empty review context: {path}");
        let outline = headings(&lines);
        let mut ranges = Vec::new();
        {
            for title in requested {
                let matches: Vec<_> = outline
                    .iter()
                    .enumerate()
                    .filter(|(_, (_, _, name))| name == title)
                    .collect();
                ensure!(
                    matches.len() == 1,
                    "context heading must exist exactly once: {path}: {title}"
                );
                let (position, (start, depth, _)) = matches[0];
                let end = outline
                    .iter()
                    .skip(position + 1)
                    .find(|(_, level, _)| level <= depth)
                    .map_or(lines.len(), |(line, _, _)| *line);
                ranges.push((*start, end));
            }
        }
        if whole_file {
            ranges = vec![(0, lines.len())];
        }
        ranges.sort_unstable();
        let mut merged: Vec<(usize, usize)> = Vec::new();
        for (start, end) in ranges {
            if let Some(previous) = merged.last_mut().filter(|previous| start <= previous.1) {
                previous.1 = previous.1.max(end);
            } else {
                merged.push((start, end));
            }
        }
        for (start, end) in merged {
            let text = lines[start..end].join("\n");
            total += text.len();
            ensure!(
                total <= MAX_CONTEXT_BYTES,
                "selected review context exceeds {} bytes; select fewer headings",
                MAX_CONTEXT_BYTES
            );
            excerpts.push(Excerpt {
                path: path.into(),
                start_line: start + 1,
                end_line: end,
                text,
            });
        }
    }
    Ok(excerpts)
}
