use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

mod chunk;
mod manifest;

pub use chunk::{ChunkError, ChunkMetadata, StoreChunkOutcome, read_chunk, store_chunk};
pub use manifest::{
    FileEntry, FileType, ManifestError, ManifestSummary, SnapshotManifest, read_manifest,
    validate_manifest, verify_manifest_chunks, write_manifest,
};

const CONFIG_FILE: &str = "config.toml";
const FORMAT_VERSION: u32 = 0;
const CHUNK_SIZE_BYTES: u64 = 4 * 1024 * 1024;
const REPOSITORY_DIRECTORIES: [&str; 6] =
    ["chunks", "snapshots", "indexes", "staging", "locks", "logs"];

#[derive(Debug, Error)]
pub enum RepositoryError {
    #[error("repository path is not a directory: {0}")]
    NotDirectory(PathBuf),
    #[error("directory is not empty and is not a TraceBack repository: {0}")]
    IncompatibleDirectory(PathBuf),
    #[error("repository config is invalid at {path}: {source}")]
    InvalidConfig {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("repository config has unsupported value: {0}")]
    UnsupportedConfig(String),
    #[error("repository is missing required directory: {0}")]
    MissingDirectory(PathBuf),
    #[error("failed to serialize repository config: {0}")]
    SerializeConfig(#[from] toml::ser::Error),
    #[error("failed to format repository creation timestamp: {0}")]
    FormatTimestamp(#[from] time::error::Format),
    #[error("filesystem error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InitOutcome {
    Created(RepositoryConfig),
    AlreadyInitialized(RepositoryConfig),
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RepositoryConfig {
    pub repository_id: String,
    pub format_version: u32,
    pub created_at: String,
    pub hash_algorithm: String,
    pub chunking: String,
    pub chunk_size_bytes: u64,
    pub compression: String,
    pub compression_level: u8,
    pub encrypted: bool,
}

impl RepositoryConfig {
    fn new() -> Result<Self, RepositoryError> {
        Ok(Self {
            repository_id: format!("repo_{}", Uuid::new_v4().simple()),
            format_version: FORMAT_VERSION,
            created_at: OffsetDateTime::now_utc().format(&Rfc3339)?,
            hash_algorithm: "blake3".to_owned(),
            chunking: "fixed".to_owned(),
            chunk_size_bytes: CHUNK_SIZE_BYTES,
            compression: "zstd".to_owned(),
            compression_level: 3,
            encrypted: false,
        })
    }
}

pub fn init_repository(path: &Path) -> Result<InitOutcome, RepositoryError> {
    if path.exists() {
        if !path.is_dir() {
            return Err(RepositoryError::NotDirectory(path.to_owned()));
        }

        let config_path = path.join(CONFIG_FILE);
        if config_path.exists() {
            return Ok(InitOutcome::AlreadyInitialized(validate_repository(path)?));
        }

        if directory_has_entries(path)? {
            return Err(RepositoryError::IncompatibleDirectory(path.to_owned()));
        }
    } else {
        fs::create_dir_all(path).map_err(|source| io_error(path, source))?;
    }

    for directory in REPOSITORY_DIRECTORIES {
        let directory = path.join(directory);
        fs::create_dir(&directory).map_err(|source| io_error(&directory, source))?;
    }

    let config = RepositoryConfig::new()?;
    let config_contents = toml::to_string_pretty(&config)?;
    let config_path = path.join(CONFIG_FILE);
    fs::write(&config_path, config_contents).map_err(|source| io_error(&config_path, source))?;

    Ok(InitOutcome::Created(config))
}

pub fn read_config(path: &Path) -> Result<RepositoryConfig, RepositoryError> {
    let config_path = path.join(CONFIG_FILE);
    let contents =
        fs::read_to_string(&config_path).map_err(|source| io_error(&config_path, source))?;

    let config: RepositoryConfig =
        toml::from_str(&contents).map_err(|source| RepositoryError::InvalidConfig {
            path: config_path,
            source,
        })?;
    validate_config(&config)?;
    Ok(config)
}

pub fn validate_repository(path: &Path) -> Result<RepositoryConfig, RepositoryError> {
    let config = read_config(path)?;

    for directory in REPOSITORY_DIRECTORIES {
        let directory = path.join(directory);
        if !directory.is_dir() {
            return Err(RepositoryError::MissingDirectory(directory));
        }
    }

    Ok(config)
}

fn validate_config(config: &RepositoryConfig) -> Result<(), RepositoryError> {
    if config.format_version != FORMAT_VERSION {
        return Err(RepositoryError::UnsupportedConfig(format!(
            "format_version must be {FORMAT_VERSION}"
        )));
    }
    if config.hash_algorithm != "blake3" {
        return Err(RepositoryError::UnsupportedConfig(
            "hash_algorithm must be blake3".to_owned(),
        ));
    }
    if config.chunking != "fixed" || config.chunk_size_bytes == 0 {
        return Err(RepositoryError::UnsupportedConfig(
            "chunking must be fixed with a non-zero chunk_size_bytes".to_owned(),
        ));
    }
    if config.compression != "zstd" {
        return Err(RepositoryError::UnsupportedConfig(
            "compression must be zstd".to_owned(),
        ));
    }
    if config.encrypted {
        return Err(RepositoryError::UnsupportedConfig(
            "encrypted repositories are not supported yet".to_owned(),
        ));
    }

    Ok(())
}

fn directory_has_entries(path: &Path) -> Result<bool, RepositoryError> {
    let mut entries = fs::read_dir(path).map_err(|source| io_error(path, source))?;
    Ok(entries
        .next()
        .transpose()
        .map_err(|source| io_error(path, source))?
        .is_some())
}

fn io_error(path: &Path, source: io::Error) -> RepositoryError {
    RepositoryError::Io {
        path: path.to_owned(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{
        CONFIG_FILE, InitOutcome, REPOSITORY_DIRECTORIES, RepositoryError, init_repository,
        read_config,
    };

    #[test]
    fn initializes_repository_layout_and_config() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");

        let outcome = init_repository(&repository).expect("repository should initialize");
        let InitOutcome::Created(created_config) = outcome else {
            panic!("new repository should be created");
        };

        for directory in REPOSITORY_DIRECTORIES {
            assert!(repository.join(directory).is_dir());
        }

        assert!(repository.join(CONFIG_FILE).is_file());
        assert_eq!(
            read_config(&repository).expect("config should be readable"),
            created_config
        );
        assert_eq!(created_config.format_version, 0);
        assert_eq!(created_config.chunk_size_bytes, 4 * 1024 * 1024);
        assert_eq!(created_config.compression, "zstd");
    }

    #[test]
    fn initializing_valid_repository_again_is_safe() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        let first = init_repository(&repository).expect("repository should initialize");
        let second = init_repository(&repository).expect("repeat initialization should succeed");

        let InitOutcome::Created(first_config) = first else {
            panic!("first initialization should create repository");
        };
        assert_eq!(second, InitOutcome::AlreadyInitialized(first_config));
    }

    #[test]
    fn rejects_non_empty_incompatible_directory() {
        let temporary = tempdir().expect("temporary directory should be created");
        fs::write(temporary.path().join("existing.txt"), "data")
            .expect("test file should be written");

        let error = init_repository(temporary.path()).expect_err("directory should be rejected");

        assert!(matches!(error, RepositoryError::IncompatibleDirectory(_)));
    }

    #[test]
    fn rejects_file_as_repository_path() {
        let temporary = tempdir().expect("temporary directory should be created");
        let file_path = temporary.path().join("file.txt");
        fs::write(&file_path, "data").expect("test file should be written");

        let error = init_repository(&file_path).expect_err("file path should be rejected");

        assert!(matches!(error, RepositoryError::NotDirectory(_)));
    }

    #[test]
    fn rejects_invalid_existing_config() {
        let temporary = tempdir().expect("temporary directory should be created");
        fs::write(temporary.path().join(CONFIG_FILE), "not valid toml =")
            .expect("invalid config should be written");

        let error = init_repository(temporary.path()).expect_err("config should be rejected");

        assert!(matches!(error, RepositoryError::InvalidConfig { .. }));
    }

    #[test]
    fn rejects_repository_missing_required_directory() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        init_repository(&repository).expect("repository should initialize");
        fs::remove_dir(repository.join("logs")).expect("logs directory should be removed");

        let error = init_repository(&repository).expect_err("repository should be rejected");

        assert!(matches!(error, RepositoryError::MissingDirectory(_)));
    }

    #[test]
    fn rejects_unsupported_format_version() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        init_repository(&repository).expect("repository should initialize");
        let config_path = repository.join(CONFIG_FILE);
        let config = fs::read_to_string(&config_path).expect("config should be readable");
        fs::write(
            &config_path,
            config.replace("format_version = 0", "format_version = 99"),
        )
        .expect("config should be replaced");

        let error = init_repository(&repository).expect_err("repository should be rejected");

        assert!(matches!(error, RepositoryError::UnsupportedConfig(_)));
    }
}
