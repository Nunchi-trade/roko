//! Agent-output sanitizers.
//!
//! Real-world LLM CLIs (notably `ollama run`) emit ANSI cursor-movement
//! escapes and — for reasoning models like glm-4 or gemma — chain-of-thought
//! traces that users rarely want in the persisted `AgentOutput` signal.
//!
//! [`clean`] is the full pipeline: strip ANSI, collapse carriage returns,
//! remove thinking traces. The pipeline is deliberately stateless; callers
//! that need the raw text can keep it alongside the cleaned version.

/// Strip ANSI CSI escape sequences, emulating a line-buffered terminal well
/// enough to cope with `ollama run`'s progress spinner.
///
/// Handled sequences:
///   - `CSI {n} D` — cursor-back-N: delete the last N chars of the current line
///   - `CSI K` — clear-to-end-of-line: drop everything after the cursor on
///     the current line (in our single-pass model this is a no-op because
///     we only track what was emitted, but we still swallow it)
///   - `CSI {n} C` — cursor-forward-N: treated as N spaces
///   - All other CSI params (colors, attributes, save/restore): swallow
///   - OSC (`ESC ]`): swallow until BEL or `ESC \`
///   - Any other ESC-prefixed single-char sequence: swallow two bytes
///
/// This is not a full terminal emulator — it just handles the overwrite
/// pattern `…partial_text CSI{N}D CSI K \n full_text` that `ollama run`
/// emits when it line-wraps its own output. That's the overwhelmingly common
/// case. Complex sequences (scroll regions, bracketed paste, 2D cursor
/// positioning) are just stripped, not emulated.
#[must_use]
pub fn strip_ansi(input: &str) -> String {
    let mut out: Vec<char> = Vec::with_capacity(input.len());
    let mut line_start: usize = 0;
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\n' {
            out.push(c);
            line_start = out.len();
            continue;
        }
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        // ESC. Peek the next byte.
        match chars.next() {
            Some('[') => {
                let (params, final_byte) = consume_csi(&mut chars);
                apply_csi(&mut out, line_start, &params, final_byte);
            }
            Some(']') => {
                // OSC: skip until BEL or ESC\.
                while let Some(next) = chars.next() {
                    if next == '\x07' {
                        break;
                    }
                    if next == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            _ => {} // Other single-char ESC sequence — swallow.
        }
    }
    out.into_iter().collect()
}

fn consume_csi<I>(chars: &mut std::iter::Peekable<I>) -> (String, char)
where
    I: Iterator<Item = char>,
{
    let mut params = String::new();
    while let Some(&next) = chars.peek() {
        let b = next as u32;
        if (0x40..=0x7e).contains(&b) {
            chars.next();
            return (params, next);
        }
        params.push(next);
        chars.next();
    }
    // Unterminated CSI — treat as identity.
    (params, '\0')
}

fn apply_csi(out: &mut Vec<char>, line_start: usize, params: &str, final_byte: char) {
    let n: usize = params
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    match final_byte {
        'D' => {
            // Cursor-back-N: drop the last N chars of the current line (but
            // don't cross the line_start boundary).
            let line_len = out.len() - line_start;
            let drop = n.min(line_len);
            out.truncate(out.len() - drop);
        }
        'C' => {
            // Cursor-forward-N: pad with N spaces.
            for _ in 0..n {
                out.push(' ');
            }
        }
        // Other CSI (colors, attributes, clear-to-EOL etc.): swallow silently.
        // Clear-to-EOL is a no-op in our linear model — nothing exists past
        // the cursor until something is written next.
        _ => {}
    }
}

/// Collapse `\r` carriage returns so that each line keeps only its final
/// version (the way a terminal would render overwritten progress lines).
#[must_use]
pub fn collapse_carriage_returns(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for line in input.split('\n') {
        // Keep only what comes after the last `\r` — that's what a real
        // terminal would have displayed.
        let tail = line.rsplit('\r').next().unwrap_or(line);
        out.push_str(tail);
        out.push('\n');
    }
    // We added a trailing newline; drop it if the original didn't end with one.
    if !input.ends_with('\n') {
        out.pop();
    }
    out
}

/// Remove common reasoning-model "thinking" traces.
///
/// Handles three shapes:
///   1. `<think>…</think>` tags (used by glm, qwen-reasoning, others)
///   2. `Thinking...\n…\n...done thinking.\n` blocks (ollama reasoning output)
///   3. `<thinking>…</thinking>` (Claude-style, seen in some fine-tunes)
#[must_use]
pub fn strip_thinking(input: &str) -> String {
    let mut s = input.to_string();
    s = remove_tag(&s, "<think>", "</think>");
    s = remove_tag(&s, "<thinking>", "</thinking>");
    s = remove_ollama_thinking(&s);
    s
}

fn remove_tag(input: &str, open: &str, close: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find(open) {
        out.push_str(&rest[..start]);
        let after_open = &rest[start + open.len()..];
        match after_open.find(close) {
            Some(end) => {
                rest = &after_open[end + close.len()..];
                // Drop a single leading newline after the closing tag, if present.
                rest = rest.strip_prefix('\n').unwrap_or(rest);
            }
            None => {
                // No closing tag — drop the rest of the input, be defensive.
                return out;
            }
        }
    }
    out.push_str(rest);
    out
}

fn remove_ollama_thinking(input: &str) -> String {
    // Look for "Thinking..." on its own line and "...done thinking." on its
    // own line. Remove everything between them inclusive.
    let open = "Thinking...";
    let close = "...done thinking.";
    let Some(start) = input.find(open) else {
        return input.to_string();
    };
    let after_open = &input[start + open.len()..];
    let Some(end_rel) = after_open.find(close) else {
        return input.to_string();
    };
    let absolute_end = start + open.len() + end_rel + close.len();
    let mut out = String::with_capacity(input.len());
    out.push_str(&input[..start]);
    let tail = &input[absolute_end..];
    out.push_str(tail.strip_prefix('\n').unwrap_or(tail));
    // Also strip the newline immediately *before* "Thinking..." if present.
    if out.ends_with('\n') {
        out.pop();
        // But preserve a single newline between pre-thinking text and what follows.
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
    }
    out.trim_start_matches('\n').to_string()
}

/// Full sanitizer: strip ANSI, collapse `\r`, remove thinking traces, trim.
#[must_use]
pub fn clean(input: &str) -> String {
    let s = strip_ansi(input);
    let s = collapse_carriage_returns(&s);
    let s = strip_thinking(&s);
    s.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_ansi_cursor_moves() {
        // `hello` + back-1 + clear-to-EOL + `world` = `hell` + `world` = `hellworld`
        // (clear-to-EOL removes the char under the cursor and everything after).
        let raw = "hello\x1b[1D\x1b[Kworld";
        assert_eq!(strip_ansi(raw), "hellworld");
    }

    #[test]
    fn strips_ansi_cursor_back_without_clear() {
        // Back 3 with no clear: the next write overwrites the last 3 chars.
        // Since our model is linear append-after-truncate, back-3 drops 3 and
        // then `xyz` is written — so `abcdef` + back-3 + `xyz` = `abcxyz`.
        let raw = "abcdef\x1b[3Dxyz";
        assert_eq!(strip_ansi(raw), "abcxyz");
    }

    #[test]
    fn strips_ansi_cursor_forward_pads_with_spaces() {
        let raw = "x\x1b[3Cy";
        assert_eq!(strip_ansi(raw), "x   y");
    }

    #[test]
    fn cursor_back_does_not_cross_newline() {
        // Line starts get pinned; cursor-back stops at the current line head.
        let raw = "first\nab\x1b[10Dcd";
        assert_eq!(strip_ansi(raw), "first\ncd");
    }

    #[test]
    fn strips_ansi_colors() {
        let raw = "\x1b[31mred\x1b[0m text";
        assert_eq!(strip_ansi(raw), "red text");
    }

    #[test]
    fn strips_ansi_long_params() {
        let raw = "\x1b[1;34;47mboth\x1b[0m";
        assert_eq!(strip_ansi(raw), "both");
    }

    #[test]
    fn strips_osc_title_sequence() {
        let raw = "\x1b]0;title\x07after";
        assert_eq!(strip_ansi(raw), "after");
    }

    #[test]
    fn non_ansi_text_unchanged() {
        assert_eq!(strip_ansi("plain text"), "plain text");
        assert_eq!(strip_ansi(""), "");
    }

    #[test]
    fn carriage_returns_collapse_to_last_version() {
        // Progress bar: rewrites the same line 3 times then moves on.
        let raw = "downloading...\rloading...\rdone!\nnext line";
        assert_eq!(collapse_carriage_returns(raw), "done!\nnext line");
    }

    #[test]
    fn strips_think_tag() {
        let raw = "prefix <think>inner reasoning</think>\noutput";
        assert_eq!(strip_thinking(raw), "prefix output");
    }

    #[test]
    fn strips_thinking_tag_variant() {
        let raw = "<thinking>\nstep 1\nstep 2\n</thinking>\nanswer";
        assert_eq!(strip_thinking(raw), "answer");
    }

    #[test]
    fn strips_ollama_thinking_block() {
        let raw =
            "Thinking...\nlots of reasoning\nmore steps\n...done thinking.\n\nFinal answer: 42";
        let out = strip_thinking(raw);
        assert!(!out.contains("reasoning"));
        assert!(!out.contains("Thinking..."));
        assert!(out.contains("Final answer: 42"));
    }

    #[test]
    fn strip_thinking_noop_when_absent() {
        let raw = "just the answer";
        assert_eq!(strip_thinking(raw), "just the answer");
    }

    #[test]
    fn unterminated_think_tag_drops_the_rest() {
        // Defensive: if </think> is missing, we bail cleanly.
        let raw = "before <think>incomplete...";
        assert_eq!(strip_thinking(raw), "before ");
    }

    #[test]
    fn clean_combines_all_pipelines() {
        let raw = "\x1b[2K\rThinking...\ncomplex reasoning\n...done thinking.\n\n```rust\nfn hi() {}\n```";
        let out = clean(raw);
        assert!(!out.contains("Thinking"));
        assert!(!out.contains("reasoning"));
        assert!(!out.contains('\x1b'));
        assert!(out.contains("fn hi()"));
    }

    #[test]
    fn clean_handles_realistic_ollama_progress_escapes() {
        // Representative of what `ollama run` actually emits — cursor-back-N + clear-to-EOL.
        // The leading backtick (1 char before the [16D point) is kept; everything after
        // it is cleared, then rewritten.
        let raw = "`af1349b9f5f9a1a6\x1b[16D\x1b[K`af1349b9f5f9a1a6a0404dea36`";
        let out = clean(raw);
        assert_eq!(out, "``af1349b9f5f9a1a6a0404dea36`");
    }

    #[test]
    fn clean_overwrites_full_wrapped_line() {
        // When ollama wraps a line it does cursor-back-N where N is the entire
        // visible line width — so everything on that line gets rewritten.
        let raw = "prefix\n`af1349b9f5f9a1a6\x1b[17D\x1b[K`af1349b9f5f9a1a6a0404dea36`";
        let out = clean(raw);
        assert_eq!(out, "prefix\n`af1349b9f5f9a1a6a0404dea36`");
    }

    #[test]
    fn clean_preserves_code_blocks() {
        let raw = "```rust\nfn main() {\n    println!(\"hi\");\n}\n```";
        assert_eq!(clean(raw), raw.trim());
    }
}
