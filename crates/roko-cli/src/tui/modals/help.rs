//! Help modal.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::tui::dashboard::Theme;
use crate::tui::layout;

/// Render the global help modal.
pub fn render_help_modal(frame: &mut Frame<'_>, area: Rect, theme: &Theme) {
    let popup = layout::centered_rect(86, 84, area);
    frame.render_widget(Clear, popup);

    let block = Block::default()
        .borders(Borders::ALL)
        .title("help")
        .border_style(theme.accent());
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    let paragraph = Paragraph::new(help_lines(theme))
        .style(theme.text())
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, inner);
}

fn help_lines(theme: &Theme) -> Vec<Line<'static>> {
    vec![
        Line::from(Span::styled(
            "roko dashboard keybindings",
            theme.accent_bold(),
        )),
        Line::from(""),
        Line::from(Span::styled("Navigation", theme.accent_bold())),
        Line::from("F1-F7      switch tabs (Dashboard/Plans/Agents/Git/Logs/Config/Inspect)"),
        Line::from("F8 / u     queue overview modal"),
        Line::from("Tab        cycle focus between panels"),
        Line::from("Shift+Tab  cycle focus backward"),
        Line::from("j/k ↑/↓    scroll focused panel"),
        Line::from("PgUp/PgDn  page scroll"),
        Line::from("Enter      expand/drill into selection"),
        Line::from("Esc        close overlay / drill out"),
        Line::from("q          close overlay or quit"),
        Line::from(""),
        Line::from(Span::styled("Dashboard Sub-Tabs (F1)", theme.accent_bold())),
        Line::from("a          Agents panel"),
        Line::from("o          Output panel"),
        Line::from("d          Diff panel"),
        Line::from("e          Errors panel"),
        Line::from("g          Git panel"),
        Line::from("m          MCP / Context panel"),
        Line::from("P          Processes panel"),
        Line::from(""),
        Line::from(Span::styled("Modals & Modes", theme.accent_bold())),
        Line::from("?          toggle this help"),
        Line::from("w          wave overview"),
        Line::from("p          pause/resume pipeline"),
        Line::from("i          inject message to agent"),
        Line::from("/          filter mode (Plans/Logs)"),
        Line::from("Ctrl-t     task picker"),
        Line::from("Ctrl-e     toggle screen postfx"),
        Line::from("Ctrl-a     approve all pending"),
        Line::from("Ctrl-x     force advance (confirm)"),
        Line::from("Ctrl-d     reset selected plan (confirm)"),
        Line::from(""),
        Line::from(Span::styled("Agent Controls (F3)", theme.accent_bold())),
        Line::from("y          approve pending command"),
        Line::from("A          approve all pending"),
        Line::from("x          reject pending command"),
        Line::from("`          cycle agent tabs"),
        Line::from("1-7        switch agent tab directly"),
        Line::from("G/End      resume auto-scroll"),
        Line::from(""),
        Line::from(Span::styled("Plans (F2)", theme.accent_bold())),
        Line::from("e          expand/collapse plan"),
        Line::from("[/]        wave prev/next"),
        Line::from("h/l ←/→    drill out/in"),
        Line::from("s          soft retry plan"),
        Line::from("R          restart phase"),
        Line::from("F          force advance"),
        Line::from("V / c      re-verify plan"),
        Line::from("S          repair (preserve completed)"),
    ]
}
