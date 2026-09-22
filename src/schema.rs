//! `idk schema write|validate`: publish and check the config JSON Schema.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::config::{self, Config};

/// Writes the config JSON Schema to `path`, printing how to reference it from YAML.
pub async fn write(path: &Path) -> ExitCode {
    let mut json = serde_json::to_string_pretty(&config::schema()).expect("schema serializes");
    json.push('\n');
    if let Err(err) = tokio::fs::write(path, json).await {
        eprintln!("error: {}: {err}", path.display());
        return ExitCode::FAILURE;
    }
    println!("wrote {}", path.display());
    println!(
        "reference it from a config file with: # yaml-language-server: $schema={}",
        schema_reference(path).display()
    );
    ExitCode::SUCCESS
}

/// Validates the config file at `path`, reporting every violation.
pub async fn validate(path: &Path) -> ExitCode {
    match Config::load(path).await {
        Ok(_) => {
            println!("{}: valid", path.display());
            ExitCode::SUCCESS
        }
        Err(err) => err.report(),
    }
}

/// `./idk.yaml.json` for a bare relative path, so editors resolve it next to the config.
fn schema_reference(path: &Path) -> PathBuf {
    if path.is_relative() && !path.starts_with(".") && !path.starts_with("..") {
        Path::new(".").join(path)
    } else {
        path.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_bare_relative_paths_from_current_directory() {
        assert_eq!(
            schema_reference(Path::new("idk.yaml.json")),
            Path::new("./idk.yaml.json")
        );
        assert_eq!(
            schema_reference(Path::new("../s.json")),
            Path::new("../s.json")
        );
        assert_eq!(
            schema_reference(Path::new("/tmp/s.json")),
            Path::new("/tmp/s.json")
        );
    }
}
