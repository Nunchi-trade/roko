//! Post-processing effects for modal overlays.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Widget;

/// A dim overlay rendered behind modals to reduce visual noise.
pub struct DimOverlay;

impl Widget for DimOverlay {
    fn render(self, area: Rect, buf: &mut Buffer) {
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                if let Some(cell) = buf.cell_mut((x, y)) {
                    cell.set_style(Style::default().fg(Color::DarkGray));
                }
            }
        }
    }
}
