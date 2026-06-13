use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

use crate::{ChunkError, read_chunk};

const MANIFEST_VERSION: u32 = 0;
const COMPLETE_STATE: &str = "complete";

#[derive(Debug, Error)]
pub enum ManifestError {
    #[error("manifest uses unsupported version {0}")]
    UnsupportedVersion(u32),
    #[error("manifest state must be complete: {0}")]
    UnsupportedState(String),
    #[error("snapshot ID is invalid: {0}")]
    InvalidSnapshotId(String),
    #[error("timestamp is invalid: {0}")]
    InvalidTimestamp(String),
    #[error("manifest must contain at least one source")]
    MissingSources,
    #[error("manifest path is invalid: {0}")]
    InvalidPath(String),
    #[error("chunk hash is invalid: {0}")]
    InvalidChunkHash(String),
    #[error("file entry is invalid for {path}: {reason}")]
    InvalidFileEntry { path: String, reason: String },
    #[error("manifest summary does not match file entries: {0}")]
    InvalidSummary(String),
    #[error("manifest JSON is invalid at {path}: {source}")]
    InvalidJson {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("snapshot manifest already exists: {0}")]
    AlreadyExists(PathBuf),
    #[error("chunk verification failed for {hash}: {source}")]
    Chunk { hash: String, source: ChunkError },
    #[error("filesystem error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct SnapshotManifest {
    pub manifest_version: u32,
    pub snapshot_id: String,
    pub state: String,
    pub created_at: String,
    pub sources: Vec<String>,
    pub files: Vec<FileEntry>,
    pub summary: ManifestSummary,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct FileEntry {
    pub path: String,
    #[serde(rename = "type")]
    pub file_type: FileType,
    pub size: u64,
    pub modified_at: Option<String>,
    #[serde(default)]
    pub permissions: Option<u32>,
    pub content_hash: Option<String>,
    #[serde(default)]
    pub chunks: Vec<String>,
    pub symlink_target: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FileType {
    Directory,
    File,
    Symlink,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ManifestSummary {
    pub file_count: u64,
    pub logical_bytes: u64,
    pub newly_stored_bytes: u64,
}

pub fn validate_manifest(manifest: &SnapshotManifest) -> Result<(), ManifestError> {
    if manifest.manifest_version != MANIFEST_VERSION {
        return Err(ManifestError::UnsupportedVersion(manifest.manifest_version));
    }
    if manifest.state != COMPLETE_STATE {
        return Err(ManifestError::UnsupportedState(manifest.state.clone()));
    }
    validate_snapshot_id(&manifest.snapshot_id)?;
    parse_timestamp(&manifest.created_at)?;
    if manifest.sources.is_empty() {
        return Err(ManifestError::MissingSources);
    }
    for source in &manifest.sources {
        validate_portable_path(source)?;
    }

    let mut file_count = 0_u64;
    let mut logical_bytes = 0_u64;
    for file in &manifest.files {
        validate_file_entry(file)?;
        if file.file_type == FileType::File {
            file_count += 1;
            logical_bytes = logical_bytes.checked_add(file.size).ok_or_else(|| {
                ManifestError::InvalidSummary("logical byte count overflows u64".to_owned())
            })?;
        }
    }

    if manifest.summary.file_count != file_count {
        return Err(ManifestError::InvalidSummary(format!(
            "file_count expected {file_count}, found {}",
            manifest.summary.file_count
        )));
    }
    if manifest.summary.logical_bytes != logical_bytes {
        return Err(ManifestError::InvalidSummary(format!(
            "logical_bytes expected {logical_bytes}, found {}",
            manifest.summary.logical_bytes
        )));
    }

    Ok(())
}

pub fn verify_manifest_chunks(
    repository: &Path,
    manifest: &SnapshotManifest,
) -> Result<(), ManifestError> {
    validate_manifest(manifest)?;

    for file in &manifest.files {
        for hash in &file.chunks {
            read_chunk(repository, hash).map_err(|source| ManifestError::Chunk {
                hash: hash.clone(),
                source,
            })?;
        }
    }

    Ok(())
}

pub fn write_manifest(
    repository: &Path,
    manifest: &SnapshotManifest,
) -> Result<PathBuf, ManifestError> {
    validate_manifest(manifest)?;

    let path = manifest_path(repository, &manifest.snapshot_id)?;
    if path.exists() {
        return Err(ManifestError::AlreadyExists(path));
    }
    let parent = path
        .parent()
        .expect("manifest path has snapshots directory");
    fs::create_dir_all(parent).map_err(|source| io_error(parent, source))?;
    let staging = repository.join("staging");
    fs::create_dir_all(&staging).map_err(|source| io_error(&staging, source))?;
    let staged_path = staging.join(format!(
        "{}.{}.json.tmp",
        manifest.snapshot_id,
        Uuid::new_v4().simple()
    ));
    let json =
        serde_json::to_string_pretty(manifest).map_err(|source| ManifestError::InvalidJson {
            path: staged_path.clone(),
            source,
        })?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&staged_path)
        .map_err(|source| io_error(&staged_path, source))?;
    file.write_all(json.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|source| {
            let _ = fs::remove_file(&staged_path);
            io_error(&staged_path, source)
        })?;
    drop(file);

    let staged_json =
        fs::read_to_string(&staged_path).map_err(|source| io_error(&staged_path, source))?;
    let staged_manifest: SnapshotManifest =
        serde_json::from_str(&staged_json).map_err(|source| ManifestError::InvalidJson {
            path: staged_path.clone(),
            source,
        })?;
    if let Err(error) = validate_manifest(&staged_manifest) {
        let _ = fs::remove_file(&staged_path);
        return Err(error);
    }

    fs::rename(&staged_path, &path).map_err(|source| {
        let _ = fs::remove_file(&staged_path);
        if source.kind() == io::ErrorKind::AlreadyExists {
            ManifestError::AlreadyExists(path.clone())
        } else {
            io_error(&path, source)
        }
    })?;
    Ok(path)
}

pub fn read_manifest(
    repository: &Path,
    snapshot_id: &str,
) -> Result<SnapshotManifest, ManifestError> {
    validate_snapshot_id(snapshot_id)?;
    let path = manifest_path(repository, snapshot_id)?;
    let json = fs::read_to_string(&path).map_err(|source| io_error(&path, source))?;
    let manifest: SnapshotManifest = serde_json::from_str(&json)
        .map_err(|source| ManifestError::InvalidJson { path, source })?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

pub fn list_manifests(repository: &Path) -> Result<Vec<SnapshotManifest>, ManifestError> {
    let snapshots = repository.join("snapshots");
    let entries = fs::read_dir(&snapshots).map_err(|source| io_error(&snapshots, source))?;
    let mut manifests = Vec::new();

    for entry in entries {
        let entry = entry.map_err(|source| io_error(&snapshots, source))?;
        let path = entry.path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("json") {
            continue;
        }

        let json = fs::read_to_string(&path).map_err(|source| io_error(&path, source))?;
        let manifest: SnapshotManifest =
            serde_json::from_str(&json).map_err(|source| ManifestError::InvalidJson {
                path: path.clone(),
                source,
            })?;
        validate_manifest(&manifest)?;
        manifests.push(manifest);
    }

    manifests.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.snapshot_id.cmp(&right.snapshot_id))
    });
    Ok(manifests)
}

fn manifest_path(repository: &Path, snapshot_id: &str) -> Result<PathBuf, ManifestError> {
    validate_snapshot_id(snapshot_id)?;
    Ok(repository
        .join("snapshots")
        .join(format!("{snapshot_id}.json")))
}

fn validate_file_entry(file: &FileEntry) -> Result<(), ManifestError> {
    validate_portable_path(&file.path)?;
    if let Some(modified_at) = &file.modified_at {
        parse_timestamp(modified_at)?;
    }

    match file.file_type {
        FileType::Directory => {
            if file.size != 0 || file.content_hash.is_some() || !file.chunks.is_empty() {
                return invalid_entry(file, "directories cannot have content metadata");
            }
            if file.symlink_target.is_some() {
                return invalid_entry(file, "directories cannot have symlink targets");
            }
        }
        FileType::File => {
            let Some(content_hash) = &file.content_hash else {
                return invalid_entry(file, "files require a content hash");
            };
            validate_chunk_hash(content_hash)?;
            if file.chunks.is_empty() && file.size > 0 {
                return invalid_entry(file, "non-empty files require chunk references");
            }
            for hash in &file.chunks {
                validate_chunk_hash(hash)?;
            }
            if file.symlink_target.is_some() {
                return invalid_entry(file, "files cannot have symlink targets");
            }
        }
        FileType::Symlink => {
            if file.size != 0 || file.content_hash.is_some() || !file.chunks.is_empty() {
                return invalid_entry(file, "symlinks cannot have content metadata");
            }
            let Some(target) = &file.symlink_target else {
                return invalid_entry(file, "symlinks require a target");
            };
            if target.is_empty() || target.contains('\0') {
                return invalid_entry(file, "symlink target is invalid");
            }
        }
    }

    Ok(())
}

fn invalid_entry(file: &FileEntry, reason: &str) -> Result<(), ManifestError> {
    Err(ManifestError::InvalidFileEntry {
        path: file.path.clone(),
        reason: reason.to_owned(),
    })
}

fn validate_snapshot_id(snapshot_id: &str) -> Result<(), ManifestError> {
    if !snapshot_id.starts_with("snap_")
        || snapshot_id.len() <= "snap_".len()
        || !snapshot_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(ManifestError::InvalidSnapshotId(snapshot_id.to_owned()));
    }

    Ok(())
}

fn validate_portable_path(path: &str) -> Result<(), ManifestError> {
    if path.is_empty()
        || path.starts_with('/')
        || path.starts_with("\\\\")
        || path.contains('\\')
        || path.contains(':')
        || path.contains('\0')
    {
        return Err(ManifestError::InvalidPath(path.to_owned()));
    }

    for segment in path.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(ManifestError::InvalidPath(path.to_owned()));
        }
    }

    Ok(())
}

fn validate_chunk_hash(hash: &str) -> Result<(), ManifestError> {
    if hash.len() != 64
        || !hash
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ManifestError::InvalidChunkHash(hash.to_owned()));
    }

    Ok(())
}

fn parse_timestamp(timestamp: &str) -> Result<OffsetDateTime, ManifestError> {
    OffsetDateTime::parse(timestamp, &Rfc3339)
        .map_err(|_| ManifestError::InvalidTimestamp(timestamp.to_owned()))
}

fn io_error(path: &Path, source: io::Error) -> ManifestError {
    ManifestError::Io {
        path: path.to_owned(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use crate::{StoreChunkOutcome, init_repository, store_chunk};

    use super::{
        FileEntry, FileType, ManifestError, ManifestSummary, SnapshotManifest, read_manifest,
        validate_manifest, verify_manifest_chunks, write_manifest,
    };

    #[test]
    fn validates_complete_manifest() {
        validate_manifest(&manifest()).expect("manifest should be valid");
    }

    #[test]
    fn writes_and_reads_manifest_json() {
        let repository = tempdir().expect("temporary repository should be created");
        init_repository(repository.path()).expect("repository should initialize");
        let manifest = manifest();

        let path =
            write_manifest(repository.path(), &manifest).expect("manifest should be written");
        let read = read_manifest(repository.path(), &manifest.snapshot_id)
            .expect("manifest should be readable");

        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some("snap_001.json")
        );
        assert_eq!(read, manifest);
    }

    #[test]
    fn reads_older_manifest_without_permissions_field() {
        let json = serde_json::to_string(&manifest()).expect("manifest should serialize");
        let value: serde_json::Value =
            serde_json::from_str(&json).expect("manifest JSON should parse");
        let mut value = value;
        for file in value["files"]
            .as_array_mut()
            .expect("files should be an array")
        {
            file.as_object_mut()
                .expect("file should be an object")
                .remove("permissions");
        }

        let parsed: SnapshotManifest =
            serde_json::from_value(value).expect("older manifest should deserialize");

        assert!(parsed.files.iter().all(|file| file.permissions.is_none()));
    }

    #[test]
    fn write_manifest_refuses_to_overwrite_published_snapshot() {
        let repository = tempdir().expect("temporary repository should be created");
        init_repository(repository.path()).expect("repository should initialize");
        let manifest = manifest();
        write_manifest(repository.path(), &manifest).expect("first manifest should be written");

        let error = write_manifest(repository.path(), &manifest)
            .expect_err("second manifest write should fail");

        assert!(matches!(error, ManifestError::AlreadyExists(_)));
    }

    #[test]
    fn write_manifest_removes_staged_manifest_after_publish() {
        let repository = tempdir().expect("temporary repository should be created");
        init_repository(repository.path()).expect("repository should initialize");

        write_manifest(repository.path(), &manifest()).expect("manifest should be written");

        let staging_entries = std::fs::read_dir(repository.path().join("staging"))
            .expect("staging should be readable")
            .collect::<Result<Vec<_>, _>>()
            .expect("staging entries should be readable");
        assert!(staging_entries.is_empty());
    }

    #[test]
    fn rejects_unsupported_version() {
        let mut manifest = manifest();
        manifest.manifest_version = 1;

        let error = validate_manifest(&manifest).expect_err("version should be rejected");

        assert!(matches!(error, ManifestError::UnsupportedVersion(1)));
    }

    #[test]
    fn rejects_incomplete_state() {
        let mut manifest = manifest();
        manifest.state = "staged".to_owned();

        let error = validate_manifest(&manifest).expect_err("state should be rejected");

        assert!(matches!(error, ManifestError::UnsupportedState(_)));
    }

    #[test]
    fn rejects_missing_sources() {
        let mut manifest = manifest();
        manifest.sources.clear();

        let error = validate_manifest(&manifest).expect_err("sources should be required");

        assert!(matches!(error, ManifestError::MissingSources));
    }

    #[test]
    fn rejects_invalid_timestamp() {
        let mut manifest = manifest();
        manifest.created_at = "not-a-time".to_owned();

        let error = validate_manifest(&manifest).expect_err("timestamp should be rejected");

        assert!(matches!(error, ManifestError::InvalidTimestamp(_)));
    }

    #[test]
    fn rejects_absolute_and_parent_paths() {
        for path in ["/absolute", "a/../b", "C:/drive", "a\\b", "a//b"] {
            let mut manifest = manifest();
            manifest.files[0].path = path.to_owned();

            let error = validate_manifest(&manifest).expect_err("path should be rejected");

            assert!(matches!(error, ManifestError::InvalidPath(_)));
        }
    }

    #[test]
    fn rejects_bad_chunk_hash() {
        let mut manifest = manifest();
        manifest.files[0].chunks = vec!["not-a-hash".to_owned()];

        let error = validate_manifest(&manifest).expect_err("hash should be rejected");

        assert!(matches!(error, ManifestError::InvalidChunkHash(_)));
    }

    #[test]
    fn rejects_summary_mismatch() {
        let mut manifest = manifest();
        manifest.summary.logical_bytes = 999;

        let error = validate_manifest(&manifest).expect_err("summary should be rejected");

        assert!(matches!(error, ManifestError::InvalidSummary(_)));
    }

    #[test]
    fn rejects_directory_with_content_metadata() {
        let mut manifest = manifest();
        manifest.files[0].file_type = FileType::Directory;

        let error = validate_manifest(&manifest).expect_err("directory should be rejected");

        assert!(matches!(error, ManifestError::InvalidFileEntry { .. }));
    }

    #[test]
    fn verifies_existing_chunk_references() {
        let repository = tempdir().expect("temporary repository should be created");
        init_repository(repository.path()).expect("repository should initialize");
        let StoreChunkOutcome::Stored(chunk) =
            store_chunk(repository.path(), b"hello").expect("chunk should be stored")
        else {
            panic!("first chunk write should store");
        };
        let mut manifest = manifest();
        manifest.files[0].size = 5;
        manifest.files[0].content_hash = Some(chunk.hash.clone());
        manifest.files[0].chunks = vec![chunk.hash.clone()];
        manifest.summary.logical_bytes = 5;
        manifest.summary.newly_stored_bytes = chunk.stored_size;

        verify_manifest_chunks(repository.path(), &manifest)
            .expect("chunk references should verify");
    }

    #[test]
    fn rejects_missing_chunk_references() {
        let repository = tempdir().expect("temporary repository should be created");
        init_repository(repository.path()).expect("repository should initialize");

        let error = verify_manifest_chunks(repository.path(), &manifest())
            .expect_err("missing chunk should be rejected");

        assert!(matches!(error, ManifestError::Chunk { .. }));
    }

    #[test]
    fn lists_published_manifests_in_stable_order() {
        let repository = tempdir().expect("temporary repository should be created");
        init_repository(repository.path()).expect("repository should initialize");
        let mut second = manifest();
        second.snapshot_id = "snap_b".to_owned();
        second.created_at = "2026-06-02T00:00:01Z".to_owned();
        let mut first = manifest();
        first.snapshot_id = "snap_a".to_owned();
        first.created_at = "2026-06-02T00:00:00Z".to_owned();

        write_manifest(repository.path(), &second).expect("second manifest should be written");
        write_manifest(repository.path(), &first).expect("first manifest should be written");

        let listed = super::list_manifests(repository.path()).expect("manifests should list");

        assert_eq!(
            listed
                .iter()
                .map(|manifest| manifest.snapshot_id.as_str())
                .collect::<Vec<_>>(),
            ["snap_a", "snap_b"]
        );
    }

    fn manifest() -> SnapshotManifest {
        let hash = "a".repeat(64);

        SnapshotManifest {
            manifest_version: 0,
            snapshot_id: "snap_001".to_owned(),
            state: "complete".to_owned(),
            created_at: "2026-06-02T00:00:00Z".to_owned(),
            sources: vec!["source".to_owned()],
            files: vec![
                FileEntry {
                    path: "source/file.txt".to_owned(),
                    file_type: FileType::File,
                    size: 123,
                    modified_at: Some("2026-06-02T00:00:00Z".to_owned()),
                    permissions: None,
                    content_hash: Some(hash.clone()),
                    chunks: vec![hash],
                    symlink_target: None,
                },
                FileEntry {
                    path: "source/dir".to_owned(),
                    file_type: FileType::Directory,
                    size: 0,
                    modified_at: None,
                    permissions: None,
                    content_hash: None,
                    chunks: Vec::new(),
                    symlink_target: None,
                },
                FileEntry {
                    path: "source/link".to_owned(),
                    file_type: FileType::Symlink,
                    size: 0,
                    modified_at: None,
                    permissions: None,
                    content_hash: None,
                    chunks: Vec::new(),
                    symlink_target: Some("file.txt".to_owned()),
                },
            ],
            summary: ManifestSummary {
                file_count: 1,
                logical_bytes: 123,
                newly_stored_bytes: 50,
            },
        }
    }
}
