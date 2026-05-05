//! Rate derivative, spread monitor, and volatility watcher agents (#8, #9, #14).

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::time::interval;

use super::{FeedAgent, FeedAgentContext, FeedDescriptor};

// ---------------------------------------------------------------------------
// #8 — Rate Derivative Agent
// ---------------------------------------------------------------------------

pub struct RateDerivativeAgent;

impl FeedAgent for RateDerivativeAgent {
    fn agent_id(&self) -> &'static str {
        "isfr-rate-derivative"
    }
    fn display_name(&self) -> &'static str {
        "Rate Derivative"
    }
    fn capabilities(&self) -> Vec<&str> {
        vec!["isfr", "derivative", "analytics"]
    }
    fn feeds(&self) -> Vec<FeedDescriptor> {
        vec![FeedDescriptor {
            feed_id: "isfr-derivative".into(),
            topic: "feed:isfr:derivative".into(),
            name: "Rate Derivative".into(),
            description: "1m/5m/15m rate-of-change on the ISFR composite".into(),
            kind: "derived".into(),
            rate: "15s".into(),
            schema: None,
        }]
    }
    fn run(
        self: Arc<Self>,
        ctx: FeedAgentContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(async move {
            let mut tick = interval(Duration::from_secs(15));
            let mut ring: VecDeque<(i64, u64)> = VecDeque::with_capacity(64);
            loop {
                tokio::select! {
                    _ = ctx.cancel.cancelled() => break,
                    _ = tick.tick() => {
                        let rate = ctx.state.isfr.current_rate.read().await;
                        let payload = if let Some(r) = rate.as_ref() {
                            let now_ms = r.timestamp_ms as i64;
                            ring.push_back((now_ms, r.composite_bps));
                            // Trim older than 16 minutes.
                            while ring.front().is_some_and(|(t, _)| now_ms - t > 960_000) {
                                ring.pop_front();
                            }

                            let roc = |window_ms: i64| -> f64 {
                                let cutoff = now_ms - window_ms;
                                let oldest = ring.iter().find(|(t, _)| *t >= cutoff);
                                match oldest {
                                    Some((_, old_bps)) if *old_bps > 0 => {
                                        (r.composite_bps as f64 - *old_bps as f64) / *old_bps as f64
                                    }
                                    _ => 0.0,
                                }
                            };

                            json!({
                                "composite_bps": r.composite_bps,
                                "roc_1m": roc(60_000),
                                "roc_5m": roc(300_000),
                                "roc_15m": roc(900_000),
                            })
                        } else {
                            json!({
                                "composite_bps": null,
                                "roc_1m": null,
                                "roc_5m": null,
                                "roc_15m": null,
                                "status": "waiting",
                            })
                        };
                        ctx.publish_tick(
                            self.agent_id(),
                            "isfr-derivative",
                            "feed:isfr:derivative",
                            payload,
                        );
                    }
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// #9 — Spread Monitor Agent
// ---------------------------------------------------------------------------

pub struct SpreadMonitorAgent;

impl FeedAgent for SpreadMonitorAgent {
    fn agent_id(&self) -> &'static str {
        "isfr-spread-monitor"
    }
    fn display_name(&self) -> &'static str {
        "Spread Monitor"
    }
    fn capabilities(&self) -> Vec<&str> {
        vec!["isfr", "spread", "analytics"]
    }
    fn feeds(&self) -> Vec<FeedDescriptor> {
        vec![FeedDescriptor {
            feed_id: "isfr-spreads".into(),
            topic: "feed:isfr:spreads".into(),
            name: "Protocol Spreads".into(),
            description: "Inter-protocol rate spreads".into(),
            kind: "derived".into(),
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
                        let rate = ctx.state.isfr.current_rate.read().await;
                        let payload = if let Some(r) = rate.as_ref() {
                            let classes = [
                                ("lending", r.lending_bps),
                                ("structured", r.structured_bps),
                                ("funding", r.funding_bps),
                                ("staking", r.staking_bps),
                            ];
                            let active: Vec<_> = classes.iter().filter(|(_, v)| *v > 0).collect();
                            let max = active.iter().map(|(_, v)| *v).max().unwrap_or(0);
                            let min = active.iter().map(|(_, v)| *v).min().unwrap_or(0);

                            json!({
                                "lending_bps": r.lending_bps,
                                "structured_bps": r.structured_bps,
                                "funding_bps": r.funding_bps,
                                "staking_bps": r.staking_bps,
                                "max_spread_bps": max.saturating_sub(min),
                                "active_classes": active.len(),
                            })
                        } else {
                            json!({
                                "lending_bps": null,
                                "structured_bps": null,
                                "funding_bps": null,
                                "staking_bps": null,
                                "max_spread_bps": null,
                                "active_classes": 0,
                                "status": "waiting",
                            })
                        };
                        ctx.publish_tick(
                            self.agent_id(),
                            "isfr-spreads",
                            "feed:isfr:spreads",
                            payload,
                        );
                    }
                }
            }
        })
    }
}

// ---------------------------------------------------------------------------
// #14 — Volatility Watcher Agent
// ---------------------------------------------------------------------------

pub struct VolatilityWatcherAgent;

impl FeedAgent for VolatilityWatcherAgent {
    fn agent_id(&self) -> &'static str {
        "isfr-volatility"
    }
    fn display_name(&self) -> &'static str {
        "Volatility Watcher"
    }
    fn capabilities(&self) -> Vec<&str> {
        vec!["isfr", "volatility", "analytics"]
    }
    fn feeds(&self) -> Vec<FeedDescriptor> {
        vec![FeedDescriptor {
            feed_id: "isfr-volatility".into(),
            topic: "feed:isfr:volatility".into(),
            name: "Rate Volatility".into(),
            description: "Rolling standard deviation of the ISFR composite".into(),
            kind: "derived".into(),
            rate: "20s".into(),
            schema: None,
        }]
    }
    fn run(
        self: Arc<Self>,
        ctx: FeedAgentContext,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>> {
        Box::pin(async move {
            let mut tick = interval(Duration::from_secs(20));
            loop {
                tokio::select! {
                    _ = ctx.cancel.cancelled() => break,
                    _ = tick.tick() => {
                        let history = ctx.state.isfr.rate_history.read().await;
                        let payload = if history.len() < 2 {
                            json!({
                                "stddev_bps": null,
                                "mean_bps": null,
                                "sample_count": history.len(),
                                "min_bps": null,
                                "max_bps": null,
                                "status": "waiting",
                            })
                        } else {
                            let rates: Vec<f64> = history.iter().map(|r| r.composite_bps as f64).collect();
                            let n = rates.len() as f64;
                            let mean = rates.iter().sum::<f64>() / n;
                            let variance = rates.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
                            let stddev = variance.sqrt();

                            json!({
                                "stddev_bps": stddev,
                                "mean_bps": mean,
                                "sample_count": history.len(),
                                "min_bps": rates.iter().copied().fold(f64::INFINITY, f64::min),
                                "max_bps": rates.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                            })
                        };
                        ctx.publish_tick(
                            self.agent_id(),
                            "isfr-volatility",
                            "feed:isfr:volatility",
                            payload,
                        );
                    }
                }
            }
        })
    }
}
