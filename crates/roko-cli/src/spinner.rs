//! CLI spinner helpers built on `indicatif`.

use indicatif::{ProgressBar, ProgressStyle};

/// Create and start a spinner with elapsed-time display.
///
/// The spinner ticks every 80ms and shows the given message plus `(elapsed)`.
pub fn cli_spinner(msg: impl Into<String>) -> ProgressBar {
    let pb = ProgressBar::new_spinner().with_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg} ({elapsed})")
            .unwrap(),
    );
    pb.set_message(msg.into());
    pb.enable_steady_tick(std::time::Duration::from_millis(80));
    pb
}
