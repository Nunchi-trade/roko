# Formal Verification Pipeline

> **Layer**: L3 Harness (verification gates), integrated with L1 Framework (chain domain plugin)
>
> **Crate**: Target: `roko-chain` (chain domain verification), with hooks into `roko-gate` (pipeline as a Gate)
>
> **Synapse traits**: `Gate` (each pipeline stage is a verification gate), `Policy` (emit verification-level Engrams)
>
> **Prerequisites**: [00-defense-in-depth.md](00-defense-in-depth.md), [08-threat-model.md](08-threat-model.md)


> **Implementation**: Specified

---

## Overview

Smart contracts manage billions of dollars with no recourse for bugs. An autonomous agent that simulates, triages, and acts on DeFi transactions needs verification woven into its pipeline, not bolted on after the fact. This document covers five tools — Heimdall-rs, Slither, Echidna, hevm, and Certora/Kontrol — and how each integrates with a Rust-based agent that must verify contract behavior before committing capital.

The core tension: formal methods are slow and thorough, fuzzing is fast and probabilistic, static analysis is instant and shallow. A production pipeline uses all three in sequence, matching verification depth to available time and risk magnitude.

**Note:** This is a chain-domain safety capability. Roko's core is domain-agnostic, but the chain domain plugin (`roko-chain`) adds DeFi-specific verification. The pipeline pattern (fast filter → medium analysis → deep proof) generalizes to other domains: a coding agent might run clippy (fast) → tests (medium) → formal property verification (deep).

---

## The Five-Stage Pipeline

The five tools compose into a pipeline ordered by speed and depth. Each stage gates the next — there is no point running symbolic execution on a contract that static analysis already flagged as having an unprotected selfdestruct.

```
Stage 1: Heimdall-rs (decompilation)      ~milliseconds
    ↓
Stage 2: Slither (static analysis)        ~seconds
    ↓
Stage 3: Echidna (property-based fuzzing) ~30-60 seconds
    ↓
Stage 4: hevm (symbolic execution)        ~1-2 minutes
    ↓
Stage 5: Certora/Kontrol (formal proofs)  ~5-30 minutes
```

Each stage produces a `VerificationLevel` that gates trading decisions. The agent will not commit capital to an interaction involving a contract below `StaticClean` unless the expected value overwhelms the verification gap.

### VerificationLevel Enum

```rust
use std::time::Duration;

/// Verification confidence levels, ordered by rigor.
#[derive(Debug, Clone, PartialEq, PartialOrd)]
pub enum VerificationLevel {
    /// No verification performed.
    Unknown,
    /// Heimdall-rs decompiled the bytecode; basic structure recovered.
    Decompiled,
    /// Slither static analysis found no high-severity issues.
    StaticClean,
    /// Echidna fuzzing passed N iterations with no property violations.
    FuzzPassed { iterations: u64 },
    /// hevm symbolic execution proved properties for all inputs.
    SymbolicProved { properties: Vec<String> },
    /// Certora/Kontrol formal verification of invariants.
    FormallyVerified { rules: Vec<String> },
}
```

### Pipeline Configuration

```rust
/// Configuration for the verification pipeline.
/// Time budgets control how long each stage can run before
/// the pipeline moves on.
pub struct PipelineConfig {
    pub slither_timeout: Duration,
    pub echidna_time_budget: Duration,
    pub echidna_iterations: u64,
    pub hevm_solver_timeout: Duration,
    pub certora_enabled: bool,
    pub rpc_url: String,
}

impl Default for PipelineConfig {
    fn default() -> Self {
        Self {
            slither_timeout: Duration::from_secs(30),
            echidna_time_budget: Duration::from_secs(60),
            echidna_iterations: 50_000,
            hevm_solver_timeout: Duration::from_secs(120),
            certora_enabled: false, // expensive; opt-in
            rpc_url: "http://localhost:8545".to_string(),
        }
    }
}
```

---

## Stage 1: Heimdall-rs — Bytecode Decompilation in Rust

Heimdall-rs (github.com/Jon-Becker/heimdall-rs) is the only tool in the pipeline written natively in Rust. It decompiles EVM bytecode — turning raw hex into readable function signatures, control flow graphs, and approximate Solidity source.

### Why It Matters

The agent constantly encounters unverified contracts. Etherscan has source code for a fraction of deployed contracts. When the triage pipeline flags an interesting transaction involving an unverified contract, Heimdall-rs can:

1. **Disassemble** bytecode into EVM opcodes
2. **Recover control flow** — identify basic blocks, loops, and branches
3. **Resolve function signatures** — match 4-byte selectors to known function names via signature databases
4. **Decompile** into pseudo-Solidity that humans (or LLMs) can reason about
5. **Dump storage layout** — identify storage slot assignments

### Rust Integration

Since Heimdall-rs is a Rust crate, integration is direct — no subprocess spawning needed:

```rust
use alloy::primitives::Address;

/// Represents a decompiled contract with recovered structure.
pub struct DecompiledContract {
    pub address: Address,
    pub functions: Vec<DecompiledFunction>,
    pub storage_layout: Vec<StorageSlot>,
    pub bytecode_size: usize,
}

pub struct DecompiledFunction {
    pub selector: [u8; 4],
    pub signature: Option<String>,  // e.g., "transfer(address,uint256)"
    pub decompiled_body: String,     // pseudo-Solidity
    pub state_mutability: StateMutability,
}

#[derive(Debug)]
pub enum StateMutability {
    Pure,
    View,
    Nonpayable,
    Payable,
}

pub struct StorageSlot {
    pub slot: u64,
    pub inferred_type: String,
    pub label: Option<String>,
}

/// Wrapper around Heimdall-rs for bytecode analysis.
pub struct HeimdallRunner {
    binary_path: String,
    rpc_url: String,
}

impl HeimdallRunner {
    pub fn new(binary_path: &str, rpc_url: &str) -> Self {
        Self {
            binary_path: binary_path.to_string(),
            rpc_url: rpc_url.to_string(),
        }
    }

    /// Decompile a contract at the given address.
    pub fn decompile(&self, address: &str) -> anyhow::Result<String> {
        let output = std::process::Command::new(&self.binary_path)
            .arg("decompile")
            .arg("--target")
            .arg(address)
            .arg("--rpc-url")
            .arg(&self.rpc_url)
            .output()?;

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Disassemble bytecode into raw opcodes.
    pub fn disassemble(&self, bytecode: &str) -> anyhow::Result<String> {
        let output = std::process::Command::new(&self.binary_path)
            .arg("disassemble")
            .arg("--target")
            .arg(bytecode)
            .output()?;

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// Resolve function selectors from bytecode to known signatures.
    pub fn resolve_selectors(&self, address: &str) -> anyhow::Result<Vec<(String, String)>> {
        let output = std::process::Command::new(&self.binary_path)
            .arg("decompile")
            .arg("--target")
            .arg(address)
            .arg("--rpc-url")
            .arg(&self.rpc_url)
            .arg("--include-sol")
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut selectors = Vec::new();
        for line in stdout.lines() {
            if line.contains("function ") {
                if let Some(sig) = line.strip_prefix("function ") {
                    let selector = sig
                        .split('(')
                        .next()
                        .unwrap_or("")
                        .to_string();
                    selectors.push((selector, sig.to_string()));
                }
            }
        }
        Ok(selectors)
    }

    /// Generate a control flow graph for visual analysis.
    pub fn generate_cfg(&self, address: &str, output_path: &str) -> anyhow::Result<()> {
        std::process::Command::new(&self.binary_path)
            .arg("cfg")
            .arg("--target")
            .arg(address)
            .arg("--rpc-url")
            .arg(&self.rpc_url)
            .arg("--output")
            .arg(output_path)
            .output()?;

        Ok(())
    }
}
```

In the pipeline, Heimdall-rs sits at the front. When triage flags a transaction involving an unknown contract, Heimdall-rs decompiles it. The decompiled output feeds into Slither for static analysis (if close enough to valid Solidity) and into the LLM for semantic understanding. If the contract looks sufficiently complex or high-value, Echidna and hevm take over for deeper verification.

---

## Stage 2: Slither — Static Analysis at Compilation Speed

Slither, from Trail of Bits, is a static analysis framework for Solidity. It does not execute code. It parses Solidity into **SlithIR** (an intermediate representation), builds control flow graphs, and runs detectors over the IR.

### What It Catches

Slither ships 90+ vulnerability detectors covering:
- Reentrancy (all variants: cross-function, cross-contract, read-only)
- Unprotected selfdestruct
- Arbitrary send (ETH to attacker-controlled address)
- Unchecked low-level calls
- Shadowed state variables
- Unused return values
- Missing access control
- Floating pragmas
- Centralization risks

It also includes "printers" that output contract summaries, inheritance graphs, function call graphs, and storage layouts.

### Custom DeFi Detectors

Slither's plugin architecture allows custom detectors in Python. For the chain domain, custom detectors flag DeFi-specific patterns:
- Unguarded flash loan callbacks
- Missing slippage checks
- Oracle manipulation vulnerabilities
- Uncapped minting functions
- Unprotected fee changes

### Rust Integration

```rust
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct SlitherOutput {
    pub success: bool,
    pub results: SlitherResults,
}

#[derive(Debug, Deserialize)]
pub struct SlitherResults {
    pub detectors: Vec<SlitherDetection>,
    pub printers: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
pub struct SlitherDetection {
    pub check: String,
    pub impact: String,          // "High", "Medium", "Low", "Informational"
    pub confidence: String,      // "High", "Medium", "Low"
    pub description: String,
    pub elements: Vec<SlitherElement>,
}

#[derive(Debug, Deserialize)]
pub struct SlitherElement {
    pub name: String,
    #[serde(rename = "type")]
    pub element_type: String,
    pub source_mapping: Option<serde_json::Value>,
}

pub struct SlitherRunner {
    binary_path: String,
}

impl SlitherRunner {
    pub fn new(binary_path: &str) -> Self {
        Self {
            binary_path: binary_path.to_string(),
        }
    }

    /// Run Slither analysis on a contract, returning all detections.
    pub fn analyze(&self, contract_path: &str) -> anyhow::Result<SlitherOutput> {
        let output = std::process::Command::new(&self.binary_path)
            .arg(contract_path)
            .arg("--json")
            .arg("-")
            .output()?;

        let result: SlitherOutput = serde_json::from_slice(&output.stdout)?;
        Ok(result)
    }

    /// Filter detections to only high-impact, high-confidence findings.
    pub fn critical_findings(output: &SlitherOutput) -> Vec<&SlitherDetection> {
        output
            .results
            .detectors
            .iter()
            .filter(|d| d.impact == "High" && d.confidence == "High")
            .collect()
    }

    /// Run a specific set of detectors (faster than running all 90+).
    pub fn analyze_targeted(
        &self,
        contract_path: &str,
        detectors: &[&str],
    ) -> anyhow::Result<SlitherOutput> {
        let detector_list = detectors.join(",");
        let output = std::process::Command::new(&self.binary_path)
            .arg(contract_path)
            .arg("--detect")
            .arg(&detector_list)
            .arg("--json")
            .arg("-")
            .output()?;

        let result: SlitherOutput = serde_json::from_slice(&output.stdout)?;
        Ok(result)
    }
}
```

**Academic context.** Feist, Grieco, and Groce ("Slither: A Static Analysis Framework for Smart Contracts," WETSEB 2019) describe the SlithIR design and detector architecture. The key contribution is that SlithIR preserves enough semantic information for cross-function analysis while remaining simple enough for fast traversal.

---

## Stage 3: Echidna — Property-Based Fuzzing

Echidna is a property-based fuzzer built by Trail of Bits for Solidity smart contracts. It generates semi-random transaction sequences, executes them against a contract, and checks whether user-defined invariants hold. When an invariant breaks, Echidna reports a minimal reproducing sequence.

### How It Works

Echidna uses grammar-based fuzzing guided by coverage feedback. It constructs sequences of contract function calls with random arguments, biasing generation toward inputs that explore new code paths. Properties are Solidity functions prefixed with `echidna_` that return `bool` — the fuzzer tries to make them return `false`.

Since version 2.1, Echidna supports **on-chain forking** via RPC endpoints. This means it can fuzz against actual mainnet state — the same state the mirage-rs (Roko's in-process EVM simulator, `mirage-rs`) fork simulation uses.

### Configuration

```yaml
# echidna-config.yaml
testLimit: 50000
shrinkLimit: 5000
seqLen: 10
corpusDir: "corpus"
cryticArgs: ["--compile-force-framework", "foundry"]
rpcUrl: "http://localhost:8545"
rpcBlock: "latest"
deployer: "0xDeaDbeefdEAdbeefdEadbEEFdeadbeEFdEaDbeeF"
```

### Integration with mirage-rs Fork Testing

The key insight: Echidna and mirage-rs can share the same forked state. Point Echidna's `rpcUrl` at the same Anvil or Reth node the simulation layer uses. Write Echidna properties that encode the invariants the agent cares about:

```solidity
// Property: a swap on this pool should never drain more than 5% of reserves
function echidna_swap_bounded() public returns (bool) {
    uint256 reserveBefore = pool.getReserve0();
    pool.swap(amount0, amount1, address(this), "");
    uint256 reserveAfter = pool.getReserve0();
    return reserveAfter >= (reserveBefore * 95) / 100;
}
```

When the agent encounters an unfamiliar contract, it can deploy Echidna properties against the forked state, fuzz for a bounded time (30 seconds), and flag the contract if any property breaks. This is faster than symbolic execution and catches a different class of bugs — sequence-dependent state corruption that single-transaction analysis misses.

### Rust Integration

```rust
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct EchidnaResult {
    pub name: String,
    pub status: EchidnaStatus,
    pub reproducer: Option<Vec<EchidnaCall>>,
    pub coverage: f64,
}

#[derive(Debug, Deserialize)]
pub enum EchidnaStatus {
    Passed,
    Failed,
    Fuzzing,
}

#[derive(Debug, Deserialize)]
pub struct EchidnaCall {
    pub function: String,
    pub args: Vec<String>,
    pub value: String,
    pub sender: String,
}

pub struct EchidnaRunner {
    binary_path: String,
    config_path: String,
    working_dir: String,
}

impl EchidnaRunner {
    pub fn new(binary_path: &str, config_path: &str, working_dir: &str) -> Self {
        Self {
            binary_path: binary_path.to_string(),
            config_path: config_path.to_string(),
            working_dir: working_dir.to_string(),
        }
    }

    /// Run Echidna against a contract file with a given config.
    pub fn run(
        &self,
        contract_file: &str,
        contract_name: &str,
    ) -> anyhow::Result<Vec<EchidnaResult>> {
        let output = std::process::Command::new(&self.binary_path)
            .arg(contract_file)
            .arg("--contract")
            .arg(contract_name)
            .arg("--config")
            .arg(&self.config_path)
            .arg("--format")
            .arg("json")
            .current_dir(&self.working_dir)
            .output()?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("Echidna failed: {}", stderr);
        }

        let results: Vec<EchidnaResult> = serde_json::from_slice(&output.stdout)?;
        Ok(results)
    }

    /// Check whether any property failed.
    pub fn find_violations(&self, results: &[EchidnaResult]) -> Vec<&EchidnaResult> {
        results
            .iter()
            .filter(|r| matches!(r.status, EchidnaStatus::Failed))
            .collect()
    }
}
```

**Academic context.** Grieco et al. ("Echidna: effective, usable, and fast fuzzing for smart contracts," ISSTA 2020) demonstrated that Echidna found bugs in contracts that Mythril and Manticore missed, particularly bugs requiring multi-transaction sequences. The grammar-based approach achieves higher coverage than blackbox fuzzing because it generates valid ABI-encoded calls rather than random byte sequences.

---

## Stage 4: hevm — Symbolic Execution with SMT Solving

hevm, maintained by the Ethereum Foundation, is a symbolic EVM implementation that proves properties hold for **all** possible inputs rather than checking specific ones. Where Echidna asks "can I find an input that breaks this?", hevm asks "does any input exist that breaks this?"

### How It Works

hevm executes EVM bytecode symbolically, representing unknown values (function arguments, msg.sender, block.timestamp) as symbolic variables. When execution branches on a symbolic condition, hevm forks into both paths, accumulating path constraints. At each `assert` or property check, it queries an SMT solver (typically Z3 or CVC5) to determine whether any assignment of symbolic variables can violate the assertion.

- If the solver returns **SAT**: a concrete counterexample exists
- If the solver returns **UNSAT**: the property holds for all inputs on that execution path

### RPC State Fetching

Unlike Halmos (a16z's symbolic testing tool, which cannot fork mainnet because symbolic keccak256 is incompatible with concrete storage lookups), hevm supports **RPC state fetching**. It can consume state from a forked environment, making it the strongest candidate for integration with mirage-rs. The agent runs symbolic analysis against real mainnet state — real balances, real pool configurations, real access control settings.

### Equivalence Checking

hevm can prove that two bytecodes behave identically for all inputs. This is valuable when a protocol upgrades a contract: verify that the new implementation preserves all behaviors of the old one, or identify exactly where they diverge.

### Rust Integration

```rust
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct HevmResult {
    pub property: String,
    pub outcome: HevmOutcome,
    pub counterexample: Option<HevmCounterexample>,
    pub solver_time_ms: u64,
    pub paths_explored: u64,
}

#[derive(Debug, Deserialize, PartialEq)]
pub enum HevmOutcome {
    Proved,
    Counterexample,
    Timeout,
    Unknown,
}

#[derive(Debug, Deserialize)]
pub struct HevmCounterexample {
    pub calldata: String,
    pub msg_value: String,
    pub msg_sender: String,
}

pub struct HevmRunner {
    binary_path: String,
    rpc_url: String,
    solver_timeout_ms: u64,
}

impl HevmRunner {
    pub fn new(binary_path: &str, rpc_url: &str, solver_timeout_ms: u64) -> Self {
        Self {
            binary_path: binary_path.to_string(),
            rpc_url: rpc_url.to_string(),
            solver_timeout_ms,
        }
    }

    /// Run symbolic execution against a Foundry test contract.
    pub fn prove(
        &self,
        project_dir: &str,
        contract_name: &str,
    ) -> anyhow::Result<Vec<HevmResult>> {
        let output = std::process::Command::new(&self.binary_path)
            .arg("test")
            .arg("--match")
            .arg(format!("prove_{}", contract_name))
            .arg("--rpc")
            .arg(&self.rpc_url)
            .arg("--smttimeout")
            .arg(self.solver_timeout_ms.to_string())
            .arg("--json")
            .current_dir(project_dir)
            .output()?;

        let results: Vec<HevmResult> = serde_json::from_slice(&output.stdout)?;
        Ok(results)
    }

    /// Check equivalence between two contract bytecodes.
    pub fn check_equivalence(
        &self,
        bytecode_a: &str,
        bytecode_b: &str,
    ) -> anyhow::Result<HevmOutcome> {
        let output = std::process::Command::new(&self.binary_path)
            .arg("equivalence")
            .arg("--code-a")
            .arg(bytecode_a)
            .arg("--code-b")
            .arg(bytecode_b)
            .arg("--smttimeout")
            .arg(self.solver_timeout_ms.to_string())
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        if stdout.contains("No differences found") {
            Ok(HevmOutcome::Proved)
        } else if stdout.contains("Counterexample") {
            Ok(HevmOutcome::Counterexample)
        } else {
            Ok(HevmOutcome::Unknown)
        }
    }

    /// Verify a specific property against forked state at a given block.
    pub fn verify_property(
        &self,
        project_dir: &str,
        test_function: &str,
        block_number: u64,
    ) -> anyhow::Result<HevmResult> {
        let output = std::process::Command::new(&self.binary_path)
            .arg("test")
            .arg("--match")
            .arg(test_function)
            .arg("--rpc")
            .arg(&self.rpc_url)
            .arg("--block")
            .arg(block_number.to_string())
            .arg("--smttimeout")
            .arg(self.solver_timeout_ms.to_string())
            .arg("--json")
            .current_dir(project_dir)
            .output()?;

        let results: Vec<HevmResult> = serde_json::from_slice(&output.stdout)?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| anyhow::anyhow!("No results returned from hevm"))
    }
}
```

**When to use hevm vs. Echidna.** Use Echidna first — it runs faster and catches sequence-dependent bugs. Use hevm when a proof is needed, not just high confidence. If Echidna's 50,000 test runs all pass, hevm can prove the property holds unconditionally. Echidna acts as a fast filter; hevm provides the formal guarantee.

**Academic context.** The symbolic execution approach builds on work by Mossberg et al. ("Manticore: A User-Friendly Symbolic Execution Framework for Binaries and Smart Contracts," ASE 2019), which established the pattern of symbolic EVM analysis with SMT backends. hevm improves on Manticore's approach with better EVM spec compliance, parallel solver queries, and RPC forking capability.

---

## Stage 5: Certora and Kontrol — Formal Verification at Scale

Certora and Kontrol represent the upper end of verification rigor. They prove invariants about contract behavior using mathematical specifications, not testing.

### Certora

Certora uses the **Certora Verification Language (CVL)**, a specification language for writing rules about Solidity contracts. CVL rules describe relationships between pre-states and post-states, and the Certora Prover checks whether any execution path violates them.

**CVL specification example:**

```cvl
// spec/Pool.spec

methods {
    function getReserve0() external returns (uint256) envfree;
    function getReserve1() external returns (uint256) envfree;
    function totalSupply() external returns (uint256) envfree;
    function swap(uint256, uint256, address, bytes) external;
}

// Ghost variable tracking cumulative swap volume
ghost mathint cumulativeVolume {
    init_state axiom cumulativeVolume == 0;
}

// Hook: update ghost on every swap
hook Sstore reserves[0] uint256 newVal (uint256 oldVal) {
    cumulativeVolume = cumulativeVolume + (newVal > oldVal ?
        newVal - oldVal : oldVal - newVal);
}

// Rule: swaps preserve the constant product invariant (within rounding)
rule swap_preserves_k(uint256 amount0, uint256 amount1, address to) {
    env e;
    mathint k_before = getReserve0() * getReserve1();
    swap(e, amount0, amount1, to, "");
    mathint k_after = getReserve0() * getReserve1();
    assert k_after >= k_before,
        "Swap violated constant product invariant";
}

// Invariant: total supply is zero iff reserves are zero
invariant liquidity_consistency()
    (totalSupply() == 0) <=> (getReserve0() == 0 && getReserve1() == 0);
```

**Scale and track record.** Certora has verified contracts securing over $100B in total value locked, including Aave, MakerDAO, Uniswap, Lido, and EigenLayer. The Certora Prover found an invariant violation in MakerDAO that put $10B at risk. Over 70,000 verification rules have been written by the community. The prover was recently open-sourced.

### Kontrol

Kontrol, built by Runtime Verification, takes a different approach. It uses **KEVM** — a mathematically formalized EVM semantics written in the K Framework (Hildenbrandt et al., "KEVM: A Complete Formal Semantics of the Ethereum Virtual Machine," CSF 2018). Where Certora abstracts the EVM into its own IR, Kontrol reasons about the actual EVM bytecode semantics, making its proofs closer to ground truth.

Kontrol integrates with Foundry test suites. Tests prefixed with `prove_` are symbolically executed using compositional reasoning — breaking large proofs into smaller lemmas that compose. Optimism uses Kontrol to validate codebase changes before production merges.

**When to use which.** Certora is faster to write specs for and has a larger ecosystem of existing rules. Kontrol is more thorough because it reasons about actual EVM semantics rather than an abstraction. For the chain intelligence pipeline, Certora handles the common case (verifying known protocol invariants), while Kontrol handles the hard case (proving properties about novel contract interactions where the abstraction gap matters).

---

## DeFi Verification Properties

The properties worth verifying for an autonomous trading agent, as referenced in the formal verification literature:

- **Position bounds**: exposure to any single protocol stays within configured limits
- **Slippage bounds**: maximum output deviation from expected, provable symbolically
- **Composability safety**: adding the agent's transaction to a block does not create new extractable value for third parties (Babel et al., "Clockwork Finance," IEEE S&P 2023, epsilon-composability)
- **Liquidation safety**: the agent's positions remain above liquidation thresholds under a range of price movements
- **Reentrancy freedom**: no callback path can re-enter the agent's contracts in an unexpected state

Tolmach et al. ("Formal Analysis of Composable DeFi Protocols," FC Workshops 2021) used process algebra (CSP#) to model composed protocol interactions (Curve + Compound), demonstrating that formal methods can reason about cross-protocol behavior. VerX (Permenev et al., IEEE S&P 2020) extends temporal safety specifications with `always` and `once` operators, verifying properties like "once a loan is repaid, the collateral is always unlocked." These temporal patterns map to the agent's need to verify multi-step DeFi strategies.

---

## Pipeline as a Gate

In Roko's architecture, the entire verification pipeline implements the `Gate` Synapse trait. Each stage is a sub-gate, and the pipeline is a sequential composition:

```
HeimdallGate → SlitherGate → EchidnaGate → HevmGate → CertoraGate
```

This follows the same pattern as the existing coding-domain gate pipeline (CompileGate → TestGate → ClippyGate → DiffGate) documented in `roko-gate`. Each sub-gate produces a `Verdict` Engram with a confidence score. The pipeline's overall verdict is the minimum confidence across all stages.

The `Gate` trait (defined in `roko-core`) is:
```rust
#[async_trait]
pub trait Gate: Send + Sync {
    async fn verify(&self, engram: &Signal) -> Result<Verdict>;
}
```

Where `Signal` will be renamed to `Engram` in Tier 0D of the implementation plan.

### Timeout handling per stage with fallback logic

Each pipeline stage has a time budget. When a stage times out, the pipeline records a partial result and moves on. The overall verification level reflects the deepest stage that completed.

```rust
/// Run the five-stage verification pipeline with per-stage timeouts.
/// Returns the highest VerificationLevel achieved before timeout or failure.
pub async fn run_pipeline(
    config: &PipelineConfig,
    target: &VerificationTarget,
) -> VerificationResult {
    let mut level = VerificationLevel::Unknown;
    let mut findings: Vec<Finding> = Vec::new();
    let mut stages_run = 0u32;

    // Stage 1: Heimdall decompilation.
    match tokio::time::timeout(
        Duration::from_secs(10),
        heimdall.decompile(&target.address),
    )
    .await
    {
        Ok(Ok(decompiled)) => {
            level = VerificationLevel::Decompiled;
            stages_run += 1;
        }
        Ok(Err(e)) => {
            findings.push(Finding::error("heimdall", e));
            // Cannot proceed without decompilation.
            return VerificationResult { level, findings, stages_run };
        }
        Err(_timeout) => {
            findings.push(Finding::timeout("heimdall", 10));
            return VerificationResult { level, findings, stages_run };
        }
    }

    // Stage 2: Slither static analysis.
    match tokio::time::timeout(
        config.slither_timeout,
        slither.analyze(&target.source_path),
    )
    .await
    {
        Ok(Ok(output)) => {
            let critical = SlitherRunner::critical_findings(&output);
            if critical.is_empty() {
                level = VerificationLevel::StaticClean;
            } else {
                findings.extend(critical.iter().map(|d| Finding::from_slither(d)));
                // Critical Slither findings: skip deeper analysis, report.
                return VerificationResult { level, findings, stages_run };
            }
            stages_run += 1;
        }
        Ok(Err(e)) => {
            findings.push(Finding::error("slither", e));
            // Slither failed, but we can still try fuzzing.
        }
        Err(_timeout) => {
            findings.push(Finding::timeout("slither", config.slither_timeout.as_secs()));
            // Timeout: skip to next stage.
        }
    }

    // Stage 3: Echidna fuzzing.
    match tokio::time::timeout(
        config.echidna_time_budget + Duration::from_secs(5), // grace period
        echidna.run(&target.source_path, &target.contract_name),
    )
    .await
    {
        Ok(Ok(results)) => {
            let violations = echidna.find_violations(&results);
            if violations.is_empty() {
                let iterations = results.iter().map(|r| config.echidna_iterations).sum();
                level = VerificationLevel::FuzzPassed { iterations };
            } else {
                findings.extend(violations.iter().map(|r| Finding::from_echidna(r)));
            }
            stages_run += 1;
        }
        Ok(Err(e)) => findings.push(Finding::error("echidna", e)),
        Err(_timeout) => findings.push(Finding::timeout("echidna", config.echidna_time_budget.as_secs())),
    }

    // Stage 4: hevm symbolic execution.
    match tokio::time::timeout(
        config.hevm_solver_timeout + Duration::from_secs(10),
        hevm.prove(&target.project_dir, &target.contract_name),
    )
    .await
    {
        Ok(Ok(results)) => {
            let proved: Vec<String> = results
                .iter()
                .filter(|r| r.outcome == HevmOutcome::Proved)
                .map(|r| r.property.clone())
                .collect();
            if !proved.is_empty() {
                level = VerificationLevel::SymbolicProved { properties: proved };
            }
            let counterexamples: Vec<_> = results
                .iter()
                .filter(|r| r.outcome == HevmOutcome::Counterexample)
                .collect();
            findings.extend(counterexamples.iter().map(|r| Finding::from_hevm(r)));
            stages_run += 1;
        }
        Ok(Err(e)) => findings.push(Finding::error("hevm", e)),
        Err(_timeout) => findings.push(Finding::timeout("hevm", config.hevm_solver_timeout.as_secs())),
    }

    // Stage 5: Certora (opt-in).
    if config.certora_enabled {
        // Certora runs are long (5-30 min). Run asynchronously
        // and return result via callback.
        findings.push(Finding::info("certora", "Certora verification queued"));
    }

    VerificationResult { level, findings, stages_run }
}

/// Result of the pipeline run.
pub struct VerificationResult {
    /// Highest verification level achieved.
    pub level: VerificationLevel,
    /// All findings across all stages.
    pub findings: Vec<Finding>,
    /// Number of stages that completed.
    pub stages_run: u32,
}

pub struct Finding {
    pub stage: String,
    pub severity: FindingSeverity,
    pub message: String,
}

pub enum FindingSeverity {
    Info,
    Warning,
    Error,
    Timeout,
}
```

### Five-stage pipeline as a Gate implementation

```rust
/// Verification pipeline wrapped as a Gate.
/// Runs the five-stage pipeline and produces a Verdict.
pub struct VerificationPipelineGate {
    config: PipelineConfig,
    heimdall: HeimdallRunner,
    slither: SlitherRunner,
    echidna: EchidnaRunner,
    hevm: HevmRunner,
}

#[async_trait]
impl Gate for VerificationPipelineGate {
    async fn verify(&self, engram: &Signal) -> Result<Verdict> {
        let target = extract_verification_target(engram)?;
        let result = run_pipeline(&self.config, &target).await;

        let confidence = match &result.level {
            VerificationLevel::Unknown => 0.0,
            VerificationLevel::Decompiled => 0.2,
            VerificationLevel::StaticClean => 0.5,
            VerificationLevel::FuzzPassed { iterations } => {
                // Confidence scales logarithmically with iterations.
                0.6 + 0.15 * (*iterations as f64).log10().min(5.0) / 5.0
            }
            VerificationLevel::SymbolicProved { properties } => {
                0.8 + 0.1 * (properties.len() as f64).min(10.0) / 10.0
            }
            VerificationLevel::FormallyVerified { rules } => {
                0.95 + 0.05 * (rules.len() as f64).min(20.0) / 20.0
            }
        };

        if result.findings.iter().any(|f| matches!(f.severity, FindingSeverity::Error)) {
            Ok(Verdict::Fail {
                confidence,
                message: format!(
                    "Verification pipeline found {} issues at level {:?}",
                    result.findings.len(),
                    result.level
                ),
                violations: vec![], // Detailed findings in the Engram body.
            })
        } else {
            Ok(Verdict::Pass {
                confidence,
                message: format!(
                    "Verification pipeline passed {} stages, level: {:?}",
                    result.stages_run, result.level
                ),
            })
        }
    }
}
```

### mirage-rs fork state sharing

The pipeline shares forked EVM state with mirage-rs (Roko's in-process EVM simulator) through a common Anvil/Reth node. Both Echidna and hevm connect to the same RPC endpoint, ensuring they verify against identical state.

```
orchestrate.rs
  |
  +--> mirage-rs: fork mainnet state at block N
  |      starts local Anvil instance at rpc_url
  |
  +--> VerificationPipelineGate::verify()
         |
         +--> HeimdallRunner: decompile bytecode (no RPC needed for disassembly)
         +--> SlitherRunner: static analysis of source (no RPC needed)
         +--> EchidnaRunner: fuzz against rpc_url (same Anvil instance)
         +--> HevmRunner: symbolic execution against rpc_url (same Anvil instance)
```

State consistency: both mirage-rs simulation and formal verification analyze the same block. This prevents the TOCTOU (time-of-check-time-of-use) issue where state changes between simulation and verification.

### Test criteria

- `HeimdallRunner::decompile()` returns non-empty output for a known contract address
- `SlitherRunner::analyze()` parses JSON output and reports detections with severity levels
- `SlitherRunner::critical_findings()` filters to only high-impact, high-confidence results
- `EchidnaRunner::run()` returns results with `Passed` or `Failed` status per property
- `HevmRunner::prove()` returns `Proved` for a tautological property and `Counterexample` for a violated one
- `HevmRunner::check_equivalence()` returns `Proved` for identical bytecodes
- Pipeline timeout at each stage produces a `Finding::Timeout` without crashing the pipeline
- Pipeline continues past a Slither error to attempt Echidna fuzzing
- Pipeline stops at critical Slither findings (does not fuzz a contract with unprotected selfdestruct)
- `VerificationPipelineGate` produces `Verdict::Pass` with confidence scaling from 0.0 to 1.0
- mirage-rs and the pipeline share the same Anvil RPC endpoint

---

## Implementation Status

| Component | Status | Location |
|---|---|---|
| Gate pipeline (sequential composition) | Built | `roko-gate/` (11 gates, 6-rung pipeline) |
| Coding-domain gates (compile, test, clippy) | Built | `roko-gate/src/` |
| Chain domain verification structs | Design only | Target: `roko-chain` |
| Heimdall-rs integration | Design only | Target: Tier 3 (chain domain) |
| Slither integration | Design only | Target: Tier 3 (chain domain) |
| Echidna integration | Design only | Target: Tier 3 (chain domain) |
| hevm integration | Design only | Target: Tier 4 (chain domain) |
| Certora/Kontrol integration | Design only | Target: Tier 4 (chain domain) |
| mirage-rs (in-process EVM simulator) | Built (141 tests) | `mirage-rs/` |

---

## Academic References

| Paper | Contribution |
|---|---|
| Grieco et al. (2020), "Echidna: effective, usable, and fast fuzzing for smart contracts," ISSTA 2020 | Grammar-based fuzzing with coverage feedback for Solidity |
| Mossberg et al. (2019), "Manticore: A User-Friendly Symbolic Execution Framework," ASE 2019 | Symbolic EVM analysis with SMT backends |
| Hildenbrandt et al. (2018), "KEVM: A Complete Formal Semantics of the Ethereum Virtual Machine," CSF 2018 | K Framework formalization of EVM semantics used by Kontrol |
| Feist, Grieco, Groce (2019), "Slither: A Static Analysis Framework for Smart Contracts," WETSEB 2019 | SlithIR design and detector architecture |
| Babel et al. (2023), "Clockwork Finance: Automated Analysis of Economic Security in Smart Contracts," IEEE S&P 2023 | Epsilon-composability and MEV formal analysis |
| Tolmach et al. (2021), "Formal Analysis of Composable DeFi Protocols," FC Workshops 2021 | Process algebra (CSP#) for cross-protocol reasoning |
| Permenev et al. (2020), "VerX: Safety Verification of Smart Contracts," IEEE S&P 2020 | Temporal safety specifications with always/once operators |
| Agent Behavioral Contracts (arXiv:2602.22302, 2026) | Design-by-Contract for agents, drift bounds theorem |
| AgentSpec (Wang et al., ICSE '26, arXiv:2503.18666) | Customizable runtime enforcement DSL, >90% unsafe action prevention |
| AgentGuard (Koohestani et al., 2025, arXiv:2509.23864) | Dynamic probabilistic assurance via MDP model checking |
| Pro2Guard (arXiv:2508.00500, 2025) | Proactive enforcement via probabilistic model checking |
| VeriGuard (arXiv:2510.05156, 2025) | Verified code generation with pre/post-conditions |
| Agent Contracts (arXiv:2601.08815, 2026) | Resource-bounded formal framework for agents |

---

## Verification-Guided Agent Design

The chain-domain verification pipeline above (Heimdall → Slither → Echidna → hevm → Certora) verifies *smart contracts*. This section extends formal verification to the *agent itself* — proving that the agent's behavior satisfies safety, liveness, completeness, and fairness properties regardless of model outputs.

### Research foundation

Three recent papers establish the theoretical basis:

**Formalizing Properties of Agentic AI** (arXiv:2510.14133, October 2025). Introduces 17 host-agent properties and 14 task-lifecycle properties expressed in temporal logic. Safety: "no unauthorized tool execution." Liveness: "every task reaches terminal state." Completeness: "all acceptance criteria verified." Fairness: "equitable resource allocation."

**Verifiably Safe Tool Use** (ICSE 2026 NIER, Doshi et al., CMU/Georgia Tech). Applies STPA (System-Theoretic Process Analysis) to derive safety requirements from hazards, formalized as specifications on data flows and tool sequences. Demonstrated with Alloy relational logic.

**Agent Behavioral Contracts** (arXiv:2602.22302, February 2026). Design-by-Contract for AI agents. Preconditions, invariants, postconditions per tool call. Hard constraints (zero tolerance) vs. soft constraints (bounded recovery). Validated on AgentContract-Bench: contracted agents detect 5.2-6.8 more violations per session.

### Agent safety properties

Properties expressed in temporal logic for the Roko agent system:

```rust
/// Formal safety properties for agent verification.
/// Each property can be model-checked (offline, exhaustive)
/// or runtime-monitored (online, per-event).
pub struct AgentSafetyProperty {
    /// Human-readable name.
    pub name: String,
    /// The property category.
    pub category: PropertyCategory,
    /// Temporal logic formula (LTL or CTL).
    pub formula: String,
    /// Verification method: model checking or runtime monitoring.
    pub verification: VerificationMethod,
    /// Severity if violated.
    pub severity: ViolationSeverity,
}

pub enum PropertyCategory {
    /// Something bad never happens.
    Safety,
    /// Something good eventually happens.
    Liveness,
    /// All required work is covered.
    Completeness,
    /// Resources are allocated equitably.
    Fairness,
}

pub enum VerificationMethod {
    /// Exhaustive model checking (offline, pre-execution).
    ModelChecking,
    /// Runtime monitoring via Buchi automata.
    RuntimeMonitoring,
    /// Both model checking and runtime monitoring.
    Both,
}
```

#### Host-agent properties (from arXiv:2510.14133)

| ID | Property | Category | LTL Formula | Roko Enforcement |
|---|---|---|---|---|
| HA-1 | No unauthorized tool execution | Safety | `G(tool_call(T) → authorized(T, role))` | `ToolPermission.satisfied_by()` |
| HA-2 | No data leakage across namespaces | Safety | `G(¬(data_flow(ns_a, ns_b) ∧ ¬channel(ns_a, ns_b)))` | `CognitiveNamespace` ACL |
| HA-3 | Tool calls bounded by rate limit | Safety | `G(call_count(role, tool, window) ≤ max)` | `RateLimiter` sliding window |
| HA-4 | Agent eventually responds | Liveness | `G(request → F(response))` | `ProcessSupervisor` timeout |
| HA-5 | Paused agent eventually resumes or shuts down | Liveness | `G(paused → F(resumed ∨ shutdown))` | `CognitiveSignal` timeout escalation |
| HA-6 | All tool outputs are scrubbed | Safety | `G(tool_result → scrubbed(tool_result))` | `ScrubPolicy` in `SafetyLayer` |
| HA-7 | Files stay within sandbox | Safety | `G(file_op(path) → within_worktree(path))` | `PathPolicy` canonicalization |

#### Task-lifecycle properties

| ID | Property | Category | LTL Formula | Roko Enforcement |
|---|---|---|---|---|
| TL-1 | Every task reaches terminal state | Liveness | `G(task_started(T) → F(completed(T) ∨ failed(T)))` | `ProcessSupervisor` + ghost turn detection |
| TL-2 | DAG ordering respected | Safety | `G(task_started(T) → completed(deps(T)))` | DAG executor in `roko-orchestrator` |
| TL-3 | Gate runs after every task | Completeness | `G(task_completed(T) → F(gate_verdict(T)))` | Gate pipeline in `orchestrate.rs` |
| TL-4 | Budget not exceeded | Safety | `G(¬budget_exceeded(session))` | `SafetyBudgetTracker` |
| TL-5 | Failed tasks trigger recovery | Liveness | `G(task_failed(T) → F(retry(T) ∨ skip(T) ∨ abort))` | Circuit breaker + conductor |

### Tool behavioral contracts

Each tool in the registry carries a contract specifying what must be true before, during, and after invocation:

```rust
/// Behavioral contract for a tool.
/// Based on Agent Behavioral Contracts (arXiv:2602.22302).
pub struct ToolContract {
    /// Tool this contract applies to.
    pub tool_name: String,
    /// Preconditions: must hold before invocation.
    pub preconditions: Vec<ContractPredicate>,
    /// Postconditions: must hold after invocation.
    pub postconditions: Vec<ContractPredicate>,
    /// Invariants: must hold throughout invocation.
    pub invariants: Vec<ContractPredicate>,
    /// Hard constraints: zero-tolerance violations (immediate abort).
    pub hard_constraints: Vec<ContractPredicate>,
    /// Soft constraints: bounded recovery window.
    pub soft_constraints: Vec<SoftConstraint>,
}

pub struct ContractPredicate {
    /// Human-readable description.
    pub description: String,
    /// The predicate function.
    pub check: Box<dyn Fn(&ToolCallContext) -> bool + Send + Sync>,
    /// Severity if violated.
    pub severity: ViolationSeverity,
}

pub struct SoftConstraint {
    pub predicate: ContractPredicate,
    /// Maximum time to recover from violation.
    pub recovery_window: Duration,
    /// Number of violations before escalating to hard constraint.
    pub max_violations: u32,
}

/// Contract-aware tool dispatcher.
/// Wraps ToolDispatcher to enforce contracts at dispatch time.
pub struct ContractEnforcingDispatcher {
    inner: ToolDispatcher,
    contracts: HashMap<String, ToolContract>,
    /// Track soft constraint violations per tool.
    violation_counts: HashMap<String, HashMap<String, u32>>,
}

impl ContractEnforcingDispatcher {
    pub async fn dispatch(&mut self, call: &ToolCall) -> Result<ToolResult> {
        let contract = self.contracts.get(&call.name);

        if let Some(contract) = contract {
            // Check preconditions
            for pre in &contract.preconditions {
                if !(pre.check)(&call.context) {
                    return Err(ContractViolation::Precondition(pre.description.clone()));
                }
            }

            // Check hard constraints
            for hard in &contract.hard_constraints {
                if !(hard.check)(&call.context) {
                    return Err(ContractViolation::HardConstraint(hard.description.clone()));
                }
            }
        }

        // Execute through inner dispatcher (includes SafetyLayer)
        let result = self.inner.dispatch(call).await?;

        if let Some(contract) = contract {
            // Check postconditions
            for post in &contract.postconditions {
                if !(post.check)(&call.context) {
                    // Log violation but return result (postcondition failure
                    // is informational unless it's also a hard constraint)
                    self.log_postcondition_violation(&call.name, &post.description);
                }
            }

            // Check soft constraints
            for soft in &contract.soft_constraints {
                if !(soft.predicate.check)(&call.context) {
                    let count = self.violation_counts
                        .entry(call.name.clone())
                        .or_default()
                        .entry(soft.predicate.description.clone())
                        .or_insert(0);
                    *count += 1;
                    if *count >= soft.max_violations {
                        // Escalate to hard constraint
                        return Err(ContractViolation::EscalatedSoft(
                            soft.predicate.description.clone(),
                        ));
                    }
                }
            }
        }

        Ok(result)
    }
}
```

### Example contracts for built-in tools

| Tool | Precondition | Postcondition | Hard Constraint |
|---|---|---|---|
| `write_file` | File path within worktree | File exists at path after write | Never write to `.env`, `Cargo.lock` outside plan scope |
| `bash` | Command passes `BashPolicy` | Exit code logged | Never execute `rm -rf /`, `sudo`, `chmod 777` |
| `edit_file` | File exists and was previously read | Edit produces valid diff | Net line count change < 500 per call |
| `git_push` | All gates passed for current task | Remote ref updated | Never force-push to protected branches |
| `web_fetch` | URL passes `NetworkPolicy` | Response status logged | Never fetch from private networks |

### VeriGuard dual-stage verification

Adapting VeriGuard (arXiv:2510.05156) for Roko: offline formal proofs establish property guarantees, online runtime monitors enforce them with minimal overhead.

**Offline stage** (pre-deployment):
1. Model the agent's state space as a finite transition system
2. Express safety properties in LTL/CTL
3. Use model checking (or SMT solving via Z3/CVC5) to prove properties hold
4. Generate runtime monitors as compiled Buchi automata

**Online stage** (during execution):
1. Feed each agent event to the compiled monitors
2. Each monitor runs in O(|formula|) space per event (Havelund & Rosu, 2004)
3. Violation triggers immediate response (log/alert/pause/abort)

```rust
/// VeriGuard dual-stage verification.
/// Offline proofs + online monitors for agent safety.
pub struct VeriGuard {
    /// Offline-verified properties (proved before deployment).
    pub verified_properties: Vec<VerifiedProperty>,
    /// Online runtime monitors (compiled from LTL formulas).
    pub runtime_monitors: Vec<BuchiAutomaton>,
    /// Current monitor states.
    pub monitor_states: Vec<AutomatonState>,
}

pub struct VerifiedProperty {
    /// Property name.
    pub name: String,
    /// LTL/CTL formula.
    pub formula: String,
    /// Verification result.
    pub result: VerificationResult,
    /// Timestamp of last verification.
    pub verified_at: i64,
    /// Hash of the system model at verification time.
    pub model_hash: [u8; 32],
}

pub enum VerificationResult {
    /// Property proved to hold for all reachable states.
    Proved,
    /// Property disproved: counterexample found.
    Disproved { counterexample: Vec<String> },
    /// Verification timed out (state space too large).
    Timeout { explored_states: u64 },
}

impl VeriGuard {
    /// Process an agent event through all runtime monitors.
    pub fn check_event(&mut self, event: &EventLabel) -> Vec<SafetyViolation> {
        let mut violations = Vec::new();
        for (i, monitor) in self.runtime_monitors.iter().enumerate() {
            let new_state = monitor.transition(&self.monitor_states[i], event);
            if monitor.is_rejecting(&new_state) {
                violations.push(SafetyViolation {
                    property: self.verified_properties[i].name.clone(),
                    formula: self.verified_properties[i].formula.clone(),
                    severity: ViolationSeverity::Critical,
                });
            }
            self.monitor_states[i] = new_state;
        }
        violations
    }
}
```

### Configuration

```toml
[safety.verification]
# Enable contract enforcement on tool dispatch.
contracts_enabled = true
# Enable VeriGuard runtime monitoring.
veriguard_enabled = true
# Path to verified properties file.
verified_properties_path = ".roko/verified-properties.json"
# Maximum soft constraint violations before escalation.
# Range: 1..100. Default: 3.
default_max_soft_violations = 3
# Soft constraint recovery window in seconds.
# Range: 10..3600. Default: 300.
default_recovery_window_secs = 300
# Online monitor check interval (events).
# Range: 1..100. Default: 1 (every event).
monitor_check_interval = 1
```

### Test criteria

- `ToolContract` precondition failure blocks tool execution
- `ToolContract` hard constraint failure triggers immediate abort
- `ToolContract` soft constraint violation increments counter; escalates at max_violations
- `ContractEnforcingDispatcher` correctly chains with inner `ToolDispatcher` and `SafetyLayer`
- `VeriGuard::check_event()` detects violations matching compiled Buchi automaton
- `VerifiedProperty` with `Proved` result enables corresponding runtime monitor
- `VerifiedProperty` with `Disproved` result raises alert at startup
- Host-agent property HA-1 (no unauthorized tool execution) correctly rejects calls from unauthorized roles
- Task-lifecycle property TL-2 (DAG ordering) correctly rejects out-of-order task execution
- Contract for `write_file` blocks writes to `.env` files

### Academic references

| Paper | Contribution |
|---|---|
| Formalizing Properties of Agentic AI (arXiv:2510.14133, 2025) | 17 host-agent + 14 task-lifecycle properties |
| Verifiably Safe Tool Use (ICSE 2026, Doshi et al.) | STPA-derived safety requirements for agents |
| Agent Behavioral Contracts (arXiv:2602.22302, 2026) | Design-by-Contract for AI agents |
| VeriGuard (arXiv:2510.05156) | Dual-stage offline proof + online monitor |
| Pro2Guard | Probabilistic model checking via DTMC |
| Agent-C | DSL for temporal safety constraints with SMT |
| Flyvy (VMware Research) | Rust framework for FO-LTL verification |
| Havelund & Rosu (2004, FMSD) | O(|formula|) runtime monitoring |

---

## Related Topics

- [08-threat-model.md](08-threat-model.md) — Threats that formal verification mitigates
- [10-mev-protection.md](10-mev-protection.md) — MEV detection (runtime) complements formal verification (pre-deployment)
- [11-temporal-logic.md](11-temporal-logic.md) — Temporal properties verified by the pipeline
- [12-witness-dag.md](12-witness-dag.md) — Verification results stored as DAG vertices
