# Roko on Railway — Deployment Runbook

End-to-end steps to turn the nunchi-dashboard "1-click deploy" button into
real agent containers running on Railway.

Architecture:

```
nunchi-dashboard
   │  POST /api/deployments { template, params, backend: "railway-api" }
   ▼
roko-serve          ─── Railway GraphQL API ───► Railway project
 (control plane)                                    │
                                                    ▼
                                            pulls worker image from GHCR,
                                            injects env, starts container
                                                    │
                                                    ▼
                                           roko-worker (runs `roko worker`,
                                           decodes ROKO_TEMPLATE_JSON,
                                           calls back to control plane)
```

## What's already wired (don't redo)

- **Dashboard → roko-serve**: `DeployPanel` posts to `POST /api/deployments`
  with `backend: "railway-api"`.
  (`src/components/agent-studio/DeployPanel.tsx`,
  `src/services/mirage-api.ts:1400`)
- **roko-serve → Railway**: `RailwayApiBackend` (831 lines) calls Railway's
  GraphQL API — `serviceCreate`, `variableCollectionUpsert`,
  `serviceInstanceDeploy`, status polling, teardown, logs.
  (`crates/roko-serve/src/deploy/railway_api.rs`)
- **Worker container**: `roko worker` subcommand decodes
  `ROKO_TEMPLATE_JSON`, serves `/health`, `/task`, `/status` on `$PORT`.
  (`crates/roko-cli/src/worker/`)
- **Worker image build**: `Dockerfile.worker` + GitHub Action
  `.github/workflows/publish-worker-image.yml` publishes
  `ghcr.io/nunchi-trade/roko-worker:{latest,sha-*}` on every push to `main`.
- **Control-plane deploy config**: `railway.json` at repo root tells Railway
  to use the root `Dockerfile` with healthcheck `/api/health`.
- **Env-driven config**: `DeployConfig::apply_process_env()` reads
  `ROKO_DEPLOY_*` vars at server startup, so secrets stay out of
  `roko.toml`. `ROKO_CONTROL_PLANE_URL` in the server env overrides the
  computed bind:port so workers can reach the public control plane.

## What Will needs to do (Phase 4)

### 1. Publish the worker image once

The GitHub Action publishes on push to `main`. For the first run, either
merge this branch or trigger `workflow_dispatch` manually in the Actions
tab. After it succeeds, verify the image is pullable:

```bash
docker pull ghcr.io/nunchi-trade/roko-worker:latest
```

If GHCR visibility defaults to private, make the package public in GitHub
→ Nunchi-trade → Packages → `roko-worker` → Settings.

### 2. Create the Railway account + project

1. Sign in to Railway, create a new project (e.g., `nunchi-roko`).
2. Capture the project ID and production environment ID:

   ```bash
   railway login
   railway link            # pick the new project
   railway status          # shows project id + environment id
   ```

### 3. Deploy the control plane (roko-serve)

From the roko repo root:

```bash
railway up                # uses railway.json → Dockerfile
```

After the build completes, generate a public URL (Settings → Networking →
Generate domain). Capture that URL — you'll pin it as
`ROKO_CONTROL_PLANE_URL` below.

### 4. Create a Railway API token

Railway → Account Settings → Tokens → Create. Scope: the project you just
created. Save the token securely.

### 5. Wire secrets on the control-plane service

On the roko-serve Railway service, set these variables (Variables tab):

| Variable | Value |
|---|---|
| `ROKO_DEPLOY_BACKEND` | `railway-api` |
| `ROKO_DEPLOY_RAILWAY_API_TOKEN` | the token from step 4 |
| `ROKO_DEPLOY_PROJECT_ID` | project id from step 2 |
| `ROKO_DEPLOY_ENVIRONMENT_ID` | environment id from step 2 |
| `ROKO_DEPLOY_WORKER_IMAGE` | `ghcr.io/nunchi-trade/roko-worker:latest` |
| `ROKO_CONTROL_PLANE_URL` | the public URL from step 3 (e.g. `https://roko-serve-production.up.railway.app`) |
| `ANTHROPIC_API_KEY` | forwarded into each worker at deploy time |

Railway will restart the service after each change.

### 6. Point the dashboard at the deployed control plane

In the nunchi-dashboard deployment env (Vercel / wherever):

```
VITE_ROKO_URL=https://roko-serve-production.up.railway.app
```

Redeploy the dashboard.

### 7. Smoke test

1. Open the dashboard → Agent Studio → Setup & Connect.
2. Click **Deploy** on any of the three templates.
3. You should see `Deploying...` → `Live — <url>` within ~2 minutes.
4. In the Railway UI, confirm a new service named `roko-worker-<template>`
   appeared and is healthy.
5. Click **Teardown** on the dashboard → confirm the Railway service is
   deleted.

## Troubleshooting

- **`unknown deploy backend: railway`**: dashboard is on an old build;
  rebuild `nunchi-dashboard` with the `backend: "railway-api"` fix.
- **`railway-api backend requires railway_api_token`**: the env var isn't
  reaching the server. Double-check `ROKO_DEPLOY_RAILWAY_API_TOKEN` is set
  on the roko-serve service (not the worker) and redeploy.
- **`template 'isfr-observer' not found`**: the server is older than this
  change. Rebuild roko-serve from this branch.
- **Worker boots but never calls home**: check `ROKO_CONTROL_PLANE_URL` on
  the worker service; it should be the public URL of roko-serve.
- **Image pull fails**: GHCR package visibility is still private; make it
  public or attach a pull secret to the Railway project.
