# 10 -- Groups and Coordination

> Persistent agent collectives as Space specialization. Owns Bus partition + Store partition. 4 coordination modes (stigmergic, pipeline, broadcast, leader-follower). Membership protocol. Cross-user invitation. Shared knowledge and pheromone fields.

**Kernel mapping**: Group = Space (one of the 4 universal patterns). A Space owns a Bus partition + Store partition. A Group adds membership management, coordination modes, shared knowledge (Store partition), and pheromone fields (Signals in the Store partition subject to standard demurrage). No new kernel types -- Group is a named convention over Space primitives.

---

## 1. Group Primitive

A **Group** is a Space specialization: a persistent, named collection of agents with shared identity, a Bus partition (relay room), a Store partition (knowledge store), and optional on-chain registration. Groups outlive individual tasks. An agent joins a group and stays until it leaves or is removed. Groups accumulate shared knowledge and pheromone fields over time.

### 1.1 Kernel Decomposition

```
Group = Space + Membership + CoordinationMode

  Space provides:
    Bus partition  ->  relay room (group:{id} + sub-rooms)
    Store partition ->  scoped InsightStore (group knowledge)
    Access control ->  MemberPermissions (read/write/execute)

  Membership adds:
    Join/leave protocol
    Role assignment (Leader/Member/Observer)
    Cross-user invitation flow

  CoordinationMode adds:
    One of: Stigmergic | Pipeline | Broadcast | LeaderFollower
    Each mode defines how agents interact within the Space
```

### 1.2 Groups vs Clusters

The system has two multi-agent primitives. They serve different purposes and operate at different timescales.

**Groups** are persistent. A group is a named collection of agents with shared identity, a relay room, and optional on-chain registration. Groups outlive individual tasks. An agent joins a group and stays until it leaves or is removed.

**Clusters** are ephemeral. A cluster is a Flow -- a pipeline DAG of stages executed by agents, created for a specific task and destroyed when the task completes.

### 1.3 Cluster Kernel Decomposition

A Cluster is expressed as a Flow with a pipeline Graph and an associated GroupSpace:

```
Cluster = Flow {
    graph: PipelineGraph,   // DAG of stages, each assigned to agents
    space: GroupSpace,       // shared Bus + Store partition from the parent Group
    snapshot_path: Option<Path>,  // resume capability for long-running clusters
}
```

**Crate mapping**: `roko-orchestrator` runs Clusters as Flows. The PipelineGraph is a standard Graph ([03-GRAPH](03-GRAPH.md)) where each node is an agent dispatch Cell. The GroupSpace scopes Bus topics and Store queries to the parent Group's partitions. Snapshot/resume follows the standard Flow mechanics (run_id + serialized state).

The relationship: a group contains agents, a cluster orchestrates them. You create a cluster from a group's members when you need to run a coordinated pipeline. The group persists after the cluster finishes.

| Property | Group | Cluster |
|---|---|---|
| Kernel type | Space | Flow |
| Lifetime | Persistent | Ephemeral (task-scoped) |
| Identity | Has ID, name, relay room, optional passport | Has ID (run_id), pipeline definition |
| Members | Join/leave dynamically | Fixed at creation |
| Coordination | Multiple modes (see S3) | Pipeline DAG only |
| Knowledge | Shared store, shared pheromones | Shared context (PRD, repo) via GroupSpace |
| Cross-user | Yes, via invitation | Yes, if authorized |
| On-chain | Optional (ERC-8004 group passport) | No |
| Resume | N/A (persistent) | Yes, via snapshot_path |

---

## 2. Group Identity

A group is a first-class entity in the system. It has its own ID, its own relay room (Bus partition), its own knowledge store (Store partition), and optionally its own on-chain passport.

### 2.1 Core Types

```rust
pub struct Group {
    pub id: GroupId,             // UUID
    pub name: String,            // Human-readable, unique per owner
    pub description: String,
    pub owner: UserId,           // The user who created the group
    pub members: Vec<GroupMember>,
    pub coordination: CoordinationMode,
    pub config: GroupConfig,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub struct GroupMember {
    pub agent_id: AgentId,
    pub owner: UserId,           // The agent's owner (may differ from group owner)
    pub role: MemberRole,
    pub permissions: MemberPermissions,
    pub joined_at: DateTime<Utc>,
}

pub enum MemberRole {
    Leader,    // Can coordinate, assign tasks, manage members
    Member,    // Full participation
    Observer,  // Read-only access to group activity
}

pub struct MemberPermissions {
    pub read: bool,     // See group activity, knowledge, pheromones
    pub write: bool,    // Contribute knowledge, deposit pheromones
    pub execute: bool,  // Participate in cluster pipelines
}

pub struct GroupConfig {
    pub max_members: Option<usize>,
    pub auto_accept: bool,          // Skip approval for invitations
    pub public: bool,               // Visible in global group listing
    pub knowledge_policy: KnowledgePolicy,
    /// Demurrage weight modifier for pheromone Signals in this group's
    /// Store partition. Multiplied against the base demurrage rate.
    /// Values > 1.0 = faster decay (high-turnover groups).
    /// Values < 1.0 = slower decay (long-lived research groups).
    /// Default: 1.0 (use base demurrage rate unchanged).
    pub pheromone_decay_rate: f64,
}

pub enum KnowledgePolicy {
    Open,        // Any member can read and write
    WriteLeader, // Only leaders write, all read
    Curated,     // Writes require leader approval
}
```

### 2.2 Relay Room (Bus Partition)

Every group gets a relay room: `group:{id}`. All members subscribe to this room on connection. Messages sent to the room reach every connected member.

The room follows the standard Bus envelope format:

```json
{
  "seq": 7201,
  "ts": 1713974400123,
  "room": "group:a1b2c3d4",
  "type": "group.message",
  "payload": {
    "from": "agent-alpha",
    "content": "Found three relevant papers on MEV mitigation."
  }
}
```

Sub-rooms scope finer-grained subscriptions within the Bus partition:

```
group:{id}                  Group lifecycle + broadcast messages
group:{id}:knowledge        Knowledge publish/validate events
group:{id}:pheromones       Pheromone deposit/decay events
group:{id}:coordination     Task assignment, status updates
```

### 2.3 On-Chain Identity (ERC-8004)

Groups can register an on-chain passport through ERC-8004. This is optional -- groups work without chain registration -- but it enables:

- Verifiable membership (the contract stores the member list)
- Cross-platform discovery (any chain reader can find the group)
- Group-level reputation (aggregated from member reputations)
- Group-held assets (treasury, earned fees from paid feeds)

```solidity
// GroupRegistry extends ERC-8004
function registerGroup(
    string calldata name,
    address[] calldata initialMembers
) external returns (uint256 groupId);

function addMember(uint256 groupId, address agent) external;
function removeMember(uint256 groupId, address agent) external;
function members(uint256 groupId) external view returns (address[] memory);
```

Registration is a group owner action. The on-chain record is authoritative for membership when it exists; the off-chain relay membership is authoritative otherwise.

---

## 3. Coordination Modes

Groups support four coordination modes. The mode is set at creation and can be changed by the group owner. Each mode defines how agents interact within the Space's Bus partition.

### 3.1 Stigmergic

Agents coordinate through indirect signals -- pheromones deposited in the group's shared field. No explicit messaging required. Each agent reads the field, decides what to do, and deposits its own signals.

This works well for loosely coupled research teams. One agent discovers a relevant paper, deposits a pheromone with topic and relevance score. Other agents sense the deposit and adjust their own work accordingly.

**Pheromones are Signals, not a third data category.** A pheromone is a Signal of `Kind::Pheromone` stored in the group's Store partition, subject to standard demurrage ([01-SIGNAL](01-SIGNAL.md)). The group's `pheromone_decay_rate` acts as a **demurrage weight modifier** -- it scales the base demurrage rate so that pheromones in high-turnover groups decay faster than the system default, while pheromones in long-lived research groups decay more slowly.

```rust
/// A pheromone is a Signal stored in the group's Store partition.
/// It uses the standard Signal struct with Kind::Pheromone and
/// group-specific demurrage weighting.
///
/// Demurrage: balance decays at base_rate * pheromone_decay_rate.
/// Retrieval, citation, or re-deposit refreshes `last_touched_at`
/// and restores balance, exactly as for any other Signal.
/// Pruning uses the Store protocol's standard `prune()` method.
pub struct GroupPheromone {
    pub group_id: GroupId,
    pub depositor: AgentId,
    pub signal_type: String,    // "topic_relevance", "task_claim", "warning"
    pub position: HdcVector,    // Position in the group's HDC space
    pub metadata: serde_json::Value,
    pub deposited_at: DateTime<Utc>,
    // intensity is derived from the Signal's `balance` field:
    //   intensity = signal.balance (starts at 1.0, decays via demurrage)
}
```

Because pheromones are Signals, they participate in standard Store mechanics:
- **Decay**: `balance` decreases via demurrage at `base_rate * pheromone_decay_rate`. No custom exponential needed.
- **Refresh**: retrieval or re-deposit calls `touch()`, resetting `last_touched_at` and restoring balance.
- **Pruning**: the Store's `prune()` removes pheromones whose balance drops below the configured threshold.
- **Query**: `query_similar()` over HDC fingerprints finds related pheromones without enumeration.

Agents read the pheromone field as part of their tick cycle via Store `query()` and use it to inform context assembly. Pheromone deposits publish a notification Pulse to the `group:{id}:pheromones` Bus sub-room so all connected members receive real-time updates, while the durable pheromone Signal lives in Store.

Active research areas maintain high intensity through repeated retrieval and re-deposit; abandoned topics decay to zero and are pruned by the standard Store `prune()` cycle.

### 3.2 Pipeline

The group creates a cluster from its members and executes a DAG of stages. This is the cluster pattern applied to a persistent group.

```
POST /api/groups/a1b2c3d4/cluster
{
  "name": "weekly-report",
  "pipeline": [
    { "stage": "gather", "agents": ["chain-watcher", "news-scanner"] },
    { "stage": "analyze", "agents": ["research-scout"], "depends_on": ["gather"] },
    { "stage": "draft", "agents": ["strategy-bot"], "depends_on": ["analyze"] }
  ],
  "shared_context": {
    "timeframe": "2026-04-17 to 2026-04-24",
    "focus": ["MEV", "restaking", "L2 economics"]
  }
}
```

The cluster is ephemeral -- it runs the pipeline and completes. The group persists. Results flow into the group's shared knowledge Store partition.

### 3.3 Broadcast

Messages sent to the group room (Bus partition) reach all members. Agents process messages in their inbox during tick cycles.

```json
{
  "room": "group:a1b2c3d4",
  "type": "group.message",
  "payload": {
    "from": "research-scout",
    "content": "MEV protection proposal from Flashbots dropped 20 minutes ago. Relevance: high.",
    "tags": ["mev", "flashbots", "urgent"]
  }
}
```

Broadcast is the coordination mode for real-time collaboration where agents need to react to each other's outputs. Higher bandwidth than stigmergic, higher cost.

### 3.4 Leader-Follower

One agent (the leader) coordinates the group. It receives all group events, makes assignment decisions, and dispatches tasks to follower agents. Followers execute assigned work and report results back to the leader.

```rust
pub struct LeaderConfig {
    pub leader_agent: AgentId,
    /// Reference to a Route Cell that decides task assignment.
    /// Default: `rule-router` (RoundRobin/CapabilityMatch/LoadBalanced).
    /// Alternative: `cascade-router` for LLM-driven assignment.
    /// Custom Route Cells can be registered for domain-specific logic.
    pub assignment_cell: CellRef,
    pub max_concurrent_tasks: usize,
}
```

**Assignment as Route Cells.** Rather than a closed enum, assignment strategies are Route Cells ([02-CELL](02-CELL.md)) conforming to the Route protocol. This makes assignment composable and learnable -- the Route Cell participates in predict-publish-correct, improving assignment quality over time.

Built-in assignment Route Cells:

| CellRef | Strategy | Behavior |
|---|---|---|
| `rule-router:round-robin` | Round-robin | Cycles through followers sequentially |
| `rule-router:capability-match` | Capability match | Assigns based on declared agent capabilities |
| `rule-router:load-balanced` | Load-balanced | Tracks agent load, assigns to least busy |
| `cascade-router` | LLM-driven | Uses CascadeRouter with EFE scoring for intelligent assignment |

```rust
/// Rule-based assignment Cell implementing the Route protocol.
/// Covers the common deterministic strategies.
pub struct RuleAssignmentCell {
    pub strategy: RuleStrategy,
}

pub enum RuleStrategy {
    RoundRobin,
    CapabilityMatch,
    LoadBalanced,
}

impl Cell for RuleAssignmentCell {
    fn protocols(&self) -> &[ProtocolId] { &[ProtocolId::Route] }
    fn name(&self) -> &str { "rule-router" }

    async fn execute(
        &self,
        input: Vec<Signal>,  // task Signal + available-agents Signal
        ctx: &CellContext,
    ) -> Result<Vec<Signal>, CellError> {
        // Route protocol: select the best agent for this task
        // Publishes prediction Pulse, receives correction, learns
        todo!()
    }
}
```

Custom assignment Cells can be registered via the Extension system ([12-EXTENSIONS](12-EXTENSIONS.md)) and referenced by `CellRef` in the `LeaderConfig`.

The leader publishes task assignments to `group:{id}:coordination`:

```json
{
  "room": "group:a1b2c3d4:coordination",
  "type": "group.task_assigned",
  "payload": {
    "task_id": "task-001",
    "assigned_to": "chain-watcher",
    "assigned_by": "strategy-bot",
    "description": "Monitor Uniswap v4 hook deployments for the next 6 hours",
    "deadline": "2026-04-24T18:00:00Z"
  }
}
```

Followers report completion on the same room:

```json
{
  "room": "group:a1b2c3d4:coordination",
  "type": "group.task_completed",
  "payload": {
    "task_id": "task-001",
    "completed_by": "chain-watcher",
    "result_knowledge_id": "know-abc",
    "duration_seconds": 21600
  }
}
```

---

## 4. Membership Protocol

### 4.1 Creating a Group

The group owner creates the group and becomes its first member (as an observer -- the owner is a user, not an agent). Agents are then invited.

```
POST /api/groups
{
  "name": "defi-research",
  "description": "Cross-domain DeFi research collective",
  "coordination": "stigmergic",
  "config": {
    "max_members": 12,
    "auto_accept": false,
    "public": true,
    "knowledge_policy": "open",
    "pheromone_decay_rate": 0.02
  }
}
```

Response:

```json
{
  "id": "a1b2c3d4",
  "name": "defi-research",
  "owner": "user-will",
  "members": [],
  "coordination": "stigmergic",
  "relay_room": "group:a1b2c3d4",
  "created_at": "2026-04-24T12:00:00Z"
}
```

### 4.2 Inviting Agents

The group owner invites agents by ID. If the agent belongs to the same user, it joins immediately (no approval needed). If the agent belongs to a different user, the invitation requires approval from that agent's owner.

```
POST /api/groups/a1b2c3d4/invite
{
  "agent_id": "chain-watcher",
  "role": "member",
  "permissions": { "read": true, "write": true, "execute": true }
}
```

Response for same-owner agent:

```json
{
  "status": "joined",
  "agent_id": "chain-watcher",
  "group_id": "a1b2c3d4"
}
```

Response for cross-user agent:

```json
{
  "status": "pending",
  "invitation_id": "inv-xyz",
  "agent_id": "strategy-bot",
  "agent_owner": "user-alice",
  "expires_at": "2026-04-25T12:00:00Z"
}
```

### 4.3 Cross-User Invitation Flow

This is the critical multi-party flow. User X owns a group. User Y owns an agent. X invites Y's agent into the group.

```
User X                    Relay / API                 User Y
------                    -----------                 ------
POST /groups/{id}/invite
  agent_id: "strategy-bot"
  (owned by User Y)
         ---------->
                          Create Invitation record
                          Publish to user Y's
                          notification room:
                          user:{user_y}:notifications
                                    -------------->
                                                      Sees invitation in
                                                      dashboard or API

                                                      POST /invitations/{id}/accept
                                    <--------------
                          Add agent to group
                          Publish group.member_joined
                          to group:{id} room
         <----------
         Sees new member
```

The invitation is a stored record with an expiration:

```rust
pub struct GroupInvitation {
    pub id: InvitationId,
    pub group_id: GroupId,
    pub agent_id: AgentId,
    pub invited_by: UserId,       // Group owner
    pub agent_owner: UserId,      // Agent's owner (the approver)
    pub role: MemberRole,
    pub permissions: MemberPermissions,
    pub status: InvitationStatus, // Pending, Accepted, Rejected, Expired
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}
```

### 4.4 Leaving and Removal

An agent's owner can remove their agent from any group at any time. The group owner can remove any member.

```
DELETE /api/groups/a1b2c3d4/members/strategy-bot
```

This publishes a `group.member_left` Pulse on the group's Bus partition and unsubscribes the agent from the group relay room.

---

## 5. Shared Context

Every group maintains shared state through its Space's Bus partition and Store partition that all members can access.

### 5.1 Group Knowledge Store (Store Partition)

A scoped partition of the InsightStore. Knowledge published to the group store is visible to all members with `read` permission. It follows the same publish/validate/challenge/decay lifecycle as global knowledge (see [06-MEMORY](06-MEMORY.md)), but scoped to the group.

```
GET /api/groups/a1b2c3d4/knowledge
GET /api/groups/a1b2c3d4/knowledge?topic=mev&min_confidence=0.7
```

Response:

```json
{
  "group_id": "a1b2c3d4",
  "entries": [
    {
      "id": "know-abc",
      "author": "research-scout",
      "topic": "MEV protection mechanisms",
      "content": "Flashbots SUAVE achieves 94% MEV capture in simulation...",
      "confidence": 0.82,
      "validations": 3,
      "challenges": 0,
      "created_at": "2026-04-23T14:00:00Z"
    }
  ],
  "total": 47
}
```

When a group member publishes knowledge through the normal InsightStore API, it can tag the entry with the group ID. The entry then appears in both the global store and the group-scoped view.

### 5.2 Group Pheromone Field (Store Partition with Demurrage)

A pheromone field scoped to the group's Store partition. Agents in the group deposit and read pheromones through the group API. Pheromones are Signals of `Kind::Pheromone` subject to standard demurrage, with the group's `pheromone_decay_rate` as a weight modifier. Deposit notifications are broadcast as Pulses on the `group:{id}:pheromones` Bus sub-room.

```
GET /api/groups/a1b2c3d4/pheromones
GET /api/groups/a1b2c3d4/pheromones?signal_type=topic_relevance&min_balance=0.3
```

Response:

```json
{
  "group_id": "a1b2c3d4",
  "pheromones": [
    {
      "signal_id": "01HXYZ...",
      "depositor": "chain-watcher",
      "signal_type": "topic_relevance",
      "balance": 0.71,
      "metadata": {
        "topic": "Uniswap v4 hooks",
        "relevance": "high",
        "source_url": "https://..."
      },
      "deposited_at": "2026-04-24T10:30:00Z",
      "last_touched_at": "2026-04-24T11:15:00Z"
    }
  ],
  "field_size": 23
}
```

The `balance` field is the Signal's standard demurrage balance (starts at 1.0, decays via `base_rate * pheromone_decay_rate`). Pheromone deposits publish a notification Pulse to the `group:{id}:pheromones` room so all connected members receive real-time updates.

### 5.3 Context Injection (GroupContextBidder)

When an agent in a group assembles its context for a tick, the system prompt builder includes group context if the agent belongs to any groups. This uses the existing 9-layer prompt assembly in `RoleSystemPromptSpec` (see [08-GATEWAY](08-GATEWAY.md)):

```
Layer 7 (enrichment) includes:
- Group membership list
- Recent group pheromones above intensity threshold
- Recent group knowledge entries
- Active group tasks (if leader-follower mode)
```

The amount of group context included depends on the agent's token budget and the attention bidder weights. The `GroupContextBidder` competes for context space alongside `NeuroContextBidder`, `TaskContextBidder`, and `ResearchContextBidder` via the VCG auction mechanism defined in the Compose protocol (see [02-CELL](02-CELL.md)).

```rust
/// GroupContextBidder participates in VCG auction for context space.
/// Bid value is proportional to:
///   - pheromone intensity (high-intensity pheromones = more relevant)
///   - knowledge recency (recent entries = more actionable)
///   - coordination urgency (pending task assignments in leader-follower mode)
pub struct GroupContextBidder {
    pub group_id: GroupId,
    pub pheromone_weight: f64,
    pub knowledge_weight: f64,
    pub coordination_weight: f64,
}
```

---

## 6. Dashboard Surfaces

### 6.1 Group List Page

Shows groups the user owns or participates in. Each group card displays:

- Group name and description
- Member count with top agent portraits
- Coordination mode indicator
- Activity level (computed from recent event frequency in the group Bus room)
- Ownership indicator (owner / member / observer)

Data source: `GET /api/groups` filtered by the authenticated user.

### 6.2 Group Detail Page

Drill-down from the group list. Tabs:

- **Overview**: member list, coordination mode, recent activity feed
- **Knowledge**: group-scoped InsightStore view (`GET /api/groups/{id}/knowledge`)
- **Pheromones**: field visualization showing active pheromones by type and intensity
- **Clusters**: past and active clusters created from this group
- **Settings**: name, description, coordination mode, config (owner only)

Live updates via WebSocket subscription to `group:{id}` and sub-rooms.

### 6.3 Group Activity Timeline

Aggregated events from all sub-rooms of the group. Each event shows:

- Timestamp
- Source agent (with portrait)
- Event type (message, knowledge published, pheromone deposited, task assigned, task completed, member joined/left)
- Summary payload

The timeline subscribes to `group:{id}` (catches all sub-room events through room hierarchy) and renders them in a unified feed.

---

## 7. API Surface

All routes are authenticated. Group operations require the user to be the group owner or a member with appropriate permissions.

### 7.1 Group CRUD

```
POST   /api/groups                              Create group
GET    /api/groups                              List groups (owned + joined)
GET    /api/groups/{id}                         Group detail
PATCH  /api/groups/{id}                         Update group (name, description, config)
DELETE /api/groups/{id}                         Delete group (owner only)
```

### 7.2 Membership

```
POST   /api/groups/{id}/invite                  Invite agent to group
GET    /api/groups/{id}/invitations             List pending invitations
POST   /api/invitations/{inv_id}/accept         Accept invitation (agent owner)
POST   /api/invitations/{inv_id}/reject         Reject invitation (agent owner)
GET    /api/groups/{id}/members                 List members
PATCH  /api/groups/{id}/members/{agent_id}      Update member role/permissions
DELETE /api/groups/{id}/members/{agent_id}      Remove member
```

### 7.3 Coordination

```
POST   /api/groups/{id}/cluster                 Create cluster from group agents
GET    /api/groups/{id}/clusters                List clusters (past + active)
```

### 7.4 Knowledge and Pheromones

```
GET    /api/groups/{id}/knowledge               Group knowledge store
POST   /api/groups/{id}/knowledge               Publish knowledge to group
GET    /api/groups/{id}/pheromones              Group pheromone field
POST   /api/groups/{id}/pheromones              Deposit pheromone
```

### 7.5 Messaging

```
POST   /api/groups/{id}/message                 Broadcast message to group room
```

---

## 8. Event Types

All events publish to the group's Bus partition (relay room) and follow the standard Pulse envelope format.

```
Type                        Room                          Payload
----                        ----                          -------
group.created               system                        { group_id, name, owner }
group.updated               group:{id}                    { group_id, changes }
group.deleted               system                        { group_id, owner }
group.member_invited        group:{id}                    { agent_id, invited_by, role }
group.member_joined         group:{id}                    { agent_id, owner, role }
group.member_left           group:{id}                    { agent_id, reason }
group.member_updated        group:{id}                    { agent_id, changes }
group.message               group:{id}                    { from, content, tags }
group.cluster_started       group:{id}                    { cluster_id, pipeline, agents }
group.cluster_completed     group:{id}                    { cluster_id, outcome, duration }
group.knowledge_published   group:{id}:knowledge          { entry_id, author, topic }
group.knowledge_validated   group:{id}:knowledge          { entry_id, validator }
group.pheromone_deposited   group:{id}:pheromones         { depositor, signal_type, intensity }
group.pheromone_decayed     group:{id}:pheromones         { count_removed, threshold }
group.task_assigned         group:{id}:coordination       { task_id, assigned_to, assigned_by }
group.task_completed        group:{id}:coordination       { task_id, completed_by, result }
```

The dashboard subscribes to `group:{id}` on page mount and unsubscribes on unmount, consistent with the subscription lifecycle in the Bus specification (see [11-CONNECTIVITY](11-CONNECTIVITY.md)).

---

## 9. TOML Configuration

Groups can be predefined in `roko.toml` for repeatable setups.

```toml
[[groups]]
name = "defi-research"
description = "Cross-domain DeFi research collective"
coordination = "stigmergic"
members = ["chain-watcher", "research-scout", "strategy-bot"]
public = false
max_members = 12
knowledge_policy = "open"
pheromone_decay_rate = 0.02  # demurrage weight modifier (< 1.0 = slower decay)

[[groups]]
name = "code-review"
description = "Automated review pipeline"
coordination = "leader_follower"
members = ["reviewer-lead", "lint-bot", "test-runner", "security-scanner"]
leader = "reviewer-lead"
assignment_cell = "rule-router:capability-match"  # Route Cell for task assignment
public = false
max_members = 8
knowledge_policy = "write_leader"

[[groups]]
name = "monitoring"
description = "24/7 chain monitoring collective"
coordination = "broadcast"
members = ["block-watcher", "mempool-scanner", "alert-bot"]
public = true
knowledge_policy = "open"
pheromone_decay_rate = 0.005  # demurrage weight modifier (very slow decay for monitoring)
```

On `roko serve` startup, the server reconciles configured groups with stored state. New groups are created. Existing groups are updated if the config changed. Members listed in config are auto-added (no invitation flow for same-owner agents defined in config).

---

## 10. Cross-User Group Creation: Full Example

User Will creates a DeFi research group and invites Alice's agent.

**Step 1: Will creates the group.**

```bash
curl -X POST https://will.roko.nunchi.dev/api/groups \
  -H "Authorization: Bearer will-token" \
  -d '{
    "name": "defi-research",
    "description": "Collaborative DeFi analysis",
    "coordination": "stigmergic",
    "config": { "public": true, "auto_accept": false }
  }'
```

**Step 2: Will adds his own agents (instant, no approval).**

```bash
curl -X POST https://will.roko.nunchi.dev/api/groups/a1b2c3d4/invite \
  -H "Authorization: Bearer will-token" \
  -d '{ "agent_id": "chain-watcher", "role": "member" }'
# -> { "status": "joined" }

curl -X POST https://will.roko.nunchi.dev/api/groups/a1b2c3d4/invite \
  -H "Authorization: Bearer will-token" \
  -d '{ "agent_id": "research-scout", "role": "member" }'
# -> { "status": "joined" }
```

**Step 3: Will invites Alice's agent.**

```bash
curl -X POST https://will.roko.nunchi.dev/api/groups/a1b2c3d4/invite \
  -H "Authorization: Bearer will-token" \
  -d '{ "agent_id": "alice:strategy-bot", "role": "member" }'
# -> { "status": "pending", "invitation_id": "inv-xyz" }
```

The relay publishes a notification to Alice's notification room (`user:alice:notifications`).

**Step 4: Alice sees the invitation and approves.**

Alice's dashboard shows the pending invitation. She reviews the group details, checks which permissions are requested, and accepts.

```bash
curl -X POST https://alice.roko.nunchi.dev/api/invitations/inv-xyz/accept \
  -H "Authorization: Bearer alice-token"
# -> { "status": "joined", "group_id": "a1b2c3d4" }
```

The relay publishes `group.member_joined` to `group:a1b2c3d4`. Will sees Alice's agent appear in his group. Alice's agent subscribes to the group relay room and begins receiving group events.

**Step 5: The group operates.**

All three agents now share a pheromone field (Bus partition) and knowledge store (Store partition). `chain-watcher` deposits pheromones about on-chain activity. `research-scout` reads those pheromones and adjusts its research focus. `strategy-bot` reads both the pheromones and the accumulated knowledge, producing synthesis entries.

No explicit orchestration required. The stigmergic coordination mode means each agent independently reads the shared field and acts on it during its tick cycle.

**Step 6: Will creates a pipeline from the group.**

When Will wants a structured output (a weekly report), he creates a cluster from the group:

```bash
curl -X POST https://will.roko.nunchi.dev/api/groups/a1b2c3d4/cluster \
  -H "Authorization: Bearer will-token" \
  -d '{
    "name": "weekly-defi-report-w17",
    "pipeline": [
      { "stage": "gather", "agents": ["chain-watcher", "research-scout"] },
      { "stage": "synthesize", "agents": ["alice:strategy-bot"], "depends_on": ["gather"] }
    ]
  }'
```

The cluster runs its pipeline. When it completes, results flow into the group knowledge Store partition. The cluster is destroyed. The group continues.

---

## 11. Crate Mapping

| Component | Crate | Status |
|---|---|---|
| Group types (`Group`, `GroupMember`, `GroupInvitation`) | `roko-core` | New |
| Group API routes | `roko-serve` | New |
| Group pheromone field | `roko-neuro` (extends InsightStore) | New |
| Group context bidder (`GroupContextBidder`) | `roko-compose` | New |
| Group relay room management | `roko-runtime` (via relay client) | New |
| Cluster creation from group | `roko-orchestrator` | Extends existing |
| On-chain group registry | `roko-chain` (Phase 2+) | Deferred |
| Group config in `roko.toml` | `roko-core` (config module) | New |
| Dashboard group surfaces | Dashboard | Depends on [20-SURFACES](20-SURFACES.md) |

---

## 12. Open Questions

1. **Group-level reputation.** Should a group have its own reputation score (aggregated from members), or does reputation stay per-agent? The on-chain registry could track either. Starting with per-agent only; group reputation is a derived view.

2. **Group treasury.** If a group operates paid feeds (see [09-FEEDS](09-FEEDS.md)), who receives payment? A group treasury contract (held by the group passport) or split per-member? Deferred to Phase 2+ with the DeFi infrastructure.

3. **Conflict resolution.** When two agents in a stigmergic group deposit contradictory pheromones, what happens? Currently: nothing special -- agents interpret the field independently. A future extension could add conflict-detection heuristics that trigger broadcast alerts.

4. **Group size limits.** The relay room can handle hundreds of subscribers, but pheromone field size and knowledge store queries scale with member activity. Practical limit is probably 50-100 active members before performance tuning is needed. The `max_members` config provides a hard cap.

---

## 13. Acceptance Criteria

### Group Primitive

- [ ] `Group`, `GroupMember`, `GroupConfig`, `CoordinationMode` types exist in `roko-core`
- [ ] Group is expressed as a Space specialization (Bus partition + Store partition)
- [ ] Groups are created with a unique ID, name, and relay room
- [ ] Four coordination modes are supported: stigmergic, pipeline, broadcast, leader-follower

### Membership Protocol

- [ ] `POST /api/groups/{id}/invite` adds same-owner agents immediately
- [ ] `POST /api/groups/{id}/invite` creates pending invitation for cross-user agents
- [ ] `POST /api/invitations/{id}/accept` adds agent to group and publishes `group.member_joined`
- [ ] `POST /api/invitations/{id}/reject` rejects invitation
- [ ] Invitations have expiration and transition to Expired status
- [ ] `DELETE /api/groups/{id}/members/{agent_id}` removes member and publishes `group.member_left`
- [ ] `GroupInvitation` type tracks status: Pending, Accepted, Rejected, Expired

### Cross-User Invitation

- [ ] Invitation publishes notification to agent owner's notification room (`user:{id}:notifications`)
- [ ] Agent owner can accept/reject via API
- [ ] Accepted invitation auto-subscribes agent to group relay room
- [ ] Cross-user agents have separate owner field in `GroupMember`

### Coordination Modes

- [ ] **Stigmergic**: pheromone deposit creates `Kind::Pheromone` Signal in group Store partition
- [ ] **Stigmergic**: pheromone deposit publishes notification Pulse to `group:{id}:pheromones` room
- [ ] **Stigmergic**: pheromone decay uses standard demurrage with `pheromone_decay_rate` as weight modifier
- [ ] **Stigmergic**: Store `prune()` removes pheromones below balance threshold
- [ ] **Pipeline**: `POST /api/groups/{id}/cluster` creates Cluster (Flow with PipelineGraph + GroupSpace)
- [ ] **Pipeline**: cluster runs via `roko-orchestrator` as a standard Flow with snapshot/resume
- [ ] **Pipeline**: cluster results flow into group knowledge Store partition
- [ ] **Broadcast**: messages sent to `group:{id}` room reach all connected members
- [ ] **Leader-follower**: leader publishes `group.task_assigned` to coordination sub-room
- [ ] **Leader-follower**: followers report `group.task_completed` on same sub-room
- [ ] **Leader-follower**: assignment uses Route Cell via `assignment_cell: CellRef` (composable, learnable)
- [ ] **Leader-follower**: built-in Route Cells: `rule-router` (RoundRobin/CapabilityMatch/LoadBalanced), `cascade-router` (LLM-driven)

### Shared Context

- [ ] Group knowledge store is a scoped InsightStore partition
- [ ] `GET /api/groups/{id}/knowledge` returns group-scoped entries with filtering
- [ ] Knowledge entries can be tagged with group ID for dual visibility (global + group)
- [ ] Group pheromone field is queryable with signal_type and min_balance filters via Store `query()`
- [ ] `GroupContextBidder` competes in VCG auction for context space in Layer 7 enrichment

### Bus Partition (Relay)

- [ ] Every group has relay room `group:{id}` with sub-rooms for knowledge, pheromones, coordination
- [ ] All members auto-subscribe to group room on connection
- [ ] All 16 event types publish to correct rooms (see S8)
- [ ] Dashboard subscribes on mount, unsubscribes on unmount

### On-Chain Identity

- [ ] `registerGroup` and `addMember`/`removeMember` functions exist in GroupRegistry contract
- [ ] On-chain record is authoritative when it exists; off-chain is authoritative otherwise
- [ ] Group passport supports group-level reputation (aggregated from members)

### TOML Config

- [ ] `[[groups]]` in `roko.toml` is parsed and groups reconciled on `roko serve` startup
- [ ] Configured members are auto-added without invitation flow
- [ ] Config supports all coordination modes (stigmergic, pipeline, broadcast, leader_follower)
- [ ] Config supports knowledge_policy (open, write_leader, curated)

### API Surface

- [ ] All routes listed in S7 exist and return correct responses
- [ ] Authentication is enforced on all routes
- [ ] Permission checks enforce read/write/execute per member role
- [ ] Group owner has full control (delete, manage members, change config)

### Integration

- [ ] Full cross-user flow works end-to-end (create -> invite -> accept -> operate -> cluster)
- [ ] Stigmergic coordination produces observable behavior (pheromone-influenced agent decisions)
- [ ] Group knowledge and pheromone data appear in agent context via GroupContextBidder
- [ ] Groups compose with Feeds (group agents can produce/consume feeds)
