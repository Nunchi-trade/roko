//! REPL mode — interactive read-eval-print loop.
//!
//! Activated when stdin is a TTY and no positional prompt argument is given.
//! Reads lines from stdin, processes built-in commands (`:quit`, `:help`,
//! `:status`), and dispatches everything else as a prompt through the
//! universal loop.

use std::io::{self, BufRead, Write};

/// Built-in REPL commands (prefixed with `:`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplCommand {
    /// Exit the REPL.
    Quit,
    /// Print available commands.
    Help,
    /// Show session status.
    Status,
    /// Send a prompt to the agent.
    Prompt(String),
}

/// Interactive REPL session state.
#[derive(Debug)]
pub struct ReplMode {
    /// Session identifier for this REPL session.
    pub session_id: String,
    /// Number of prompts processed so far.
    pub prompt_count: usize,
    /// Whether the REPL is still running.
    running: bool,
}

impl ReplMode {
    /// Create a new REPL session with the given session ID.
    #[must_use]
    pub const fn new(session_id: String) -> Self {
        Self {
            session_id,
            prompt_count: 0,
            running: false,
        }
    }

    /// Parse a line of user input into a REPL command.
    #[must_use]
    pub fn parse_input(line: &str) -> ReplCommand {
        let trimmed = line.trim();
        match trimmed {
            ":quit" | ":q" | ":exit" => ReplCommand::Quit,
            ":help" | ":h" | ":?" => ReplCommand::Help,
            ":status" | ":s" => ReplCommand::Status,
            _ => ReplCommand::Prompt(trimmed.to_string()),
        }
    }

    /// Run the REPL loop, reading from the provided reader and writing to the
    /// provided writer. Returns the number of prompts processed.
    ///
    /// Each non-command line is collected but not actually dispatched to an
    /// agent — the caller is responsible for wiring `ReplCommand::Prompt`
    /// values into the universal loop.
    pub fn run<R: BufRead, W: Write>(
        &mut self,
        reader: &mut R,
        writer: &mut W,
    ) -> io::Result<Vec<ReplCommand>> {
        self.running = true;
        let mut commands = Vec::new();

        writeln!(writer, "roko repl (session: {})", self.session_id)?;
        writeln!(writer, "type :help for commands, :quit to exit")?;

        loop {
            if !self.running {
                break;
            }
            write!(writer, "roko> ")?;
            writer.flush()?;

            let mut line = String::new();
            let bytes = reader.read_line(&mut line)?;
            if bytes == 0 {
                // EOF
                break;
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }

            let cmd = Self::parse_input(trimmed);
            match &cmd {
                ReplCommand::Quit => {
                    writeln!(
                        writer,
                        "goodbye ({} prompt(s) processed)",
                        self.prompt_count
                    )?;
                    self.running = false;
                    commands.push(cmd);
                    break;
                }
                ReplCommand::Help => {
                    writeln!(writer, "commands:")?;
                    writeln!(writer, "  :help, :h, :?    show this help")?;
                    writeln!(writer, "  :status, :s       show session status")?;
                    writeln!(writer, "  :quit, :q, :exit  exit the REPL")?;
                    writeln!(writer, "  <text>            send prompt to agent")?;
                    commands.push(cmd);
                }
                ReplCommand::Status => {
                    writeln!(writer, "session : {}", self.session_id)?;
                    writeln!(writer, "prompts : {}", self.prompt_count)?;
                    commands.push(cmd);
                }
                ReplCommand::Prompt(text) => {
                    if text.is_empty() {
                        continue;
                    }
                    self.prompt_count += 1;
                    writeln!(writer, "[prompt {}] {}", self.prompt_count, text)?;
                    commands.push(cmd);
                }
            }
        }

        Ok(commands)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_quit_variants() {
        assert_eq!(ReplMode::parse_input(":quit"), ReplCommand::Quit);
        assert_eq!(ReplMode::parse_input(":q"), ReplCommand::Quit);
        assert_eq!(ReplMode::parse_input(":exit"), ReplCommand::Quit);
    }

    #[test]
    fn parse_help_variants() {
        assert_eq!(ReplMode::parse_input(":help"), ReplCommand::Help);
        assert_eq!(ReplMode::parse_input(":h"), ReplCommand::Help);
        assert_eq!(ReplMode::parse_input(":?"), ReplCommand::Help);
    }

    #[test]
    fn parse_status() {
        assert_eq!(ReplMode::parse_input(":status"), ReplCommand::Status);
        assert_eq!(ReplMode::parse_input(":s"), ReplCommand::Status);
    }

    #[test]
    fn parse_prompt() {
        assert_eq!(
            ReplMode::parse_input("hello world"),
            ReplCommand::Prompt("hello world".to_string())
        );
    }

    #[test]
    fn parse_trims_whitespace() {
        assert_eq!(
            ReplMode::parse_input("  :quit  "),
            ReplCommand::Quit
        );
        assert_eq!(
            ReplMode::parse_input("  some prompt  "),
            ReplCommand::Prompt("some prompt".to_string())
        );
    }

    #[test]
    fn repl_processes_commands() {
        let input = ":help\n:status\nhello agent\n:quit\n";
        let mut reader = io::Cursor::new(input);
        let mut output = Vec::new();
        let mut repl = ReplMode::new("test-session".into());

        let commands = repl.run(&mut reader, &mut output).unwrap();

        assert_eq!(commands.len(), 4);
        assert_eq!(commands[0], ReplCommand::Help);
        assert_eq!(commands[1], ReplCommand::Status);
        assert_eq!(
            commands[2],
            ReplCommand::Prompt("hello agent".to_string())
        );
        assert_eq!(commands[3], ReplCommand::Quit);
        assert_eq!(repl.prompt_count, 1);
    }

    #[test]
    fn repl_handles_eof() {
        let input = "first prompt\nsecond prompt\n";
        let mut reader = io::Cursor::new(input);
        let mut output = Vec::new();
        let mut repl = ReplMode::new("eof-test".into());

        let commands = repl.run(&mut reader, &mut output).unwrap();

        assert_eq!(commands.len(), 2);
        assert_eq!(repl.prompt_count, 2);
    }

    #[test]
    fn repl_skips_blank_lines() {
        let input = "\n\n  \nhello\n:quit\n";
        let mut reader = io::Cursor::new(input);
        let mut output = Vec::new();
        let mut repl = ReplMode::new("blank-test".into());

        let commands = repl.run(&mut reader, &mut output).unwrap();

        assert_eq!(commands.len(), 2);
        assert_eq!(
            commands[0],
            ReplCommand::Prompt("hello".to_string())
        );
        assert_eq!(commands[1], ReplCommand::Quit);
    }

    #[test]
    fn repl_output_contains_banner() {
        let input = ":quit\n";
        let mut reader = io::Cursor::new(input);
        let mut output = Vec::new();
        let mut repl = ReplMode::new("banner-test".into());

        repl.run(&mut reader, &mut output).unwrap();

        let out_str = String::from_utf8(output).unwrap();
        assert!(out_str.contains("roko repl"));
        assert!(out_str.contains("banner-test"));
        assert!(out_str.contains("goodbye"));
    }
}
