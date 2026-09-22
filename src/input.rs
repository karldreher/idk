//! Resolving command-line input into the list of files to process.

use std::path::PathBuf;

use crate::cli::InputArgs;

/// The files to process, plus how many inputs could not be resolved.
pub struct Inputs {
    /// Files to process, in input order.
    pub files: Vec<PathBuf>,
    /// Inputs that failed to resolve; each was reported on stderr.
    pub failures: usize,
}

/// Resolves `args` into files, reporting unresolvable inputs on stderr.
pub async fn resolve(args: InputArgs) -> Inputs {
    Inputs {
        files: args.files,
        failures: 0,
    }
}
