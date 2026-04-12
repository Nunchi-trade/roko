# Context Engineering

> Academic foundations for context assembly, prompt optimization, retrieval-augmented generation, and attention management in Roko's Composer and context pipeline.

**Topic**: [References](./INDEX.md)
**Prerequisites**: [Architecture](../00-architecture/INDEX.md), [Scaffold](../02-scaffold/INDEX.md)
**Key sources**: `bardo-backup/prd/02-mortality/14-research-foundations.md` §8, `bardo-backup/prd/shared/citations.md` §27, `bardo-backup/tmp/mori-agents/12-references.md`

---

## Abstract

Context failures, not model failures, cause most agent breakdowns. For agents running long tasks, context assembly is the highest-leverage cognitive system. The research here establishes that effective context management requires active curation, not passive accumulation — and that the same resource pressure that shapes agent behavior also shapes what enters the context window. The 6x context reduction achievable through proper context engineering (CSO) directly reduces compute costs.

---

## Agentic Context Engineering

- Zhang, H. et al. (2026). ACE: Agentic Context Engineering. _ICLR_, 2026. arXiv:2510.04618.
  *Grounds: Generator-Reflector-Curator cycle — treats context as an evolving playbook; +10.6% on AppWorld. Context assembly self-improves via cybernetic feedback. The three-role architecture (Generator creates, Reflector critiques, Curator compresses) maps directly to Roko's compose-verify-persist loop.*

- Samsung Research (2025). CSO: Context State Object Architecture. arXiv:2511.03728.
  *Grounds: Structured context compression — 6x initial reduction, 10-25x growth rate reduction. Compressed structured context replaces raw history. Grounds the Composer's structured context representation.*

- Kang, S. et al. (2025). ACON: Agentic Context Compression. arXiv:2510.00615.
  *Grounds: Failure-driven compression — 26-54% peak token reduction. Compaction optimized by learning which compressions preserve task-relevant information and which lose it.*

- Lindenbauer, T. et al. (2025). Observation Masking in Agent Context. _NeurIPS_, 2025.
  *Grounds: T0 suppression pattern — observation masking halves cost while matching LLM summarization quality. Mask stale observations rather than summarize them. Validates the T0 probe approach: skip LLM entirely when observations haven't changed.*

---

## Context Attribution

- Cohen-Wang, B., Shah, H., Georgiev, B., & Madry, A. (2024). ContextCite: Attributing Model Generation to Context. arXiv:2409.00729.
  *Grounds: Context pruning by attribution — contributive attribution via sparse linear model with 64 ablation passes. Context items pruned by measured attribution score, not heuristic importance.*

---

## Retrieval-Augmented Generation

- Lewis, P. et al. (2020). Retrieval-Augmented Generation for Knowledge-Intensive NLP Tasks. _NeurIPS_, 2020. arXiv:2005.11401.
  *Grounds: RAG architecture — augmenting language models with a retrieval step that fetches relevant documents before generation. Roko's per-tick context assembly (query NeuroStore → assemble context pack → inject into prompt) is a structural implementation of RAG.*

- Sarthi, P. et al. (2024). RAPTOR: Recursive Abstractive Processing for Tree-Organized Retrieval. _ICLR_, 2024.
  *Grounds: Hierarchical retrieval — recursive summarization creates a tree of abstractions at multiple granularities. Grounds the multi-tier knowledge retrieval in NeuroStore.*

- Gutierrez, B., Yang, Y., & Yu, J. (2024). HippoRAG: Neurobiologically-Inspired Long-Term Memory for LLMs. arXiv:2405.14831.
  *Grounds: Hippocampal retrieval — neurobiologically-inspired retrieval architecture combining pattern separation and pattern completion. Informs the NeuroStore's dual-process retrieval.*

---

## Context Window Behavior

- Liu, N.F. et al. (2024). Lost in the Middle: How Language Models Use Long Contexts. _TACL_, 2024. arXiv:2307.03172.
  *Grounds: Context position strategy — U-shaped attention: models attend most to the beginning and end of context, largely ignoring the middle. Directly motivates context assembly: highest-priority content at the beginning, second-highest at the end.*

- Du, Y. et al. (2025). Context Length Hurts: Even Whitespace Degrades Performance 13.9-85%. _EMNLP_, 2025.
  *Grounds: Aggressive compression mandate — even whitespace and formatting overhead degrades model performance. Motivates aggressive compression in prompt assembly.*

- Shi, F. et al. (2023). Large Language Models Can Be Easily Distracted by Irrelevant Context. _ICML_, 2023.
  *Grounds: Context filtering — irrelevant context actively degrades performance. Quality filtering in context assembly is not optional but required.*

- Joren, T. et al. (2025). Sufficient Context: A New Lens on Retrieval Augmented Generation Systems. _ICLR_, 2025.
  *Grounds: Insufficient context danger — insufficient context can make models 6x worse than no context at all (10.2% → 66.1% incorrect). Motivates aggressive context quality filtering.*

---

## Prompt Engineering Foundations

- Anthropic (2025). Context Engineering for Agents. anthropic.com.
  *Grounds: Two-layer context — effective context = pre-loaded static + just-in-time retrieval. Roko implements this as `roko.toml` static config + per-tick dynamic RAG.*

- Karpathy, A. (2026). autoresearch. GitHub.
  *Grounds: Automated research context — automated context assembly for research tasks. Informs the context engineering approach in Roko's research agent.*

- Wei, J. et al. (2022). Chain-of-Thought Prompting Elicits Reasoning in Large Language Models. _NeurIPS_, 2022. arXiv:2201.11903.
  *Grounds: StrategyFragment knowledge type — step-by-step reasoning exemplars dramatically improve performance. Agents post procedural knowledge as StrategyFragment Engrams that other agents retrieve as chain-of-thought exemplars.*

---

## Prompt Compression

- Pan, Z. et al. (2024). LLMLingua-2: Data Distillation for Efficient and Faithful Task-Agnostic Prompt Compression. _ACL_, 2024.
  *Grounds: Prompt compression — data distillation approach to task-agnostic prompt compression. Applicable to Composer's context budget management.*

- Factory.ai (2026). Evaluating Context Compression for Long-Context LLM Applications. 2026.
  *Grounds: Compression evaluation — systematic evaluation of context compression methods for production LLM applications.*

---

## Cross-references

- See [06-self-learning-systems.md](./06-self-learning-systems.md) for ACE in the learning context
- See [14-agent-harnesses-and-tool-use.md](./14-agent-harnesses-and-tool-use.md) for Meta-Harness
- See [21-mechanism-design.md](./21-mechanism-design.md) for VCG attention auction
- See topic [02-scaffold](../02-scaffold/INDEX.md) for full Scaffold layer design
