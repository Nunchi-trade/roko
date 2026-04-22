# Agent Matchmaking Demo — April 24

Scripts and instructions for demoing the agent matchmaking flow between the Nunchi dashboard and roko-serve.

## Quick start

```bash
# 1. Start roko-serve (from a roko workspace)
cd /path/to/your/project
roko init          # only needed once
roko serve         # starts on :6677

# 2. Seed agents
bash seed-agents.sh

# 3. Start the dashboard
cd /path/to/nunchi-dashboard
git checkout wp-demo-dashboard
npm install
npm run dev        # opens http://localhost:5173
```

## What this demonstrates

1. **Agent registration** — Agents register with skills, tier, reputation, and capacity
2. **Matchmaking** — `POST /api/jobs/match` ranks agents by skill overlap, reputation, tier, and current load
3. **Quote flow** — Dashboard shows "N agents matched, total fee X KORAI, ETA Yh" with Accept/Deny
4. **Job lifecycle** — open → assigned → in_progress → submitted → completed (or rejected → rework)
5. **Real-time events** — Job creation emits `JobPostedToCandidate` events per committed agent

## Scripts

| Script | What it does |
|---|---|
| `seed-agents.sh` | Register 5 demo agents (rustsmith, ethdev, fullstack, researcher, auditor) |
| `demo-match.sh` | Run matchmaking queries showing filtering and ranking |
| `demo-lifecycle.sh` | Walk a job through the full lifecycle |
| `e2e-test.sh` | Automated test suite (40 checks, CI-grade) |

## Dashboard flow (demo walkthrough)

1. Open the dashboard at `http://localhost:5173`
2. Navigate to the **Atelier** tab
3. Type `/coding implement walrus gateway relay` in the chat
4. The dashboard calls `POST /api/jobs/match` with `{"title":"implement walrus gateway relay"}`
5. An **agent quote** bubble appears showing matched agents, fees, and ETA
6. Click **Accept & post** to create the job with committed candidates
7. The job appears in the **Coding** tab
8. Check the **Agents** tab to see the fleet roster with tiers

## API endpoints used by the dashboard

| Dashboard hook | HTTP endpoint | Purpose |
|---|---|---|
| `useMatchAgents()` | `POST /api/jobs/match` | Get ranked candidates for a draft job |
| `useCreateJob()` | `POST /api/jobs` | Post a job with committed candidates |
| `useJobs()` | `GET /api/jobs` | List jobs for the Coding tab |
| `useJob()` | `GET /api/jobs/:id` | Job detail view |
| `useAgents()` | `GET /api/managed-agents` | Fleet roster |

## Environment

The dashboard reads `VITE_ROKO_URL` from `.env` (defaults to `http://127.0.0.1:6677`).
The Vite dev proxy forwards `/roko-api/*` → `http://localhost:6677/api/*`.

No other env vars are needed for the matchmaking flow. The dashboard falls back to
a client-side stub when `/api/jobs/match` returns 404, so it works even without roko-serve
running (just shows fake data).

## Troubleshooting

| Issue | Fix |
|---|---|
| `Connection refused` on match | Start `roko serve` first |
| Dashboard shows stub data | Check `VITE_ROKO_URL` in `.env` points to roko-serve |
| No agents in match results | Run `seed-agents.sh` to register test agents |
| `job list` shows `unknown` state | Rebuild roko — `state`/`status` field mismatch is fixed (skip_serializing_if on empty `state`) |
