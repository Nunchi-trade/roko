#![allow(missing_docs)]

//! Simple ratatui-based demo mode.

use std::collections::BTreeMap;
use std::future::Future;
use std::io::{self, Stdout};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self, Event as CEvent, KeyCode};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, List, ListItem, Paragraph, Wrap};
use tokio::sync::mpsc;

use crate::events::{ChannelEmitter, DemoEvent, EventEmitter, KnowledgeNode};

#[derive(Default)]
struct AgentState {
    model: String,
    reputation: String,
    earned: u128,
    status: String,
}

#[derive(Default)]
struct KnowledgeState {
    insight_count: usize,
    latest: Vec<String>,
}

#[derive(Default)]
struct EconomicsState {
    total_distributed: u128,
    treasury: u128,
}

#[derive(Default)]
struct TuiState {
    current_round: u32,
    active_winner: Option<String>,
    agents: BTreeMap<String, AgentState>,
    log: Vec<String>,
    log_scroll: u16,
    knowledge: KnowledgeState,
    economics: EconomicsState,
}

impl TuiState {
    fn apply_event(&mut self, event: DemoEvent) {
        match event {
            DemoEvent::RoundStarted { round, .. } => {
                self.current_round = round;
                self.push_log(format!("Round {round} started"));
            }
            DemoEvent::KnowledgeQueried { worker, .. } => {
                self.agent_mut(&worker).status = "Querying".into();
                self.push_log(format!("{worker} queried shared knowledge"));
            }
            DemoEvent::AgentBid {
                worker,
                model,
                confidence,
                ..
            } => {
                let agent = self.agent_mut(&worker);
                agent.model = model;
                agent.status = format!("Bidding ({confidence:.2})");
                self.push_log(format!("{worker} submitted a route proposal"));
            }
            DemoEvent::JobAssigned { worker, .. } => {
                self.active_winner = Some(worker.clone());
                self.agent_mut(&worker).status = "Winner".into();
                self.push_log(format!("{worker} won the job"));
            }
            DemoEvent::ExecutionStarted { worker, .. } => {
                self.agent_mut(&worker).status = "Executing".into();
                self.push_log(format!("{worker} started execution"));
            }
            DemoEvent::ExecutionCompleted {
                worker,
                actual_output_eth,
                ..
            } => {
                self.agent_mut(&worker).status = format!("Output {actual_output_eth:.2} ETH");
                self.push_log(format!("{worker} produced {actual_output_eth:.2} ETH"));
            }
            DemoEvent::FeesDistributed {
                amount_wei,
                treasury_share_wei,
                agent_share_wei,
                ..
            } => {
                self.economics.total_distributed += parse_u128(&amount_wei);
                self.economics.treasury += parse_u128(&treasury_share_wei);
                if let Some(winner) = self.active_winner.clone() {
                    self.agent_mut(&winner).earned += parse_u128(&agent_share_wei);
                }
                self.push_log("fees distributed across the flywheel".into());
            }
            DemoEvent::ReputationUpdated { worker, reputation } => {
                self.agent_mut(&worker).reputation = reputation.clone();
                self.push_log(format!("{worker} reputation updated to {reputation}"));
            }
            DemoEvent::InsightPosted {
                poster, insight_id, ..
            } => {
                self.knowledge
                    .latest
                    .push(format!("{poster} posted insight #{insight_id}"));
                self.knowledge.latest.truncate(6);
                self.push_log(format!("{poster} posted new knowledge"));
            }
            DemoEvent::InsightConfirmed {
                confirmer,
                insight_id,
                ..
            } => {
                self.push_log(format!("{confirmer} confirmed insight #{insight_id}"));
            }
            DemoEvent::KnowledgeGraphUpdate { nodes, .. } => {
                self.update_knowledge(nodes);
                self.push_log("knowledge graph refreshed".into());
            }
            DemoEvent::AgentSlashed {
                worker, amount_wei, ..
            } => {
                self.agent_mut(&worker).status = "Slashed".into();
                self.push_log(format!("{worker} slashed by {amount_wei} wei"));
            }
            DemoEvent::ScenarioCompleted {
                improvement_bps, ..
            } => {
                self.push_log(format!("scenario completed with +{improvement_bps} bps"));
            }
            DemoEvent::Error { message } => self.push_log(format!("error: {message}")),
            _ => {}
        }
    }

    fn agent_mut(&mut self, name: &str) -> &mut AgentState {
        self.agents
            .entry(name.into())
            .or_insert_with(|| AgentState {
                status: "Idle".into(),
                ..AgentState::default()
            })
    }

    fn push_log(&mut self, message: String) {
        self.log.push(message);
        if self.log.len() > 200 {
            let overflow = self.log.len() - 200;
            self.log.drain(0..overflow);
        }
    }

    fn update_knowledge(&mut self, nodes: Vec<KnowledgeNode>) {
        self.knowledge.insight_count = nodes.len();
        self.knowledge.latest = nodes
            .into_iter()
            .take(6)
            .map(|node| format!("{} ({})", node.id, node.poster))
            .collect();
    }
}

/// Run a TUI while a demo future emits events into the supplied emitter.
pub async fn run_tui<F, Fut>(title: &str, scenario_future: F) -> anyhow::Result<()>
where
    F: FnOnce(Arc<dyn EventEmitter>) -> Fut,
    Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
{
    let (sender, mut receiver) = mpsc::channel::<DemoEvent>(512);
    let emitter = Arc::new(ChannelEmitter::new(sender));
    let mut task = Some(tokio::spawn(scenario_future(emitter)));

    let _guard = TerminalGuard::enter()?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;
    let mut state = TuiState::default();
    let mut handle_done = false;
    let mut idle_ticks = 0u8;

    loop {
        while let Ok(event) = receiver.try_recv() {
            state.apply_event(event);
            idle_ticks = 0;
        }

        terminal.draw(|frame| render(frame, &state, title))?;

        if !handle_done {
            if task
                .as_ref()
                .is_some_and(tokio::task::JoinHandle::is_finished)
            {
                task.take()
                    .ok_or_else(|| anyhow::anyhow!("missing TUI task handle"))?
                    .await??;
                handle_done = true;
            }
        } else {
            idle_ticks = idle_ticks.saturating_add(1);
            if idle_ticks >= 3 {
                break;
            }
        }

        if event::poll(Duration::from_millis(50))? {
            if let CEvent::Key(key) = event::read()? {
                match key.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('j') => {
                        state.log_scroll = state.log_scroll.saturating_add(1);
                    }
                    KeyCode::Char('k') => {
                        state.log_scroll = state.log_scroll.saturating_sub(1);
                    }
                    _ => {}
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }

    Ok(())
}

fn render(frame: &mut ratatui::Frame<'_>, state: &TuiState, title: &str) {
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Percentage(58),
            Constraint::Percentage(39),
        ])
        .split(frame.area());
    let middle = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(28), Constraint::Percentage(72)])
        .split(layout[1]);
    let bottom = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(28), Constraint::Percentage(72)])
        .split(layout[2]);

    let title_text = Paragraph::new(Line::from(vec![
        Span::styled(
            title,
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("  Round {}", state.current_round)),
        Span::raw("  q/Esc quit · j/k scroll"),
    ]))
    .block(Block::default().borders(Borders::ALL).title("Demo"));
    frame.render_widget(title_text, layout[0]);

    let agent_items = state
        .agents
        .iter()
        .map(|(name, agent)| {
            ListItem::new(vec![
                Line::from(Span::styled(
                    name.clone(),
                    Style::default().fg(Color::Yellow),
                )),
                Line::from(format!(
                    "{} · rep {} · earned {}",
                    blank_if_empty(&agent.model, "unknown"),
                    blank_if_empty(&agent.reputation, "n/a"),
                    agent.earned
                )),
                Line::from(blank_if_empty(&agent.status, "Idle")),
            ])
        })
        .collect::<Vec<_>>();
    frame.render_widget(
        List::new(agent_items).block(Block::default().borders(Borders::ALL).title("Agents")),
        middle[0],
    );

    let log_start = state
        .log
        .len()
        .saturating_sub(14 + state.log_scroll as usize);
    let log_items = state.log[log_start..]
        .iter()
        .map(|entry| ListItem::new(entry.clone()))
        .collect::<Vec<_>>();
    frame.render_widget(
        List::new(log_items).block(Block::default().borders(Borders::ALL).title("Activity")),
        middle[1],
    );

    let knowledge = Paragraph::new(
        state
            .knowledge
            .latest
            .iter()
            .map(|line| Line::from(line.as_str()))
            .collect::<Vec<_>>(),
    )
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!("Knowledge ({})", state.knowledge.insight_count)),
    )
    .wrap(Wrap { trim: true });
    frame.render_widget(knowledge, bottom[0]);

    let economics = Paragraph::new(vec![
        Line::from(format!(
            "Total distributed: {}",
            state.economics.total_distributed
        )),
        Line::from(format!("Treasury: {}", state.economics.treasury)),
        Line::from(format!(
            "Current winner: {}",
            state.active_winner.clone().unwrap_or_else(|| "n/a".into())
        )),
    ])
    .block(Block::default().borders(Borders::ALL).title("Economics"))
    .wrap(Wrap { trim: true });
    frame.render_widget(economics, bottom[1]);
}

fn blank_if_empty<'a>(value: &'a str, fallback: &'a str) -> &'a str {
    if value.is_empty() { fallback } else { value }
}

fn parse_u128(value: &str) -> u128 {
    u128::from_str(value).unwrap_or(0)
}

struct TerminalGuard {
    stdout: Stdout,
}

impl TerminalGuard {
    fn enter() -> anyhow::Result<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        Ok(Self { stdout })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(self.stdout, LeaveAlternateScreen);
    }
}
