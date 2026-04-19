# Dream Token Economy

> Make dream cycles a market good: dreams that promote hypotheses to Consolidated knowledge earn more dream budget for the next cycle; dreams that produce no promotions deplete budget via depreciation, creating a self-regulating Return on Imagination (ROI) loop.

**Kind**: Innovation
**Status**: Speculative
**Source domain**: Economics (mechanism design, intrinsic motivation), reinforcement learning
**Affects subsystems**: Dreams (roko-dreams), Learning (roko-learn), Neuro (knowledge promotion)
**Last reviewed**: 2026-04-19

---

## The idea

Roko's dream cycle consumes real resources: NREM replay uses Haiku (~$0.001/episode), REM imagination uses Sonnet (~$0.01/counterfactual). The learning system has budget guardrails (per-task, per-session, per-day). But dream spending is unregulated — there's no feedback loop between dream *quality* and dream *budget*.

The missing link: **dreams should be a market good**. High-quality dreams (verified hypotheses that promote to Consolidated knowledge) should earn the agent more dream budget. Low-quality dreams (unverifiable, redundant, or violated hypotheses) should cost budget without return.

The Dream Economy formula:

```
DreamBudget(t+1) = DreamBudget(t) × (1 + ROI(t) − depreciation)

ROI(t) = earned(t) / spent(t)

earned(t) = Σ (tier_value × validation_count × novelty_bonus)
            for each hypothesis promoted from dream cycle t

tier_value = {Transient: 0.01, Working: 0.10, Consolidated: 1.00, Persistent: 10.00}
novelty_bonus = 2.0 if novel (HDC-dissimilar), else 1.0
depreciation = 0.05 per cycle
Budget ∈ [$0.01, $1.00]
```

Dream quality score is a weighted composite:
- 30% verification rate (verified scenarios / total)
- 30% promotion rate (promoted / staged hypotheses)
- 20% novelty rate (novel insights / total insights)
- 20% depotentiation effect (arousal reduction / pre-dream arousal)

Quality grade drives phase allocation: Grade A/B → 60% REM (creative); Grade D → 70% NREM (consolidation safer).

This creates a self-regulating system: agents that dream well earn the right to dream more. Agents that dream poorly are forced to rely on waking experience until dream quality improves.

## Origin

- **Mantiuk, Becker & Wu (2025)** "From Curiosity to Competence: How World Models Interact with the Dynamics of Exploration," arXiv:2507.08210. Two-way interaction: world model accuracy determines intrinsic motivation value.
- **"INTUITOR" (2025)** "Learning to Reason without External Rewards," arXiv:2505.19590. Model self-certainty as intrinsic reward — epistemic quality is the currency.
- **Burda et al. / DreamerV3-XP (2025)** "Optimising Exploration Through Uncertainty Estimation," arXiv:2510.21418. Ensemble disagreement as intrinsic reward; dream quality (ensemble agreement) is the internal economy token.
- **Lin et al. (2025)** "Scaling LLM Test-Time Compute Optimally Can be More Effective than Scaling Model Parameters." 5× reduction in test-time compute via offline training — but only if dream quality is high.

## Application to Roko

Seven integration steps are specified:

1. Add `DreamBudget` to `roko-learn/src/dream_economy.rs`.
2. Initialise budget from `roko.toml` `[dreams.budget]` section (`initial_usd`, `min`, `max`).
3. Check `can_afford()` before dream cycle in `roko-dreams/src/scheduler.rs`.
4. Track knowledge promotions from dream hypotheses via `roko-neuro` promotion events.
5. Call `budget.update()` after cycle completion in `roko-dreams/src/runner.rs`.
6. Log economy events to `.roko/learn/dream-economy.jsonl`.
7. Wire allocation fractions into phase token limits in `roko-dreams/src/runner.rs`.

## Estimated impact

Source states: "Budget increases when dream produces Consolidated knowledge (ROI > 1.0)." "Budget never drops below $0.01." "Budget never exceeds $1.00." "Phase allocation sums to 1.0 for all quality grades." Ranked **P1** in the implementation priority table (small effort).

## Prerequisites

- Knowledge promotion events linkable to their source dream cycle.
- Dream cycle cost tracking (model tokens × price per token).
- `roko-dreams` scheduler refactored to accept budget gating.

## Status

Speculative — idea only; no formal evaluation. Ranked **P1** in the source implementation priority table (small effort, high value).

## Risks and objections

- The tier_values (Transient: 0.01, Persistent: 10.00) are arbitrary; miscalibration could cause budget runaway or premature budget starvation.
- A 5% per-cycle depreciation forces the agent to continuously produce high-quality dreams or budget shrinks — this may create pressure to over-dream during otherwise idle periods.
- Linking promotions to source dream cycles requires careful provenance tracking; incorrect attribution (false positives) inflates ROI.
- The $1.00 ceiling may be too restrictive for agents running intensive learning phases.

## Related innovations

- [dream-verification](./dream-verification.md) — verification results directly determine the verification_rate component of quality score
- [hdc-active-inference](./hdc-active-inference.md) — NREM Delta consolidation that updates μ_prior is funded by dream budget
- [knowledge-morphogenesis](./knowledge-morphogenesis.md) — Persistent-tier knowledge (highest dream revenue) also dominates morphogenetic concentration
- [witness-world-model](./witness-world-model.md) — dream-enriched world model edges also contribute to earned dream revenue

## References

- Mantiuk, Becker & Wu (2025). From Curiosity to Competence. arXiv:2507.08210.
- INTUITOR (2025). Learning to Reason without External Rewards. arXiv:2505.19590.
- Burda et al. / DreamerV3-XP (2025). Optimising Exploration Through Uncertainty Estimation. arXiv:2510.21418.
- Lin et al. (2025). Scaling LLM Test-Time Compute Optimally Can be More Effective than Scaling Model Parameters.
