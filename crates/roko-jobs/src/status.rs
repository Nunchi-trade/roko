//! Heartbeat + reward persistence. Ports `cli/jobs/status.py`.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatRecord {
    pub block_number: u64,
    pub timestamp_ms: u128,
    pub success: bool,
    #[serde(default)]
    pub tx_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewardRecord {
    pub amount_eth: f64,
    pub block_number: u64,
    pub timestamp_ms: u128,
    #[serde(default)]
    pub tx_hash: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobStatusTracker {
    pub job_id: String,
    pub agent_id: String,
    pub data_dir: String,
    pub heartbeats: Vec<HeartbeatRecord>,
    pub rewards: Vec<RewardRecord>,
    pub started_at: u128,
}

impl JobStatusTracker {
    #[must_use]
    pub fn new(job_id: impl Into<String>, agent_id: impl Into<String>, data_dir: impl Into<String>) -> Self {
        Self {
            job_id: job_id.into(),
            agent_id: agent_id.into(),
            data_dir: data_dir.into(),
            heartbeats: vec![],
            rewards: vec![],
            started_at: now_ms(),
        }
    }

    fn state_path(&self) -> PathBuf {
        let mut p = PathBuf::from(&self.data_dir);
        p.push(format!("{}-{}.json", self.job_id, self.agent_id));
        p
    }

    pub fn record_heartbeat(&mut self, block_number: u64, success: bool, tx_hash: Option<String>) {
        self.heartbeats.push(HeartbeatRecord {
            block_number,
            timestamp_ms: now_ms(),
            success,
            tx_hash,
        });
    }

    pub fn record_reward(&mut self, amount_eth: f64, block_number: u64, tx_hash: impl Into<String>) {
        self.rewards.push(RewardRecord {
            amount_eth,
            block_number,
            timestamp_ms: now_ms(),
            tx_hash: tx_hash.into(),
        });
    }

    pub fn save(&self) -> std::io::Result<()> {
        if let Some(parent) = self.state_path().parent() {
            fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        fs::write(self.state_path(), json)
    }

    pub fn load(job_id: &str, agent_id: &str, data_dir: &str) -> Option<Self> {
        let mut p = PathBuf::from(data_dir);
        p.push(format!("{job_id}-{agent_id}.json"));
        let body = fs::read_to_string(&p).ok()?;
        serde_json::from_str(&body).ok()
    }

    #[must_use]
    pub fn total_reward_eth(&self) -> f64 {
        self.rewards.iter().map(|r| r.amount_eth).sum()
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_persistence() {
        let dir = tempfile::tempdir().unwrap();
        let mut t = JobStatusTracker::new("oracle_updater", "agent-1", dir.path().to_string_lossy());
        t.record_heartbeat(10, true, Some("0xdead".into()));
        t.record_reward(0.5, 10, "0xbeef");
        t.save().unwrap();

        let reloaded = JobStatusTracker::load("oracle_updater", "agent-1", &t.data_dir).unwrap();
        assert_eq!(reloaded.heartbeats.len(), 1);
        assert_eq!(reloaded.rewards.len(), 1);
        assert!((reloaded.total_reward_eth() - 0.5).abs() < 1e-9);
    }
}
