# Permissioned Subnets: Private Collective Meshes

> **Layer**: L4 Orchestration (organizational coordination), L1 Framework (access control)
>
> **Synapse traits**: `Substrate` (scoped storage with access control), `Gate` (permission
> verification), `Policy` (publishing decisions)
>
> **Prerequisites**: `05-pheromone-scope.md` (scope hierarchy),
> `06-agent-mesh-sync.md` (Agent Mesh transport)


> **Implementation**: Specified

---

## Overview

A permissioned subnet is a private Agent Mesh scope within the broader Mesh layer. While the
standard `Mesh(CollectiveId)` scope makes pheromones visible to all members of a Collective,
permissioned subnets add access control — restricting visibility to invited agents or role-based
groups within a Collective.

Permissioned subnets address the needs of organizations that want the benefits of stigmergic
coordination (emergent specialization, indirect communication, collective intelligence) while
maintaining control over information flow. A company running a Collective of agents can:

1. Keep internal knowledge private by default
2. Maintain an internal reputation system separate from public reputation
3. Allow selective publishing of validated findings to the broader Mesh or Global scope
4. Create project-specific or team-specific coordination spaces within the organization

---

## Architecture

### Subnet as Nested Scope

A permissioned subnet is a refinement of the `Mesh(CollectiveId)` scope:

```
Scope Hierarchy:
├── Global (Korai chain — all agents)
├── Mesh(CollectiveId) (standard Collective — all members)
│   ├── Subnet("engineering") (permissioned — engineering agents only)
│   ├── Subnet("research") (permissioned — research agents only)
│   └── Subnet("security") (permissioned — security agents + admins)
└── Local(SubstrateId) (agent-private)
```

Subnets do not replace the standard Mesh scope — they add a finer-grained layer within it.
An agent in the "engineering" subnet can still see standard Mesh-scope pheromones from all
Collective members, but pheromones deposited specifically at subnet scope are visible only
to other "engineering" subnet members.

### Subnet Identity

Each subnet is identified by a `SubnetId` that combines the parent `CollectiveId` with a
subnet name:

```rust
/// A permissioned subnet within a Collective.
///
/// Subnets provide access-controlled pheromone scopes within the
/// broader Collective Mesh. Only invited or role-matching agents
/// can see subnet-scoped pheromones.
pub struct SubnetId {
    /// The parent Collective.
    pub collective: CollectiveId,
    /// The subnet name. Must be unique within the Collective.
    /// Alphanumeric + hyphens, max 64 chars.
    pub name: String,
}

/// Extended scope enum with subnet support.
pub enum PheromoneScope {
    Local(SubstrateId),
    Mesh(CollectiveId),
    Subnet(SubnetId),     // New: permissioned scope within a Collective
    Global,
}
```

---

## Access Control Models

Three access control models for subnet membership:

### 1. Invite-Based

The subnet creator explicitly invites agents by `AgentId`. Only invited agents can join and
see subnet-scoped pheromones.

```toml
[mesh.subnets.security-team]
access_model = "invite"
members = [
    "agent-security-lead",
    "agent-security-scanner",
    "agent-security-reviewer",
]
```

**Use case**: Sensitive projects, security work, pre-release development.

### 2. Role-Based

Agents matching a specific role predicate automatically gain subnet access. The predicate can
check agent type, capabilities, or domain specialization.

```toml
[mesh.subnets.engineering]
access_model = "role"
role_predicate = "agent_type == 'coding' OR agent_type == 'testing'"
```

**Use case**: Team-based organization where agents are assigned roles.

### 3. Reputation-Based

Agents above a reputation threshold in the relevant domain automatically gain access. This
creates a meritocratic subnet where proven competence is the admission criterion.

```toml
[mesh.subnets.elite-research]
access_model = "reputation"
min_reputation = 0.85
domain = "research"
```

**Use case**: High-stakes decision-making, quality-gated knowledge sharing.

---

## Internal Reputation

Each permissioned subnet can maintain its own internal reputation system, separate from the
public Collective and Global reputation:

### Why Separate Reputation?

Public reputation measures an agent's general track record across all interactions. Internal
reputation measures performance within a specific organizational context:

| Aspect | Public Reputation | Subnet Reputation |
|--------|------------------|-------------------|
| Scope | All agents on the network | Subnet members only |
| Evaluation | Marketplace and cross-collective interactions | Project-specific contributions |
| Visibility | Public (anyone can query) | Private (subnet members only) |
| Trust level | Lower (anonymous/pseudonymous) | Higher (known organizational context) |
| Update frequency | Per-job completion | Per-contribution (finer-grained) |

### Internal Reputation Scoring

Subnet reputation uses the same EMA (Exponential Moving Average) mechanics as public
reputation, but with different input signals:

```
Internal EMA: R_new = α × contribution_quality + (1-α) × R_old
```

Where `contribution_quality` is scored based on:

- Pheromone accuracy: Did the agent's pheromone deposits match later-validated reality?
- Task completion: Did the agent's work pass gate verification?
- Knowledge value: Were the agent's Wisdom deposits confirmed by peers?
- Collaboration: Did the agent's work enable other agents' success?

---

## Opt-In Publishing

Permissioned subnets support controlled publishing of knowledge to broader scopes:

### Publishing Pipeline

```
Subnet-scope pheromone (private)
    ↓ [Publishing gate: minimum confirmations + human approval]
Mesh-scope pheromone (Collective-wide)
    ↓ [Promotion gate: consensus + reputation]
Global-scope Engram (public)
```

Each transition is controlled by a gate:

| Transition | Gate | Requirements |
|-----------|------|-------------|
| Subnet → Mesh | Publishing gate | ≥2 subnet member confirmations + optional human approval |
| Mesh → Global | Promotion gate | ≥4 Collective member confirmations + minimum reputation |

### Publishing Configuration

```toml
[mesh.subnets.engineering.publishing]
# Automatic publishing to Mesh scope when conditions are met
auto_publish = false  # Require explicit approval
min_confirmations = 2
require_human_approval = true

# What types of pheromones can be published
publishable_kinds = ["Wisdom", "Consensus", "Pattern"]
# Kinds that never leave the subnet
restricted_kinds = ["Threat", "Alpha"]  # Threats and alpha stay private
```

### Information Boundary Enforcement

The publishing gate enforces an information boundary: subnet-private pheromones cannot leak
to broader scopes without passing through the gate. This is implemented at the transport
layer — the Agent Mesh relay refuses to forward subnet-scoped messages to non-members,
regardless of what the sending agent requests.

```rust
/// Verify that a pheromone propagation request respects scope boundaries.
///
/// Returns Err if the sender attempts to:
/// - Forward a subnet-scoped pheromone to a non-member
/// - Publish a restricted-kind pheromone beyond the subnet
/// - Bypass the publishing gate
pub fn verify_scope_boundary(
    sender: &AgentId,
    pheromone: &Pheromone,
    target_scope: &PheromoneScope,
    subnet_config: &SubnetConfig,
) -> Result<(), ScopeBoundaryViolation> {
    // Check: is sender a member of the source subnet?
    if let PheromoneScope::Subnet(ref subnet_id) = pheromone.scope {
        if !subnet_config.is_member(sender, subnet_id) {
            return Err(ScopeBoundaryViolation::NotAMember);
        }
    }

    // Check: is the target scope broader than the source?
    if target_scope.is_broader_than(&pheromone.scope) {
        // Publishing gate must be satisfied
        if !subnet_config.publishing_gate_satisfied(pheromone) {
            return Err(ScopeBoundaryViolation::PublishingGateNotMet);
        }
        // Check restricted kinds
        if subnet_config.is_restricted_kind(&pheromone.kind) {
            return Err(ScopeBoundaryViolation::RestrictedKind);
        }
    }

    Ok(())
}
```

---

## Morphogenetic Specialization Within Subnets

Morphogenetic specialization (see `07-morphogenetic-specialization.md`) operates within
subnets just as it does within full Collectives. Agents in an "engineering" subnet specialize
into different engineering roles (feature development, testing, refactoring, documentation)
based on the subnet's pheromone field.

The inhibition signals for morphogenetic specialization are computed from the subnet's member
population, not the full Collective. This means a small subnet (e.g., 3 agents in "security")
produces different specialization patterns than a large subnet (e.g., 10 agents in
"engineering").

### Cross-Subnet Specialization

When an agent belongs to multiple subnets (e.g., an agent in both "engineering" and "security"),
its morphogenetic state reflects the combined inhibition pressure from all its subnets. This
naturally pushes it toward the intersection of roles — a security-focused engineering specialist,
for example.

---

## Organizational Patterns

### Pattern 1: Team-Based Subnets

```
Collective: "acme-corp"
├── Subnet: "frontend" (role: coding agents with frontend domain)
├── Subnet: "backend" (role: coding agents with backend domain)
├── Subnet: "devops" (role: operations agents)
├── Subnet: "security" (invite: security-auditor, security-scanner)
└── Standard Mesh: all agents see company-wide pheromones
```

### Pattern 2: Project-Based Subnets

```
Collective: "acme-corp"
├── Subnet: "project-alpha" (invite: agents assigned to Project Alpha)
├── Subnet: "project-beta" (invite: agents assigned to Project Beta)
├── Subnet: "shared-infra" (role: infrastructure agents)
└── Standard Mesh: cross-project discoveries and company-wide signals
```

### Pattern 3: Clearance-Based Subnets

```
Collective: "research-lab"
├── Subnet: "public-research" (reputation >= 0.5)
├── Subnet: "advanced-research" (reputation >= 0.75)
├── Subnet: "classified" (invite-only, no auto-publish)
└── Standard Mesh: general coordination
```

---

## Security Considerations

### Threat Model

| Threat | Protection |
|--------|-----------|
| Unauthorized subnet access | Access control gate (invite/role/reputation) |
| Pheromone leakage to broader scope | Scope boundary enforcement at transport layer |
| Sybil attack on subnet membership | Reputation-based access requires established track record |
| Internal member compromise | Subnet reputation tracks member behavior; anomalous agents can be expelled |
| Cross-subnet information leakage | Agents in multiple subnets must respect each subnet's publishing policy |

### Audit Trail

All subnet operations (join, leave, deposit, publish) are logged to the Collective's audit log.
Subnet administrators can review the audit trail to detect policy violations.

---

## Relationship to Club Goods Theory

Permissioned subnets implement what economists call "club goods" — goods that are excludable
(non-members can be prevented from accessing them) but non-rivalrous (one member's use does
not diminish the value for other members) [Buchanan, J.M. "An Economic Theory of Clubs."
*Economica*, 32(125):1-14, 1965].

In Roko's context:

- **Excludability**: Subnet pheromones are visible only to members (enforced by access
  control)
- **Non-rivalrousness**: One agent sensing a pheromone does not diminish its availability to
  other agents

This economic structure incentivizes collective knowledge production within subnets: members
benefit from shared knowledge without the free-rider problem that affects pure public goods
(Global scope). The opt-in publishing mechanism allows subnets to selectively convert club
goods into public goods when the collective benefit outweighs the competitive advantage of
privacy.

---

## Configuration

```toml
# Define subnets in roko.toml
[mesh.subnets]

[mesh.subnets.engineering]
access_model = "role"
role_predicate = "agent_type == 'coding' OR agent_type == 'testing'"
morphogenetic_enabled = true

[mesh.subnets.engineering.publishing]
auto_publish = false
min_confirmations = 2
require_human_approval = true
publishable_kinds = ["Wisdom", "Consensus", "Pattern"]
restricted_kinds = ["Threat", "Alpha"]

[mesh.subnets.security]
access_model = "invite"
members = ["agent-security-lead", "agent-security-scanner"]
morphogenetic_enabled = true

[mesh.subnets.security.publishing]
auto_publish = false
min_confirmations = 1
require_human_approval = true
publishable_kinds = ["Threat", "Wisdom"]  # Security Threats CAN be published
restricted_kinds = ["Alpha"]

[mesh.subnets.research]
access_model = "reputation"
min_reputation = 0.7
domain = "research"
morphogenetic_enabled = true

[mesh.subnets.research.publishing]
auto_publish = true  # Research findings auto-publish after confirmation
min_confirmations = 3
require_human_approval = false
publishable_kinds = ["Wisdom", "Consensus"]
restricted_kinds = []
```

---

## Current Implementation Status

Permissioned subnets are **not yet implemented** in the Roko codebase. The design is specified
in the refactoring PRD (`refactoring-prd/04-knowledge-and-mesh.md`) and the legacy Styx
architecture (`bardo-backup/prd/20-styx/03-clade-sync.md`). Implementation would require:

1. Extending `PheromoneScope` with the `Subnet(SubnetId)` variant
2. Adding access control checks to the Agent Mesh relay
3. Implementing the publishing gate
4. Adding subnet configuration to `roko.toml` parsing
5. Updating the morphogenetic coordinator to operate within subnet boundaries

This is tracked as a future capability (post-Tier 5 in the implementation roadmap). The
current system supports only the three-scope model (Local, Mesh, Global) without intra-Mesh
access control.

---

## References

- [Buchanan 1965] Economic Theory of Clubs, *Economica*
- [Grossman & Stiglitz 1980] Informationally Efficient Markets, *AER*
- [Ostrom, E. 1990] *Governing the Commons*, Cambridge University Press — Framework for
  managing shared resources without privatization or state control

---

## Cross-References

- `05-pheromone-scope.md` — The three-scope hierarchy that subnets refine
- `06-agent-mesh-sync.md` — Transport layer that enforces scope boundaries
- `07-morphogenetic-specialization.md` — Specialization within subnet boundaries
- `11-collective-intelligence-metrics.md` — Measuring subnet effectiveness
