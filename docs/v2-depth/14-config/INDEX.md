# 14-config — Depth Index

Depth for [14-CONFIG-AND-AUTHORING.md](../../unified/14-CONFIG-AND-AUTHORING.md)

---

## Source docs (2)

### Configuration schema and resolution

| Source doc | Status |
|---|---|
| `docs/00-architecture/20-configuration-schema.md` | Done |
| `docs/12-interfaces/04-configuration-layered-resolution.md` | Absorbed |

---

## Depth docs

| Depth doc | Source | What it adds |
|---|---|---|
| [config-as-signal.md](config-as-signal.md) | `20-configuration-schema.md` | Config as Signal (content-addressed, versioned, demurrage), Compose protocol for override resolution, Verify Cell for schema validation, Trigger Cell for hot reload, L4 config evolution |
| [02-layered-resolution-and-reload.md](02-layered-resolution-and-reload.md) | `04-configuration-layered-resolution.md` | Four-layer merge Pipeline (CLI > env > TOML > defaults), hot-reload via Trigger Cell (inotify/kqueue), ROKO_* env convention, auto-detection as Score Cells, domain profiles as Rack macros, provenance tracking via lineage |
