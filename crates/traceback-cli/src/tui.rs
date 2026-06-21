use std::{
    collections::BTreeSet,
    error::Error,
    io::{self, Stdout},
    path::PathBuf,
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
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use traceback_repo::{
    FileType, RepositoryConfig, RepositoryError, SnapshotManifest, list_manifests,
    validate_repository,
};

#[derive(Debug)]
pub struct TuiApp {
    repository: PathBuf,
    repository_id: String,
    snapshots: Vec<SnapshotRow>,
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

    fn handle_key(&mut self, code: KeyCode) {
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

    fn handle_restore_confirmation_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                self.restore_confirmation = RestoreConfirmation::Confirmed;
            }
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
        if self.restore_confirmation == RestoreConfirmation::Awaiting {
            return "Restore preview only | y confirm command preparation | n/Esc cancel | q quit"
                .to_owned();
        }

        if self.restore_confirmation == RestoreConfirmation::Confirmed {
            return "Restore command prepared | n clear preview | q/Esc quit".to_owned();
        }

        if self.filtering_files {
            return "Type path filter | Enter accept | Backspace edit | Esc stop filtering"
                .to_owned();
        }

        if self.show_help {
            "Tab focus | Up/Down or j/k move | Home/End jump | / filter files | c clear filter | r restore preview | q/Esc quit | ?/F1 hide help".to_owned()
        } else {
            "?/F1 help | q/Esc quit".to_owned()
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
        let target = restore_target(&snapshot.snapshot_id, selected_path.as_deref());
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

    let title = Paragraph::new(Line::from(vec![
        Span::styled("TraceBack", Style::default().add_modifier(Modifier::BOLD)),
        Span::raw(" terminal browser"),
    ]))
    .alignment(Alignment::Center)
    .block(Block::default().borders(Borders::ALL));
    frame.render_widget(title, chunks[0]);

    let body = Paragraph::new(vec![
        Line::from(format!("Repository: {}", app.repository.display())),
        Line::from(format!("Repository ID: {}", app.repository_id)),
        Line::from(format!(
            "Snapshots: {}{}",
            app.snapshot_count(),
            app.selected_snapshot_id()
                .map(|snapshot| format!(" | selected: {snapshot}"))
                .unwrap_or_default()
        )),
    ])
    .wrap(Wrap { trim: true })
    .block(Block::default().title("Overview").borders(Borders::ALL));
    frame.render_widget(body, chunks[1]);

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
                .borders(Borders::ALL),
        );
    frame.render_widget(snapshots, browser[0]);

    let files = Paragraph::new(file_lines(app))
        .wrap(Wrap { trim: false })
        .block(
            Block::default()
                .title(file_browser_title(app))
                .borders(Borders::ALL),
        );
    frame.render_widget(files, browser[1]);

    let details = Paragraph::new(snapshot_detail_lines(app))
        .wrap(Wrap { trim: true })
        .block(
            Block::default()
                .title("Snapshot Details")
                .borders(Borders::ALL),
        );
    frame.render_widget(details, browser[2]);

    let footer = Paragraph::new(app.help_text())
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(footer, chunks[3]);
}

fn focused_title(title: &str, focused: bool) -> String {
    if focused {
        format!("{title} *")
    } else {
        title.to_owned()
    }
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
        Line::from("Warnings: none recorded"),
        Line::from(""),
    ];

    if let Some(file) = app.selected_file() {
        lines.extend([
            Line::from("Selected entry:"),
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
        RestoreConfirmation::Confirmed => "command prepared",
    };
    let scope = plan.selected_path.as_deref().unwrap_or("<entire snapshot>");

    vec![
        Line::from(""),
        Line::from("Restore preview:"),
        Line::from(format!("Status: {status}")),
        Line::from("Safety: preview only; no TUI writes."),
        Line::from(format!("Snapshot: {}", plan.snapshot_id)),
        Line::from(format!("Path: {scope}")),
        Line::from(format!("Target: {}", plan.target.display())),
        Line::from(format!("Command: {}", plan.command)),
    ]
}

fn snapshot_lines(app: &TuiApp) -> Vec<Line<'static>> {
    if app.snapshots.is_empty() {
        return vec![Line::from("No snapshots found.")];
    }

    app.snapshots
        .iter()
        .enumerate()
        .map(|(index, snapshot)| {
            let marker = if index == app.selected_snapshot {
                ">"
            } else {
                " "
            };
            Line::from(format!(
                "{marker} {:<36}  {:<20}  logical {:>8} B  stored {:>8} B  {}",
                snapshot.snapshot_id,
                display_created_at(&snapshot.created_at),
                snapshot.logical_bytes,
                snapshot.newly_stored_bytes,
                snapshot.sources
            ))
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
            let marker = if index == app.selected_file { ">" } else { " " };
            Line::from(format!(
                "{marker} {:<4} {:>8} B  {}",
                display_file_type(file.file_type),
                file.size,
                file.path
            ))
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

fn restore_target(snapshot_id: &str, selected_path: Option<&str>) -> PathBuf {
    let mut target = PathBuf::from("traceback-restore").join(snapshot_id);
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
        FileEntry, FileType, RepositoryConfig, SnapshotManifest, init_repository,
    };

    use super::{RestoreConfirmation, TuiApp, app_for_repository, render};

    #[test]
    fn app_starts_with_help_and_quits_on_q_or_escape() {
        let mut app = app_with_snapshots(0);

        assert!(!app.should_quit());
        assert_eq!(
            app.help_text(),
            "Tab focus | Up/Down or j/k move | Home/End jump | / filter files | c clear filter | r restore preview | q/Esc quit | ?/F1 hide help"
        );

        app.handle_key(KeyCode::Char('?'));
        assert_eq!(app.help_text(), "?/F1 help | q/Esc quit");
        assert!(!app.should_quit());

        app.handle_key(KeyCode::Esc);
        assert!(app.should_quit());
    }

    #[test]
    fn navigation_selects_snapshots_without_leaving_bounds() {
        let mut app = app_with_snapshots(3);

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
        let mut app = app_with_snapshots(0);

        app.handle_key(KeyCode::Down);
        app.handle_key(KeyCode::End);

        assert_eq!(app.selected_snapshot_id(), None);
        assert_eq!(app.selected_snapshot, 0);
    }

    #[test]
    fn tab_switches_focus_and_file_navigation_tracks_selected_snapshot() {
        let mut app = app_with_snapshots(2);

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
        let mut app = app_with_snapshots(1);

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
            "Tab focus | Up/Down or j/k move | Home/End jump | / filter files | c clear filter | r restore preview | q/Esc quit | ?/F1 hide help"
        );

        app.handle_key(KeyCode::Char('c'));

        assert_eq!(app.filtered_file_count(), 4);
        assert!(app.file_filter.is_empty());
    }

    #[test]
    fn restore_preview_defaults_to_snapshot_and_requires_confirmation() {
        let mut app = app_with_snapshots(1);

        app.handle_key(KeyCode::Char('r'));

        let plan = app
            .restore_plan
            .as_ref()
            .expect("restore plan should be prepared");
        assert_eq!(app.restore_confirmation, RestoreConfirmation::Awaiting);
        assert_eq!(plan.snapshot_id, "snap_001");
        assert_eq!(plan.selected_path, None);
        let expected_target = PathBuf::from("traceback-restore").join("snap_001");
        assert_eq!(plan.target, expected_target);
        assert_eq!(
            plan.command,
            format!(
                "traceback restore snap_001 --repo ./repo --target {}",
                expected_target.display()
            )
        );
        assert!(!app.should_quit());

        app.handle_key(KeyCode::Char('y'));

        assert_eq!(app.restore_confirmation, RestoreConfirmation::Confirmed);
        assert_eq!(
            app.help_text(),
            "Restore command prepared | n clear preview | q/Esc quit"
        );

        app.handle_key(KeyCode::Char('n'));

        assert!(app.restore_plan.is_none());
        assert_eq!(app.restore_confirmation, RestoreConfirmation::None);
    }

    #[test]
    fn file_restore_preview_uses_selected_path_and_clear_target() {
        let mut app = app_with_snapshots(1);

        app.handle_key(KeyCode::Tab);
        app.handle_key(KeyCode::Down);
        app.handle_key(KeyCode::Char('r'));

        let plan = app
            .restore_plan
            .as_ref()
            .expect("selected file restore plan should be prepared");
        assert_eq!(app.restore_confirmation, RestoreConfirmation::Awaiting);
        assert_eq!(plan.selected_path.as_deref(), Some("source/file-b.txt"));
        let expected_target = PathBuf::from("traceback-restore")
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
    fn render_snapshot_browser_includes_selection_and_snapshot_fields() {
        let mut app = app_with_snapshots(2);
        app.handle_key(KeyCode::Down);
        let backend = TestBackend::new(130, 20);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        terminal
            .draw(|frame| render(frame, &app))
            .expect("frame should render");
        let buffer = terminal.backend().buffer();
        let rendered = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

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
        let mut app = app_with_snapshots(2);
        app.handle_key(KeyCode::Down);
        let backend = TestBackend::new(110, 30);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        terminal
            .draw(|frame| render(frame, &app))
            .expect("frame should render");
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

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
        let mut app = app_with_snapshots(1);
        app.handle_key(KeyCode::Tab);
        app.handle_key(KeyCode::Down);
        let backend = TestBackend::new(150, 36);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        terminal
            .draw(|frame| render(frame, &app))
            .expect("frame should render");
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

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
        let mut app = app_with_snapshots(1);
        app.handle_key(KeyCode::Char('r'));
        let backend = TestBackend::new(170, 42);
        let mut terminal = Terminal::new(backend).expect("test terminal should initialize");

        terminal
            .draw(|frame| render(frame, &app))
            .expect("frame should render");
        let rendered = terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();

        assert!(rendered.contains("Restore preview:"));
        assert!(rendered.contains("Status: awaiting confirmation"));
        assert!(rendered.contains("Safety: preview only; no TUI writes."));
        assert!(rendered.contains(&format!(
            "Target: {}",
            PathBuf::from("traceback-restore")
                .join("snap_001")
                .display()
        )));
        assert!(rendered.contains("Command: traceback restore snap_001"));
    }

    #[test]
    fn app_for_repository_validates_and_loads_repository() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        init_repository(&repository).expect("repository should initialize");

        let app = app_for_repository(repository).expect("tui app should prepare");

        assert_eq!(app.snapshot_count(), 0);
        assert!(!app.repository_id.is_empty());
    }

    #[test]
    fn app_for_repository_rejects_invalid_repository() {
        let temporary = tempdir().expect("temporary directory should be created");

        let error = app_for_repository(temporary.path().join("missing"))
            .expect_err("missing repository should fail");

        assert!(error.to_string().contains("config"));
    }

    fn app_with_snapshots(count: usize) -> TuiApp {
        TuiApp::new(
            PathBuf::from("./repo"),
            config(),
            (1..=count).map(manifest).collect(),
        )
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
