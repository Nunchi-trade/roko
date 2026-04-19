# Subsystem: Heartbeat / Tier Gating

Innovations that affect the heartbeat loop and T0/T1/T2 tier gating mechanism.

| Slug | Interaction |
|---|---|
| [hdc-active-inference](../hdc-active-inference.md) | Free energy F derived from Hamming distance replaces the scalar anomaly-count formula for tier gating; F < 0.10 → T0, F < 0.25 → T1, F ≥ 0.25 → T2. |
| [code-somatic-markers](../code-somatic-markers.md) | Somatic query bias before task dispatch can escalate the tier bias (e.g., danger zone → T2). |
| [witness-world-model](../witness-world-model.md) | Expected Free Energy over the causal world model replaces heuristic routing in heartbeat step 3 (ATTEND). |
