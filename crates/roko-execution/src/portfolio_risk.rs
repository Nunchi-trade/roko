//! Portfolio-level risk checks (correlation + direction + margin). Ports
//! `execution/portfolio_risk.py`.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Correlation group membership — maps asset → group name. Copied verbatim
/// from the Python `CORRELATION_GROUPS`.
pub static CORRELATION_GROUPS: &[(&str, &[&str])] = &[
    ("large_cap", &["BTC", "ETH"]),
    ("l2", &["ARB", "OP", "STRK", "MANTA", "BLAST"]),
    ("alt_l1", &["SOL", "AVAX", "SUI", "SEI", "APT", "TIA"]),
    ("defi_blue", &["AAVE", "UNI", "MKR", "COMP", "CRV", "SNX", "LINK"]),
    ("meme", &["DOGE", "SHIB", "PEPE", "WIF", "BONK"]),
    ("ai", &["FET", "RNDR", "TAO", "NEAR"]),
];

fn coin_to_group(coin: &str) -> Option<&'static str> {
    for (group, coins) in CORRELATION_GROUPS {
        if coins.contains(&coin) {
            return Some(*group);
        }
    }
    None
}

fn instrument_to_asset(inst: &str) -> String {
    // Strip HL-style suffix like "-PERP", "-USDYP". Matches the behavior of
    // `common.models.instrument_to_asset`.
    for suffix in &["-PERP", "-USDYP"] {
        if let Some(bare) = inst.strip_suffix(suffix) {
            return bare.to_string();
        }
    }
    inst.to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioRiskConfig {
    pub max_correlated_positions: u32,
    pub max_same_direction_total: u32,
    pub margin_utilization_warn: f64,
    pub margin_utilization_block: f64,
    pub enabled: bool,
}

impl Default for PortfolioRiskConfig {
    fn default() -> Self {
        Self {
            max_correlated_positions: 2,
            max_same_direction_total: 3,
            margin_utilization_warn: 0.7,
            margin_utilization_block: 0.9,
            enabled: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionSummary {
    pub direction: String,
    pub notional: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PortfolioRiskState {
    pub positions: HashMap<String, PositionSummary>,
    pub margin_utilization: f64,
    pub correlated_groups: HashMap<String, Vec<String>>,
    pub warnings: Vec<String>,
    pub blocked: bool,
    pub block_reason: String,
}

pub struct PortfolioRiskManager {
    config: PortfolioRiskConfig,
}

impl PortfolioRiskManager {
    #[must_use]
    pub fn new(config: PortfolioRiskConfig) -> Self {
        Self { config }
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn assess(
        &self,
        positions: HashMap<String, PositionSummary>,
        account_value: Option<f64>,
        total_margin: Option<f64>,
    ) -> PortfolioRiskState {
        let mut state = PortfolioRiskState {
            positions: positions.clone(),
            ..PortfolioRiskState::default()
        };
        if !self.config.enabled {
            return state;
        }

        for (inst, _pos) in &positions {
            let coin = instrument_to_asset(inst);
            let group = coin_to_group(&coin).unwrap_or("ungrouped").to_string();
            state.correlated_groups.entry(group).or_default().push(inst.clone());
        }

        for (group, insts) in &state.correlated_groups {
            if group == "ungrouped" {
                continue;
            }
            if insts.len() > self.config.max_correlated_positions as usize {
                state.warnings.push(format!(
                    "Correlation limit: {} positions in '{}' group (max {}): {:?}",
                    insts.len(),
                    group,
                    self.config.max_correlated_positions,
                    insts
                ));
            }
        }

        let longs: Vec<&String> = positions.iter().filter_map(|(i, p)| (p.direction == "long").then_some(i)).collect();
        let shorts: Vec<&String> = positions.iter().filter_map(|(i, p)| (p.direction == "short").then_some(i)).collect();
        if longs.len() > self.config.max_same_direction_total as usize {
            state.warnings.push(format!(
                "Direction concentration: {} longs (max {})",
                longs.len(),
                self.config.max_same_direction_total
            ));
        }
        if shorts.len() > self.config.max_same_direction_total as usize {
            state.warnings.push(format!(
                "Direction concentration: {} shorts (max {})",
                shorts.len(),
                self.config.max_same_direction_total
            ));
        }

        if let (Some(value), Some(margin)) = (account_value, total_margin) {
            if value > 0.0 {
                state.margin_utilization = margin / value;
                if state.margin_utilization >= self.config.margin_utilization_block {
                    state.blocked = true;
                    state.block_reason = format!(
                        "Margin utilization {:.0}% >= {:.0}%",
                        state.margin_utilization * 100.0,
                        self.config.margin_utilization_block * 100.0
                    );
                    let msg = state.block_reason.clone();
                    state.warnings.push(msg);
                } else if state.margin_utilization >= self.config.margin_utilization_warn {
                    state.warnings.push(format!(
                        "High margin utilization: {:.0}%",
                        state.margin_utilization * 100.0
                    ));
                }
            }
        }

        state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(dir: &str) -> PositionSummary {
        PositionSummary { direction: dir.into(), notional: 1_000.0 }
    }

    #[test]
    fn correlation_limit_warning() {
        let mgr = PortfolioRiskManager::new(PortfolioRiskConfig::default());
        let mut positions = HashMap::new();
        positions.insert("BTC-PERP".into(), p("long"));
        positions.insert("ETH-PERP".into(), p("long"));
        // default max_correlated_positions = 2, so 2 in large_cap is OK
        let state = mgr.assess(positions, None, None);
        assert!(state.warnings.is_empty());
    }

    #[test]
    fn direction_concentration_warning() {
        let mgr = PortfolioRiskManager::new(PortfolioRiskConfig::default());
        let mut positions = HashMap::new();
        for inst in ["ARB-PERP", "OP-PERP", "STRK-PERP", "MANTA-PERP"] {
            positions.insert(inst.into(), p("long"));
        }
        let state = mgr.assess(positions, None, None);
        assert!(state.warnings.iter().any(|w| w.contains("longs")));
    }

    #[test]
    fn margin_block_above_90_pct() {
        let mgr = PortfolioRiskManager::new(PortfolioRiskConfig::default());
        let state = mgr.assess(HashMap::new(), Some(1_000.0), Some(950.0));
        assert!(state.blocked);
    }
}
