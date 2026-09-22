//! Resolving command-line input into the list of files to process.
//!
//! Each argument is taken literally when that path exists. Otherwise, if it
//! contains `*`, `?` or `[`, idk expands it as a glob itself, so patterns work
//! the same on Windows (whose shells don't expand them) as on Unix.

use std::path::{Path, PathBuf};

use walkdir::WalkDir;

use crate::cli::InputArgs;

/// The files to process, plus how many inputs could not be resolved.
pub struct Inputs {
    /// Files to process, in input order.
    pub files: Vec<PathBuf>,
    /// Inputs that failed to resolve; each was reported on stderr.
    pub failures: usize,
}

/// Resolves `args` into files, reporting unresolvable inputs on stderr.
///
/// Runs on the blocking pool, since globbing and directory walks are file I/O.
pub async fn resolve(args: InputArgs) -> Inputs {
    let (files, errors) = tokio::task::spawn_blocking(move || resolve_all(&args))
        .await
        .expect("input task panicked");
    for error in &errors {
        eprintln!("error: {error}");
    }
    Inputs {
        files,
        failures: errors.len(),
    }
}

/// Expands every argument, collecting files and error messages.
fn resolve_all(args: &InputArgs) -> (Vec<PathBuf>, Vec<String>) {
    let mut files = Vec::new();
    let mut errors = Vec::new();
    for arg in &args.files {
        expand(arg, args.recursive, &mut files, &mut errors);
    }
    (files, errors)
}

/// Expands one argument: a directory, an existing or missing literal path, or a glob.
fn expand(arg: &Path, recursive: bool, files: &mut Vec<PathBuf>, errors: &mut Vec<String>) {
    if arg.is_dir() {
        if recursive {
            walk(arg, files, errors);
        } else {
            errors.push(format!("{}: is a directory (use -r)", arg.display()));
        }
        return;
    }
    // Missing literal paths pass through, so the operation reports them per file.
    if arg.exists() || !is_glob(arg) {
        files.push(arg.to_owned());
        return;
    }
    let pattern = arg.to_string_lossy();
    let paths = match glob::glob(&pattern) {
        Ok(paths) => paths,
        Err(err) => {
            errors.push(format!("{pattern}: invalid pattern: {err}"));
            return;
        }
    };
    let mut matched = false;
    for entry in paths {
        match entry {
            // Directories matched by a pattern like `*` are only descended into with -r.
            Ok(path) if path.is_dir() => {
                if recursive {
                    matched = true;
                    walk(&path, files, errors);
                }
            }
            Ok(path) => {
                matched = true;
                files.push(path);
            }
            Err(err) => errors.push(format!("{}: {}", err.path().display(), err.error())),
        }
    }
    if !matched {
        errors.push(format!("{pattern}: no files match"));
    }
}

/// Whether `path` contains glob metacharacters.
fn is_glob(path: &Path) -> bool {
    path.to_string_lossy().contains(['*', '?', '['])
}

/// Adds every `.mp3` file under `dir`, sorted by name. Directory symlinks are not followed.
fn walk(dir: &Path, files: &mut Vec<PathBuf>, errors: &mut Vec<String>) {
    for entry in WalkDir::new(dir).follow_links(false).sort_by_file_name() {
        match entry {
            Ok(entry) if entry.path().is_file() && is_mp3(entry.path()) => {
                files.push(entry.into_path());
            }
            Ok(_) => {}
            Err(err) => errors.push(err.to_string()),
        }
    }
}

/// Whether `path` has a `.mp3` extension, in any case.
fn is_mp3(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("mp3"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn touch(dir: &TempDir, name: &str) -> PathBuf {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"x").unwrap();
        path
    }

    fn resolve(files: Vec<PathBuf>, recursive: bool) -> (Vec<PathBuf>, Vec<String>) {
        resolve_all(&InputArgs { files, recursive })
    }

    #[test]
    fn literal_paths_pass_through_in_order_even_when_missing() {
        let dir = TempDir::new().unwrap();
        let b = touch(&dir, "b.mp3");
        let missing = dir.path().join("missing.mp3");

        let (files, errors) = resolve(vec![b.clone(), missing.clone()], false);

        assert_eq!(files, [b, missing]);
        assert!(errors.is_empty());
    }

    #[test]
    fn existing_paths_with_brackets_are_not_globbed() {
        let dir = TempDir::new().unwrap();
        let live = touch(&dir, "Song [Live].mp3");

        let (files, errors) = resolve(vec![live.clone()], false);

        assert_eq!(files, [live]);
        assert!(errors.is_empty());
    }

    #[test]
    fn expands_globs_in_sorted_order() {
        let dir = TempDir::new().unwrap();
        let b = touch(&dir, "b.mp3");
        let a = touch(&dir, "a.mp3");
        touch(&dir, "notes.txt");
        let nested = touch(&dir, "disc/c.mp3");

        let (files, _) = resolve(vec![dir.path().join("*.mp3")], false);
        assert_eq!(files, [a.clone(), b.clone()]);

        let (files, _) = resolve(vec![dir.path().join("**/*.mp3")], false);
        assert_eq!(files, [a, b, nested]);
    }

    #[test]
    fn unmatched_glob_is_an_error() {
        let dir = TempDir::new().unwrap();
        let pattern = dir.path().join("*.flac");

        let (files, errors) = resolve(vec![pattern.clone()], false);

        assert!(files.is_empty());
        assert_eq!(errors, [format!("{}: no files match", pattern.display())]);
    }

    #[test]
    fn directories_need_recursive() {
        let dir = TempDir::new().unwrap();
        touch(&dir, "a.mp3");

        let (files, errors) = resolve(vec![dir.path().to_owned()], false);

        assert!(files.is_empty());
        assert_eq!(
            errors,
            [format!("{}: is a directory (use -r)", dir.path().display())]
        );
    }

    #[test]
    fn recursive_walks_nested_mp3s_sorted_by_name() {
        let dir = TempDir::new().unwrap();
        let b = touch(&dir, "Artist/B Album/01.MP3");
        let a = touch(&dir, "Artist/A Album/02.mp3");
        let top = touch(&dir, "z.mp3");
        touch(&dir, "Artist/A Album/cover.jpg");

        let (files, errors) = resolve(vec![dir.path().to_owned()], true);

        assert_eq!(files, [a, b, top]);
        assert!(errors.is_empty());
    }

    #[test]
    fn explicit_non_mp3_files_are_kept() {
        let dir = TempDir::new().unwrap();
        let odd = touch(&dir, "track.mpeg");

        let (files, _) = resolve(vec![odd.clone()], true);

        assert_eq!(files, [odd]);
    }
}
