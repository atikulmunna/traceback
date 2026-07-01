use std::{
    collections::BTreeSet,
    error::Error,
    io::{self, Stdout},
    path::{Path, PathBuf},
    time::Duration,
};

use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use traceback_repo::{
    CheckReport, DoctorReport, FileType, FindingLevel, InitOutcome, RepositoryConfig,
    RepositoryError, SnapshotManifest, check_repository, diff_snapshots, doctor_repository,
    init_repository, list_manifests, rehearse_restore, restore_snapshot, restore_snapshot_path,
    validate_repository,
};

use crate::{BackupRequest, run_backup};

const TEAL: Color = Color::Rgb(45, 212, 191);
const SOFT_TEAL: Color = Color::Rgb(94, 234, 212);

#[derive(Debug)]
pub struct TuiApp {
    repository: PathBuf,
    repository_id: String,
    snapshots: Vec<SnapshotRow>,
    view: TuiView,
    selected_menu_item: usize,
    status_message: Option<String>,
    path_input: Option<PathInput>,
    backup_source: Option<PathBuf>,
    backup_result: Option<BackupRunSummary>,
    restore_target: Option<PathBuf>,
    restore_result: Option<RestoreRunSummary>,
    rehearsal_result: Option<RestoreRunSummary>,
    health_report: Option<HealthCheckSummary>,
    doctor_report: Option<DoctorRunSummary>,
    diff_old_snapshot: usize,
    diff_new_snapshot: usize,
    diff_focus_old: bool,
    diff_result: Option<DiffRunSummary>,
    selected_snapshot: usize,
    selected_file: usize,
    focus: TuiFocus,
    file_filter: String,
    filtering_files: bool,
    restore_plan: Option<RestorePlan>,
    restore_confirmation: RestoreConfirmation,
    show_help: bool,
    should_quit: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TuiView {
    MainMenu,
    Browser,
    PathInput,
    BackupReview,
    RestoreRehearsal,
    HealthCheck,
    DoctorReport,
    SnapshotDiff,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TuiFocus {
    Snapshots,
    Files,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RestoreConfirmation {
    None,
    Awaiting,
    Confirmed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MenuAction {
    BrowseSnapshots,
    ChangeRepository,
    InitializeRepository,
    CreateBackup,
    RestoreFiles,
    RehearseRestore,
    CheckHealth,
    DoctorReport,
    CompareSnapshots,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MenuItem {
    label: &'static str,
    description: &'static str,
    action: MenuAction,
    enabled: bool,
}

const MENU_ITEMS: [MenuItem; 10] = [
    MenuItem {
        label: "Browse snapshots",
        description: "Inspect snapshots, files, metadata, and restore previews.",
        action: MenuAction::BrowseSnapshots,
        enabled: true,
    },
    MenuItem {
        label: "Change repository",
        description: "Enter another repository path and reload snapshots.",
        action: MenuAction::ChangeRepository,
        enabled: true,
    },
    MenuItem {
        label: "Initialize repository",
        description: "Create or load a repository at the current path.",
        action: MenuAction::InitializeRepository,
        enabled: true,
    },
    MenuItem {
        label: "Create backup",
        description: "Select a source path for the guided backup flow.",
        action: MenuAction::CreateBackup,
        enabled: true,
    },
    MenuItem {
        label: "Restore files",
        description: "Open the browser and preview safe restore commands.",
        action: MenuAction::RestoreFiles,
        enabled: true,
    },
    MenuItem {
        label: "Rehearse restore",
        description: "Verify a snapshot restore without writing to a chosen target.",
        action: MenuAction::RehearseRestore,
        enabled: true,
    },
    MenuItem {
        label: "Check repository health",
        description: "Run integrity checks and review findings.",
        action: MenuAction::CheckHealth,
        enabled: true,
    },
    MenuItem {
        label: "Doctor report",
        description: "Review reliability score, evidence gaps, and recommendations.",
        action: MenuAction::DoctorReport,
        enabled: true,
    },
    MenuItem {
        label: "Compare snapshots",
        description: "Select two snapshots and review changed paths.",
        action: MenuAction::CompareSnapshots,
        enabled: true,
    },
    MenuItem {
        label: "Exit",
        description: "Close the terminal UI.",
        action: MenuAction::Quit,
        enabled: true,
    },
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PathInputKind {
    Repository,
    BackupSource,
    RestoreTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PathInput {
    kind: PathInputKind,
    value: String,
    error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct BackupRunSummary {
    snapshot_id: String,
    files_scanned: u64,
    logical_bytes: u64,
    newly_stored_bytes: u64,
    warning_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RestoreRunSummary {
    files: u64,
    directories: u64,
    symlinks: u64,
    bytes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct HealthCheckSummary {
    passed: bool,
    manifests_checked: usize,
    chunks_verified: usize,
    orphaned_chunks: usize,
    staging_leftovers: usize,
    temporary_chunk_files: usize,
    issues: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DoctorRunSummary {
    latest_snapshot_id: Option<String>,
    latest_snapshot_age_seconds: Option<i64>,
    integrity_passed: bool,
    latest_check_passed: Option<bool>,
    latest_rehearsal_passed: Option<bool>,
    health_score: u8,
    scoring_version: String,
    findings: Vec<DoctorFindingSummary>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DoctorFindingSummary {
    level: FindingLevel,
    code: String,
    message: String,
    recommendation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiffRunSummary {
    old_snapshot_id: String,
    new_snapshot_id: String,
    added: Vec<DiffEntrySummary>,
    removed: Vec<DiffEntrySummary>,
    modified: Vec<DiffEntrySummary>,
    unchanged: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiffEntrySummary {
    path: String,
    byte_delta: i128,
    type_changed: bool,
    content_changed: bool,
}

impl From<CheckReport> for HealthCheckSummary {
    fn from(report: CheckReport) -> Self {
        Self {
            passed: report.passed(),
            manifests_checked: report.manifests_checked,
            chunks_verified: report.chunks_verified,
            orphaned_chunks: report.orphaned_chunks,
            staging_leftovers: report.abandoned_staging_entries,
            temporary_chunk_files: report.temporary_chunk_files,
            issues: report
                .issues
                .into_iter()
                .map(|issue| issue.to_string())
                .collect(),
        }
    }
}

impl From<DoctorReport> for DoctorRunSummary {
    fn from(report: DoctorReport) -> Self {
        Self {
            latest_snapshot_id: report.latest_snapshot_id,
            latest_snapshot_age_seconds: report.latest_snapshot_age_seconds,
            integrity_passed: report.integrity_passed,
            latest_check_passed: report.latest_check_passed,
            latest_rehearsal_passed: report.latest_rehearsal_passed,
            health_score: report.health_score,
            scoring_version: report.scoring_version,
            findings: report
                .findings
                .into_iter()
                .map(|finding| DoctorFindingSummary {
                    level: finding.level,
                    code: finding.code,
                    message: finding.message,
                    recommendation: finding.recommendation,
                })
                .collect(),
        }
    }
}

impl From<traceback_repo::DiffEntry> for DiffEntrySummary {
    fn from(entry: traceback_repo::DiffEntry) -> Self {
        Self {
            path: entry.path,
            byte_delta: entry.byte_delta,
            type_changed: entry.type_changed,
            content_changed: entry.content_changed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SnapshotRow {
    snapshot_id: String,
    created_at: String,
    state: String,
    sources: String,
    source_count: usize,
    file_count: u64,
    directory_count: usize,
    symlink_count: usize,
    logical_bytes: u64,
    newly_stored_bytes: u64,
    chunk_references: usize,
    unique_chunks: usize,
    entries: Vec<FileRow>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileRow {
    path: String,
    file_type: FileType,
    size: u64,
    content_hash: Option<String>,
    chunks: Vec<String>,
    symlink_target: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RestorePlan {
    snapshot_id: String,
    selected_path: Option<String>,
    target: PathBuf,
    command: String,
}

impl TuiApp {
    pub fn new(
        repository: PathBuf,
        config: RepositoryConfig,
        manifests: Vec<SnapshotManifest>,
    ) -> Self {
        let snapshots = manifests.into_iter().map(SnapshotRow::from).collect();
        Self {
            repository,
            repository_id: config.repository_id,
            snapshots,
            view: TuiView::MainMenu,
            selected_menu_item: 0,
            status_message: None,
            path_input: None,
            backup_source: None,
            backup_result: None,
            restore_target: None,
            restore_result: None,
            rehearsal_result: None,
            health_report: None,
            doctor_report: None,
            diff_old_snapshot: 0,
            diff_new_snapshot: 1,
            diff_focus_old: true,
            diff_result: None,
            selected_snapshot: 0,
            selected_file: 0,
            focus: TuiFocus::Snapshots,
            file_filter: String::new(),
            filtering_files: false,
            restore_plan: None,
            restore_confirmation: RestoreConfirmation::None,
            show_help: true,
            should_quit: false,
        }
    }

    pub fn uninitialized(repository: PathBuf, message: String) -> Self {
        Self {
            repository,
            repository_id: "<not initialized>".to_owned(),
            snapshots: Vec::new(),
            view: TuiView::MainMenu,
            selected_menu_item: 0,
            status_message: Some(message),
            path_input: None,
            backup_source: None,
            backup_result: None,
            restore_target: None,
            restore_result: None,
            rehearsal_result: None,
            health_report: None,
            doctor_report: None,
            diff_old_snapshot: 0,
            diff_new_snapshot: 1,
            diff_focus_old: true,
            diff_result: None,
            selected_snapshot: 0,
            selected_file: 0,
            focus: TuiFocus::Snapshots,
            file_filter: String::new(),
            filtering_files: false,
            restore_plan: None,
            restore_confirmation: RestoreConfirmation::None,
            show_help: true,
            should_quit: false,
        }
    }

    pub(crate) fn set_backup_source(&mut self, source: PathBuf) {
        self.backup_source = Some(source.clone());
        self.backup_result = None;
        self.status_message = Some(format!("Backup source selected: {}", source.display()));
        self.view = TuiView::BackupReview;
    }

    fn handle_key(&mut self, code: KeyCode) {
        if self.view == TuiView::MainMenu {
            self.handle_main_menu_key(code);
            return;
        }

        if self.view == TuiView::PathInput {
            self.handle_path_input_key(code);
            return;
        }

        if self.view == TuiView::BackupReview {
            self.handle_backup_review_key(code);
            return;
        }

        if self.view == TuiView::RestoreRehearsal {
            self.handle_restore_rehearsal_key(code);
            return;
        }

        if self.view == TuiView::HealthCheck {
            self.handle_health_check_key(code);
            return;
        }

        if self.view == TuiView::DoctorReport {
            self.handle_doctor_report_key(code);
            return;
        }

        if self.view == TuiView::SnapshotDiff {
            self.handle_snapshot_diff_key(code);
            return;
        }

        if self.filtering_files {
            self.handle_filter_key(code);
            return;
        }

        if self.restore_confirmation == RestoreConfirmation::Awaiting {
            self.handle_restore_confirmation_key(code);
            return;
        }

        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Backspace => self.view = TuiView::MainMenu,
            KeyCode::Char('t') => self.start_path_input(PathInputKind::RestoreTarget),
            KeyCode::Char('?') | KeyCode::F(1) => self.show_help = !self.show_help,
            KeyCode::Tab => self.toggle_focus(),
            KeyCode::Char('r') => self.prepare_restore_plan(),
            KeyCode::Char('n') if self.restore_confirmation == RestoreConfirmation::Confirmed => {
                self.clear_restore_plan();
            }
            KeyCode::Char('/') => {
                self.focus = TuiFocus::Files;
                self.filtering_files = true;
                self.file_filter.clear();
                self.selected_file = 0;
            }
            KeyCode::Char('c') if self.focus == TuiFocus::Files => {
                self.file_filter.clear();
                self.selected_file = 0;
            }
            KeyCode::Down | KeyCode::Char('j') => self.select_next(),
            KeyCode::Up | KeyCode::Char('k') => self.select_previous(),
            KeyCode::Home => self.select_first(),
            KeyCode::End => self.select_last(),
            _ => {}
        }
    }

    fn handle_main_menu_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('?') | KeyCode::F(1) => self.show_help = !self.show_help,
            KeyCode::Down | KeyCode::Char('j') => self.select_next_menu_item(),
            KeyCode::Up | KeyCode::Char('k') => self.select_previous_menu_item(),
            KeyCode::Home => self.selected_menu_item = 0,
            KeyCode::End => self.selected_menu_item = MENU_ITEMS.len() - 1,
            KeyCode::Enter => self.activate_menu_item(),
            _ => {}
        }
    }

    fn handle_path_input_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc => self.cancel_path_input(),
            KeyCode::Enter => self.accept_path_input(),
            KeyCode::Backspace => {
                if let Some(input) = &mut self.path_input {
                    input.value.pop();
                    input.error = None;
                }
            }
            KeyCode::Char(character) if !character.is_control() => {
                if let Some(input) = &mut self.path_input {
                    input.value.push(character);
                    input.error = None;
                }
            }
            _ => {}
        }
    }

    fn handle_backup_review_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Enter => self.run_guided_backup(),
            KeyCode::Char('e') | KeyCode::Char('E') => {
                self.start_path_input(PathInputKind::BackupSource);
            }
            KeyCode::Backspace | KeyCode::Esc => self.view = TuiView::MainMenu,
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') | KeyCode::F(1) => self.show_help = !self.show_help,
            _ => {}
        }
    }

    fn handle_restore_rehearsal_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Enter => self.run_restore_rehearsal(),
            KeyCode::Down | KeyCode::Char('j') => self.select_next_snapshot(),
            KeyCode::Up | KeyCode::Char('k') => self.select_previous_snapshot(),
            KeyCode::Home => self.select_first_snapshot(),
            KeyCode::End => self.select_last_snapshot(),
            KeyCode::Backspace | KeyCode::Esc => self.view = TuiView::MainMenu,
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') | KeyCode::F(1) => self.show_help = !self.show_help,
            _ => {}
        }
    }

    fn handle_health_check_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Enter => self.run_health_check(),
            KeyCode::Backspace | KeyCode::Esc => self.view = TuiView::MainMenu,
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') | KeyCode::F(1) => self.show_help = !self.show_help,
            _ => {}
        }
    }

    fn handle_doctor_report_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Enter => self.run_doctor_report(),
            KeyCode::Backspace | KeyCode::Esc => self.view = TuiView::MainMenu,
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') | KeyCode::F(1) => self.show_help = !self.show_help,
            _ => {}
        }
    }

    fn handle_snapshot_diff_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Enter => self.run_snapshot_diff(),
            KeyCode::Tab => {
                self.diff_focus_old = !self.diff_focus_old;
                self.diff_result = None;
            }
            KeyCode::Down | KeyCode::Char('j') => self.select_next_diff_snapshot(),
            KeyCode::Up | KeyCode::Char('k') => self.select_previous_diff_snapshot(),
            KeyCode::Home => self.select_first_diff_snapshot(),
            KeyCode::End => self.select_last_diff_snapshot(),
            KeyCode::Backspace | KeyCode::Esc => self.view = TuiView::MainMenu,
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('?') | KeyCode::F(1) => self.show_help = !self.show_help,
            _ => {}
        }
    }

    fn handle_restore_confirmation_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('y') | KeyCode::Char('Y') => self.run_guided_restore(),
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => self.clear_restore_plan(),
            KeyCode::Char('q') => self.should_quit = true,
            _ => {}
        }
    }

    fn handle_filter_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Esc | KeyCode::Enter => self.filtering_files = false,
            KeyCode::Backspace => {
                self.file_filter.pop();
                self.selected_file = 0;
            }
            KeyCode::Char(character) if !character.is_control() => {
                self.file_filter.push(character);
                self.selected_file = 0;
            }
            _ => {}
        }
    }

    fn should_quit(&self) -> bool {
        self.should_quit
    }

    fn help_text(&self) -> String {
        if self.view == TuiView::PathInput {
            return "Type path | Enter validate | Backspace edit | Esc cancel".to_owned();
        }

        if self.view == TuiView::BackupReview {
            return "Enter run backup | e edit source | Backspace/Esc menu | q quit".to_owned();
        }

        if self.view == TuiView::RestoreRehearsal {
            return "Up/Down choose snapshot | Enter rehearse | Backspace/Esc menu | q quit"
                .to_owned();
        }

        if self.view == TuiView::HealthCheck {
            return "Enter rerun check | Backspace/Esc menu | q quit".to_owned();
        }

        if self.view == TuiView::DoctorReport {
            return "Enter rerun doctor | Backspace/Esc menu | q quit".to_owned();
        }

        if self.view == TuiView::SnapshotDiff {
            return "Tab old/new | Up/Down choose | Enter diff | Backspace/Esc menu | q quit"
                .to_owned();
        }

        if self.view == TuiView::MainMenu {
            return if self.show_help {
                "Up/Down or j/k choose | Enter select | q/Esc quit | ?/F1 hide help".to_owned()
            } else {
                "?/F1 help | q/Esc quit".to_owned()
            };
        }

        if self.restore_confirmation == RestoreConfirmation::Awaiting {
            return "y run restore to shown target | n/Esc cancel | t change target | q quit"
                .to_owned();
        }

        if self.restore_confirmation == RestoreConfirmation::Confirmed {
            return "Restore completed | n clear preview | Backspace menu | q/Esc quit".to_owned();
        }

        if self.filtering_files {
            return "Type path filter | Enter accept | Backspace edit | Esc stop filtering"
                .to_owned();
        }

        if self.show_help {
            "Backspace menu | Tab focus | Up/Down or j/k move | Home/End jump | / filter files | c clear filter | t target | r restore preview | q/Esc quit | ?/F1 hide help".to_owned()
        } else {
            "Backspace menu | ?/F1 help | q/Esc quit".to_owned()
        }
    }

    fn snapshot_count(&self) -> usize {
        self.snapshots.len()
    }

    fn selected_snapshot_id(&self) -> Option<&str> {
        self.snapshots
            .get(self.selected_snapshot)
            .map(|snapshot| snapshot.snapshot_id.as_str())
    }

    fn selected_snapshot(&self) -> Option<&SnapshotRow> {
        self.snapshots.get(self.selected_snapshot)
    }

    fn selected_file(&self) -> Option<&FileRow> {
        self.filtered_files().get(self.selected_file).copied()
    }

    fn filtered_file_count(&self) -> usize {
        self.filtered_files().len()
    }

    fn filtered_files(&self) -> Vec<&FileRow> {
        let Some(snapshot) = self.selected_snapshot() else {
            return Vec::new();
        };

        let filter = self.file_filter.to_lowercase();
        snapshot
            .entries
            .iter()
            .filter(|entry| filter.is_empty() || entry.path.to_lowercase().contains(&filter))
            .collect()
    }

    fn toggle_focus(&mut self) {
        self.focus = match self.focus {
            TuiFocus::Snapshots => TuiFocus::Files,
            TuiFocus::Files => TuiFocus::Snapshots,
        };
    }

    fn select_next_menu_item(&mut self) {
        if self.selected_menu_item + 1 < MENU_ITEMS.len() {
            self.selected_menu_item += 1;
        }
    }

    fn select_previous_menu_item(&mut self) {
        self.selected_menu_item = self.selected_menu_item.saturating_sub(1);
    }

    fn activate_menu_item(&mut self) {
        let item = MENU_ITEMS[self.selected_menu_item];
        if !item.enabled {
            return;
        }

        match item.action {
            MenuAction::BrowseSnapshots | MenuAction::RestoreFiles => {
                self.view = TuiView::Browser;
                self.focus = if item.action == MenuAction::RestoreFiles {
                    TuiFocus::Files
                } else {
                    TuiFocus::Snapshots
                };
            }
            MenuAction::ChangeRepository => self.start_path_input(PathInputKind::Repository),
            MenuAction::CreateBackup => {
                if self.backup_source.is_some() {
                    self.view = TuiView::BackupReview;
                } else {
                    self.start_path_input(PathInputKind::BackupSource);
                }
            }
            MenuAction::RehearseRestore => self.open_restore_rehearsal(),
            MenuAction::InitializeRepository => self.initialize_repository(),
            MenuAction::CheckHealth => {
                self.view = TuiView::HealthCheck;
                self.run_health_check();
            }
            MenuAction::DoctorReport => {
                self.view = TuiView::DoctorReport;
                self.run_doctor_report();
            }
            MenuAction::CompareSnapshots => self.open_snapshot_diff(),
            MenuAction::Quit => self.should_quit = true,
        }
    }

    fn open_snapshot_diff(&mut self) {
        self.view = TuiView::SnapshotDiff;
        self.diff_focus_old = true;
        self.diff_result = None;
        self.normalize_diff_selection();
        if self.snapshots.len() < 2 {
            self.status_message = Some("Create at least two snapshots to compare.".to_owned());
        } else {
            self.status_message =
                Some("Choose two snapshots, then press Enter to diff.".to_owned());
        }
    }

    fn start_path_input(&mut self, kind: PathInputKind) {
        let value = match kind {
            PathInputKind::Repository => self.repository.display().to_string(),
            PathInputKind::BackupSource => self
                .backup_source
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
            PathInputKind::RestoreTarget => self
                .restore_target
                .as_ref()
                .map(|path| path.display().to_string())
                .unwrap_or_default(),
        };
        self.path_input = Some(PathInput {
            kind,
            value,
            error: None,
        });
        self.status_message = None;
        self.view = TuiView::PathInput;
    }

    fn cancel_path_input(&mut self) {
        self.path_input = None;
        self.view = TuiView::MainMenu;
        self.status_message = Some("Path entry cancelled.".to_owned());
    }

    fn accept_path_input(&mut self) {
        let Some(input) = self.path_input.clone() else {
            self.view = TuiView::MainMenu;
            return;
        };
        let path = PathBuf::from(input.value.trim());
        match self.validate_and_store_path(input.kind, &path) {
            Ok(message) => {
                self.path_input = None;
                self.view = match input.kind {
                    PathInputKind::BackupSource => TuiView::BackupReview,
                    PathInputKind::Repository | PathInputKind::RestoreTarget => TuiView::MainMenu,
                };
                self.status_message = Some(message);
            }
            Err(error) => {
                if let Some(input) = &mut self.path_input {
                    input.error = Some(error);
                }
            }
        }
    }

    fn validate_and_store_path(
        &mut self,
        kind: PathInputKind,
        path: &Path,
    ) -> Result<String, String> {
        if path.as_os_str().is_empty() {
            return Err("path must not be empty".to_owned());
        }

        match kind {
            PathInputKind::Repository => {
                self.repository = path.to_owned();
                match self.reload_repository() {
                    Ok(()) => Ok(format!("Repository loaded: {}", path.display())),
                    Err(error) => {
                        self.repository_id = "<not initialized>".to_owned();
                        self.snapshots.clear();
                        self.selected_snapshot = 0;
                        self.selected_file = 0;
                        self.clear_restore_plan();
                        Ok(format!(
                            "Repository path selected but not loaded: {error}. Use Initialize repository."
                        ))
                    }
                }
            }
            PathInputKind::BackupSource => {
                if !path.exists() {
                    return Err("source path does not exist".to_owned());
                }
                self.backup_source = Some(path.to_owned());
                self.backup_result = None;
                Ok(format!("Backup source selected: {}", path.display()))
            }
            PathInputKind::RestoreTarget => {
                if let Some(parent) = path.parent()
                    && !parent.as_os_str().is_empty()
                    && !parent.exists()
                {
                    return Err("restore target parent does not exist".to_owned());
                }
                self.restore_target = Some(path.to_owned());
                self.clear_restore_plan();
                self.restore_result = None;
                Ok(format!("Restore target selected: {}", path.display()))
            }
        }
    }

    fn run_guided_backup(&mut self) {
        let Some(source) = self.backup_source.clone() else {
            self.status_message = Some("Select a backup source before running backup.".to_owned());
            self.start_path_input(PathInputKind::BackupSource);
            return;
        };

        match run_backup(BackupRequest {
            paths: vec![source],
            repo: self.repository.clone(),
            policy_ignore_patterns: Vec::new(),
            fail_on_changed_file: false,
        }) {
            Ok(result) => {
                self.backup_result = Some(BackupRunSummary {
                    snapshot_id: result.snapshot_id.clone(),
                    files_scanned: result.files_scanned,
                    logical_bytes: result.logical_bytes,
                    newly_stored_bytes: result.newly_stored_bytes,
                    warning_count: result.warning_count,
                });
                match list_manifests(&self.repository) {
                    Ok(manifests) => {
                        self.snapshots = manifests.into_iter().map(SnapshotRow::from).collect();
                        self.selected_snapshot = self.snapshots.len().saturating_sub(1);
                        self.selected_file = 0;
                        self.status_message =
                            Some(format!("Backup completed: {}", result.snapshot_id));
                    }
                    Err(error) => {
                        self.status_message = Some(format!(
                            "Backup completed, but snapshots could not reload: {error}"
                        ));
                    }
                }
            }
            Err(error) => {
                self.status_message = Some(format!("Backup failed: {error}"));
            }
        }
    }

    fn initialize_repository(&mut self) {
        match init_repository(&self.repository) {
            Ok(InitOutcome::Created(config)) => {
                self.repository_id = config.repository_id;
                self.snapshots.clear();
                self.selected_snapshot = 0;
                self.selected_file = 0;
                self.clear_restore_plan();
                self.status_message = Some(format!(
                    "Repository initialized at {}",
                    self.repository.display()
                ));
            }
            Ok(InitOutcome::AlreadyInitialized(_)) => match self.reload_repository() {
                Ok(()) => {
                    self.status_message =
                        Some(format!("Repository loaded: {}", self.repository.display()));
                }
                Err(error) => {
                    self.status_message = Some(format!("Repository load failed: {error}"));
                }
            },
            Err(error) => {
                self.status_message = Some(format!("Repository init failed: {error}"));
            }
        }
    }

    fn open_restore_rehearsal(&mut self) {
        self.view = TuiView::RestoreRehearsal;
        self.rehearsal_result = None;
        self.status_message = if self.snapshots.is_empty() {
            Some("Create a snapshot before rehearsing restore.".to_owned())
        } else {
            Some("Choose a snapshot, then press Enter to rehearse restore.".to_owned())
        };
    }

    fn run_restore_rehearsal(&mut self) {
        let Some(snapshot_id) = self.selected_snapshot_id().map(ToOwned::to_owned) else {
            self.status_message = Some("Create a snapshot before rehearsing restore.".to_owned());
            return;
        };

        match rehearse_restore(&self.repository, &snapshot_id) {
            Ok(summary) => {
                self.rehearsal_result = Some(RestoreRunSummary {
                    files: summary.files,
                    directories: summary.directories,
                    symlinks: summary.symlinks,
                    bytes: summary.bytes,
                });
                self.status_message = Some(format!("Restore rehearsal passed for {snapshot_id}."));
            }
            Err(error) => {
                self.rehearsal_result = None;
                self.status_message = Some(format!("Restore rehearsal failed: {error}"));
            }
        }
    }

    fn reload_repository(&mut self) -> Result<(), String> {
        let config = validate_repository(&self.repository).map_err(|error| error.to_string())?;
        let manifests = list_manifests(&self.repository).map_err(|error| error.to_string())?;
        self.repository_id = config.repository_id;
        self.snapshots = manifests.into_iter().map(SnapshotRow::from).collect();
        self.selected_snapshot = 0;
        self.selected_file = 0;
        self.clear_restore_plan();
        Ok(())
    }

    fn prepare_restore_plan(&mut self) {
        self.restore_plan = self.build_restore_plan();
        self.restore_confirmation = if self.restore_plan.is_some() {
            RestoreConfirmation::Awaiting
        } else {
            RestoreConfirmation::None
        };
    }

    fn clear_restore_plan(&mut self) {
        self.restore_plan = None;
        self.restore_confirmation = RestoreConfirmation::None;
    }

    fn build_restore_plan(&self) -> Option<RestorePlan> {
        let snapshot = self.selected_snapshot()?;
        let selected_path = if self.focus == TuiFocus::Files {
            self.selected_file().map(|file| file.path.clone())
        } else {
            None
        };
        let target = restore_target(
            &self.repository,
            &snapshot.snapshot_id,
            selected_path.as_deref(),
        );
        let target = self.restore_target.clone().unwrap_or(target);
        let snapshot_expression = selected_path
            .as_ref()
            .map(|path| format!("{}:{path}", snapshot.snapshot_id))
            .unwrap_or_else(|| snapshot.snapshot_id.clone());
        let command = format!(
            "traceback restore {} --repo {} --target {}",
            shell_arg(&snapshot_expression),
            shell_arg(&self.repository.display().to_string()),
            shell_arg(&target.display().to_string())
        );

        Some(RestorePlan {
            snapshot_id: snapshot.snapshot_id.clone(),
            selected_path,
            target,
            command,
        })
    }

    fn run_guided_restore(&mut self) {
        let Some(plan) = self.restore_plan.clone() else {
            self.status_message = Some("Preview a restore before running it.".to_owned());
            return;
        };

        let result = if let Some(path) = &plan.selected_path {
            restore_snapshot_path(&self.repository, &plan.snapshot_id, path, &plan.target)
        } else {
            restore_snapshot(&self.repository, &plan.snapshot_id, &plan.target)
        };

        match result {
            Ok(summary) => {
                self.restore_result = Some(RestoreRunSummary {
                    files: summary.files,
                    directories: summary.directories,
                    symlinks: summary.symlinks,
                    bytes: summary.bytes,
                });
                self.restore_confirmation = RestoreConfirmation::Confirmed;
                self.status_message =
                    Some(format!("Restore completed to {}", plan.target.display()));
            }
            Err(error) => {
                self.status_message = Some(format!("Restore failed: {error}"));
            }
        }
    }

    fn run_health_check(&mut self) {
        let report = check_repository(&self.repository);
        let passed = report.passed();
        self.health_report = Some(HealthCheckSummary::from(report));
        self.status_message = Some(if passed {
            "Repository health check passed.".to_owned()
        } else {
            "Repository health check found issues.".to_owned()
        });
    }

    fn run_doctor_report(&mut self) {
        match doctor_repository(&self.repository) {
            Ok(report) => {
                let score = report.health_score;
                self.doctor_report = Some(DoctorRunSummary::from(report));
                self.status_message = Some(format!("Doctor report completed: score {score}/100."));
            }
            Err(error) => {
                self.doctor_report = None;
                self.status_message = Some(format!("Doctor report failed: {error}"));
            }
        }
    }

    fn run_snapshot_diff(&mut self) {
        if self.snapshots.len() < 2 {
            self.status_message = Some("Create at least two snapshots to compare.".to_owned());
            return;
        }
        self.normalize_diff_selection();
        let old_snapshot_id = self.snapshots[self.diff_old_snapshot].snapshot_id.clone();
        let new_snapshot_id = self.snapshots[self.diff_new_snapshot].snapshot_id.clone();
        if old_snapshot_id == new_snapshot_id {
            self.status_message = Some("Choose two different snapshots to compare.".to_owned());
            return;
        }

        match diff_snapshots(&self.repository, &old_snapshot_id, &new_snapshot_id) {
            Ok(diff) => {
                let changed = diff.changed_count();
                self.diff_result = Some(DiffRunSummary {
                    old_snapshot_id: diff.old_snapshot_id,
                    new_snapshot_id: diff.new_snapshot_id,
                    added: diff.added.into_iter().map(DiffEntrySummary::from).collect(),
                    removed: diff
                        .removed
                        .into_iter()
                        .map(DiffEntrySummary::from)
                        .collect(),
                    modified: diff
                        .modified
                        .into_iter()
                        .map(DiffEntrySummary::from)
                        .collect(),
                    unchanged: diff.unchanged,
                });
                self.status_message =
                    Some(format!("Snapshot diff found {changed} changed path(s)."));
            }
            Err(error) => {
                self.status_message = Some(format!("Snapshot diff failed: {error}"));
            }
        }
    }

    fn normalize_diff_selection(&mut self) {
        let snapshot_count = self.snapshots.len();
        if snapshot_count == 0 {
            self.diff_old_snapshot = 0;
            self.diff_new_snapshot = 0;
            return;
        }
        self.diff_old_snapshot = self.diff_old_snapshot.min(snapshot_count - 1);
        self.diff_new_snapshot = self.diff_new_snapshot.min(snapshot_count - 1);
        if snapshot_count > 1 && self.diff_old_snapshot == self.diff_new_snapshot {
            self.diff_new_snapshot = (self.diff_old_snapshot + 1).min(snapshot_count - 1);
            if self.diff_new_snapshot == self.diff_old_snapshot {
                self.diff_old_snapshot = self.diff_old_snapshot.saturating_sub(1);
            }
        }
    }

    fn select_next_diff_snapshot(&mut self) {
        if self.snapshots.is_empty() {
            return;
        }
        if self.diff_focus_old {
            self.diff_old_snapshot = (self.diff_old_snapshot + 1).min(self.snapshots.len() - 1);
        } else {
            self.diff_new_snapshot = (self.diff_new_snapshot + 1).min(self.snapshots.len() - 1);
        }
        self.diff_result = None;
    }

    fn select_previous_diff_snapshot(&mut self) {
        if self.diff_focus_old {
            self.diff_old_snapshot = self.diff_old_snapshot.saturating_sub(1);
        } else {
            self.diff_new_snapshot = self.diff_new_snapshot.saturating_sub(1);
        }
        self.diff_result = None;
    }

    fn select_first_diff_snapshot(&mut self) {
        if self.diff_focus_old {
            self.diff_old_snapshot = 0;
        } else {
            self.diff_new_snapshot = 0;
        }
        self.diff_result = None;
    }

    fn select_last_diff_snapshot(&mut self) {
        let Some(last) = self.snapshots.len().checked_sub(1) else {
            return;
        };
        if self.diff_focus_old {
            self.diff_old_snapshot = last;
        } else {
            self.diff_new_snapshot = last;
        }
        self.diff_result = None;
    }

    fn select_next(&mut self) {
        match self.focus {
            TuiFocus::Snapshots => self.select_next_snapshot(),
            TuiFocus::Files => self.select_next_file(),
        }
    }

    fn select_previous(&mut self) {
        match self.focus {
            TuiFocus::Snapshots => self.select_previous_snapshot(),
            TuiFocus::Files => self.select_previous_file(),
        }
    }

    fn select_first(&mut self) {
        match self.focus {
            TuiFocus::Snapshots => self.select_first_snapshot(),
            TuiFocus::Files => self.select_first_file(),
        }
    }

    fn select_last(&mut self) {
        match self.focus {
            TuiFocus::Snapshots => self.select_last_snapshot(),
            TuiFocus::Files => self.select_last_file(),
        }
    }

    fn select_next_snapshot(&mut self) {
        if self.selected_snapshot + 1 < self.snapshots.len() {
            self.selected_snapshot += 1;
            self.selected_file = 0;
            self.clear_restore_plan();
        }
    }

    fn select_previous_snapshot(&mut self) {
        let previous = self.selected_snapshot;
        self.selected_snapshot = self.selected_snapshot.saturating_sub(1);
        if self.selected_snapshot != previous {
            self.selected_file = 0;
            self.clear_restore_plan();
        }
    }

    fn select_first_snapshot(&mut self) {
        self.selected_snapshot = 0;
        self.selected_file = 0;
        self.clear_restore_plan();
    }

    fn select_last_snapshot(&mut self) {
        if let Some(last) = self.snapshots.len().checked_sub(1) {
            self.selected_snapshot = last;
            self.selected_file = 0;
            self.clear_restore_plan();
        }
    }

    fn select_next_file(&mut self) {
        if self.selected_file + 1 < self.filtered_file_count() {
            self.selected_file += 1;
            self.clear_restore_plan();
        }
    }

    fn select_previous_file(&mut self) {
        let previous = self.selected_file;
        self.selected_file = self.selected_file.saturating_sub(1);
        if self.selected_file != previous {
            self.clear_restore_plan();
        }
    }

    fn select_first_file(&mut self) {
        self.selected_file = 0;
        self.clear_restore_plan();
    }

    fn select_last_file(&mut self) {
        if let Some(last) = self.filtered_file_count().checked_sub(1) {
            self.selected_file = last;
            self.clear_restore_plan();
        }
    }
}

impl From<SnapshotManifest> for SnapshotRow {
    fn from(manifest: SnapshotManifest) -> Self {
        let directory_count = manifest
            .files
            .iter()
            .filter(|file| file.file_type == FileType::Directory)
            .count();
        let symlink_count = manifest
            .files
            .iter()
            .filter(|file| file.file_type == FileType::Symlink)
            .count();
        let chunk_references = manifest
            .files
            .iter()
            .map(|file| file.chunks.len())
            .sum::<usize>();
        let unique_chunks = manifest
            .files
            .iter()
            .flat_map(|file| file.chunks.iter())
            .collect::<BTreeSet<_>>()
            .len();

        Self {
            snapshot_id: manifest.snapshot_id,
            created_at: manifest.created_at,
            state: manifest.state,
            source_count: manifest.sources.len(),
            sources: manifest.sources.join(", "),
            file_count: manifest.summary.file_count,
            directory_count,
            symlink_count,
            logical_bytes: manifest.summary.logical_bytes,
            newly_stored_bytes: manifest.summary.newly_stored_bytes,
            chunk_references,
            unique_chunks,
            entries: manifest.files.into_iter().map(FileRow::from).collect(),
        }
    }
}

impl From<traceback_repo::FileEntry> for FileRow {
    fn from(entry: traceback_repo::FileEntry) -> Self {
        Self {
            path: entry.path,
            file_type: entry.file_type,
            size: entry.size,
            content_hash: entry.content_hash,
            chunks: entry.chunks,
            symlink_target: entry.symlink_target,
        }
    }
}

pub(crate) fn app_for_repository(repository: PathBuf) -> Result<TuiApp, RepositoryError> {
    let config = validate_repository(&repository)?;
    let manifests = list_manifests(&repository).map_err(repository_error)?;
    Ok(TuiApp::new(repository, config, manifests))
}

pub(crate) fn app_for_repository_or_path(repository: PathBuf) -> TuiApp {
    match app_for_repository(repository.clone()) {
        Ok(app) => app,
        Err(error) => TuiApp::uninitialized(
            repository,
            format!("Repository is not initialized: {error}. Use Initialize repository."),
        ),
    }
}

fn repository_error(error: traceback_repo::ManifestError) -> RepositoryError {
    RepositoryError::UnsupportedConfig(format!("repository snapshots could not be loaded: {error}"))
}

pub fn run(app: TuiApp) -> Result<(), Box<dyn Error>> {
    let mut terminal = setup_terminal()?;
    let result = run_event_loop(&mut terminal, app);
    restore_terminal(&mut terminal)?;
    result
}

fn setup_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>, Box<dyn Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(stdout)).map_err(Into::into)
}

fn restore_terminal(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
) -> Result<(), Box<dyn Error>> {
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn run_event_loop<B: Backend>(
    terminal: &mut Terminal<B>,
    mut app: TuiApp,
) -> Result<(), Box<dyn Error>> {
    while !app.should_quit() {
        terminal.draw(|frame| render(frame, &app))?;
        if event::poll(Duration::from_millis(250))?
            && let Event::Key(key) = event::read()?
        {
            app.handle_key(key.code);
        }
    }
    Ok(())
}

fn render(frame: &mut ratatui::Frame<'_>, app: &TuiApp) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(5),
            Constraint::Min(8),
            Constraint::Length(3),
        ])
        .split(area);

    let title_text = match app.view {
        TuiView::Browser => " terminal browser",
        TuiView::MainMenu
        | TuiView::PathInput
        | TuiView::BackupReview
        | TuiView::RestoreRehearsal
        | TuiView::HealthCheck
        | TuiView::DoctorReport
        | TuiView::SnapshotDiff => " guided terminal",
    };
    let title = Paragraph::new(Line::from(vec![
        Span::styled(
            "TraceBack",
            Style::default().fg(TEAL).add_modifier(Modifier::BOLD),
        ),
        Span::raw(title_text),
    ]))
    .alignment(Alignment::Center)
    .block(accent_block(""));
    frame.render_widget(title, chunks[0]);

    let mut overview_lines = vec![
        Line::from(format!("Repository: {}", app.repository.display())),
        Line::from(format!("Repository ID: {}", app.repository_id)),
        Line::from(format!(
            "Snapshots: {}{}",
            app.snapshot_count(),
            app.selected_snapshot_id()
                .map(|snapshot| format!(" | selected: {snapshot}"))
                .unwrap_or_default()
        )),
    ];
    if let Some(status) = &app.status_message {
        overview_lines.push(Line::from(format!("Status: {status}")));
    }

    let body = Paragraph::new(overview_lines)
        .wrap(Wrap { trim: true })
        .block(accent_block("Overview"));
    frame.render_widget(body, chunks[1]);

    if app.view == TuiView::PathInput {
        let input = Paragraph::new(path_input_lines(app))
            .wrap(Wrap { trim: false })
            .block(accent_block("Path Input"));
        frame.render_widget(input, chunks[2]);

        let footer = Paragraph::new(app.help_text())
            .alignment(Alignment::Center)
            .style(Style::default().fg(SOFT_TEAL))
            .block(accent_block(""));
        frame.render_widget(footer, chunks[3]);
        return;
    }

    if app.view == TuiView::BackupReview {
        let review = Paragraph::new(backup_review_lines(app))
            .wrap(Wrap { trim: false })
            .block(accent_block("Backup Review"));
        frame.render_widget(review, chunks[2]);

        let footer = Paragraph::new(app.help_text())
            .alignment(Alignment::Center)
            .style(Style::default().fg(SOFT_TEAL))
            .block(accent_block(""));
        frame.render_widget(footer, chunks[3]);
        return;
    }

    if app.view == TuiView::RestoreRehearsal {
        let rehearsal = Paragraph::new(restore_rehearsal_lines(app))
            .wrap(Wrap { trim: false })
            .block(accent_block("Restore Rehearsal"));
        frame.render_widget(rehearsal, chunks[2]);

        let footer = Paragraph::new(app.help_text())
            .alignment(Alignment::Center)
            .style(Style::default().fg(SOFT_TEAL))
            .block(accent_block(""));
        frame.render_widget(footer, chunks[3]);
        return;
    }

    if app.view == TuiView::HealthCheck {
        let health = Paragraph::new(health_check_lines(app))
            .wrap(Wrap { trim: false })
            .block(accent_block("Repository Health"));
        frame.render_widget(health, chunks[2]);

        let footer = Paragraph::new(app.help_text())
            .alignment(Alignment::Center)
            .style(Style::default().fg(SOFT_TEAL))
            .block(accent_block(""));
        frame.render_widget(footer, chunks[3]);
        return;
    }

    if app.view == TuiView::DoctorReport {
        let doctor = Paragraph::new(doctor_report_lines(app))
            .wrap(Wrap { trim: false })
            .block(accent_block("Doctor Report"));
        frame.render_widget(doctor, chunks[2]);

        let footer = Paragraph::new(app.help_text())
            .alignment(Alignment::Center)
            .style(Style::default().fg(SOFT_TEAL))
            .block(accent_block(""));
        frame.render_widget(footer, chunks[3]);
        return;
    }

    if app.view == TuiView::SnapshotDiff {
        let diff = Paragraph::new(snapshot_diff_lines(app))
            .wrap(Wrap { trim: false })
            .block(accent_block("Snapshot Diff"));
        frame.render_widget(diff, chunks[2]);

        let footer = Paragraph::new(app.help_text())
            .alignment(Alignment::Center)
            .style(Style::default().fg(SOFT_TEAL))
            .block(accent_block(""));
        frame.render_widget(footer, chunks[3]);
        return;
    }

    if app.view == TuiView::MainMenu {
        let menu = Paragraph::new(menu_lines(app))
            .wrap(Wrap { trim: false })
            .block(accent_block("Main Menu"));
        frame.render_widget(menu, chunks[2]);

        let footer = Paragraph::new(app.help_text())
            .alignment(Alignment::Center)
            .style(Style::default().fg(SOFT_TEAL))
            .block(accent_block(""));
        frame.render_widget(footer, chunks[3]);
        return;
    }

    let browser = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(36),
            Constraint::Percentage(34),
            Constraint::Percentage(30),
        ])
        .split(chunks[2]);

    let snapshots = Paragraph::new(snapshot_lines(app))
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .title(focused_title("Snapshots", app.focus == TuiFocus::Snapshots))
                .borders(Borders::ALL)
                .border_style(panel_border_style(app.focus == TuiFocus::Snapshots)),
        );
    frame.render_widget(snapshots, browser[0]);

    let files = Paragraph::new(file_lines(app))
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .title(file_browser_title(app))
                .borders(Borders::ALL)
                .border_style(panel_border_style(app.focus == TuiFocus::Files)),
        );
    frame.render_widget(files, browser[1]);

    let details = Paragraph::new(snapshot_detail_lines(app))
        .wrap(Wrap { trim: true })
        .block(
            Block::default()
                .title("Snapshot Details")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        );
    frame.render_widget(details, browser[2]);

    let footer = Paragraph::new(app.help_text())
        .alignment(Alignment::Center)
        .style(Style::default().fg(SOFT_TEAL))
        .block(accent_block(""));
    frame.render_widget(footer, chunks[3]);
}

fn accent_block(title: &'static str) -> Block<'static> {
    Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(TEAL))
        .title_style(Style::default().fg(SOFT_TEAL).add_modifier(Modifier::BOLD))
}

fn panel_border_style(focused: bool) -> Style {
    if focused {
        Style::default().fg(TEAL)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn menu_lines(app: &TuiApp) -> Vec<Line<'static>> {
    let mut lines = vec![
        Line::from(Span::styled(
            "Choose what you want to do.",
            Style::default().fg(SOFT_TEAL),
        )),
        Line::from(""),
    ];

    lines.extend(MENU_ITEMS.iter().enumerate().map(|(index, item)| {
        let availability = if item.enabled { "" } else { " (coming soon)" };
        if index == app.selected_menu_item {
            Line::from(vec![
                Span::styled("> ", Style::default().fg(TEAL).add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!("{:<24}", item.label),
                    Style::default().fg(TEAL).add_modifier(Modifier::BOLD),
                ),
                Span::raw(format!(" {}{}", item.description, availability)),
            ])
        } else {
            Line::from(format!(
                "  {:<24} {}{}",
                item.label, item.description, availability
            ))
        }
    }));

    lines.push(Line::from(""));
    if let Some(source) = &app.backup_source {
        lines.push(Line::from(format!("Backup source: {}", source.display())));
    }
    if let Some(target) = &app.restore_target {
        lines.push(Line::from(format!("Restore target: {}", target.display())));
    }
    if let Some(status) = &app.status_message {
        lines.push(Line::from(format!("Status: {status}")));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("Tip: ", Style::default().fg(SOFT_TEAL)),
        Span::raw("start with Browse snapshots for the current repository."),
    ]));
    lines
}

fn path_input_lines(app: &TuiApp) -> Vec<Line<'static>> {
    let Some(input) = &app.path_input else {
        return vec![Line::from("No path input is active.")];
    };

    let (title, description) = match input.kind {
        PathInputKind::Repository => (
            "Repository path",
            "Enter a TraceBack repository path. The TUI will validate and reload it.",
        ),
        PathInputKind::BackupSource => (
            "Backup source path",
            "Enter an existing file or directory to use for the guided backup flow.",
        ),
        PathInputKind::RestoreTarget => (
            "Restore target path",
            "Enter a restore target path. Existing targets are not written from this screen.",
        ),
    };

    let mut lines = vec![
        Line::from(Span::styled(
            title,
            Style::default().fg(TEAL).add_modifier(Modifier::BOLD),
        )),
        Line::from(description),
        Line::from(""),
        Line::from(vec![
            Span::styled("> ", Style::default().fg(TEAL).add_modifier(Modifier::BOLD)),
            Span::raw(input.value.clone()),
        ]),
    ];
    if let Some(error) = &input.error {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled("Error: ", Style::default().fg(Color::Red)),
            Span::raw(error.clone()),
        ]));
    }
    lines
}

fn backup_review_lines(app: &TuiApp) -> Vec<Line<'static>> {
    let source = app
        .backup_source
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<not selected>".to_owned());
    let mut lines = vec![
        Line::from(Span::styled(
            "Review backup plan.",
            Style::default().fg(SOFT_TEAL),
        )),
        Line::from(""),
        Line::from(format!("Repository: {}", app.repository.display())),
        Line::from(format!("Source: {source}")),
        Line::from("Mode: retry changed files, warn if they keep changing"),
        Line::from(""),
        Line::from("Press Enter to run backup, or e to edit source."),
    ];

    if let Some(result) = &app.backup_result {
        lines.extend([
            Line::from(""),
            Line::from(Span::styled(
                "Last backup result:",
                Style::default().fg(TEAL).add_modifier(Modifier::BOLD),
            )),
            Line::from(format!("Snapshot: {}", result.snapshot_id)),
            Line::from(format!("Files scanned: {}", result.files_scanned)),
            Line::from(format!("Logical: {} B", result.logical_bytes)),
            Line::from(format!("Stored: {} B", result.newly_stored_bytes)),
            Line::from(format!("Warnings: {}", result.warning_count)),
        ]);
    }
    if let Some(status) = &app.status_message {
        lines.push(Line::from(""));
        lines.push(status_line(status));
    }
    lines
}

fn restore_rehearsal_lines(app: &TuiApp) -> Vec<Line<'static>> {
    if app.snapshots.is_empty() {
        return vec![
            Line::from("Restore rehearsal needs at least one snapshot."),
            Line::from("Create a backup, then return here."),
        ];
    }

    let selected = app.selected_snapshot_id().unwrap_or("<none>");
    let mut lines = vec![
        Line::from(Span::styled(
            "Verify restore without writing to a user target.",
            Style::default().fg(SOFT_TEAL),
        )),
        Line::from(""),
        Line::from(format!("Selected snapshot: {selected}")),
        Line::from("Press Enter to run rehearsal in a temporary directory."),
        Line::from(""),
        Line::from("Snapshots:"),
    ];
    lines.extend(snapshot_lines(app).into_iter().take(8));

    if let Some(result) = &app.rehearsal_result {
        lines.extend([
            Line::from(""),
            Line::from(Span::styled(
                "Last rehearsal result:",
                Style::default().fg(TEAL).add_modifier(Modifier::BOLD),
            )),
            Line::from("Result: PASS"),
            Line::from(format!("Files verified: {}", result.files)),
            Line::from(format!("Directories verified: {}", result.directories)),
            Line::from(format!("Symlinks verified: {}", result.symlinks)),
            Line::from(format!("Bytes verified: {} B", result.bytes)),
        ]);
    }

    lines
}

fn health_check_lines(app: &TuiApp) -> Vec<Line<'static>> {
    let Some(report) = &app.health_report else {
        return vec![
            Line::from("Repository health check has not run yet."),
            Line::from("Press Enter to run it now."),
        ];
    };

    let result = if report.passed { "PASS" } else { "FAIL" };
    let mut lines = vec![
        Line::from(Span::styled(
            "Repository health check",
            Style::default().fg(SOFT_TEAL),
        )),
        Line::from(""),
        Line::from(vec![
            Span::raw("Result: "),
            Span::styled(
                result,
                Style::default()
                    .fg(if report.passed { TEAL } else { Color::Red })
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(format!("Manifests checked: {}", report.manifests_checked)),
        Line::from(format!("Chunks verified: {}", report.chunks_verified)),
        Line::from(format!("Orphaned chunks: {}", report.orphaned_chunks)),
        Line::from(format!("Staging leftovers: {}", report.staging_leftovers)),
        Line::from(format!(
            "Temporary chunks: {}",
            report.temporary_chunk_files
        )),
        Line::from(""),
    ];

    if report.issues.is_empty() {
        lines.push(Line::from(Span::styled(
            "No issues found.",
            Style::default().fg(TEAL),
        )));
        lines.push(Line::from(
            "Recommendation: keep regular backups and rehearse restores.",
        ));
    } else {
        lines.push(Line::from("Issues:"));
        lines.extend(
            report
                .issues
                .iter()
                .take(8)
                .map(|issue| Line::from(format!("- {issue}"))),
        );
        if report.issues.len() > 8 {
            lines.push(Line::from(format!(
                "... {} more issue(s)",
                report.issues.len() - 8
            )));
        }
        lines.push(Line::from(""));
        lines.push(Line::from(
            "Recommendation: run recover or inspect the listed paths.",
        ));
    }

    lines
}

fn doctor_report_lines(app: &TuiApp) -> Vec<Line<'static>> {
    let Some(report) = &app.doctor_report else {
        return vec![
            Line::from("Repository doctor has not run yet."),
            Line::from("Press Enter to run it now."),
        ];
    };

    let mut lines = vec![
        Line::from(Span::styled(
            "Reliability summary",
            Style::default().fg(SOFT_TEAL),
        )),
        Line::from(""),
        Line::from(vec![
            Span::raw("Health score: "),
            Span::styled(
                format!("{}/100", report.health_score),
                Style::default()
                    .fg(score_color(report.health_score))
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(format!("Scoring: {}", report.scoring_version)),
        Line::from(format!(
            "Latest snapshot: {}",
            report.latest_snapshot_id.as_deref().unwrap_or("<none>")
        )),
        Line::from(format!(
            "Latest snapshot age: {}",
            format_age(report.latest_snapshot_age_seconds)
        )),
        Line::from(format!(
            "Current integrity: {}",
            pass_label(Some(report.integrity_passed))
        )),
        Line::from(format!(
            "Recorded check evidence: {}",
            pass_label(report.latest_check_passed)
        )),
        Line::from(format!(
            "Recorded rehearsal evidence: {}",
            pass_label(report.latest_rehearsal_passed)
        )),
        Line::from(""),
        Line::from(Span::styled(
            "Findings:",
            Style::default().fg(TEAL).add_modifier(Modifier::BOLD),
        )),
    ];

    for finding in report.findings.iter().take(8) {
        lines.push(doctor_finding_line(finding));
        if let Some(recommendation) = &finding.recommendation {
            lines.push(Line::from(format!("  Recommendation: {recommendation}")));
        }
    }
    if report.findings.len() > 8 {
        lines.push(Line::from(format!(
            "... {} more finding(s)",
            report.findings.len() - 8
        )));
    }

    lines
}

fn snapshot_diff_lines(app: &TuiApp) -> Vec<Line<'static>> {
    if app.snapshots.len() < 2 {
        return vec![
            Line::from("Snapshot diff needs at least two snapshots."),
            Line::from("Create another backup, then return here."),
        ];
    }

    let old = diff_snapshot_label(app, app.diff_old_snapshot);
    let new = diff_snapshot_label(app, app.diff_new_snapshot);
    let old_marker = if app.diff_focus_old { ">" } else { " " };
    let new_marker = if app.diff_focus_old { " " } else { ">" };
    let mut lines = vec![
        Line::from(Span::styled(
            "Select snapshots to compare.",
            Style::default().fg(SOFT_TEAL),
        )),
        Line::from(""),
        selector_line(old_marker, "Old", &old),
        selector_line(new_marker, "New", &new),
        Line::from(""),
        Line::from("Press Enter to run diff."),
    ];

    if let Some(result) = &app.diff_result {
        lines.extend([
            Line::from(""),
            Line::from(Span::styled(
                "Diff result:",
                Style::default().fg(TEAL).add_modifier(Modifier::BOLD),
            )),
            Line::from(format!(
                "{} -> {}",
                result.old_snapshot_id, result.new_snapshot_id
            )),
            Line::from(format!("Added: {}", result.added.len())),
            Line::from(format!("Removed: {}", result.removed.len())),
            Line::from(format!("Modified: {}", result.modified.len())),
            Line::from(format!("Unchanged: {}", result.unchanged)),
            Line::from(""),
            Line::from("Changed paths:"),
        ]);
        let entries = result
            .added
            .iter()
            .map(|entry| ("A", entry))
            .chain(result.removed.iter().map(|entry| ("R", entry)))
            .chain(result.modified.iter().map(|entry| ("M", entry)));
        let mut shown = 0usize;
        for (kind, entry) in entries.take(10) {
            shown += 1;
            lines.push(diff_entry_line(kind, entry));
        }
        let changed_count = result.added.len() + result.removed.len() + result.modified.len();
        if changed_count == 0 {
            lines.push(Line::from("No changed paths."));
        } else if changed_count > shown {
            lines.push(Line::from(format!(
                "... {} more changed path(s)",
                changed_count - shown
            )));
        }
    }

    lines
}

fn diff_snapshot_label(app: &TuiApp, index: usize) -> String {
    app.snapshots
        .get(index)
        .map(|snapshot| {
            format!(
                "{}  {}  {}",
                snapshot.snapshot_id,
                display_created_at(&snapshot.created_at),
                snapshot.sources
            )
        })
        .unwrap_or_else(|| "<none>".to_owned())
}

fn focused_title(title: &str, focused: bool) -> String {
    if focused {
        format!("{title} *")
    } else {
        title.to_owned()
    }
}

fn status_line(status: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled("Status: ", Style::default().fg(SOFT_TEAL)),
        Span::raw(status.to_owned()),
    ])
}

fn pass_label(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "PASS",
        Some(false) => "FAIL",
        None => "not recorded",
    }
}

fn score_color(score: u8) -> Color {
    if score >= 80 {
        TEAL
    } else if score >= 50 {
        Color::Yellow
    } else {
        Color::Red
    }
}

fn format_age(seconds: Option<i64>) -> String {
    let Some(seconds) = seconds else {
        return "not available".to_owned();
    };
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m");
    }
    let hours = minutes / 60;
    if hours < 24 {
        return format!("{hours}h");
    }
    format!("{}d", hours / 24)
}

fn doctor_finding_line(finding: &DoctorFindingSummary) -> Line<'static> {
    let (label, color) = match finding.level {
        FindingLevel::Good => ("GOOD", TEAL),
        FindingLevel::Warning => ("WARN", Color::Yellow),
        FindingLevel::Critical => ("CRITICAL", Color::Red),
    };
    Line::from(vec![
        Span::styled(
            format!("- {label:<8}"),
            Style::default().fg(color).add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!(" {}: {}", finding.code, finding.message)),
    ])
}

fn selector_line(marker: &str, label: &'static str, value: &str) -> Line<'static> {
    if marker == ">" {
        Line::from(vec![
            Span::styled("> ", Style::default().fg(TEAL).add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("{label}: "),
                Style::default().fg(TEAL).add_modifier(Modifier::BOLD),
            ),
            Span::raw(value.to_owned()),
        ])
    } else {
        Line::from(format!("  {label}: {value}"))
    }
}

fn diff_entry_line(kind: &str, entry: &DiffEntrySummary) -> Line<'static> {
    let kind_style = match kind {
        "A" => Style::default().fg(TEAL).add_modifier(Modifier::BOLD),
        "R" => Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
        _ => Style::default().fg(SOFT_TEAL).add_modifier(Modifier::BOLD),
    };
    Line::from(vec![
        Span::styled(kind.to_owned(), kind_style),
        Span::raw(format!(
            " {:>6} B  {}",
            format!("{:+}", entry.byte_delta),
            entry.path
        )),
        Span::styled(
            if entry.type_changed { " type" } else { "" },
            Style::default().fg(SOFT_TEAL),
        ),
        Span::styled(
            if entry.content_changed {
                " content"
            } else {
                ""
            },
            Style::default().fg(SOFT_TEAL),
        ),
    ])
}

fn file_browser_title(app: &TuiApp) -> String {
    let title = focused_title("Files", app.focus == TuiFocus::Files);
    if app.file_filter.is_empty() {
        return title;
    }

    format!("{title} | filter: {}", app.file_filter)
}

fn snapshot_detail_lines(app: &TuiApp) -> Vec<Line<'static>> {
    let Some(snapshot) = app.selected_snapshot() else {
        return vec![
            Line::from("No snapshot selected."),
            Line::from("Create a backup to inspect details here."),
        ];
    };

    let mut lines = vec![
        Line::from(format!("ID: {}", snapshot.snapshot_id)),
        Line::from(format!(
            "Created: {}",
            display_created_at(&snapshot.created_at)
        )),
        Line::from(format!("State: {}", snapshot.state)),
        Line::from(format!("Sources: {}", snapshot.source_count)),
        Line::from(format!("Files: {}", snapshot.file_count)),
        Line::from(format!("Directories: {}", snapshot.directory_count)),
        Line::from(format!("Symlinks: {}", snapshot.symlink_count)),
        Line::from(format!("Logical: {} B", snapshot.logical_bytes)),
        Line::from(format!(
            "Stored in snapshot: {} B",
            snapshot.newly_stored_bytes
        )),
        Line::from(format!("Chunk refs: {}", snapshot.chunk_references)),
        Line::from(format!("Unique chunks: {}", snapshot.unique_chunks)),
        Line::from(vec![
            Span::raw("Warnings: "),
            Span::styled("none recorded", Style::default().fg(TEAL)),
        ]),
        Line::from(""),
    ];

    if let Some(file) = app.selected_file() {
        lines.extend([
            Line::from(Span::styled(
                "Selected entry:",
                Style::default().fg(TEAL).add_modifier(Modifier::BOLD),
            )),
            Line::from(format!("Path: {}", file.path)),
            Line::from(format!("Type: {}", display_file_type(file.file_type))),
            Line::from(format!("Size: {} B", file.size)),
            Line::from(format!(
                "Content hash: {}",
                file.content_hash.as_deref().unwrap_or("<none>")
            )),
            Line::from(format!("Chunks: {}", file.chunks.len())),
            Line::from(format!(
                "First chunk: {}",
                file.chunks
                    .first()
                    .map(|chunk| abbreviate(chunk, 16))
                    .unwrap_or_else(|| "<none>".to_owned())
            )),
        ]);

        if let Some(target) = &file.symlink_target {
            lines.push(Line::from(format!("Symlink target: {target}")));
        }
    } else if !app.file_filter.is_empty() {
        lines.push(Line::from("No file matches the current filter."));
    } else {
        lines.push(Line::from("No file selected."));
    }

    lines.extend(restore_plan_lines(app));
    lines
}

fn restore_plan_lines(app: &TuiApp) -> Vec<Line<'static>> {
    let Some(plan) = &app.restore_plan else {
        return vec![
            Line::from(""),
            Line::from("Restore: press r to preview a safe restore command."),
        ];
    };

    let status = match app.restore_confirmation {
        RestoreConfirmation::None => "not prepared",
        RestoreConfirmation::Awaiting => "awaiting confirmation",
        RestoreConfirmation::Confirmed => "restore completed",
    };
    let scope = plan.selected_path.as_deref().unwrap_or("<entire snapshot>");

    vec![
        Line::from(""),
        Line::from(Span::styled(
            "Restore preview:",
            Style::default().fg(TEAL).add_modifier(Modifier::BOLD),
        )),
        status_line(status),
        Line::from("Safety: restore writes only after y confirmation."),
        Line::from(format!("Snapshot: {}", plan.snapshot_id)),
        Line::from(format!("Path: {scope}")),
        Line::from(format!("Target: {}", plan.target.display())),
        Line::from(format!("Command: {}", plan.command)),
    ]
    .into_iter()
    .chain(app.restore_result.as_ref().into_iter().flat_map(|result| {
        [
            Line::from(""),
            Line::from(Span::styled(
                "Restore result:",
                Style::default().fg(TEAL).add_modifier(Modifier::BOLD),
            )),
            Line::from(format!("Files: {}", result.files)),
            Line::from(format!("Directories: {}", result.directories)),
            Line::from(format!("Symlinks: {}", result.symlinks)),
            Line::from(format!("Bytes: {} B", result.bytes)),
        ]
    }))
    .collect()
}

fn snapshot_lines(app: &TuiApp) -> Vec<Line<'static>> {
    if app.snapshots.is_empty() {
        return vec![Line::from("No snapshots found.")];
    }

    app.snapshots
        .iter()
        .enumerate()
        .map(|(index, snapshot)| {
            let text = format!(
                "{:<36}  {:<20}  logical {:>8} B  stored {:>8} B  {}",
                snapshot.snapshot_id,
                display_created_at(&snapshot.created_at),
                snapshot.logical_bytes,
                snapshot.newly_stored_bytes,
                snapshot.sources
            );
            if index == app.selected_snapshot {
                Line::from(vec![
                    Span::styled("> ", Style::default().fg(TEAL).add_modifier(Modifier::BOLD)),
                    Span::styled(text, Style::default().fg(TEAL)),
                ])
            } else {
                Line::from(format!("  {text}"))
            }
        })
        .collect()
}

fn file_lines(app: &TuiApp) -> Vec<Line<'static>> {
    let files = app.filtered_files();
    if files.is_empty() {
        if app.file_filter.is_empty() {
            return vec![Line::from("No file entries in this snapshot.")];
        }

        return vec![Line::from("No file entries match the current filter.")];
    }

    files
        .iter()
        .enumerate()
        .map(|(index, file)| {
            let text = format!(
                "{:<4} {:>8} B  {}",
                display_file_type(file.file_type),
                file.size,
                file.path
            );
            if index == app.selected_file {
                Line::from(vec![
                    Span::styled("> ", Style::default().fg(TEAL).add_modifier(Modifier::BOLD)),
                    Span::styled(text, Style::default().fg(TEAL)),
                ])
            } else {
                Line::from(format!("  {text}"))
            }
        })
        .collect()
}

fn display_file_type(file_type: FileType) -> &'static str {
    match file_type {
        FileType::File => "file",
        FileType::Directory => "dir",
        FileType::Symlink => "link",
    }
}

fn restore_target(repository: &Path, snapshot_id: &str, selected_path: Option<&str>) -> PathBuf {
    let base = repository
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut target = base.join("traceback-restore").join(snapshot_id);
    if let Some(path) = selected_path {
        for segment in path
            .split('/')
            .filter(|segment| !segment.is_empty() && *segment != "." && *segment != "..")
        {
            target.push(sanitize_target_segment(segment));
        }
    }
    target
}

fn sanitize_target_segment(segment: &str) -> String {
    segment
        .chars()
        .map(|character| match character {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' | '\0' => '_',
            character => character,
        })
        .collect()
}

fn shell_arg(value: &str) -> String {
    if value.is_empty() || value.chars().any(|character| character.is_whitespace()) {
        format!("\"{}\"", value.replace('"', "\\\""))
    } else {
        value.to_owned()
    }
}

fn abbreviate(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let abbreviated = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{abbreviated}...")
    } else {
        abbreviated
    }
}

fn display_created_at(timestamp: &str) -> String {
    timestamp
        .split_once('T')
        .map(|(date, time)| {
            format!(
                "{date} {}",
                time.trim_end_matches('Z').split('.').next().unwrap_or(time)
            )
        })
        .unwrap_or_else(|| timestamp.to_owned())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crossterm::event::KeyCode;
    use ratatui::{Terminal, backend::TestBackend};
    use tempfile::tempdir;
    use traceback_repo::{
        FileEntry, FileType, FindingLevel, RepositoryConfig, SnapshotManifest, init_repository,
    };

    use crate::{BackupRequest, run_backup};

    use super::{
        BackupRunSummary, DiffEntrySummary, DiffRunSummary, MENU_ITEMS, MenuAction,
        RestoreConfirmation, TEAL, TuiApp, TuiFocus, TuiView, app_for_repository, render,
    };

    #[test]
    fn app_starts_on_main_menu_and_quits_on_q_or_escape() {
        let mut app = app_with_snapshots(0);

        assert!(!app.should_quit());
        assert_eq!(app.view, TuiView::MainMenu);
        assert_eq!(
            app.help_text(),
            "Up/Down or j/k choose | Enter select | q/Esc quit | ?/F1 hide help"
        );

        app.handle_key(KeyCode::Char('?'));
        assert_eq!(app.help_text(), "?/F1 help | q/Esc quit");
        assert!(!app.should_quit());

        app.handle_key(KeyCode::Esc);
        assert!(app.should_quit());
    }

    #[test]
    fn main_menu_navigation_opens_browser_and_restore_entry_focuses_files() {
        let mut app = app_with_snapshots(1);

        app.handle_key(KeyCode::Enter);

        assert_eq!(app.view, TuiView::Browser);
        assert_eq!(app.focus, TuiFocus::Snapshots);

        app.handle_key(KeyCode::Backspace);
        press_keys(
            &mut app,
            [
                KeyCode::Down,
                KeyCode::Down,
                KeyCode::Down,
                KeyCode::Down,
                KeyCode::Enter,
            ],
        );

        assert_eq!(app.view, TuiView::Browser);
        assert_eq!(app.focus, TuiFocus::Files);
    }

    #[test]
    fn compare_menu_opens_diff_screen_and_requires_two_snapshots() {
        let mut app = app_with_snapshots(1);

        select_menu_action(&mut app, MenuAction::CompareSnapshots);

        assert_eq!(app.view, TuiView::SnapshotDiff);
        assert!(!app.should_quit());
        assert_eq!(
            app.status_message.as_deref(),
            Some("Create at least two snapshots to compare.")
        );
    }

    #[test]
    fn health_check_menu_runs_repository_check() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        init_repository(&repository).expect("repository should initialize");
        let mut app = app_for_repository(repository).expect("app should load repository");

        press_keys(
            &mut app,
            [
                KeyCode::Down,
                KeyCode::Down,
                KeyCode::Down,
                KeyCode::Down,
                KeyCode::Down,
                KeyCode::Down,
                KeyCode::Enter,
            ],
        );

        assert_eq!(app.view, TuiView::HealthCheck);
        let report = app
            .health_report
            .as_ref()
            .expect("health report should be recorded");
        assert!(report.passed);
        assert_eq!(report.manifests_checked, 0);
        assert_eq!(
            app.status_message.as_deref(),
            Some("Repository health check passed.")
        );

        app.handle_key(KeyCode::Backspace);
        assert_eq!(app.view, TuiView::MainMenu);
    }

    #[test]
    fn doctor_report_menu_runs_reliability_report() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        init_repository(&repository).expect("repository should initialize");
        let mut app = app_for_repository(repository).expect("app should load repository");

        select_menu_action(&mut app, MenuAction::DoctorReport);

        assert_eq!(app.view, TuiView::DoctorReport);
        let report = app
            .doctor_report
            .as_ref()
            .expect("doctor report should be recorded");
        assert!(report.health_score < 100);
        assert!(report.findings.iter().any(|finding| {
            finding.level == FindingLevel::Warning || finding.level == FindingLevel::Critical
        }));
        assert!(
            app.status_message
                .as_deref()
                .is_some_and(|status| status.starts_with("Doctor report completed: score "))
        );

        app.handle_key(KeyCode::Backspace);
        assert_eq!(app.view, TuiView::MainMenu);
    }

    #[test]
    fn restore_rehearsal_menu_runs_rehearsal_for_snapshot() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        let source = temporary.path().join("source");
        std::fs::create_dir(&source).expect("source should be created");
        std::fs::write(source.join("hello.txt"), "hello").expect("source file should be written");
        init_repository(&repository).expect("repository should initialize");
        run_backup(BackupRequest {
            paths: vec![source],
            repo: repository.clone(),
            policy_ignore_patterns: Vec::new(),
            fail_on_changed_file: false,
        })
        .expect("backup should run");
        let mut app = app_for_repository(repository).expect("app should load repository");

        press_keys(
            &mut app,
            [
                KeyCode::Down,
                KeyCode::Down,
                KeyCode::Down,
                KeyCode::Down,
                KeyCode::Down,
                KeyCode::Enter,
                KeyCode::Enter,
            ],
        );

        assert_eq!(app.view, TuiView::RestoreRehearsal);
        let result = app
            .rehearsal_result
            .as_ref()
            .expect("rehearsal result should be recorded");
        assert_eq!(result.files, 1);
        assert_eq!(result.bytes, 5);
        assert!(
            app.status_message
                .as_deref()
                .is_some_and(|message| message.starts_with("Restore rehearsal passed"))
        );
    }

    #[test]
    fn path_input_can_cancel_back_to_menu() {
        let mut app = app_with_snapshots(1);

        press_keys(&mut app, [KeyCode::Down, KeyCode::Enter, KeyCode::Esc]);

        assert_eq!(app.view, TuiView::MainMenu);
        assert!(app.path_input.is_none());
        assert_eq!(app.status_message.as_deref(), Some("Path entry cancelled."));
    }

    #[test]
    fn initial_backup_source_opens_backup_review() {
        let mut app = app_with_snapshots(1);

        app.set_backup_source(PathBuf::from("./source"));

        assert_eq!(app.view, TuiView::BackupReview);
        assert_eq!(app.backup_source, Some(PathBuf::from("./source")));
        assert_eq!(
            app.status_message.as_deref(),
            Some("Backup source selected: ./source")
        );
    }

    #[test]
    fn repository_path_input_validates_and_reloads_snapshots() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        init_repository(&repository).expect("repository should initialize");
        let mut app = app_with_snapshots(1);

        press_keys(&mut app, [KeyCode::Down, KeyCode::Enter]);
        replace_input(&mut app, &repository.display().to_string());
        app.handle_key(KeyCode::Enter);

        assert_eq!(app.view, TuiView::MainMenu);
        assert_eq!(app.repository, repository);
        assert_eq!(app.snapshot_count(), 0);
        assert!(
            app.status_message
                .as_deref()
                .is_some_and(|message| message.starts_with("Repository loaded:"))
        );
    }

    #[test]
    fn repository_path_input_accepts_uninitialized_path() {
        let temporary = tempdir().expect("temporary directory should be created");
        let mut app = app_with_snapshots(1);
        let repository = temporary.path().join("missing");

        press_keys(&mut app, [KeyCode::Down, KeyCode::Enter]);
        replace_input(&mut app, &repository.display().to_string());
        app.handle_key(KeyCode::Enter);

        assert_eq!(app.view, TuiView::MainMenu);
        assert_eq!(app.repository, repository);
        assert_eq!(app.repository_id, "<not initialized>");
        assert_eq!(app.snapshot_count(), 0);
        assert!(
            app.status_message
                .as_deref()
                .is_some_and(|message| message.contains("Use Initialize repository"))
        );
    }

    #[test]
    fn initialize_repository_menu_creates_missing_repository() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("new-repo");
        let mut app = TuiApp::uninitialized(repository.clone(), "not ready".to_owned());

        press_keys(&mut app, [KeyCode::Down, KeyCode::Down, KeyCode::Enter]);

        assert_eq!(app.view, TuiView::MainMenu);
        assert!(repository.join("config.toml").exists());
        assert_ne!(app.repository_id, "<not initialized>");
        assert_eq!(app.snapshot_count(), 0);
        assert!(
            app.status_message
                .as_deref()
                .is_some_and(|message| message.starts_with("Repository initialized at"))
        );
    }

    #[test]
    fn source_and_target_path_inputs_store_valid_paths() {
        let temporary = tempdir().expect("temporary directory should be created");
        let source = temporary.path().join("source");
        std::fs::create_dir(&source).expect("source should be created");
        let target = temporary.path().join("restore-target");
        let mut app = app_with_snapshots(1);

        press_keys(
            &mut app,
            [KeyCode::Down, KeyCode::Down, KeyCode::Down, KeyCode::Enter],
        );
        replace_input(&mut app, &source.display().to_string());
        app.handle_key(KeyCode::Enter);

        assert_eq!(app.backup_source.as_deref(), Some(source.as_path()));
        assert_eq!(app.view, TuiView::BackupReview);
        assert!(
            app.status_message
                .as_deref()
                .is_some_and(|message| message.starts_with("Backup source selected:"))
        );

        app.view = TuiView::Browser;
        app.handle_key(KeyCode::Char('t'));
        replace_input(&mut app, &target.display().to_string());
        app.handle_key(KeyCode::Enter);

        assert_eq!(app.restore_target.as_deref(), Some(target.as_path()));
        assert!(
            app.status_message
                .as_deref()
                .is_some_and(|message| message.starts_with("Restore target selected:"))
        );
    }

    #[test]
    fn guided_backup_flow_runs_backup_and_reloads_snapshots() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        let source = temporary.path().join("source");
        std::fs::create_dir(&source).expect("source should be created");
        std::fs::write(source.join("hello.txt"), "hello").expect("source file should be written");
        init_repository(&repository).expect("repository should initialize");
        let mut app = app_for_repository(repository).expect("app should load repository");

        press_keys(
            &mut app,
            [KeyCode::Down, KeyCode::Down, KeyCode::Down, KeyCode::Enter],
        );
        replace_input(&mut app, &source.display().to_string());
        app.handle_key(KeyCode::Enter);
        assert_eq!(app.view, TuiView::BackupReview);

        app.handle_key(KeyCode::Enter);

        assert_eq!(app.snapshot_count(), 1);
        assert!(app.backup_result.is_some());
        assert!(
            app.status_message
                .as_deref()
                .is_some_and(|message| message.starts_with("Backup completed: snap_"))
        );
        assert!(app.selected_snapshot_id().is_some());
    }

    #[test]
    fn render_backup_review_includes_plan_and_result() {
        let mut app = app_with_snapshots(1);
        app.view = TuiView::BackupReview;
        app.backup_source = Some(PathBuf::from("./source"));
        app.backup_result = Some(BackupRunSummary {
            snapshot_id: "snap_result".to_owned(),
            files_scanned: 2,
            logical_bytes: 35,
            newly_stored_bytes: 20,
            warning_count: 0,
        });
        let backend = TestBackend::new(140, 32);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        let rendered = render_to_string(&mut terminal, &app);

        assert!(rendered.contains("Backup Review"));
        assert!(rendered.contains("Review backup plan."));
        assert!(rendered.contains("Source: ./source"));
        assert!(rendered.contains("Last backup result:"));
        assert!(rendered.contains("Snapshot: snap_result"));
    }

    #[test]
    fn path_input_renders_prompt_and_error() {
        let mut app = app_with_snapshots(1);
        press_keys(&mut app, [KeyCode::Down, KeyCode::Enter]);
        replace_input(&mut app, "");
        app.handle_key(KeyCode::Enter);
        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        let rendered = render_to_string(&mut terminal, &app);

        assert!(rendered.contains("TraceBack guided terminal"));
        assert!(rendered.contains("Path Input"));
        assert!(rendered.contains("Repository path"));
        assert!(rendered.contains("Error: path must not be empty"));
    }

    #[test]
    fn main_menu_exit_item_quits() {
        let mut app = app_with_snapshots(1);

        app.handle_key(KeyCode::End);
        app.handle_key(KeyCode::Enter);

        assert!(app.should_quit());
    }

    #[test]
    fn filter_mode_captures_quit_keys_until_filtering_stops() {
        let mut app = browser_app_with_snapshots(1);

        press_keys(&mut app, [KeyCode::Char('/'), KeyCode::Char('q')]);

        assert!(!app.should_quit());
        assert_eq!(app.file_filter, "q");
        assert_eq!(
            app.help_text(),
            "Type path filter | Enter accept | Backspace edit | Esc stop filtering"
        );

        press_keys(&mut app, [KeyCode::Esc, KeyCode::Char('q')]);

        assert!(app.should_quit());
    }

    #[test]
    fn navigation_selects_snapshots_without_leaving_bounds() {
        let mut app = browser_app_with_snapshots(3);

        assert_eq!(app.selected_snapshot_id(), Some("snap_001"));

        app.handle_key(KeyCode::Down);
        assert_eq!(app.selected_snapshot_id(), Some("snap_002"));

        app.handle_key(KeyCode::Char('j'));
        app.handle_key(KeyCode::Char('j'));
        assert_eq!(app.selected_snapshot_id(), Some("snap_003"));

        app.handle_key(KeyCode::Up);
        assert_eq!(app.selected_snapshot_id(), Some("snap_002"));

        app.handle_key(KeyCode::Home);
        assert_eq!(app.selected_snapshot_id(), Some("snap_001"));

        app.handle_key(KeyCode::End);
        assert_eq!(app.selected_snapshot_id(), Some("snap_003"));
    }

    #[test]
    fn empty_snapshot_navigation_is_safe() {
        let mut app = browser_app_with_snapshots(0);

        app.handle_key(KeyCode::Down);
        app.handle_key(KeyCode::End);

        assert_eq!(app.selected_snapshot_id(), None);
        assert_eq!(app.selected_snapshot, 0);
    }

    #[test]
    fn tab_switches_focus_and_file_navigation_tracks_selected_snapshot() {
        let mut app = browser_app_with_snapshots(2);

        app.handle_key(KeyCode::Tab);
        app.handle_key(KeyCode::Down);

        assert_eq!(
            app.selected_file().map(|file| file.path.as_str()),
            Some("source/file-b.txt")
        );

        app.handle_key(KeyCode::Tab);
        app.handle_key(KeyCode::Down);

        assert_eq!(app.selected_snapshot_id(), Some("snap_002"));
        assert_eq!(
            app.selected_file().map(|file| file.path.as_str()),
            Some("source/file-a.txt")
        );
    }

    #[test]
    fn file_filter_limits_visible_paths_and_can_clear() {
        let mut app = browser_app_with_snapshots(1);

        app.handle_key(KeyCode::Char('/'));
        app.handle_key(KeyCode::Char('B'));
        app.handle_key(KeyCode::Enter);

        assert_eq!(app.filtered_file_count(), 1);
        assert_eq!(
            app.selected_file().map(|file| file.path.as_str()),
            Some("source/file-b.txt")
        );
        assert_eq!(
            app.help_text(),
            "Backspace menu | Tab focus | Up/Down or j/k move | Home/End jump | / filter files | c clear filter | t target | r restore preview | q/Esc quit | ?/F1 hide help"
        );

        app.handle_key(KeyCode::Char('c'));

        assert_eq!(app.filtered_file_count(), 4);
        assert!(app.file_filter.is_empty());
    }

    #[test]
    fn restore_preview_defaults_to_safe_snapshot_target() {
        let mut app = browser_app_with_snapshots(1);

        app.handle_key(KeyCode::Char('r'));

        let plan = app
            .restore_plan
            .as_ref()
            .expect("restore plan should be prepared");
        assert_eq!(app.restore_confirmation, RestoreConfirmation::Awaiting);
        assert_eq!(plan.snapshot_id, "snap_001");
        assert_eq!(plan.selected_path, None);
        let expected_target = PathBuf::from(".")
            .join("traceback-restore")
            .join("snap_001");
        assert_eq!(plan.target, expected_target);
        assert_eq!(
            plan.command,
            format!(
                "traceback restore snap_001 --repo ./repo --target {}",
                expected_target.display()
            )
        );
        assert!(!app.should_quit());

        app.handle_key(KeyCode::Char('n'));

        assert!(app.restore_plan.is_none());
        assert_eq!(app.restore_confirmation, RestoreConfirmation::None);
    }

    #[test]
    fn restore_preview_does_not_prepare_without_snapshots() {
        let mut app = browser_app_with_snapshots(0);

        app.handle_key(KeyCode::Char('r'));

        assert!(app.restore_plan.is_none());
        assert_eq!(app.restore_confirmation, RestoreConfirmation::None);
        assert_eq!(app.selected_snapshot_id(), None);
    }

    #[test]
    fn file_restore_preview_uses_selected_path_and_clear_target() {
        let mut app = browser_app_with_snapshots(1);

        app.handle_key(KeyCode::Tab);
        app.handle_key(KeyCode::Down);
        app.handle_key(KeyCode::Char('r'));

        let plan = app
            .restore_plan
            .as_ref()
            .expect("selected file restore plan should be prepared");
        assert_eq!(app.restore_confirmation, RestoreConfirmation::Awaiting);
        assert_eq!(plan.selected_path.as_deref(), Some("source/file-b.txt"));
        let expected_target = PathBuf::from(".")
            .join("traceback-restore")
            .join("snap_001")
            .join("source")
            .join("file-b.txt");
        assert_eq!(plan.target, expected_target);
        assert!(plan.command.contains("snap_001:source/file-b.txt"));
        assert!(
            plan.command
                .contains(&expected_target.display().to_string())
        );

        app.handle_key(KeyCode::Esc);

        assert!(app.restore_plan.is_none());
        assert_eq!(app.restore_confirmation, RestoreConfirmation::None);
        assert!(!app.should_quit());
    }

    #[test]
    fn guided_restore_runs_selected_file_to_default_target() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        let source = temporary.path().join("source");
        std::fs::create_dir(&source).expect("source should be created");
        std::fs::write(source.join("hello.txt"), "hello").expect("source file should be written");
        init_repository(&repository).expect("repository should initialize");
        run_backup(BackupRequest {
            paths: vec![source],
            repo: repository.clone(),
            policy_ignore_patterns: Vec::new(),
            fail_on_changed_file: false,
        })
        .expect("backup should run");
        let mut app = app_for_repository(repository).expect("app should load repository");
        app.view = TuiView::Browser;
        app.focus = TuiFocus::Files;

        app.handle_key(KeyCode::Char('r'));
        let target = app
            .restore_plan
            .as_ref()
            .expect("restore plan should be prepared")
            .target
            .clone();
        app.handle_key(KeyCode::Char('y'));

        assert_eq!(app.restore_confirmation, RestoreConfirmation::Confirmed);
        assert_eq!(
            std::fs::read_to_string(&target).expect("default target should be restored"),
            "hello"
        );
        assert!(
            app.status_message
                .as_deref()
                .is_some_and(|message| message.starts_with("Restore completed to "))
        );
    }

    #[test]
    fn guided_restore_runs_selected_file_to_target() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        let source = temporary.path().join("source");
        let target = temporary.path().join("restored.txt");
        std::fs::create_dir(&source).expect("source should be created");
        std::fs::write(source.join("hello.txt"), "hello").expect("source file should be written");
        init_repository(&repository).expect("repository should initialize");
        run_backup(BackupRequest {
            paths: vec![source],
            repo: repository.clone(),
            policy_ignore_patterns: Vec::new(),
            fail_on_changed_file: false,
        })
        .expect("backup should run");
        let mut app = app_for_repository(repository).expect("app should load repository");
        app.view = TuiView::Browser;
        app.focus = TuiFocus::Files;
        app.restore_target = Some(target.clone());

        app.handle_key(KeyCode::Char('r'));
        app.handle_key(KeyCode::Char('y'));

        assert_eq!(app.restore_confirmation, RestoreConfirmation::Confirmed);
        assert_eq!(
            std::fs::read_to_string(&target).expect("target should be restored"),
            "hello"
        );
        let result = app
            .restore_result
            .as_ref()
            .expect("restore result should be recorded");
        assert_eq!(result.files, 1);
        assert_eq!(result.bytes, 5);
    }

    #[test]
    fn restore_confirmation_blocks_navigation_until_cancelled() {
        let mut app = browser_app_with_snapshots(2);

        press_keys(&mut app, [KeyCode::Char('r'), KeyCode::Down]);

        assert_eq!(app.selected_snapshot_id(), Some("snap_001"));
        assert_eq!(app.restore_confirmation, RestoreConfirmation::Awaiting);

        press_keys(&mut app, [KeyCode::Char('n'), KeyCode::Down]);

        assert_eq!(app.selected_snapshot_id(), Some("snap_002"));
        assert!(app.restore_plan.is_none());
        assert_eq!(app.restore_confirmation, RestoreConfirmation::None);
    }

    #[test]
    fn changing_selection_clears_confirmed_restore_preview() {
        let mut app = browser_app_with_snapshots(2);

        app.handle_key(KeyCode::Char('r'));
        app.restore_confirmation = RestoreConfirmation::Confirmed;
        app.handle_key(KeyCode::Down);

        assert_eq!(app.selected_snapshot_id(), Some("snap_002"));
        assert!(app.restore_plan.is_none());
        assert_eq!(app.restore_confirmation, RestoreConfirmation::None);
    }

    #[test]
    fn render_snapshot_browser_includes_selection_and_snapshot_fields() {
        let mut app = browser_app_with_snapshots(2);
        app.handle_key(KeyCode::Down);
        let backend = TestBackend::new(130, 20);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        let rendered = render_to_string(&mut terminal, &app);

        assert!(rendered.contains("TraceBack terminal browser"));
        assert!(rendered.contains("repo_test"));
        assert!(rendered.contains("Snapshots: 2"));
        assert!(rendered.contains("selected: snap_002"));
        assert!(rendered.contains("> snap_002"));
        assert!(rendered.contains("logical"));
        assert!(rendered.contains("source-2"));
    }

    #[test]
    fn detail_panel_follows_selected_snapshot() {
        let mut app = browser_app_with_snapshots(2);
        app.handle_key(KeyCode::Down);
        let backend = TestBackend::new(110, 30);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        let rendered = render_to_string(&mut terminal, &app);

        assert!(rendered.contains("Snapshot Details"));
        assert!(rendered.contains("ID: snap_002"));
        assert!(rendered.contains("State: complete"));
        assert!(rendered.contains("Files: 2"));
        assert!(rendered.contains("Directories: 1"));
        assert!(rendered.contains("Symlinks: 1"));
        assert!(rendered.contains("Chunk refs: 3"));
        assert!(rendered.contains("Unique chunks: 2"));
        assert!(rendered.contains("Warnings: none recorded"));
    }

    #[test]
    fn render_file_browser_includes_file_metadata() {
        let mut app = browser_app_with_snapshots(1);
        app.handle_key(KeyCode::Tab);
        app.handle_key(KeyCode::Down);
        let backend = TestBackend::new(150, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        let rendered = render_to_string(&mut terminal, &app);

        assert!(rendered.contains("Files *"));
        assert!(rendered.contains("> file"));
        assert!(rendered.contains("source/file-b.txt"));
        assert!(rendered.contains("Selected entry:"));
        assert!(rendered.contains("Content hash: hash-b"));
        assert!(rendered.contains("Chunks: 2"));
        assert!(rendered.contains("First chunk: aaaaaaaaaaaaaaaa..."));
    }

    #[test]
    fn render_restore_preview_includes_target_command_and_safety_text() {
        let mut app = browser_app_with_snapshots(1);
        app.handle_key(KeyCode::Char('r'));
        let backend = TestBackend::new(170, 42);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        let rendered = render_to_string(&mut terminal, &app);

        assert!(rendered.contains("Restore preview:"));
        assert!(rendered.contains("Status: awaiting confirmation"));
        assert!(rendered.contains("Safety: restore writes only after y confirmation."));
        assert!(rendered.contains(&format!(
            "Target: {}",
            PathBuf::from(".")
                .join("traceback-restore")
                .join("snap_001")
                .display()
        )));
        assert!(rendered.contains("Command: traceback restore snap_001"));
    }

    #[test]
    fn render_health_check_includes_result_and_counts() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        init_repository(&repository).expect("repository should initialize");
        let mut app = app_for_repository(repository).expect("app should load repository");
        app.view = TuiView::HealthCheck;
        app.run_health_check();
        let backend = TestBackend::new(130, 30);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        let rendered = render_to_string(&mut terminal, &app);

        assert!(rendered.contains("Repository Health"));
        assert!(rendered.contains("Result: PASS"));
        assert!(rendered.contains("Manifests checked: 0"));
        assert!(rendered.contains("Chunks verified: 0"));
        assert!(rendered.contains("No issues found."));
        assert!(rendered.contains("Enter rerun check"));
    }

    #[test]
    fn render_doctor_report_includes_score_and_findings() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        init_repository(&repository).expect("repository should initialize");
        let mut app = app_for_repository(repository).expect("app should load repository");
        app.view = TuiView::DoctorReport;
        app.run_doctor_report();
        let backend = TestBackend::new(150, 34);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        let rendered = render_to_string(&mut terminal, &app);

        assert!(rendered.contains("Doctor Report"));
        assert!(rendered.contains("Health score:"));
        assert!(rendered.contains("Latest snapshot: <none>"));
        assert!(rendered.contains("Recorded rehearsal evidence:"));
        assert!(rendered.contains("Findings:"));
        assert!(rendered.contains("Recommendation:"));
        assert!(rendered.contains("Enter rerun doctor"));
    }

    #[test]
    fn snapshot_diff_screen_runs_diff_between_selected_snapshots() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        let source = temporary.path().join("source");
        std::fs::create_dir(&source).expect("source should be created");
        std::fs::write(source.join("hello.txt"), "hello").expect("source file should be written");
        init_repository(&repository).expect("repository should initialize");
        run_backup(BackupRequest {
            paths: vec![source.clone()],
            repo: repository.clone(),
            policy_ignore_patterns: Vec::new(),
            fail_on_changed_file: false,
        })
        .expect("first backup should run");
        std::fs::write(source.join("hello.txt"), "hello world")
            .expect("source file should be changed");
        run_backup(BackupRequest {
            paths: vec![source],
            repo: repository.clone(),
            policy_ignore_patterns: Vec::new(),
            fail_on_changed_file: false,
        })
        .expect("second backup should run");
        let mut app = app_for_repository(repository).expect("app should load repository");

        select_menu_action(&mut app, MenuAction::CompareSnapshots);
        app.handle_key(KeyCode::Enter);

        assert_eq!(app.view, TuiView::SnapshotDiff);
        let result = app
            .diff_result
            .as_ref()
            .expect("diff result should be recorded");
        assert_eq!(result.modified.len(), 1);
        assert_eq!(result.modified[0].path, "source/hello.txt");
        assert_eq!(
            app.status_message.as_deref(),
            Some("Snapshot diff found 1 changed path(s).")
        );
    }

    #[test]
    fn render_snapshot_diff_includes_selection_and_result() {
        let mut app = app_with_snapshots(2);
        app.view = TuiView::SnapshotDiff;
        app.diff_result = Some(DiffRunSummary {
            old_snapshot_id: "snap_001".to_owned(),
            new_snapshot_id: "snap_002".to_owned(),
            added: Vec::new(),
            removed: Vec::new(),
            modified: vec![DiffEntrySummary {
                path: "source/file-a.txt".to_owned(),
                byte_delta: 5,
                type_changed: false,
                content_changed: true,
            }],
            unchanged: 3,
        });
        let backend = TestBackend::new(150, 34);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        let rendered = render_to_string(&mut terminal, &app);

        assert!(rendered.contains("Snapshot Diff"));
        assert!(rendered.contains("> Old: snap_001"));
        assert!(rendered.contains("New: snap_002"));
        assert!(rendered.contains("Diff result:"));
        assert!(rendered.contains("Modified: 1"));
        assert!(rendered.contains("source/file-a.txt"));
        assert!(rendered.contains("content"));
    }

    #[test]
    fn app_for_repository_validates_and_loads_repository() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        init_repository(&repository).expect("repository should initialize");

        let app = app_for_repository(repository).expect("tui app should prepare");

        assert_eq!(app.snapshot_count(), 0);
        assert_eq!(app.view, TuiView::MainMenu);
        assert!(!app.repository_id.is_empty());
    }

    #[test]
    fn app_for_repository_rejects_invalid_repository() {
        let temporary = tempdir().expect("temporary directory should be created");

        let error = app_for_repository(temporary.path().join("missing"))
            .expect_err("missing repository should fail");

        assert!(error.to_string().contains("config"));
    }

    #[test]
    fn restore_command_quotes_paths_with_spaces() {
        let mut app = TuiApp::new(
            PathBuf::from("./repo with spaces"),
            config(),
            vec![manifest(1)],
        );
        app.view = TuiView::Browser;

        let plan = app
            .build_restore_plan()
            .expect("snapshot restore plan should build");

        assert_eq!(
            plan.command,
            format!(
                "traceback restore snap_001 --repo \"./repo with spaces\" --target {}",
                PathBuf::from(".")
                    .join("traceback-restore")
                    .join("snap_001")
                    .display()
            )
        );
    }

    #[test]
    fn render_main_menu_includes_guided_actions() {
        let app = app_with_snapshots(2);
        let backend = TestBackend::new(140, 30);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        let rendered = render_to_string(&mut terminal, &app);

        assert!(rendered.contains("TraceBack guided terminal"));
        assert!(rendered.contains("Main Menu"));
        assert!(rendered.contains("> Browse snapshots"));
        assert!(rendered.contains("Change repository"));
        assert!(rendered.contains("Initialize repository"));
        assert!(rendered.contains("Create backup"));
        assert!(rendered.contains("Restore files"));
        assert!(rendered.contains("Rehearse restore"));
        assert!(rendered.contains("Check repository health"));
        assert!(rendered.contains("Doctor report"));
        assert!(rendered.contains("Compare snapshots"));
        assert!(rendered.contains("Exit"));
    }

    #[test]
    fn render_main_menu_uses_teal_accent() {
        let app = app_with_snapshots(1);
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        terminal
            .draw(|frame| render(frame, &app))
            .expect("frame should render");

        assert!(
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .any(|cell| cell.fg == TEAL)
        );
    }

    fn press_keys<const N: usize>(app: &mut TuiApp, keys: [KeyCode; N]) {
        for key in keys {
            app.handle_key(key);
        }
    }

    fn select_menu_action(app: &mut TuiApp, action: MenuAction) {
        app.selected_menu_item = MENU_ITEMS
            .iter()
            .position(|item| item.action == action)
            .expect("menu action should exist");
        app.handle_key(KeyCode::Enter);
    }

    fn replace_input(app: &mut TuiApp, value: &str) {
        let input = app
            .path_input
            .as_mut()
            .expect("path input should be active");
        input.value.clear();
        input.value.push_str(value);
        input.error = None;
    }

    fn render_to_string(terminal: &mut Terminal<TestBackend>, app: &TuiApp) -> String {
        terminal
            .draw(|frame| render(frame, app))
            .expect("frame should render");
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    fn app_with_snapshots(count: usize) -> TuiApp {
        TuiApp::new(
            PathBuf::from("./repo"),
            config(),
            (1..=count).map(manifest).collect(),
        )
    }

    fn browser_app_with_snapshots(count: usize) -> TuiApp {
        let mut app = app_with_snapshots(count);
        app.view = TuiView::Browser;
        app
    }

    fn config() -> RepositoryConfig {
        RepositoryConfig {
            repository_id: "repo_test".to_owned(),
            format_version: 0,
            created_at: "2026-06-19T00:00:00Z".to_owned(),
            hash_algorithm: "blake3".to_owned(),
            chunking: "fixed".to_owned(),
            chunk_size_bytes: 4 * 1024 * 1024,
            compression: "zstd".to_owned(),
            compression_level: 3,
            encrypted: false,
            encryption: None,
        }
    }

    fn manifest(index: usize) -> SnapshotManifest {
        SnapshotManifest {
            manifest_version: 0,
            snapshot_id: format!("snap_{index:03}"),
            created_at: format!("2026-06-{index:02}T00:00:00Z"),
            state: "complete".to_owned(),
            sources: vec![format!("source-{index}")],
            files: vec![
                file_entry(
                    "source/file-a.txt",
                    FileType::File,
                    5,
                    Some("hash-a"),
                    vec!["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],
                    None,
                ),
                file_entry(
                    "source/file-b.txt",
                    FileType::File,
                    5,
                    Some("hash-b"),
                    vec![
                        "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                        "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                    ],
                    None,
                ),
                file_entry("source/dir", FileType::Directory, 0, None, Vec::new(), None),
                file_entry(
                    "source/link",
                    FileType::Symlink,
                    0,
                    None,
                    Vec::new(),
                    Some("source/file-a.txt"),
                ),
            ],
            summary: traceback_repo::ManifestSummary {
                file_count: 2,
                logical_bytes: index as u64 * 10,
                newly_stored_bytes: index as u64,
            },
        }
    }

    fn file_entry(
        path: &str,
        file_type: FileType,
        size: u64,
        content_hash: Option<&str>,
        chunks: Vec<&str>,
        symlink_target: Option<&str>,
    ) -> FileEntry {
        FileEntry {
            path: path.to_owned(),
            file_type,
            size,
            modified_at: None,
            permissions: None,
            content_hash: content_hash.map(ToOwned::to_owned),
            chunks: chunks.into_iter().map(ToOwned::to_owned).collect(),
            symlink_target: symlink_target.map(ToOwned::to_owned),
        }
    }
}
