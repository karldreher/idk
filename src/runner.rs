//! Concurrent per-file execution shared by every operation.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::Arc;

use futures_util::{StreamExt, stream};
use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};

/// Runs `op` on every file on tokio's blocking pool, at most `jobs` at a time.
///
/// Duplicate inputs are processed once. Results arrive as each file finishes. Each result is passed to `on_result`
/// along with the progress bar, which should be used to print without tearing it.
pub async fn process<T, Op>(
    files: Vec<PathBuf>,
    jobs: usize,
    show_progress: bool,
    op: Op,
    mut on_result: impl FnMut(&ProgressBar, PathBuf, T),
) where
    T: Send + 'static,
    Op: Fn(&Path) -> T + Send + Sync + 'static,
{
    let files = unique_files(files).await;
    let progress = progress_bar(files.len() as u64, show_progress);
    let op = Arc::new(op);
    let tasks = stream::iter(files).map(move |path| {
        let op = Arc::clone(&op);
        async move {
            let task_path = path.clone();
            let result = tokio::task::spawn_blocking(move || op(&task_path))
                .await
                .expect("file task panicked");
            (path, result)
        }
    });
    let mut results = pin!(tasks.buffer_unordered(jobs));
    while let Some((path, result)) = results.next().await {
        on_result(&progress, path, result);
        progress.inc(1);
    }
    progress.finish_and_clear();
}

/// Builds the overall progress bar on stderr.
///
/// indicatif also hides it automatically when stderr is not a terminal, so
/// piped and scripted runs stay clean.
fn progress_bar(len: u64, show: bool) -> ProgressBar {
    if !show {
        return ProgressBar::hidden();
    }
    let style =
        ProgressStyle::with_template("{bar:40.cyan/blue} {pos}/{len} files ({per_sec}, eta {eta})")
            .expect("valid progress template")
            .progress_chars("##-");
    ProgressBar::with_draw_target(Some(len), ProgressDrawTarget::stderr()).with_style(style)
}

/// Drops inputs that resolve to the same file, keeping the first spelling.
///
/// Two concurrent writers on one file would race, so duplicates (for example
/// `a.mp3` and `./a.mp3`) must be collapsed before dispatch.
async fn unique_files(files: Vec<PathBuf>) -> Vec<PathBuf> {
    tokio::task::spawn_blocking(move || {
        let mut seen = HashSet::new();
        files
            .into_iter()
            .filter(|path| {
                seen.insert(std::fs::canonicalize(path).unwrap_or_else(|_| path.clone()))
            })
            .collect()
    })
    .await
    .expect("dedupe task panicked")
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[tokio::test]
    async fn collapses_duplicate_inputs() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("song.mp3");
        std::fs::write(&path, b"x").unwrap();
        let alias = dir.path().join(".").join("song.mp3");

        let files = unique_files(vec![path.clone(), alias, path.clone()]).await;

        assert_eq!(files, vec![path]);
    }
}
