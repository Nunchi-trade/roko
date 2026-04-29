# 22-code-intelligence — Depth Index

Depth for the code intelligence Pipeline. No direct parent in the unified spec (code intelligence
is an infrastructure Graph that feeds the Compose protocol). Covers all 11 source docs from
`docs/15-code-intelligence/`. 5 depth docs absorb all 11 source docs.

---

## Depth docs (5)

| Doc | Covers |
|---|---|
| [01-code-intelligence-as-cell-pipeline.md](01-code-intelligence-as-cell-pipeline.md) | 00-vision, 10-current-status-and-gaps |
| [02-symbol-graph-and-importance.md](02-symbol-graph-and-importance.md) | 01-tree-sitter-parsing, 02-symbol-extraction, 03-dependency-graph, 04-pagerank-symbol-importance |
| [03-hdc-fingerprints-and-similarity.md](03-hdc-fingerprints-and-similarity.md) | 05-hdc-fingerprints |
| [04-search-and-context-assembly.md](04-search-and-context-assembly.md) | 06-context-assembly-from-code |
| [05-mcp-server-and-persistence.md](05-mcp-server-and-persistence.md) | 07-mcp-context-server, 08-index-db-scaling, 09-snapshot-optimization |

---

## Source docs (11)

### Vision and status

| Source doc | Status |
|---|---|
| `docs/15-code-intelligence/00-vision.md` | **Absorbed** -> [01-code-intelligence-as-cell-pipeline.md](01-code-intelligence-as-cell-pipeline.md) |
| `docs/15-code-intelligence/10-current-status-and-gaps.md` | **Absorbed** -> [01-code-intelligence-as-cell-pipeline.md](01-code-intelligence-as-cell-pipeline.md) |

### Parsing and symbols

| Source doc | Status |
|---|---|
| `docs/15-code-intelligence/01-tree-sitter-parsing.md` | **Absorbed** -> [02-symbol-graph-and-importance.md](02-symbol-graph-and-importance.md) |
| `docs/15-code-intelligence/02-symbol-extraction.md` | **Absorbed** -> [02-symbol-graph-and-importance.md](02-symbol-graph-and-importance.md) |

### Graph and scoring

| Source doc | Status |
|---|---|
| `docs/15-code-intelligence/03-dependency-graph.md` | **Absorbed** -> [02-symbol-graph-and-importance.md](02-symbol-graph-and-importance.md) |
| `docs/15-code-intelligence/04-pagerank-symbol-importance.md` | **Absorbed** -> [02-symbol-graph-and-importance.md](02-symbol-graph-and-importance.md) |

### Fingerprints

| Source doc | Status |
|---|---|
| `docs/15-code-intelligence/05-hdc-fingerprints.md` | **Absorbed** -> [03-hdc-fingerprints-and-similarity.md](03-hdc-fingerprints-and-similarity.md) |

### Search and context

| Source doc | Status |
|---|---|
| `docs/15-code-intelligence/06-context-assembly-from-code.md` | **Absorbed** -> [04-search-and-context-assembly.md](04-search-and-context-assembly.md) |

### Server and persistence

| Source doc | Status |
|---|---|
| `docs/15-code-intelligence/07-mcp-context-server.md` | **Absorbed** -> [05-mcp-server-and-persistence.md](05-mcp-server-and-persistence.md) |
| `docs/15-code-intelligence/08-index-db-scaling.md` | **Absorbed** -> [05-mcp-server-and-persistence.md](05-mcp-server-and-persistence.md) |
| `docs/15-code-intelligence/09-snapshot-optimization.md` | **Absorbed** -> [05-mcp-server-and-persistence.md](05-mcp-server-and-persistence.md) |

---

11 of 11 source docs absorbed across 5 depth docs.
