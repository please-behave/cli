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
    io::Read,
    path::{Path, PathBuf},
};

/// The complete commit message with the line numbers findings may cite.
#[derive(Debug, serde::Serialize)]
pub struct CommitMessage {
    pub lines: Vec<MessageLine>,
}
#[derive(Debug, serde::Serialize)]
pub struct MessageLine {
    pub number: usize,
    pub text: String,
}
impl CommitMessage {
    pub fn text(&self) -> String {
        self.lines
            .iter()
            .map(|l| l.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Debug, serde::Serialize)]
pub struct PreparedReviewInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_message: Option<CommitMessage>,
    #[serde(skip)]
    message_file: Option<PathBuf>,
    pub excerpts: Vec<Excerpt>,
    pub focused_paths: Option<Vec<String>>,
    pub manifest: Vec<String>,
    pub diff: String,
}
impl PreparedReviewInput {
    pub fn prepare(project: &Project, reviews: &[&Review], snapshot: &Snapshot) -> Result<Self> {
        Self::prepare_with_message(project, reviews, snapshot, None)
    }

    /// `message_file` is given at commit-msg, where the whole message is declared input.
    pub(crate) fn prepare_with_message(
        project: &Project,
        reviews: &[&Review],
        snapshot: &Snapshot,
        message_file: Option<PathBuf>,
    ) -> Result<Self> {
        let commit_message = message_file.as_deref().map(read_message).transpose()?;
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
        let diff = if reviews.iter().any(|r| r.include_diff) {
            git_snapshot::bounded_diff_for_paths(
                &project.root,
                snapshot.head.is_some(),
                focused_paths.as_deref(),
            )?
        } else {
            String::new()
        };
        Ok(Self {
            commit_message,
            message_file,
            excerpts,
            focused_paths,
            manifest,
            diff,
        })
    }

    pub(crate) fn ensure_stable(&self) -> Result<()> {
        if let (Some(path), Some(message)) = (&self.message_file, &self.commit_message) {
            ensure!(
                read_message(path)?.text() == message.text(),
                "Commit message changed during verification"
            );
        }
        Ok(())
    }
}

fn read_message(path: &Path) -> Result<CommitMessage> {
    let metadata = path.symlink_metadata()?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "commit message must be a regular nonsymlink file"
    );
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(crate::limits::MAX_COMMIT_MESSAGE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= crate::limits::MAX_COMMIT_MESSAGE_BYTES,
        "commit message exceeds {} bytes",
        crate::limits::MAX_COMMIT_MESSAGE_BYTES
    );
    let text =
        String::from_utf8(bytes).map_err(|_| anyhow::anyhow!("commit message must be UTF-8"))?;
    ensure!(
        !text
            .chars()
            .any(|c| (c.is_control() && !matches!(c, '\t' | '\n' | '\r'))
                || matches!(c, '\u{2028}' | '\u{2029}')),
        "commit message cannot contain control characters"
    );
    Ok(CommitMessage {
        lines: text
            .lines()
            .enumerate()
            .map(|(i, text)| MessageLine {
                number: i + 1,
                text: text.into(),
            })
            .collect(),
    })
}
