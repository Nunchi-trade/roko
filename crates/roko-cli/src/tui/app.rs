//! Interactive TUI application shell.

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

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

/// Interactive dashboard shell backed by the existing snapshot renderer.
#[derive(Debug)]
pub struct App {
    workdir: PathBuf,
    dashboard: DashboardScaffold,
    pages: PageRegistry,
    events: EventHandler,
    active_page: PageId,
    content_scroll: u16,
    should_quit: bool,
}

type TuiTerminal = Terminal<CrosstermBackend<io::Stdout>>;

impl App {
    /// Build a new app from a workspace root.
    #[must_use]
    pub fn new(root: impl AsRef<Path>) -> Self {
        let workdir = root.as_ref().to_path_buf();
        let dashboard = DashboardScaffold::new_in(&workdir);
        let active_page = dashboard.active_page();
        Self {
            pages: PageRegistry::from_dashboard(&dashboard),
            workdir,
            dashboard,
            events: EventHandler::new(Duration::from_millis(250)),
            active_page,
            content_scroll: 0,
            should_quit: false,
        }
    }

    /// Return the active page.
    #[must_use]
    pub const fn active_page(&self) -> PageId {
        self.active_page
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
        terminal
            .draw(|frame| self.draw(frame))
            .context("initial TUI draw")?;

        while !self.should_quit {
            match self.events.next().context("poll TUI event")? {
                Event::Key(key) => self.handle_key(key),
                Event::Resize(_, _) => {}
                Event::Tick => self.refresh_snapshot(),
            }

            terminal
                .draw(|frame| self.draw(frame))
                .context("TUI redraw")?;
        }

        Ok(())
    }

    fn draw(&self, frame: &mut Frame<'_>) {
        widgets::render_dashboard(
            frame,
            &self.dashboard,
            &self.pages,
            self.active_page,
            self.content_scroll,
        );
    }

    fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('r') => self.refresh_snapshot(),
            KeyCode::Left | KeyCode::Char('h') | KeyCode::BackTab => self.select_previous_page(),
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => self.select_next_page(),
            KeyCode::Up | KeyCode::Char('k') => {
                self.content_scroll = self.content_scroll.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.content_scroll = self.content_scroll.saturating_add(1);
            }
            KeyCode::PageUp => {
                self.content_scroll = self.content_scroll.saturating_sub(8);
            }
            KeyCode::PageDown => {
                self.content_scroll = self.content_scroll.saturating_add(8);
            }
            KeyCode::Home => self.content_scroll = 0,
            _ => {}
        }
    }

    fn select_next_page(&mut self) {
        self.active_page = self.pages.next(self.active_page);
        self.content_scroll = 0;
    }

    fn select_previous_page(&mut self) {
        self.active_page = self.pages.previous(self.active_page);
        self.content_scroll = 0;
    }

    fn refresh_snapshot(&mut self) {
        self.dashboard = DashboardScaffold::new_in(&self.workdir);
        if self.pages.is_empty() {
            self.pages = PageRegistry::from_dashboard(&self.dashboard);
        }
        if self.pages.scaffold(self.active_page).is_none() {
            self.active_page = self.dashboard.active_page();
        }
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
