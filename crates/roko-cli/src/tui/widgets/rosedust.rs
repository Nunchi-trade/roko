//! Compatibility shim for the canonical TUI theme.
//!
//! `MoriTheme` remains available for one commit cycle as an alias to the
//! canonical `Theme` in `crate::tui::theme`.

pub(crate) use crate::tui::theme::{brighten, gradient_fire, gradient_ocean};

#[allow(dead_code)]
pub(crate) type MoriTheme = crate::tui::theme::Theme;
