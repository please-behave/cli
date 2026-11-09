//! Provider-neutral, bounded evidence prepared once for planning and execution.
use crate::{
    config::{Project, Review},
    git_snapshot::{self, Snapshot},
    limits::MAX_MANIFEST_HINTS,
    review_context::{self, Excerpt},
};
use anyhow::{Result, ensure};
use std::{
    collections::BTreeSet,
    io::{BufRead, Read},
    path::{Path, PathBuf},
};

#[derive(Debug, serde::Serialize)]
pub struct PreparedReviewInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_header: Option<String>,
    #[serde(skip)]
    header_file: Option<PathBuf>,
    pub excerpts: Vec<Excerpt>,
    pub focused_paths: Option<Vec<String>>,
    pub manifest: Vec<String>,
    pub diff: String,
    pub diff_truncated: bool,
}
impl PreparedReviewInput {
    pub fn prepare(project: &Project, reviews: &[&Review], snapshot: &Snapshot) -> Result<Self> {
        Self::prepare_with_context(project, reviews, snapshot, None)
    }

    pub(crate) fn prepare_with_context(
        project: &Project,
        reviews: &[&Review],
        snapshot: &Snapshot,
        hook: Option<&crate::reconciliation::HookContext>,
    ) -> Result<Self> {
        let header_file = if reviews
            .iter()
            .any(|r| r.input == crate::config::ReviewInput::CommitHeader)
        {
            Some(
                hook.ok_or_else(|| {
                    anyhow::anyhow!("commit-header review requires the managed commit-msg hook")
                })?
                .commit_message_path(&project.root)?,
            )
        } else {
            None
        };
        let commit_header = header_file.as_deref().map(read_header).transpose()?;
        let reviews: Vec<_> = reviews
            .iter()
            .copied()
            .filter(|r| r.input == crate::config::ReviewInput::Worktree)
            .collect();
        for path in reviews
            .iter()
            .flat_map(|r| r.files.iter().chain(r.context.iter().map(|c| &c.path)))
        {
            ensure!(
                snapshot.files.contains_key(path),
                "review context or entry file not in verified manifest: {path}"
            );
        }
        let excerpts = review_context::collect(&project.root, &reviews)?;
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
            commit_header,
            header_file,
            excerpts,
            focused_paths,
            manifest,
            diff,
            diff_truncated,
        })
    }

    pub(crate) fn for_source(&self, source: crate::config::ReviewInput) -> Self {
        let header = source == crate::config::ReviewInput::CommitHeader;
        Self {
            commit_header: if header {
                self.commit_header.clone()
            } else {
                None
            },
            header_file: if header {
                self.header_file.clone()
            } else {
                None
            },
            excerpts: if header {
                vec![]
            } else {
                self.excerpts.clone()
            },
            focused_paths: if header {
                Some(vec![])
            } else {
                self.focused_paths.clone()
            },
            manifest: if header {
                vec![]
            } else {
                self.manifest.clone()
            },
            diff: if header {
                String::new()
            } else {
                self.diff.clone()
            },
            diff_truncated: !header && self.diff_truncated,
        }
    }

    pub(crate) fn ensure_stable(&self) -> Result<()> {
        if let Some(path) = &self.header_file {
            ensure!(
                read_header(path)?.as_str() == self.commit_header.as_deref().unwrap_or_default(),
                "Commit header changed during verification"
            );
        }
        Ok(())
    }
}

fn read_header(path: &Path) -> Result<String> {
    let metadata = path.symlink_metadata()?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "commit message must be a regular nonsymlink file"
    );
    let mut bytes = Vec::new();
    // Capture only the first line, with a bound independent of body size.
    std::io::BufReader::new(
        std::fs::File::open(path)?.take((crate::limits::MAX_COMMIT_HEADER_BYTES + 3) as u64),
    )
    .read_until(b'\n', &mut bytes)?;
    if bytes.last() == Some(&b'\n') {
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
    }
    ensure!(
        !bytes.is_empty() && bytes.len() <= crate::limits::MAX_COMMIT_HEADER_BYTES,
        "commit header must contain 1..=4096 UTF-8 bytes"
    );
    let header =
        String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("commit header must be UTF-8"))?;
    ensure!(
        !header
            .chars()
            .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}')),
        "commit header cannot contain control characters"
    );
    Ok(header)
}
