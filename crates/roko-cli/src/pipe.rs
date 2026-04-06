//! Pipe mode — read all of stdin (non-TTY) then execute as a single prompt.
//!
//! Activated when stdin is *not* a TTY and no positional prompt argument is
//! given. Reads all available stdin to completion, then dispatches the
//! accumulated text as a single prompt through the universal loop.

use std::io::{self, Read};

/// Pipe-mode execution context.
#[derive(Debug, Clone)]
pub struct PipeMode {
    /// Whether to emit JSON output.
    pub json_output: bool,
    /// Whether to suppress non-essential output.
    pub quiet: bool,
    /// Maximum bytes to read from stdin (safety limit).
    pub max_bytes: usize,
}

/// Result of reading piped input.
#[derive(Debug, Clone)]
pub struct PipeInput {
    /// The text read from stdin.
    pub text: String,
    /// Whether the input was truncated due to the byte limit.
    pub truncated: bool,
    /// Number of bytes read.
    pub bytes_read: usize,
}

impl Default for PipeMode {
    fn default() -> Self {
        Self {
            json_output: false,
            quiet: false,
            max_bytes: 10 * 1024 * 1024, // 10 MiB
        }
    }
}

impl PipeMode {
    /// Create a new pipe-mode context.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Set JSON output mode.
    #[must_use]
    pub const fn with_json(mut self, json: bool) -> Self {
        self.json_output = json;
        self
    }

    /// Set quiet mode.
    #[must_use]
    pub const fn with_quiet(mut self, quiet: bool) -> Self {
        self.quiet = quiet;
        self
    }

    /// Set the maximum byte limit for stdin reads.
    #[must_use]
    pub const fn with_max_bytes(mut self, max_bytes: usize) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    /// Read all input from the provided reader up to `max_bytes`.
    pub fn read_input<R: Read>(&self, reader: &mut R) -> io::Result<PipeInput> {
        let mut buf = Vec::with_capacity(4096);
        let mut limited = reader.take(self.max_bytes as u64);
        let bytes_read = limited.read_to_end(&mut buf)?;

        // Check if we hit the limit (there might be more data).
        let truncated = bytes_read >= self.max_bytes;

        let text = String::from_utf8_lossy(&buf).into_owned();
        let text = text.trim().to_string();

        Ok(PipeInput {
            text,
            truncated,
            bytes_read,
        })
    }
}

/// Detect whether stdin is a TTY (interactive terminal).
///
/// Returns `true` if stdin is connected to a terminal, `false` if it is
/// a pipe or redirected file.
#[must_use]
pub fn stdin_is_tty() -> bool {
    use std::io::IsTerminal;
    io::stdin().is_terminal()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_values() {
        let mode = PipeMode::new();
        assert!(!mode.json_output);
        assert!(!mode.quiet);
        assert_eq!(mode.max_bytes, 10 * 1024 * 1024);
    }

    #[test]
    fn read_input_basic() {
        let input = b"hello from pipe";
        let mut cursor = io::Cursor::new(input);
        let mode = PipeMode::new();
        let result = mode.read_input(&mut cursor).unwrap();
        assert_eq!(result.text, "hello from pipe");
        assert!(!result.truncated);
        assert_eq!(result.bytes_read, 15);
    }

    #[test]
    fn read_input_trims_whitespace() {
        let input = b"  hello  \n  ";
        let mut cursor = io::Cursor::new(input);
        let mode = PipeMode::new();
        let result = mode.read_input(&mut cursor).unwrap();
        assert_eq!(result.text, "hello");
    }

    #[test]
    fn read_input_respects_max_bytes() {
        let input = b"abcdefghij"; // 10 bytes
        let mut cursor = io::Cursor::new(input);
        let mode = PipeMode::new().with_max_bytes(5);
        let result = mode.read_input(&mut cursor).unwrap();
        // Should read at most 5 bytes
        assert_eq!(result.bytes_read, 5);
        assert!(result.truncated);
    }

    #[test]
    fn read_input_empty() {
        let input = b"";
        let mut cursor = io::Cursor::new(input);
        let mode = PipeMode::new();
        let result = mode.read_input(&mut cursor).unwrap();
        assert!(result.text.is_empty());
        assert!(!result.truncated);
        assert_eq!(result.bytes_read, 0);
    }

    #[test]
    fn read_input_multiline() {
        let input = b"line one\nline two\nline three";
        let mut cursor = io::Cursor::new(input);
        let mode = PipeMode::new();
        let result = mode.read_input(&mut cursor).unwrap();
        assert!(result.text.contains("line one"));
        assert!(result.text.contains("line three"));
    }

    #[test]
    fn builder_methods() {
        let mode = PipeMode::new()
            .with_json(true)
            .with_quiet(true)
            .with_max_bytes(1024);
        assert!(mode.json_output);
        assert!(mode.quiet);
        assert_eq!(mode.max_bytes, 1024);
    }
}
