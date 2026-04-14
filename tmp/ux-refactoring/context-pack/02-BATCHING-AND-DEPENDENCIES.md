# UX Refactoring Context Pack: Batching and Dependencies

The overnight runner executes these batches:

- `A1`: `A.01-A.05`
- `A2`: `A.06-A.10`
- `B1`: `B.01-B.06`
- `B2`: `B.07-B.18`
- `C1`: `C.01-C.05`
- `C2`: `C.06-C.08`
- `D1`: `D.02-D.17`
- `E1`: `E.01-E.08`
- `D2`: `D.18-D.33`
- `D3`: `D.34-D.54`
- `F1`: `F.01-F.06`
- `F2`: `F.07-F.12`

## Hard dependencies

- `A2` after `A1`
- `B2` after `B1`
- `C2` after `C1`
- `E1` after `D1`
- `D2` after `D1` and `E1`
- `D3` after `D2`
- `F2` after `F1`

## Write-scope discipline

- `A*` and `C*` both touch `mirage-rs` and `roko-serve`
- `D*`, `E*`, and `F*` all touch `roko-cli`
- `B*` owns `roko-demo`, `contracts`, and `demo/`

Do not widen scope unless it is required for a clean compile or a real API
handoff.
