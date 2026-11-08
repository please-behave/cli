//! Provider-neutral, bounded evidence prepared once for planning and execution.
use crate::{
    config::{Project, Review},
    git_snapshot::{self, Snapshot},
    limits::MAX_MANIFEST_HINTS,
    review_context::{self, Excerpt},
};
use anyhow::{Result, ensure};
use std::collections::BTreeSet;

#[derive(Debug, serde::Serialize)]
pub struct PreparedReviewInput {
    pub excerpts: Vec<Excerpt>,
    pub focused_paths: Option<Vec<String>>,
    pub manifest: Vec<String>,
    pub diff: String,
    pub diff_truncated: bool,
}
impl PreparedReviewInput {
    pub fn prepare(project: &Project, reviews: &[&Review], snapshot: &Snapshot) -> Result<Self> {
        for path in reviews
            .iter()
            .flat_map(|r| r.files.iter().chain(r.context.iter().map(|c| &c.path)))
        {
            ensure!(
                snapshot.files.contains_key(path),
                "review context or entry file not in verified manifest: {path}"
            );
        }
        let excerpts = review_context::collect(&project.root, reviews)?;
        let focused_paths = reviews.iter().all(|r| !r.files.is_empty()).then(|| {
            reviews
                .iter()
                .flat_map(|r| r.files.iter().chain(r.context.iter().map(|c| &c.path)))
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        });
        let manifest = snapshot
            .files
            .keys()
            .filter(|path| {
                focused_paths
                    .as_ref()
                    .is_none_or(|paths| paths.contains(path))
            })
            .take(MAX_MANIFEST_HINTS)
            .cloned()
            .collect();
        let (diff, diff_truncated) = if reviews.iter().any(|r| r.include_diff) {
            git_snapshot::bounded_diff_for_paths(
                &project.root,
                snapshot.head.is_some(),
                focused_paths.as_deref(),
            )?
        } else {
            (String::new(), false)
        };
        Ok(Self {
            excerpts,
            focused_paths,
            manifest,
            diff,
            diff_truncated,
        })
    }
}
