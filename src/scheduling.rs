//! Local performance hints only; never verification evidence.
use crate::report::{Item, Status};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

const FALLBACK_MS: u64 = 1000;
const MAX_CACHE_BYTES: u64 = 1024 * 1024;

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct History {
    estimates: BTreeMap<String, BTreeMap<String, u64>>,
}

impl History {
    pub(crate) fn load(root: &Path) -> Self {
        cache_path(root)
            .and_then(|path| {
                let metadata = path.symlink_metadata().ok()?;
                if !metadata.is_file() || metadata.len() > MAX_CACHE_BYTES {
                    return None;
                }
                let text = crate::config::read_bounded(&path, MAX_CACHE_BYTES).ok()?;
                serde_json::from_str(&text).ok()
            })
            .unwrap_or_default()
    }

    pub(crate) fn estimate(&self, kind: &str, id: &str) -> u64 {
        self.estimates
            .get(kind)
            .and_then(|rules| rules.get(id))
            .copied()
            .unwrap_or(FALLBACK_MS)
            .max(1)
    }

    fn record(&mut self, items: &[Item]) {
        for item in items
            .iter()
            .filter(|i| i.status == Status::Pass && matches!(i.kind.as_str(), "check" | "review"))
        {
            let sample = item.duration_ms.max(1);
            self.estimates
                .entry(item.kind.clone())
                .or_default()
                .entry(item.id.clone())
                .and_modify(|old| {
                    *old = (*old / 2 + sample / 2 + (*old % 2 + sample % 2) / 2).max(1);
                })
                .or_insert(sample);
        }
    }

    pub(crate) fn save_results(root: &Path, items: &[Item]) {
        // Best effort: hints must never block verification or modify its outcome.
        let _ = (|| -> anyhow::Result<()> {
            if !items
                .iter()
                .any(|i| i.status == Status::Pass && matches!(i.kind.as_str(), "check" | "review"))
            {
                return Ok(());
            }
            let Some(path) = cache_path(root) else {
                return Ok(());
            };
            let mut history = Self::load(root);
            history.record(items);
            let bytes = serde_json::to_vec(&history)?;
            anyhow::ensure!(
                bytes.len() as u64 <= MAX_CACHE_BYTES,
                "scheduling history exceeds cache limit"
            );
            let mut file = tempfile::NamedTempFile::new_in(path.parent().unwrap())?;
            file.write_all(&bytes)?;
            file.persist(path)?;
            Ok(())
        })();
    }
}

fn cache_path(root: &Path) -> Option<PathBuf> {
    let output = crate::process::run(
        Command::new("git")
            .current_dir(root)
            .stdin(Stdio::null())
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("GIT_NO_LAZY_FETCH", "1")
            .args([
                "rev-parse",
                "--path-format=absolute",
                "--git-path",
                "please-scheduling.json",
            ]),
        Duration::from_secs(10),
    )
    .ok()?;
    (output.status.success() && !output.timed_out).then(|| PathBuf::from(output.stdout.text.trim()))
}

/// Reverse topological propagation: own estimate plus the longest dependent tail.
/// Dependencies have already been validated by the planner.
pub(crate) fn priorities(dependencies: &[Vec<usize>], estimates: Vec<u64>) -> Vec<u64> {
    let mut remaining = vec![0; dependencies.len()];
    for deps in dependencies {
        for &dependency in deps {
            remaining[dependency] += 1;
        }
    }
    let mut ready: Vec<_> = remaining
        .iter()
        .enumerate()
        .filter_map(|(i, &n)| (n == 0).then_some(i))
        .collect();
    let mut priority = estimates.clone();
    while let Some(index) = ready.pop() {
        for &dependency in &dependencies[index] {
            priority[dependency] =
                priority[dependency].max(estimates[dependency].saturating_add(priority[index]));
            remaining[dependency] -= 1;
            if remaining[dependency] == 0 {
                ready.push(dependency);
            }
        }
    }
    priority
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn longest_path_uses_maximum_tail_and_saturates() {
        assert_eq!(
            priorities(&[vec![], vec![0], vec![0], vec![1, 2]], vec![1, 3, 7, 2]),
            [10, 5, 9, 2]
        );
        assert_eq!(
            priorities(&[vec![], vec![0]], vec![u64::MAX, 1]),
            [u64::MAX, 1]
        );
    }
    #[test]
    fn only_pass_updates_estimates_and_identities_are_independent() {
        let mut history = History::default();
        assert_eq!(history.estimate("check", "a"), 1000);
        for (kind, id, ms, status) in [
            ("check", "a", 100, Status::Pass),
            ("check", "a", 300, Status::Pass),
            ("review", "a", 700, Status::Pass),
            ("check", "import/a", 900, Status::Pass),
            ("check", "zero", 0, Status::Pass),
        ] {
            let mut item = Item::simple(id, kind, status, String::new());
            item.duration_ms = ms;
            history.record(&[item]);
        }
        for status in [
            Status::Fail,
            Status::Error,
            Status::NotRun,
            Status::Cancelled,
        ] {
            let mut item = Item::simple("a", "check", status, String::new());
            item.duration_ms = 9999;
            history.record(&[item]);
        }
        assert_eq!(history.estimate("check", "a"), 200);
        assert_eq!(history.estimate("review", "a"), 700);
        assert_eq!(history.estimate("check", "import/a"), 900);
        assert_eq!(history.estimate("check", "zero"), 1);
    }
}
