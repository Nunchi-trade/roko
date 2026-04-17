//! Agent topology endpoint — derives an interaction graph from knowledge store data.

use std::collections::HashMap;

use axum::{Json, extract::State};
use serde::Serialize;

use super::{ApiState, now_secs};

// ---------------------------------------------------------------------------
// GET /api/agents/topology
// ---------------------------------------------------------------------------

#[derive(Serialize)]
/// Node in the derived agent interaction graph returned by `/api/agents/topology`.
pub struct AgentNode {
    /// Agent identifier (author bytes as UTF-8 lossy string).
    pub id: String,
    /// Raw author bytes as hex (for precise identification).
    pub address: String,
    /// Number of insights this agent has posted.
    pub insights_posted: usize,
    /// Number of confirmations this agent has given to others' insights.
    pub confirmations_given: usize,
    /// Number of challenges this agent has filed.
    pub challenges_given: usize,
    /// Aggregate weight of posted insights (measures influence).
    pub total_weight: f64,
}

#[derive(Serialize)]
/// Directed edge in the derived agent interaction graph.
pub struct AgentEdge {
    /// Source agent id.
    pub from: String,
    /// Target agent id.
    pub to: String,
    /// Interaction weight (number of confirmations from → to).
    pub weight: usize,
    /// Edge type: "confirmed" or "challenged".
    #[serde(rename = "type")]
    pub edge_type: String,
}

#[derive(Serialize)]
/// Response payload for `/api/agents/topology`.
pub struct TopologyResponse {
    /// Derived graph nodes keyed by agent identity.
    pub nodes: Vec<AgentNode>,
    /// Directed confirmation/challenge edges between agents.
    pub edges: Vec<AgentEdge>,
    /// Unix timestamp at which the topology snapshot was generated.
    pub timestamp: u64,
}

/// Computes the current knowledge-derived interaction topology for all known agents.
pub async fn agent_topology(State(state): State<ApiState>) -> Json<TopologyResponse> {
    let now = now_secs();
    let chain = state.chain.read();

    // Build agent profiles from the knowledge store.
    //
    // Each InsightEntry has:
    //   - author: the agent that posted it
    //   - confirmations: list of confirmer addresses
    //   - challenges: list of challenger addresses
    //
    // From this we derive:
    //   - nodes: unique agents with their post/confirm/challenge counts
    //   - edges: directed edges from confirmer → author (weighted by count)

    let mut agent_posts: HashMap<String, (String, usize, f64)> = HashMap::new(); // id → (hex, post_count, total_weight)
    let mut confirmations_given: HashMap<String, usize> = HashMap::new();
    let mut challenges_given: HashMap<String, usize> = HashMap::new();
    // (from, to, type) → count
    let mut edge_counts: HashMap<(String, String, &str), usize> = HashMap::new();

    for entry in chain.knowledge.entries() {
        let author_str = String::from_utf8_lossy(&entry.author).into_owned();
        let author_hex = hex_encode(&entry.author);

        let agent = agent_posts
            .entry(author_str.clone())
            .or_insert_with(|| (author_hex, 0, 0.0));
        agent.1 += 1;
        agent.2 += entry.weight as f64;

        // Confirmations: each confirmer has an edge → this author.
        for confirmer in &entry.confirmations {
            let confirmer_str = String::from_utf8_lossy(confirmer).into_owned();
            *confirmations_given
                .entry(confirmer_str.clone())
                .or_default() += 1;
            // Ensure confirmer shows up as a node too.
            agent_posts
                .entry(confirmer_str.clone())
                .or_insert_with(|| (hex_encode(confirmer), 0, 0.0));
            *edge_counts
                .entry((confirmer_str, author_str.clone(), "confirmed"))
                .or_default() += 1;
        }

        // Challenges: each challenger has an edge → this author.
        for challenger in &entry.challenges {
            let challenger_str = String::from_utf8_lossy(challenger).into_owned();
            *challenges_given.entry(challenger_str.clone()).or_default() += 1;
            agent_posts
                .entry(challenger_str.clone())
                .or_insert_with(|| (hex_encode(challenger), 0, 0.0));
            *edge_counts
                .entry((challenger_str, author_str.clone(), "challenged"))
                .or_default() += 1;
        }
    }

    let nodes: Vec<AgentNode> = agent_posts
        .into_iter()
        .map(|(id, (address, posts, weight))| AgentNode {
            id: id.clone(),
            address,
            insights_posted: posts,
            confirmations_given: *confirmations_given.get(&id).unwrap_or(&0),
            challenges_given: *challenges_given.get(&id).unwrap_or(&0),
            total_weight: weight,
        })
        .collect();

    let edges: Vec<AgentEdge> = edge_counts
        .into_iter()
        .filter(|(_, count)| *count > 0)
        .map(|((from, to, edge_type), weight)| AgentEdge {
            from,
            to,
            weight,
            edge_type: edge_type.to_owned(),
        })
        .collect();

    Json(TopologyResponse {
        nodes,
        edges,
        timestamp: now,
    })
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
