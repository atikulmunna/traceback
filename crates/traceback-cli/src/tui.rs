use std::{
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
    RepositoryConfig, RepositoryError, SnapshotManifest, list_manifests, validate_repository,
};

#[derive(Debug)]
pub struct TuiApp {
    repository: PathBuf,
    repository_id: String,
    snapshots: usize,
    show_help: bool,
    should_quit: bool,
}

impl TuiApp {
    pub fn new(
        repository: PathBuf,
        config: RepositoryConfig,
        manifests: Vec<SnapshotManifest>,
    ) -> Self {
        Self {
            repository,
            repository_id: config.repository_id,
            snapshots: manifests.len(),
            show_help: true,
            should_quit: false,
        }
    }

    fn handle_key(&mut self, code: KeyCode) {
        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('?') | KeyCode::F(1) => self.show_help = !self.show_help,
            _ => {}
        }
    }

    fn should_quit(&self) -> bool {
        self.should_quit
    }

    fn help_text(&self) -> &'static str {
        if self.show_help {
            "q/Esc quit | ?/F1 hide help"
        } else {
            "?/F1 help | q/Esc quit"
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
            Constraint::Min(5),
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
        Line::from(format!("Snapshots: {}", app.snapshots)),
        Line::from(""),
        Line::from("Snapshot browsing starts in T30. This scaffold validates the repository and keeps terminal controls safe."),
    ])
    .wrap(Wrap { trim: true })
    .block(Block::default().title("Overview").borders(Borders::ALL));
    frame.render_widget(body, chunks[1]);

    let footer = Paragraph::new(app.help_text())
        .alignment(Alignment::Center)
        .block(Block::default().borders(Borders::ALL));
    frame.render_widget(footer, chunks[2]);
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crossterm::event::KeyCode;
    use ratatui::{Terminal, backend::TestBackend};
    use tempfile::tempdir;
    use traceback_repo::{RepositoryConfig, SnapshotManifest, init_repository};

    use super::{TuiApp, app_for_repository, render};

    #[test]
    fn app_starts_with_help_and_quits_on_q_or_escape() {
        let mut app = app_with_snapshots(0);

        assert!(!app.should_quit());
        assert_eq!(app.help_text(), "q/Esc quit | ?/F1 hide help");

        app.handle_key(KeyCode::Char('?'));
        assert_eq!(app.help_text(), "?/F1 help | q/Esc quit");
        assert!(!app.should_quit());

        app.handle_key(KeyCode::Esc);
        assert!(app.should_quit());
    }

    #[test]
    fn render_scaffold_includes_repository_identity_and_snapshot_count() {
        let app = app_with_snapshots(2);
        let backend = TestBackend::new(80, 20);
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
    }

    #[test]
    fn app_for_repository_validates_and_loads_repository() {
        let temporary = tempdir().expect("temporary directory should be created");
        let repository = temporary.path().join("repo");
        init_repository(&repository).expect("repository should initialize");

        let app = app_for_repository(repository).expect("tui app should prepare");

        assert_eq!(app.snapshots, 0);
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
            std::iter::repeat_with(manifest).take(count).collect(),
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

    fn manifest() -> SnapshotManifest {
        SnapshotManifest {
            manifest_version: 0,
            snapshot_id: "snap_test".to_owned(),
            created_at: "2026-06-19T00:00:00Z".to_owned(),
            state: "complete".to_owned(),
            sources: vec!["source".to_owned()],
            files: Vec::new(),
            summary: traceback_repo::ManifestSummary {
                file_count: 0,
                logical_bytes: 0,
                newly_stored_bytes: 0,
            },
        }
    }
}
