# Architecture Core Queue

This plan is the handoff queue for `tmp/architecture-plans/06-architecture-implementation.md`.
It is intentionally a queue of bounded packets, not an implementation wave.

Entry points:

- Validate shape: `roko plan validate plans/architecture-core-queue`
- Enumerate/run through the self-hosting path: `roko plan run plans/architecture-core-queue`

The queue uses `queue_kind = "architecture_implementation"` so `roko plan validate`
requires every packet to declare source docs, likely crates/artifacts, verification
commands, dependency metadata, and a typed acceptance contract.
