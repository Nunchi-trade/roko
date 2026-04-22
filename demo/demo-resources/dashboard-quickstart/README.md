# Dashboard + Roko Quickstart

Get the Nunchi dashboard and roko-serve running together from scratch.

## Prerequisites

- Rust toolchain (1.91+): `rustup update stable`
- Node.js 18+
- Both repos cloned:
  - `roko`: `/path/to/nunchi/roko/roko`
  - `dashboard`: `/path/to/nunchi/nunchi-dashboard`

## Setup (one-time)

```bash
# Build roko
cd /path/to/roko
cargo build -p roko-cli

# Init a workspace (creates .roko/ and roko.toml)
roko init

# Install dashboard deps
cd /path/to/nunchi-dashboard
git checkout wp-demo-dashboard
npm install
```

## Run

Open 3 terminals:

### Terminal 1: roko-serve
```bash
cd /path/to/your-project   # any dir with roko.toml
roko serve                  # starts on http://localhost:6677
```

### Terminal 2: Seed agents (once per session)
```bash
bash /path/to/roko/demo/demo-resources/bin/roko-demo seed-agents
```

### Terminal 3: Dashboard
```bash
cd /path/to/nunchi-dashboard
npm run dev                 # opens http://localhost:5173
```

## Verify

1. Open http://localhost:5173
2. Navigate to **Network → Agents** — should show seeded agents
3. Navigate to **Atelier → Chat** — type `/coding build a relay` — should show agent quote
4. Navigate to **Atelier → PRDs** — type an idea in the quick-input field

## Dashboard environment

The `.env` file configures where the dashboard connects:

```env
VITE_ROKO_URL=http://127.0.0.1:6677    # roko-serve
VITE_CHAIN_URL=http://127.0.0.1:8545   # mirage (optional, for chain data)
```

The Vite dev proxy automatically forwards `/roko-api/*` → `http://localhost:6677/api/*`.

## What works without roko-serve

- Chat interface (localStorage only)
- Mock demos (`/mock`, `/mockcode`, `/mockresearch`, `/mockprd`)
- Settings page (shows env vars)
- Theme toggle

## What needs roko-serve

- All real commands (`/idea`, `/draft`, `/plan`, `/run`, `/coding`, `/research`, `/job`)
- Plans tab, PRDs tab, Jobs board
- Learning/metrics pages
- Agent fleet management

## Common issues

| Symptom | Cause | Fix |
|---------|-------|-----|
| "Connection refused" in console | roko-serve not running | Start `roko serve` |
| Stub data in agent quotes | `/api/jobs/match` not found | Ensure roko-serve is on the right port |
| No agents in fleet | Agents not registered | Run `seed-agents.sh` |
| Dashboard blank | Wrong branch | `git checkout wp-demo-dashboard` |
| Build fails | Old Rust | `rustup update stable` (need 1.91+) |
