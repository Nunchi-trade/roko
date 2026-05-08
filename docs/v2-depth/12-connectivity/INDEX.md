# 12-connectivity — Depth Index

Depth for [11-CONNECTIVITY.md](../../v2/11-CONNECTIVITY.md)

---

## Source docs (0)

No additional `docs/` sources beyond spec coverage.

---

## Depth docs

| # | Document | What It Covers |
|---|---|---|
| [01](01-relay-wire-protocol.md) | **Relay Wire Protocol** | Frame-level WebSocket spec: connection lifecycle, inbound/outbound frame types, envelope structure, ring buffer sequencing, request/response bridging, HTTP API surface |
| [02](02-topic-namespace.md) | **Topic Namespace & Grammar** | Dot-separated ABNF grammar, canonical topic namespace (chain, isfr, job, agent, feed, group, workspace, system), wildcard subscriptions, topic lifecycle |
| [03](03-chain-event-projection.md) | **Chain Event Projection** | Chain watcher architecture, contract events watched (AgentRegistry, MultiAgentMarket, ISFROracle), ChainEventSource trait, finality tagging, reorg handling |
| [04](04-relay-deployment.md) | **Relay Deployment Models** | Three models (sidecar, shared, validator-embedded), multi-relay connection, library+binary architecture, auth, Docker/Railway deployment, scaling |
| [05](05-coordination-patterns.md) | **Coordination Patterns** | Three primitives (pub/sub, request/response, on-chain), job discovery walkthrough, ISFR symphony example, feed subscription, presence, group coordination |
