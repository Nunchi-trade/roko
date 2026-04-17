# Mori parity check

Mechanically regenerates `tmp/mori-parity-verified.md` from the read-only
snapshot checklist in `bardo-backup/`.

Run it from anywhere inside the worktree with either entry point:

```bash
./tools/mori-parity-check.sh
./tools/mori-parity-check/mori-parity-check.sh
```

The nested wrapper delegates to the repo-root wrapper. That wrapper resolves
the checklist and Mori appendix from either the main checkout or the nested
runner worktree, then invokes `tools/mori-parity-check/verify.py` with explicit
paths.

Output is deterministic for a fixed repository state and input arguments because
the verifier:

- walks the source checklist in order
- uses a fixed verification order per line
- sorts grep hits before selecting evidence
- does not emit timestamps or random data

Heuristic caveats:

- False-positive rate is lowest for exact current-path hits and appendix path
  mappings, moderate for crate-qualified target-symbol hits, and highest for
  short or generic fallback symbols.
- False negatives still happen when an item has no useful `target:` token,
  when the live symbol name drifted substantially, or when the implementation
  is wired indirectly and the verifier only sees the helper definition.
- A `✅` is evidence of a likely current anchor, not a semantic proof that the
  full Mori behavior is implemented and wired end-to-end.
