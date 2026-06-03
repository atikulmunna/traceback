use std::{
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
    time::SystemTime,
};

use globset::{Glob, GlobSet, GlobSetBuilder};
use thiserror::Error;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

#[derive(Debug, Error)]
pub enum ScanError {
    #[error("source path does not exist: {0}")]
    SourceMissing(PathBuf),
    #[error("repository path is inside a source tree: {repository} is under {source_path}")]
    RepositoryInsideSource {
        source_path: PathBuf,
        repository: PathBuf,
    },
    #[error("ignore pattern is invalid: {pattern}: {source}")]
    InvalidIgnorePattern {
        pattern: String,
        source: globset::Error,
    },
    #[error("filesystem error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanOptions {
    pub sources: Vec<PathBuf>,
    pub repository: Option<PathBuf>,
    pub ignore_patterns: Vec<String>,
    pub changing_file_policy: ChangingFilePolicy,
}

impl ScanOptions {
    pub fn new(sources: Vec<PathBuf>) -> Self {
        Self {
            sources,
            repository: None,
            ignore_patterns: Vec::new(),
            changing_file_policy: ChangingFilePolicy::RetryThenWarn,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangingFilePolicy {
    RetryThenWarn,
    FailFast,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanInventory {
    pub entries: Vec<ScannedEntry>,
    pub warnings: Vec<ScanWarning>,
    pub ignored_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScannedEntry {
    pub source: PathBuf,
    pub path: PathBuf,
    pub relative_path: String,
    pub file_type: ScannedFileType,
    pub size: u64,
    pub modified_at: Option<String>,
    pub symlink_target: Option<PathBuf>,
    pub content: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScannedFileType {
    Directory,
    File,
    Symlink,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanWarning {
    Ignored(PathBuf),
    Unreadable { path: PathBuf, message: String },
    UnsupportedFileType(PathBuf),
    FileChanged(PathBuf),
}

pub fn scan(options: &ScanOptions) -> Result<ScanInventory, ScanError> {
    let ignore_set = build_ignore_set(&options.ignore_patterns)?;
    let sources = canonical_sources(&options.sources)?;
    let repository = canonical_repository(options.repository.as_deref())?;
    ensure_repository_is_not_inside_source(&sources, repository.as_deref())?;

    let mut inventory = ScanInventory {
        entries: Vec::new(),
        warnings: Vec::new(),
        ignored_count: 0,
    };

    for source in sources {
        scan_source(
            &source,
            &ignore_set,
            options.changing_file_policy,
            &mut inventory,
        )?;
    }

    inventory.entries.sort_by(|left, right| {
        left.relative_path
            .cmp(&right.relative_path)
            .then_with(|| left.path.cmp(&right.path))
    });
    Ok(inventory)
}

fn scan_source(
    source: &Path,
    ignore_set: &GlobSet,
    changing_file_policy: ChangingFilePolicy,
    inventory: &mut ScanInventory,
) -> Result<(), ScanError> {
    scan_path(source, source, ignore_set, changing_file_policy, inventory)
}

fn scan_path(
    source: &Path,
    path: &Path,
    ignore_set: &GlobSet,
    changing_file_policy: ChangingFilePolicy,
    inventory: &mut ScanInventory,
) -> Result<(), ScanError> {
    let relative_path = portable_relative_path(source, path);
    if should_ignore(&relative_path, ignore_set) {
        inventory.ignored_count += 1;
        inventory
            .warnings
            .push(ScanWarning::Ignored(path.to_owned()));
        return Ok(());
    }

    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(source) => {
            inventory.warnings.push(ScanWarning::Unreadable {
                path: path.to_owned(),
                message: source.to_string(),
            });
            return Ok(());
        }
    };

    let file_type = metadata.file_type();
    if file_type.is_dir() {
        inventory.entries.push(ScannedEntry {
            source: source.to_owned(),
            path: path.to_owned(),
            relative_path,
            file_type: ScannedFileType::Directory,
            size: 0,
            modified_at: modified_at(&metadata),
            symlink_target: None,
            content: None,
        });
        scan_directory_children(source, path, ignore_set, changing_file_policy, inventory)?;
    } else if file_type.is_symlink() {
        match fs::read_link(path) {
            Ok(target) => inventory.entries.push(ScannedEntry {
                source: source.to_owned(),
                path: path.to_owned(),
                relative_path,
                file_type: ScannedFileType::Symlink,
                size: 0,
                modified_at: modified_at(&metadata),
                symlink_target: Some(target),
                content: None,
            }),
            Err(source) => inventory.warnings.push(ScanWarning::Unreadable {
                path: path.to_owned(),
                message: source.to_string(),
            }),
        }
    } else if file_type.is_file() {
        match read_stable_file(path, changing_file_policy) {
            Ok(Some((metadata, content))) => inventory.entries.push(ScannedEntry {
                source: source.to_owned(),
                path: path.to_owned(),
                relative_path,
                file_type: ScannedFileType::File,
                size: metadata.len(),
                modified_at: modified_at(&metadata),
                symlink_target: None,
                content: Some(content),
            }),
            Ok(None) => inventory
                .warnings
                .push(ScanWarning::FileChanged(path.to_owned())),
            Err(source) => inventory.warnings.push(ScanWarning::Unreadable {
                path: path.to_owned(),
                message: source.to_string(),
            }),
        }
    } else {
        inventory
            .warnings
            .push(ScanWarning::UnsupportedFileType(path.to_owned()));
    }

    Ok(())
}

fn scan_directory_children(
    source: &Path,
    directory: &Path,
    ignore_set: &GlobSet,
    changing_file_policy: ChangingFilePolicy,
    inventory: &mut ScanInventory,
) -> Result<(), ScanError> {
    let children = match fs::read_dir(directory) {
        Ok(children) => children,
        Err(source) => {
            inventory.warnings.push(ScanWarning::Unreadable {
                path: directory.to_owned(),
                message: source.to_string(),
            });
            return Ok(());
        }
    };
    let mut paths = Vec::new();
    for child in children {
        match child {
            Ok(child) => paths.push(child.path()),
            Err(source) => inventory.warnings.push(ScanWarning::Unreadable {
                path: directory.to_owned(),
                message: source.to_string(),
            }),
        }
    }
    paths.sort();

    for path in paths {
        scan_path(source, &path, ignore_set, changing_file_policy, inventory)?;
    }

    Ok(())
}

fn read_stable_file(
    path: &Path,
    policy: ChangingFilePolicy,
) -> Result<Option<(fs::Metadata, Vec<u8>)>, io::Error> {
    let first = read_file_once(path)?;
    if first.changed {
        return match policy {
            ChangingFilePolicy::RetryThenWarn => {
                let second = read_file_once(path)?;
                if second.changed {
                    Ok(None)
                } else {
                    Ok(Some((second.after, second.content)))
                }
            }
            ChangingFilePolicy::FailFast => Ok(None),
        };
    }

    Ok(Some((first.after, first.content)))
}

struct FileRead {
    after: fs::Metadata,
    content: Vec<u8>,
    changed: bool,
}

fn read_file_once(path: &Path) -> Result<FileRead, io::Error> {
    let before = fs::metadata(path)?;
    maybe_mutate_file_for_test(path, &before)?;
    let mut file = fs::File::open(path)?;
    let mut content = Vec::new();
    file.read_to_end(&mut content)?;
    let after = fs::metadata(path)?;
    let changed = before.len() != after.len() || modified(&before) != modified(&after);

    Ok(FileRead {
        after,
        content,
        changed,
    })
}

fn build_ignore_set(patterns: &[String]) -> Result<GlobSet, ScanError> {
    let mut builder = GlobSetBuilder::new();
    for pattern in patterns {
        for normalized in normalize_ignore_pattern(pattern) {
            let glob =
                Glob::new(&normalized).map_err(|source| ScanError::InvalidIgnorePattern {
                    pattern: pattern.clone(),
                    source,
                })?;
            builder.add(glob);
        }
    }

    builder
        .build()
        .map_err(|source| ScanError::InvalidIgnorePattern {
            pattern: patterns.join(", "),
            source,
        })
}

fn normalize_ignore_pattern(pattern: &str) -> Vec<String> {
    let pattern = pattern.trim().replace('\\', "/");
    if pattern.ends_with('/') {
        let directory = pattern.trim_end_matches('/').to_owned();
        vec![directory.clone(), format!("{directory}/**")]
    } else if pattern.contains('/') || pattern.contains('*') || pattern.contains('?') {
        vec![pattern]
    } else {
        vec![pattern.clone(), format!("**/{pattern}")]
    }
}

fn should_ignore(relative_path: &str, ignore_set: &GlobSet) -> bool {
    !relative_path.is_empty() && ignore_set.is_match(relative_path)
}

fn canonical_sources(sources: &[PathBuf]) -> Result<Vec<PathBuf>, ScanError> {
    let mut canonical = Vec::with_capacity(sources.len());
    for source in sources {
        if !source.exists() {
            return Err(ScanError::SourceMissing(source.clone()));
        }
        canonical.push(fs::canonicalize(source).map_err(|error| io_error(source, error))?);
    }

    Ok(canonical)
}

fn canonical_repository(repository: Option<&Path>) -> Result<Option<PathBuf>, ScanError> {
    repository
        .map(|repository| fs::canonicalize(repository).map_err(|error| io_error(repository, error)))
        .transpose()
}

fn ensure_repository_is_not_inside_source(
    sources: &[PathBuf],
    repository: Option<&Path>,
) -> Result<(), ScanError> {
    let Some(repository) = repository else {
        return Ok(());
    };

    for source in sources {
        if repository.starts_with(source) {
            return Err(ScanError::RepositoryInsideSource {
                source_path: source.clone(),
                repository: repository.to_owned(),
            });
        }
    }

    Ok(())
}

fn portable_relative_path(source: &Path, path: &Path) -> String {
    match path.strip_prefix(source) {
        Ok(relative) if relative.as_os_str().is_empty() => ".".to_owned(),
        Ok(relative) => relative
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
        Err(_) => path.to_string_lossy().replace('\\', "/"),
    }
}

fn modified_at(metadata: &fs::Metadata) -> Option<String> {
    modified(metadata).and_then(|modified| {
        let duration = modified.duration_since(SystemTime::UNIX_EPOCH).ok()?;
        OffsetDateTime::from_unix_timestamp(i64::try_from(duration.as_secs()).ok()?)
            .ok()?
            .format(&Rfc3339)
            .ok()
    })
}

fn modified(metadata: &fs::Metadata) -> Option<SystemTime> {
    metadata.modified().ok()
}

fn io_error(path: &Path, source: io::Error) -> ScanError {
    ScanError::Io {
        path: path.to_owned(),
        source,
    }
}

#[cfg(not(test))]
fn maybe_mutate_file_for_test(_path: &Path, _before: &fs::Metadata) -> Result<(), io::Error> {
    Ok(())
}

#[cfg(test)]
fn maybe_mutate_file_for_test(path: &Path, _before: &fs::Metadata) -> Result<(), io::Error> {
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "changing.txt")
    {
        fs::write(path, "after")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{ChangingFilePolicy, ScanError, ScanOptions, ScanWarning, ScannedFileType, scan};

    #[test]
    fn scans_files_directories_and_contents() {
        let temporary = tempdir().expect("temporary directory should be created");
        let source = temporary.path().join("source");
        fs::create_dir(&source).expect("source should be created");
        fs::create_dir(source.join("notes")).expect("directory should be created");
        fs::write(source.join("notes").join("a.txt"), "hello").expect("file should be written");

        let inventory = scan(&ScanOptions::new(vec![source])).expect("scan should succeed");

        assert!(inventory.entries.iter().any(|entry| {
            entry.relative_path == "notes/a.txt"
                && entry.file_type == ScannedFileType::File
                && entry.content.as_deref() == Some(b"hello".as_slice())
        }));
        assert!(inventory.entries.iter().any(|entry| {
            entry.relative_path == "notes" && entry.file_type == ScannedFileType::Directory
        }));
    }

    #[test]
    fn applies_basic_ignore_patterns() {
        let temporary = tempdir().expect("temporary directory should be created");
        let source = temporary.path().join("source");
        fs::create_dir(&source).expect("source should be created");
        fs::create_dir(source.join("target")).expect("target should be created");
        fs::write(source.join("target").join("artifact.bin"), "ignored")
            .expect("ignored file should be written");
        fs::write(source.join("keep.txt"), "kept").expect("kept file should be written");
        let mut options = ScanOptions::new(vec![source]);
        options.ignore_patterns = vec!["target/".to_owned()];

        let inventory = scan(&options).expect("scan should succeed");

        assert_eq!(inventory.ignored_count, 1);
        assert!(
            inventory
                .entries
                .iter()
                .all(|entry| !entry.relative_path.starts_with("target"))
        );
        assert!(
            inventory
                .warnings
                .iter()
                .any(|warning| matches!(warning, ScanWarning::Ignored(_)))
        );
    }

    #[test]
    fn counts_ignored_files() {
        let temporary = tempdir().expect("temporary directory should be created");
        let source = temporary.path().join("source");
        fs::create_dir(&source).expect("source should be created");
        fs::write(source.join("skip.tmp"), "ignored").expect("ignored file should be written");
        fs::write(source.join("keep.txt"), "kept").expect("kept file should be written");
        let mut options = ScanOptions::new(vec![source]);
        options.ignore_patterns = vec!["*.tmp".to_owned()];

        let inventory = scan(&options).expect("scan should succeed");

        assert_eq!(inventory.ignored_count, 1);
        assert!(
            inventory
                .entries
                .iter()
                .all(|entry| entry.relative_path != "skip.tmp")
        );
    }

    #[test]
    fn rejects_repository_inside_source() {
        let temporary = tempdir().expect("temporary directory should be created");
        let source = temporary.path().join("source");
        let repository = source.join(".traceback");
        fs::create_dir_all(&repository).expect("repository should be created");
        let mut options = ScanOptions::new(vec![source]);
        options.repository = Some(repository);

        let error = scan(&options).expect_err("repository inside source should be rejected");

        assert!(matches!(error, ScanError::RepositoryInsideSource { .. }));
    }

    #[test]
    fn records_symlink_without_following_it() {
        let temporary = tempdir().expect("temporary directory should be created");
        let source = temporary.path().join("source");
        fs::create_dir(&source).expect("source should be created");
        if !create_symlink(&source.join("missing-target"), &source.join("link")) {
            return;
        }

        let inventory = scan(&ScanOptions::new(vec![source])).expect("scan should succeed");

        assert!(inventory.entries.iter().any(|entry| {
            entry.relative_path == "link"
                && entry.file_type == ScannedFileType::Symlink
                && entry.symlink_target.is_some()
        }));
    }

    #[test]
    fn reports_missing_sources() {
        let temporary = tempdir().expect("temporary directory should be created");

        let error = scan(&ScanOptions::new(vec![temporary.path().join("missing")]))
            .expect_err("missing source should fail");

        assert!(matches!(error, ScanError::SourceMissing(_)));
    }

    #[test]
    fn detects_files_that_change_during_read() {
        let temporary = tempdir().expect("temporary directory should be created");
        let source = temporary.path().join("source");
        fs::create_dir(&source).expect("source should be created");
        let file = source.join("changing.txt");
        fs::write(&file, "before").expect("file should be written");

        let mut options = ScanOptions::new(vec![source]);
        options.changing_file_policy = ChangingFilePolicy::FailFast;

        let inventory = scan(&options).expect("scan should succeed");

        assert!(
            inventory
                .warnings
                .iter()
                .any(|warning| matches!(warning, ScanWarning::FileChanged(_)))
        );
    }

    #[cfg(unix)]
    fn create_symlink(target: &std::path::Path, link: &std::path::Path) -> bool {
        std::os::unix::fs::symlink(target, link).is_ok()
    }

    #[cfg(windows)]
    fn create_symlink(target: &std::path::Path, link: &std::path::Path) -> bool {
        std::os::windows::fs::symlink_file(target, link).is_ok()
    }
}
