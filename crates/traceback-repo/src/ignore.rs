use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

use serde::Serialize;
use thiserror::Error;

const LARGE_GENERATED_BYTES: u64 = 100 * 1024 * 1024;

#[derive(Debug, Error)]
pub enum IgnoreError {
    #[error("suggestion path does not exist: {0}")]
    PathMissing(PathBuf),
    #[error("filesystem error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct IgnoreSuggestion {
    pub rule: String,
    pub category: String,
    pub estimated_bytes: u64,
    pub matched_paths: usize,
}

#[derive(Default)]
struct SuggestionTotal {
    category: String,
    estimated_bytes: u64,
    matched_paths: usize,
}

pub fn suggest_ignores(path: &Path) -> Result<Vec<IgnoreSuggestion>, IgnoreError> {
    if !path.exists() {
        return Err(IgnoreError::PathMissing(path.to_owned()));
    }
    let root = fs::canonicalize(path).map_err(|source| io_error(path, source))?;
    let mut totals = BTreeMap::<String, SuggestionTotal>::new();
    inspect_path(&root, &root, &mut totals)?;
    let mut suggestions = totals
        .into_iter()
        .map(|(rule, total)| IgnoreSuggestion {
            rule,
            category: total.category,
            estimated_bytes: total.estimated_bytes,
            matched_paths: total.matched_paths,
        })
        .collect::<Vec<_>>();
    suggestions.sort_by(|left, right| {
        right
            .estimated_bytes
            .cmp(&left.estimated_bytes)
            .then_with(|| left.rule.cmp(&right.rule))
    });
    Ok(suggestions)
}

fn inspect_path(
    root: &Path,
    path: &Path,
    totals: &mut BTreeMap<String, SuggestionTotal>,
) -> Result<(), IgnoreError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| io_error(path, source))?;
    if metadata.file_type().is_symlink() {
        return Ok(());
    }
    if metadata.is_dir() {
        if path != root
            && let Some((rule, category)) = directory_rule(path)
        {
            add_suggestion(totals, rule, category, directory_size(path)?);
            return Ok(());
        }
        for entry in fs::read_dir(path).map_err(|source| io_error(path, source))? {
            let entry = entry.map_err(|source| io_error(path, source))?;
            inspect_path(root, &entry.path(), totals)?;
        }
        return Ok(());
    }
    if !metadata.is_file() {
        return Ok(());
    }
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return Ok(());
    };
    if is_temporary_file(name) {
        add_suggestion(totals, "*.tmp", "temporary", metadata.len());
    } else if metadata.len() >= LARGE_GENERATED_BYTES && is_generated_extension(path) {
        let relative = path
            .strip_prefix(root)
            .expect("inspected path is under root")
            .to_string_lossy()
            .replace('\\', "/");
        add_suggestion(totals, &relative, "oversized_generated", metadata.len());
    }
    Ok(())
}

fn directory_rule(path: &Path) -> Option<(&'static str, &'static str)> {
    match path.file_name()?.to_str()? {
        "node_modules" => Some(("**/node_modules", "dependency")),
        "target" => Some(("**/target", "build")),
        "dist" => Some(("**/dist", "build")),
        "build" => Some(("**/build", "build")),
        ".cache" => Some(("**/.cache", "cache")),
        ".next" => Some(("**/.next", "cache")),
        "__pycache__" => Some(("**/__pycache__", "cache")),
        _ => None,
    }
}

fn directory_size(path: &Path) -> Result<u64, IgnoreError> {
    let mut total = 0_u64;
    for entry in fs::read_dir(path).map_err(|source| io_error(path, source))? {
        let entry = entry.map_err(|source| io_error(path, source))?;
        let child = entry.path();
        let metadata = fs::symlink_metadata(&child).map_err(|source| io_error(&child, source))?;
        if metadata.file_type().is_symlink() {
            continue;
        }
        let size = if metadata.is_dir() {
            directory_size(&child)?
        } else if metadata.is_file() {
            metadata.len()
        } else {
            0
        };
        total = total
            .checked_add(size)
            .expect("suggested logical byte count should fit u64");
    }
    Ok(total)
}

fn add_suggestion(
    totals: &mut BTreeMap<String, SuggestionTotal>,
    rule: &str,
    category: &str,
    bytes: u64,
) {
    let total = totals.entry(rule.to_owned()).or_default();
    total.category = category.to_owned();
    total.estimated_bytes = total
        .estimated_bytes
        .checked_add(bytes)
        .expect("suggested logical byte count should fit u64");
    total.matched_paths += 1;
}

fn is_temporary_file(name: &str) -> bool {
    name.to_ascii_lowercase().ends_with(".tmp")
}

fn is_generated_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "bin" | "img" | "iso" | "tar" | "zip"
            )
        })
}

fn io_error(path: &Path, source: io::Error) -> IgnoreError {
    IgnoreError::Io {
        path: path.to_owned(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::suggest_ignores;

    #[test]
    fn suggests_common_directories_temporary_files_and_large_generated_files() {
        let temporary = tempdir().expect("temporary directory should be created");
        let root = temporary.path();
        fs::create_dir(root.join("node_modules")).expect("directory should be created");
        fs::write(root.join("node_modules/package.js"), vec![0; 12])
            .expect("file should be written");
        fs::write(root.join("one.tmp"), vec![0; 3]).expect("file should be written");
        fs::write(root.join("two.tmp"), vec![0; 4]).expect("file should be written");
        let large = fs::File::create(root.join("artifact.iso")).expect("file should be created");
        large
            .set_len(100 * 1024 * 1024)
            .expect("sparse test file should resize");

        let suggestions = suggest_ignores(root).expect("suggestions should work");

        let dependencies = suggestions
            .iter()
            .find(|suggestion| suggestion.rule == "**/node_modules")
            .expect("dependency directory should be suggested");
        assert_eq!(dependencies.estimated_bytes, 12);
        let temporary = suggestions
            .iter()
            .find(|suggestion| suggestion.rule == "*.tmp")
            .expect("temporary rule should be suggested");
        assert_eq!(temporary.estimated_bytes, 7);
        assert_eq!(temporary.matched_paths, 2);
        assert!(
            suggestions
                .iter()
                .any(|suggestion| suggestion.rule == "artifact.iso")
        );
    }
}
