# Multi-Provider Model Routing Demo

Demonstrates roko's cascade router learning across multiple LLM providers.

## Providers

| Provider | Kind | Models | API Key |
|---|---|---|---|
| Anthropic | anthropic_api | claude-sonnet, claude-haiku | `ANTHROPIC_API_KEY` |
| Moonshot (Kimi) | openai_compat | kimi-k2.5 | `MOONSHOT_API_KEY` |
| Z.AI (GLM) | openai_compat | glm-5.1 | `ZAI_API_KEY` |
| Ollama | openai_compat | llama3.2, gemma4 | (local, no key) |

## Setup

1. **Build roko:**
   ```bash
   cargo build -p roko-cli --release
   ```

2. **Set API keys:**
   ```bash
   cp .env.example .env
   # Edit .env with your keys
   source .env
   ```

3. **Start Ollama** (optional):
   ```bash
   ollama serve &
   ollama pull llama3.2
   ollama pull gemma4
   ```

## Running the Demos

### 01 — Provider Health Check
Verifies all configured providers are reachable:
```bash
bash 01-provider-healthcheck.sh
```

### 02 — Model Comparison
Runs the same prompt through each available model:
```bash
bash 02-model-comparison.sh
bash 02-model-comparison.sh "Write a function to sort a list"
```

### 03 — Learning Loop
The core demo. Runs 30+ iterations to observe the cascade router learning:
```bash
bash 03-learning-loop.sh         # default 30 iterations
bash 03-learning-loop.sh 50      # custom count
```

Watch for:
- Cascade stage transitions (Static → Confidence → UCB)
- Model selection frequency shifts as the router learns
- Pass rate convergence

### 04 — Visualization
Visualize learning state after running the learning loop:
```bash
python3 04-visualize-learning.py .roko/learn/
```

With matplotlib installed, also generates PNG charts in `results/`.

## Interpreting Results

### Cascade Stages
1. **Static** (0-9 observations): Round-robin exploration
2. **Confidence** (10-29): Weighted by empirical pass rates
3. **UCB** (30+): LinUCB bandit balances exploration/exploitation

### What to Look For
- Models with higher pass rates should get selected more often in UCB stage
- Cost-effective models should emerge as defaults for simpler tasks
- The router should converge on a stable policy after ~30 iterations

## Troubleshooting

- **"roko binary not found"**: Run `cargo build -p roko-cli --release`
- **Provider test fails**: Check API key is set and valid
- **Ollama timeout**: Ensure `ollama serve` is running and models are pulled
- **No learning data**: The `.roko/learn/` directory is created after the first `roko run`
