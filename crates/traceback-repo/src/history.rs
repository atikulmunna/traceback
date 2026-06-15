use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

const HISTORY_VERSION: u32 = 1;
const HISTORY_FILE: &str = "operations-v1.jsonl";

#[derive(Debug, Error)]
pub enum HistoryError {
    #[error("history JSON is invalid at {path}: {source}")]
    InvalidJson {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("history uses unsupported version {0}")]
    UnsupportedVersion(u32),
    #[error("failed to format history timestamp: {0}")]
    FormatTimestamp(#[from] time::error::Format),
    #[error("filesystem error at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Check,
    Rehearse,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct OperationRecord {
    pub version: u32,
    pub timestamp: String,
    pub operation: OperationKind,
    pub snapshot_id: Option<String>,
    pub success: bool,
    pub detail: String,
}

pub fn append_operation(
    repository: &Path,
    operation: OperationKind,
    snapshot_id: Option<&str>,
    success: bool,
    detail: impl Into<String>,
) -> Result<OperationRecord, HistoryError> {
    let record = OperationRecord {
        version: HISTORY_VERSION,
        timestamp: OffsetDateTime::now_utc().format(&Rfc3339)?,
        operation,
        snapshot_id: snapshot_id.map(str::to_owned),
        success,
        detail: detail.into(),
    };
    let path = history_path(repository);
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|source| io_error(&path, source))?;
    serde_json::to_writer(&mut file, &record).map_err(|source| HistoryError::InvalidJson {
        path: path.clone(),
        source,
    })?;
    file.write_all(b"\n")
        .and_then(|()| file.sync_data())
        .map_err(|source| io_error(&path, source))?;
    Ok(record)
}

pub fn read_operation_history(repository: &Path) -> Result<Vec<OperationRecord>, HistoryError> {
    let path = history_path(repository);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let contents = fs::read_to_string(&path).map_err(|source| io_error(&path, source))?;
    let mut records = Vec::new();
    for line in contents.lines().filter(|line| !line.trim().is_empty()) {
        let record: OperationRecord =
            serde_json::from_str(line).map_err(|source| HistoryError::InvalidJson {
                path: path.clone(),
                source,
            })?;
        if record.version != HISTORY_VERSION {
            return Err(HistoryError::UnsupportedVersion(record.version));
        }
        records.push(record);
    }
    Ok(records)
}

fn history_path(repository: &Path) -> PathBuf {
    repository.join("logs").join(HISTORY_FILE)
}

fn io_error(path: &Path, source: io::Error) -> HistoryError {
    HistoryError::Io {
        path: path.to_owned(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use crate::init_repository;

    use super::{OperationKind, append_operation, read_operation_history};

    #[test]
    fn appends_and_reads_versioned_operation_records() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        init_repository(&repository).expect("repository should initialize");

        append_operation(
            &repository,
            OperationKind::Check,
            None,
            true,
            "integrity check passed",
        )
        .expect("history should append");
        append_operation(
            &repository,
            OperationKind::Rehearse,
            Some("snap_001"),
            false,
            "missing chunk",
        )
        .expect("history should append");

        let records = read_operation_history(&repository).expect("history should read");

        assert_eq!(records.len(), 2);
        assert_eq!(records[0].version, 1);
        assert_eq!(records[0].operation, OperationKind::Check);
        assert!(records[0].success);
        assert_eq!(records[1].snapshot_id.as_deref(), Some("snap_001"));
        assert!(!records[1].success);
    }
}
