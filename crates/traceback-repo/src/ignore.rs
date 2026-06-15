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
    #[error("ignore rule is invalid: {0}")]
    InvalidRule(String),
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

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ApplyIgnoreReport {
    pub path: PathBuf,
    pub added: Vec<String>,
    pub skipped_existing: Vec<String>,
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

pub fn apply_ignore_rules(path: &Path, rules: &[String]) -> Result<ApplyIgnoreReport, IgnoreError> {
    if !path.exists() {
        return Err(IgnoreError::PathMissing(path.to_owned()));
    }
    let ignore_path = if path.is_dir() {
        path.join(".tracebackignore")
    } else {
        path.parent()
            .unwrap_or_else(|| Path::new("."))
            .join(".tracebackignore")
    };
    let existing = if ignore_path.exists() {
        fs::read_to_string(&ignore_path).map_err(|source| io_error(&ignore_path, source))?
    } else {
        String::new()
    };
    let existing_rules = existing
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .collect::<std::collections::BTreeSet<_>>();
    let mut added = Vec::new();
    let mut skipped_existing = Vec::new();
    for rule in rules {
        let rule = rule.trim();
        if rule.is_empty() || rule.contains(['\0', '\n', '\r']) {
            return Err(IgnoreError::InvalidRule(rule.to_owned()));
        }
        if existing_rules.contains(rule) || added.iter().any(|added| added == rule) {
            skipped_existing.push(rule.to_owned());
        } else {
            added.push(rule.to_owned());
        }
    }
    if !added.is_empty() {
        let mut updated = existing;
        if !updated.is_empty() && !updated.ends_with('\n') {
            updated.push('\n');
        }
        if !updated.is_empty() {
            updated.push('\n');
        }
        updated.push_str("# Added by TraceBack ignore apply\n");
        for rule in &added {
            updated.push_str(rule);
            updated.push('\n');
        }
        fs::write(&ignore_path, updated).map_err(|source| io_error(&ignore_path, source))?;
    }
    Ok(ApplyIgnoreReport {
        path: ignore_path,
        added,
        skipped_existing,
    })
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

    use super::{apply_ignore_rules, suggest_ignores};

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

    #[test]
    fn applies_only_missing_rules_and_preserves_existing_content() {
        let temporary = tempdir().expect("temporary directory should be created");
        let root = temporary.path();
        fs::write(root.join(".tracebackignore"), "# user rule\n*.log\n")
            .expect("ignore file should be written");

        let report = apply_ignore_rules(
            root,
            &[
                "*.log".to_owned(),
                "**/target".to_owned(),
                "*.tmp".to_owned(),
            ],
        )
        .expect("rules should apply");

        assert_eq!(report.added, ["**/target", "*.tmp"]);
        assert_eq!(report.skipped_existing, ["*.log"]);
        let contents =
            fs::read_to_string(root.join(".tracebackignore")).expect("ignore file should read");
        assert!(contents.starts_with("# user rule\n*.log\n"));
        assert_eq!(contents.matches("*.log").count(), 1);
        assert!(contents.contains("**/target\n"));
        assert!(contents.contains("*.tmp\n"));
    }
}
