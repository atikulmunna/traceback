use std::{io, path::Path};

use serde::Serialize;
use thiserror::Error;
use time::{Duration, OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{
    CheckIssue, HistoryError, ManifestError, OperationKind, check_repository, list_manifests,
    read_operation_history,
};

const SCORING_VERSION: &str = "reliability-v1";

#[derive(Debug, Error)]
pub enum DoctorError {
    #[error("manifest error: {0}")]
    Manifest(#[from] ManifestError),
    #[error("history error: {0}")]
    History(#[from] HistoryError),
    #[error("filesystem error at {path}: {source}")]
    Io { path: String, source: io::Error },
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FindingLevel {
    Good,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DoctorFinding {
    pub code: String,
    pub level: FindingLevel,
    pub message: String,
    pub recommendation: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DoctorReport {
    pub latest_snapshot_id: Option<String>,
    pub latest_snapshot_age_seconds: Option<i64>,
    pub integrity_passed: bool,
    pub latest_check_passed: Option<bool>,
    pub latest_rehearsal_passed: Option<bool>,
    pub health_score: u8,
    pub scoring_version: String,
    pub score_categories: Vec<ScoreCategory>,
    pub findings: Vec<DoctorFinding>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScoreStatus {
    Passed,
    Failed,
    NotEvaluated,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ScoreCategory {
    pub code: String,
    pub points: u8,
    pub status: ScoreStatus,
}

pub fn doctor_repository(repository: &Path) -> Result<DoctorReport, DoctorError> {
    let manifests = list_manifests(repository)?;
    let latest = manifests.last();
    let history = read_operation_history(repository)?;
    let check = check_repository(repository);
    let latest_check_passed = history
        .iter()
        .rev()
        .find(|record| record.operation == OperationKind::Check)
        .map(|record| record.success);
    let latest_rehearsal_passed = history
        .iter()
        .rev()
        .find(|record| record.operation == OperationKind::Rehearse)
        .map(|record| record.success);
    let latest_snapshot_age_seconds = latest
        .map(|manifest| {
            OffsetDateTime::parse(&manifest.created_at, &Rfc3339)
                .map(|created| (OffsetDateTime::now_utc() - created).whole_seconds().max(0))
        })
        .transpose()
        .map_err(|_| {
            ManifestError::InvalidTimestamp(
                latest.map_or_else(String::new, |manifest| manifest.created_at.clone()),
            )
        })?;
    let score_categories = score_categories(
        latest_snapshot_age_seconds,
        check.passed(),
        latest_rehearsal_passed,
        !check
            .issues
            .iter()
            .any(|issue| matches!(issue, CheckIssue::ReferencedChunk { .. })),
    );
    let health_score = calculate_score(&score_categories);

    let mut findings = Vec::new();
    findings.push(backup_age_finding(latest_snapshot_age_seconds));
    findings.push(if check.passed() {
        good(
            "integrity_current",
            "Current repository integrity check passed.",
        )
    } else {
        critical(
            "integrity_current",
            format!(
                "Current repository check found {} issue(s).",
                check.issues.len()
            ),
            "Run `traceback check --repo <repo>` and resolve every reported issue.",
        )
    });
    findings.push(history_finding(
        "check_history",
        "repository check",
        latest_check_passed,
        "Run `traceback check --repo <repo>` to establish recorded integrity evidence.",
    ));
    findings.push(history_finding(
        "rehearsal_history",
        "restore rehearsal",
        latest_rehearsal_passed,
        "Run `traceback rehearse <latest-snapshot> --repo <repo>`.",
    ));

    let lock = repository.join("locks").join("writer.lock");
    findings.push(if lock.exists() {
        warning(
            "writer_lock",
            "A repository writer lock is present.",
            "Confirm no backup is running; use `traceback recover --repo <repo>` only for an abandoned lock.",
        )
    } else {
        good("writer_lock", "No writer lock is present.")
    });
    findings.push(
        if check.abandoned_staging_entries == 0 && check.temporary_chunk_files == 0 {
            good("staging", "No interrupted-write artifacts were found.")
        } else {
            warning(
                "staging",
                format!(
                    "{} staging and {} temporary chunk artifact(s) found.",
                    check.abandoned_staging_entries, check.temporary_chunk_files
                ),
                "Run `traceback recover --repo <repo>` after confirming no backup is active.",
            )
        },
    );

    Ok(DoctorReport {
        latest_snapshot_id: latest.map(|manifest| manifest.snapshot_id.clone()),
        latest_snapshot_age_seconds,
        integrity_passed: check.passed(),
        latest_check_passed,
        latest_rehearsal_passed,
        health_score,
        scoring_version: SCORING_VERSION.to_owned(),
        score_categories,
        findings,
    })
}

fn score_categories(
    backup_age_seconds: Option<i64>,
    integrity_passed: bool,
    rehearsal_passed: Option<bool>,
    chunks_intact: bool,
) -> Vec<ScoreCategory> {
    vec![
        score_category(
            "backup_recent",
            20,
            passed_status(backup_age_seconds.is_some_and(|seconds| seconds <= 86_400)),
        ),
        score_category("repository_check", 25, passed_status(integrity_passed)),
        score_category(
            "restore_rehearsal",
            20,
            passed_status(rehearsal_passed.unwrap_or(false)),
        ),
        score_category("chunk_integrity", 15, passed_status(chunks_intact)),
        score_category("ignore_quality", 5, ScoreStatus::NotEvaluated),
        score_category("retention_policy", 5, ScoreStatus::NotEvaluated),
        score_category("remote_copy", 5, ScoreStatus::NotEvaluated),
        score_category("encryption", 5, ScoreStatus::NotEvaluated),
    ]
}

fn score_category(code: &str, points: u8, status: ScoreStatus) -> ScoreCategory {
    ScoreCategory {
        code: code.to_owned(),
        points,
        status,
    }
}

fn passed_status(passed: bool) -> ScoreStatus {
    if passed {
        ScoreStatus::Passed
    } else {
        ScoreStatus::Failed
    }
}

fn calculate_score(categories: &[ScoreCategory]) -> u8 {
    let possible: u16 = categories
        .iter()
        .filter(|category| category.status != ScoreStatus::NotEvaluated)
        .map(|category| u16::from(category.points))
        .sum();
    if possible == 0 {
        return 100;
    }
    let earned: u16 = categories
        .iter()
        .filter(|category| category.status == ScoreStatus::Passed)
        .map(|category| u16::from(category.points))
        .sum();
    u8::try_from((earned * 100) / possible).expect("health score is at most 100")
}

fn backup_age_finding(age: Option<i64>) -> DoctorFinding {
    match age {
        None => critical(
            "backup_age",
            "No published backup snapshot exists.",
            "Run `traceback backup <path> --repo <repo>`.",
        ),
        Some(seconds) if seconds > Duration::days(7).whole_seconds() => critical(
            "backup_age",
            format!("Latest backup is {} day(s) old.", seconds / 86_400),
            "Run a backup now and review the backup schedule.",
        ),
        Some(seconds) if seconds > Duration::days(1).whole_seconds() => warning(
            "backup_age",
            format!("Latest backup is {} hour(s) old.", seconds / 3_600),
            "Run `traceback backup <path> --repo <repo>` soon.",
        ),
        Some(seconds) => good(
            "backup_age",
            format!("Latest backup is {} hour(s) old.", seconds / 3_600),
        ),
    }
}

fn history_finding(
    code: &str,
    label: &str,
    passed: Option<bool>,
    recommendation: &str,
) -> DoctorFinding {
    match passed {
        Some(true) => good(code, format!("Latest recorded {label} passed.")),
        Some(false) => critical(
            code,
            format!("Latest recorded {label} failed."),
            recommendation,
        ),
        None => warning(
            code,
            format!("No recorded {label} is available."),
            recommendation,
        ),
    }
}

fn good(code: &str, message: impl Into<String>) -> DoctorFinding {
    DoctorFinding {
        code: code.to_owned(),
        level: FindingLevel::Good,
        message: message.into(),
        recommendation: None,
    }
}

fn warning(
    code: &str,
    message: impl Into<String>,
    recommendation: impl Into<String>,
) -> DoctorFinding {
    DoctorFinding {
        code: code.to_owned(),
        level: FindingLevel::Warning,
        message: message.into(),
        recommendation: Some(recommendation.into()),
    }
}

fn critical(
    code: &str,
    message: impl Into<String>,
    recommendation: impl Into<String>,
) -> DoctorFinding {
    DoctorFinding {
        code: code.to_owned(),
        level: FindingLevel::Critical,
        message: message.into(),
        recommendation: Some(recommendation.into()),
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use crate::{OperationKind, append_operation, init_repository};

    use super::{FindingLevel, ScoreStatus, calculate_score, doctor_repository, score_category};

    #[test]
    fn reports_actionable_findings_for_empty_repository() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        init_repository(&repository).expect("repository should initialize");
        append_operation(
            &repository,
            OperationKind::Check,
            None,
            true,
            "check passed",
        )
        .expect("history should append");

        let report = doctor_repository(&repository).expect("doctor should run");

        assert!(report.latest_snapshot_id.is_none());
        let backup = report
            .findings
            .iter()
            .find(|finding| finding.code == "backup_age")
            .expect("backup finding should exist");
        assert_eq!(backup.level, FindingLevel::Critical);
        assert!(backup.recommendation.is_some());
        let rehearsal = report
            .findings
            .iter()
            .find(|finding| finding.code == "rehearsal_history")
            .expect("rehearsal finding should exist");
        assert_eq!(rehearsal.level, FindingLevel::Warning);
        assert_eq!(report.scoring_version, "reliability-v1");
        assert!(
            report
                .score_categories
                .iter()
                .filter(|category| category.status == ScoreStatus::NotEvaluated)
                .all(|category| matches!(
                    category.code.as_str(),
                    "ignore_quality" | "retention_policy" | "remote_copy" | "encryption"
                ))
        );
    }

    #[test]
    fn score_omits_unavailable_categories_from_denominator() {
        let categories = vec![
            score_category("available", 20, ScoreStatus::Passed),
            score_category("failed", 20, ScoreStatus::Failed),
            score_category("unavailable", 60, ScoreStatus::NotEvaluated),
        ];

        assert_eq!(calculate_score(&categories), 50);
    }
}
