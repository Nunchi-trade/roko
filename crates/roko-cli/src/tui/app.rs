//! Interactive TUI application shell.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture, KeyCode, KeyEvent};
use crossterm::execute;
use crossterm::terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode};
use ratatui::backend::CrosstermBackend;
use ratatui::prelude::*;
use ratatui::Terminal;

use super::dashboard::DashboardScaffold;
use super::event::{Event, EventHandler};
use super::pages::{PageId, PageRegistry};
use super::widgets;

/// Shared dashboard data model used by the TUI shell.
pub type DashboardData = DashboardScaffold;

/// Interactive dashboard shell backed by the existing snapshot renderer.
#[derive(Debug)]
pub struct App {
    workdir: PathBuf,
    /// Currently selected dashboard page.
    pub current_page: PageId,
    /// Shared dashboard data model, refreshed on tick.
    pub data: DashboardData,
    /// Whether the event loop should keep running.
    pub running: bool,
    /// Timestamp of the last data refresh.
    pub last_refresh: Instant,
    /// Per-page scroll position.
    pub scroll_offset: HashMap<PageId, u16>,
}

type TuiTerminal = Terminal<CrosstermBackend<io::Stdout>>;

impl App {
    /// Build a new app from a workspace root.
    #[must_use]
    pub fn new(root: impl AsRef<Path>) -> Self {
        let workdir = root.as_ref().to_path_buf();
        let data = DashboardScaffold::new_in(&workdir);
        let current_page = data.active_page();
        Self {
            workdir,
            current_page,
            data,
            running: true,
            last_refresh: Instant::now(),
            scroll_offset: HashMap::new(),
        }
    }

    /// Return the active page.
    #[must_use]
    pub const fn current_page(&self) -> PageId {
        self.current_page
    }

    /// Return the active page.
    #[must_use]
    pub const fn active_page(&self) -> PageId {
        self.current_page
    }

    /// Run the terminal UI until the user quits.
    pub fn run(mut self) -> Result<()> {
        let mut terminal = Self::enter_terminal()?;
        let result = self.main_loop(&mut terminal);
        let cleanup = Self::leave_terminal(&mut terminal);

        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(err), Ok(())) => Err(err),
            (Ok(()), Err(err)) => Err(err),
            (Err(err), Err(_cleanup_err)) => Err(err),
        }
    }

    fn main_loop(&mut self, terminal: &mut TuiTerminal) -> Result<()> {
        let mut events = EventHandler::new(Duration::from_millis(250));
        terminal
            .draw(|frame| self.draw(frame))
            .context("initial TUI draw")?;

        while self.running {
            match events.next().context("poll TUI event")? {
                Event::Key(key) => self.handle_key(key),
                Event::Resize(_, _) => {}
                Event::Tick => self.refresh_snapshot_if_needed(),
            }

            terminal
                .draw(|frame| self.draw(frame))
                .context("TUI redraw")?;
        }

        Ok(())
    }

    fn draw(&self, frame: &mut Frame<'_>) {
        let pages = self.pages();
        widgets::render_dashboard(
            frame,
            &self.data,
            &pages,
            self.current_page,
            self.scroll_for(self.current_page),
        );
    }

    fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.running = false,
            KeyCode::Char('r') => self.refresh_snapshot(),
            KeyCode::Left | KeyCode::Char('h') | KeyCode::BackTab => self.select_previous_page(),
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => self.select_next_page(),
            KeyCode::Up | KeyCode::Char('k') => self.adjust_scroll(-1),
            KeyCode::Down | KeyCode::Char('j') => self.adjust_scroll(1),
            KeyCode::PageUp => self.adjust_scroll(-8),
            KeyCode::PageDown => self.adjust_scroll(8),
            KeyCode::Home => self.set_scroll(0),
            _ => {}
        }
    }

    fn select_next_page(&mut self) {
        let pages = self.pages();
        self.current_page = pages.next(self.current_page);
    }

    fn select_previous_page(&mut self) {
        let pages = self.pages();
        self.current_page = pages.previous(self.current_page);
    }

    fn refresh_snapshot(&mut self) {
        self.data = DashboardScaffold::new_in(&self.workdir);
        self.last_refresh = Instant::now();
        if self.pages().scaffold(self.current_page).is_none() {
            self.current_page = self.data.active_page();
        }
    }

    fn refresh_snapshot_if_needed(&mut self) {
        if self.last_refresh.elapsed() >= Duration::from_millis(250) {
            self.refresh_snapshot();
        }
    }

    fn pages(&self) -> PageRegistry {
        PageRegistry::from_dashboard(&self.data)
    }

    fn scroll_for(&self, page: PageId) -> u16 {
        self.scroll_offset.get(&page).copied().unwrap_or(0)
    }

    fn set_scroll(&mut self, value: u16) {
        self.scroll_offset.insert(self.current_page, value);
    }

    fn adjust_scroll(&mut self, delta: i16) {
        let current = self.scroll_for(self.current_page) as i32;
        let next = (current + delta as i32).max(0).min(u16::MAX as i32) as u16;
        self.scroll_offset.insert(self.current_page, next);
    }

    fn enter_terminal() -> Result<TuiTerminal> {
        enable_raw_mode().context("enable raw mode")?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen, EnableMouseCapture)
            .context("enter alternate screen")?;
        Terminal::new(CrosstermBackend::new(stdout)).context("create terminal")
    }

    fn leave_terminal(terminal: &mut TuiTerminal) -> Result<()> {
        disable_raw_mode().context("disable raw mode")?;
        execute!(
            terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        )
        .context("leave alternate screen")?;
        Ok(())
    }
}
