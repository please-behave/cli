//! Provider-neutral, bounded evidence prepared once for planning and execution.
use crate::{
    change_set::{self, Change, ChangeSet, State},
    config::{Project, Review},
    git_snapshot::Snapshot,
    limits::{MAX_SCOPE_CONTENT_BYTES, MAX_SCOPE_FILES},
    review_context::{self, Excerpt},
    review_trigger,
};
use anyhow::{Result, ensure};
use std::{
    collections::BTreeMap,
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

/// One changed file in the scope of a review. `content` is the current content of
/// a text file; a binary file is marked and has none.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ScopeFile {
    pub path: String,
    pub change: State,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub binary: bool,
}
/// The initial file input of one review: its changed files that match the triggers.
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ReviewScope {
    pub files: Vec<ScopeFile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub change_set: Option<String>,
}

/// The changed files that are in the scope of a review: all of them without triggers.
pub fn scope_changes<'a>(review: &Review, changes: &'a ChangeSet) -> Vec<&'a Change> {
    changes
        .files
        .iter()
        .filter(|change| {
            review
                .trigger
                .as_ref()
                .is_none_or(|trigger| review_trigger::matches_path(trigger, &change.path))
        })
        .collect()
}

#[derive(Debug, serde::Serialize)]
pub struct PreparedReviewInput {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_message: Option<CommitMessage>,
    #[serde(skip)]
    message_file: Option<PathBuf>,
    pub excerpts: Vec<Excerpt>,
    /// Scope by review ID; sent to the adapter per review.
    #[serde(skip)]
    pub scopes: BTreeMap<String, ReviewScope>,
}
impl PreparedReviewInput {
    /// `message_file` is given at commit-msg, where the whole message is declared input.
    pub(crate) fn prepare_with_message(
        project: &Project,
        reviews: &[&Review],
        snapshot: &Snapshot,
        message_file: Option<PathBuf>,
        changes: &ChangeSet,
    ) -> Result<Self> {
        let commit_message = message_file.as_deref().map(read_message).transpose()?;
        for path in reviews
            .iter()
            .flat_map(|r| r.context.iter().map(|c| &c.path))
        {
            ensure!(
                snapshot.files.contains_key(path),
                "review context file not in verified manifest: {path}"
            );
        }
        let excerpts = review_context::collect(&project.root, reviews)?;
        let mut scopes = BTreeMap::new();
        for review in reviews {
            scopes.insert(
                review.id.clone(),
                scope(project, review, snapshot, changes)?,
            );
        }
        Ok(Self {
            commit_message,
            message_file,
            excerpts,
            scopes,
        })
    }

    pub fn prepare(
        project: &Project,
        reviews: &[&Review],
        snapshot: &Snapshot,
        changes: &ChangeSet,
    ) -> Result<Self> {
        Self::prepare_with_message(project, reviews, snapshot, None, changes)
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

fn scope(
    project: &Project,
    review: &Review,
    snapshot: &Snapshot,
    changes: &ChangeSet,
) -> Result<ReviewScope> {
    let selected = scope_changes(review, changes);
    ensure!(
        selected.len() <= MAX_SCOPE_FILES,
        "review {} has {} files in scope, above the limit of {MAX_SCOPE_FILES}; narrow its triggers",
        review.id,
        selected.len()
    );
    let mut total = 0;
    let mut files = Vec::new();
    for change in &selected {
        let (mut content, mut binary) = (None, false);
        let readable = review.include_content
            && change.state != State::Deleted
            && snapshot.files.contains_key(&change.path);
        if readable {
            match std::fs::read(project.root.join(&change.path))
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
                .filter(|text| !text.contains('\0'))
            {
                Some(text) => {
                    total += text.len();
                    ensure!(
                        total <= MAX_SCOPE_CONTENT_BYTES,
                        "content of the files in scope of review {} exceeds {MAX_SCOPE_CONTENT_BYTES} bytes; narrow its triggers or set include_content = false",
                        review.id
                    );
                    content = Some(text);
                }
                None => binary = true,
            }
        }
        files.push(ScopeFile {
            path: change.path.clone(),
            change: change.state,
            content,
            binary,
        });
    }
    let change_set = if review.include_change_set {
        let paths: Vec<_> = selected.iter().map(|c| c.path.clone()).collect();
        Some(change_set::patch(&project.root, &changes.source, &paths)?)
    } else {
        None
    };
    Ok(ReviewScope { files, change_set })
}
