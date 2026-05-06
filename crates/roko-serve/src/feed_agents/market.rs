//! Market and cross-asset analytics feed agents.
//!
//! - Rate correlation tracker (cross-source correlation matrix)
//! - Funding rate divergence (perp funding vs spot lending spread)
//! - Market regime classifier (trending/mean-reverting/volatile)
//! - Risk-adjusted return scorer (Sharpe-like ratio for ISFR sources)
//! - System heartbeat (aggregate health of all feed agents)

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::time::interval;

use super::{FeedAgent, FeedAgentContext, FeedDescriptor};

// ---------------------------------------------------------------------------
// Rate Correlation Tracker
// ---------------------------------------------------------------------------

pub struct CorrelationAgent;

impl FeedAgent for CorrelationAgent {
    fn agent_id(&self) -> &'static str {
        "rate-correlation"
    }
    fn display_name(&self) -> &'static str {
        "Rate Correlation"
    }
    fn capabilities(&self) -> Vec<&str> {
        vec!["analytics", "correlation", "cross-asset"]
    }
    fn feeds(&self) -> Vec<FeedDescriptor> {
        vec![FeedDescriptor {
            feed_id: "analytics-correlation".into(),
            topic: "feed:analytics:correlation".into(),
            name: "Rate Correlation Matrix".into(),
            description: "Cross-class rate correlation: lending vs staking vs funding vs structured".into(),
            kind: "composite".into(),
            rate: "30s".into(),
            schema: None,
        }]
    }
    fn run(
        self: Arc<Self>,
        ctx: FeedAgentContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(async move {
            let mut tick = interval(Duration::from_secs(30));
            let mut lending_ring: VecDeque<f64> = VecDeque::with_capacity(32);
            let mut staking_ring: VecDeque<f64> = VecDeque::with_capacity(32);
            let mut funding_ring: VecDeque<f64> = VecDeque::with_capacity(32);
            let mut structured_ring: VecDeque<f64> = VecDeque::with_capacity(32);

            loop {
                tokio::select! {
                    _ = ctx.cancel.cancelled() => break,
                    _ = tick.tick() => {
                        let rate = ctx.state.isfr.current_rate.read().await;
                        let Some(r) = rate.as_ref() else { continue };

                        lending_ring.push_back(r.lending_bps as f64);
                        staking_ring.push_back(r.staking_bps as f64);
                        funding_ring.push_back(r.funding_bps as f64);
                        structured_ring.push_back(r.structured_bps as f64);

                        for ring in [&mut lending_ring, &mut staking_ring, &mut funding_ring, &mut structured_ring] {
                            if ring.len() > 30 { ring.pop_front(); }
                        }

                        if lending_ring.len() < 3 { continue }

                        let corr = |a: &VecDeque<f64>, b: &VecDeque<f64>| -> f64 {
                            let n = a.len().min(b.len()) as f64;
                            if n < 2.0 { return 0.0; }
                            let ma = a.iter().sum::<f64>() / n;
                            let mb = b.iter().sum::<f64>() / n;
                            let mut cov = 0.0;
                            let mut va = 0.0;
                            let mut vb = 0.0;
                            for (x, y) in a.iter().zip(b.iter()) {
                                let da = x - ma;
                                let db = y - mb;
                                cov += da * db;
                                va += da * da;
                                vb += db * db;
                            }
                            let denom = (va * vb).sqrt();
                            if denom < 1e-10 { 0.0 } else { cov / denom }
                        };

                        let payload = json!({
                            "lending_staking": (corr(&lending_ring, &staking_ring) * 1000.0).round() / 1000.0,
                            "lending_funding": (corr(&lending_ring, &funding_ring) * 1000.0).round() / 1000.0,
                            "lending_structured": (corr(&lending_ring, &structured_ring) * 1000.0).round() / 1000.0,
                            "staking_funding": (corr(&staking_ring, &funding_ring) * 1000.0).round() / 1000.0,
                            "staking_structured": (corr(&staking_ring, &structured_ring) * 1000.0).round() / 1000.0,
                            "funding_structured": (corr(&funding_ring, &structured_ring) * 1000.0).round() / 1000.0,
                            "samples": lending_ring.len(),
                        });
                        ctx.publish_tick(self.agent_id(), "analytics-correlation", "feed:analytics:correlation", payload);
                    }
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Market Regime Classifier
// ---------------------------------------------------------------------------

pub struct RegimeClassifierAgent;

impl FeedAgent for RegimeClassifierAgent {
    fn agent_id(&self) -> &'static str {
        "regime-classifier"
    }
    fn display_name(&self) -> &'static str {
        "Market Regime Classifier"
    }
    fn capabilities(&self) -> Vec<&str> {
        vec!["analytics", "regime", "classification"]
    }
    fn feeds(&self) -> Vec<FeedDescriptor> {
        vec![FeedDescriptor {
            feed_id: "analytics-regime".into(),
            topic: "feed:analytics:regime".into(),
            name: "Market Regime".into(),
            description: "Classifies current rate environment: trending, mean-reverting, or volatile".into(),
            kind: "composite".into(),
            rate: "30s".into(),
            schema: None,
        }]
    }
    fn run(
        self: Arc<Self>,
        ctx: FeedAgentContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(async move {
            let mut tick = interval(Duration::from_secs(30));
            loop {
                tokio::select! {
                    _ = ctx.cancel.cancelled() => break,
                    _ = tick.tick() => {
                        let history = ctx.state.isfr.rate_history.read().await;
                        if history.len() < 5 { continue }

                        let rates: Vec<f64> = history.iter().map(|r| r.composite_bps as f64).collect();
                        let n = rates.len() as f64;
                        let mean = rates.iter().sum::<f64>() / n;
                        let variance = rates.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
                        let stddev = variance.sqrt();
                        let cv = if mean > 0.0 { stddev / mean } else { 0.0 };

                        // Trend: regression slope.
                        let x_mean = (n - 1.0) / 2.0;
                        let mut num = 0.0;
                        let mut den = 0.0;
                        for (i, r) in rates.iter().enumerate() {
                            let xi = i as f64 - x_mean;
                            num += xi * (r - mean);
                            den += xi * xi;
                        }
                        let slope = if den.abs() > 1e-10 { num / den } else { 0.0 };
                        let norm_slope = if mean > 0.0 { slope / mean } else { 0.0 };

                        // Mean reversion: autocorrelation at lag 1.
                        let mut ac_num = 0.0;
                        let mut ac_den = 0.0;
                        for i in 1..rates.len() {
                            ac_num += (rates[i] - mean) * (rates[i - 1] - mean);
                        }
                        for r in &rates {
                            ac_den += (r - mean).powi(2);
                        }
                        let autocorr = if ac_den.abs() > 1e-10 { ac_num / ac_den } else { 0.0 };

                        let regime = if cv > 0.10 { "volatile" }
                            else if norm_slope.abs() > 0.01 { "trending" }
                            else if autocorr < -0.2 { "mean_reverting" }
                            else { "stable" };

                        let trend_dir = if norm_slope > 0.005 { "up" }
                            else if norm_slope < -0.005 { "down" }
                            else { "flat" };

                        let payload = json!({
                            "regime": regime,
                            "trend_direction": trend_dir,
                            "volatility_cv": (cv * 10000.0).round() / 10000.0,
                            "norm_slope": (norm_slope * 10000.0).round() / 10000.0,
                            "autocorrelation": (autocorr * 1000.0).round() / 1000.0,
                            "samples": history.len(),
                        });
                        ctx.publish_tick(self.agent_id(), "analytics-regime", "feed:analytics:regime", payload);
                    }
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// Risk-Adjusted Return Scorer
// ---------------------------------------------------------------------------

pub struct RiskAdjustedAgent;

impl FeedAgent for RiskAdjustedAgent {
    fn agent_id(&self) -> &'static str {
        "risk-adjusted"
    }
    fn display_name(&self) -> &'static str {
        "Risk-Adjusted Scorer"
    }
    fn capabilities(&self) -> Vec<&str> {
        vec!["analytics", "risk", "sharpe"]
    }
    fn feeds(&self) -> Vec<FeedDescriptor> {
        vec![FeedDescriptor {
            feed_id: "analytics-risk-adjusted".into(),
            topic: "feed:analytics:risk-adjusted".into(),
            name: "Risk-Adjusted Returns".into(),
            description: "Sharpe-like ratio for each ISFR rate class".into(),
            kind: "composite".into(),
            rate: "30s".into(),
            schema: None,
        }]
    }
    fn run(
        self: Arc<Self>,
        ctx: FeedAgentContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(async move {
            let mut tick = interval(Duration::from_secs(30));
            loop {
                tokio::select! {
                    _ = ctx.cancel.cancelled() => break,
                    _ = tick.tick() => {
                        let history = ctx.state.isfr.rate_history.read().await;
                        if history.len() < 5 { continue }

                        // Compute Sharpe-like ratio per rate class.
                        let risk_free_bps = 300.0; // 3% risk-free proxy

                        let sharpe = |vals: &[f64]| -> f64 {
                            if vals.len() < 2 { return 0.0 }
                            let n = vals.len() as f64;
                            let mean = vals.iter().sum::<f64>() / n;
                            let var = vals.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
                            let std = var.sqrt();
                            if std < 1.0 { return 0.0 }
                            (mean - risk_free_bps) / std
                        };

                        let lending: Vec<f64> = history.iter().map(|r| r.lending_bps as f64).collect();
                        let staking: Vec<f64> = history.iter().map(|r| r.staking_bps as f64).collect();
                        let funding: Vec<f64> = history.iter().map(|r| r.funding_bps as f64).collect();
                        let structured: Vec<f64> = history.iter().map(|r| r.structured_bps as f64).collect();
                        let composite: Vec<f64> = history.iter().map(|r| r.composite_bps as f64).collect();

                        let best_class = [
                            ("lending", sharpe(&lending)),
                            ("staking", sharpe(&staking)),
                            ("funding", sharpe(&funding)),
                            ("structured", sharpe(&structured)),
                        ].iter()
                            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                            .map(|(name, _)| *name)
                            .unwrap_or("unknown");

                        let payload = json!({
                            "lending_sharpe": (sharpe(&lending) * 1000.0).round() / 1000.0,
                            "staking_sharpe": (sharpe(&staking) * 1000.0).round() / 1000.0,
                            "funding_sharpe": (sharpe(&funding) * 1000.0).round() / 1000.0,
                            "structured_sharpe": (sharpe(&structured) * 1000.0).round() / 1000.0,
                            "composite_sharpe": (sharpe(&composite) * 1000.0).round() / 1000.0,
                            "best_class": best_class,
                            "samples": history.len(),
                        });
                        ctx.publish_tick(self.agent_id(), "analytics-risk-adjusted", "feed:analytics:risk-adjusted", payload);
                    }
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// System Heartbeat
// ---------------------------------------------------------------------------

pub struct SystemHeartbeatAgent;

impl FeedAgent for SystemHeartbeatAgent {
    fn agent_id(&self) -> &'static str {
        "system-heartbeat"
    }
    fn display_name(&self) -> &'static str {
        "System Heartbeat"
    }
    fn capabilities(&self) -> Vec<&str> {
        vec!["meta", "heartbeat", "health"]
    }
    fn feeds(&self) -> Vec<FeedDescriptor> {
        vec![FeedDescriptor {
            feed_id: "meta-heartbeat".into(),
            topic: "feed:meta:heartbeat".into(),
            name: "System Heartbeat".into(),
            description: "Aggregate system health: uptime, feed agent count, event throughput".into(),
            kind: "meta".into(),
            rate: "5s".into(),
            schema: None,
        }]
    }
    fn run(
        self: Arc<Self>,
        ctx: FeedAgentContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(async move {
            let mut tick = interval(Duration::from_secs(5));
            let start = std::time::Instant::now();
            let mut tick_count: u64 = 0;
            loop {
                tokio::select! {
                    _ = ctx.cancel.cancelled() => break,
                    _ = tick.tick() => {
                        tick_count += 1;
                        let uptime_secs = start.elapsed().as_secs();

                        let catalog = ctx.state.feed_agent_catalog.read().await;
                        let active_agents = catalog.agents.len();
                        let total_feeds = catalog.feeds.len();
                        let msgs_per_sec = catalog.messages_per_sec;
                        drop(catalog);

                        let agent_count = ctx.state.agent_count.load(std::sync::atomic::Ordering::Relaxed);

                        let payload = json!({
                            "uptime_secs": uptime_secs,
                            "uptime_human": format!("{}h {}m {}s", uptime_secs / 3600, (uptime_secs % 3600) / 60, uptime_secs % 60),
                            "feed_agents_active": active_agents,
                            "total_feeds": total_feeds,
                            "relay_agents": agent_count,
                            "est_msgs_per_sec": msgs_per_sec,
                            "heartbeat_seq": tick_count,
                        });
                        ctx.publish_tick(self.agent_id(), "meta-heartbeat", "feed:meta:heartbeat", payload);
                    }
                }
            }
        })
    }
}
