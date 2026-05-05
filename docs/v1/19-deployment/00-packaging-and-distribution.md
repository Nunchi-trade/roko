# Packaging and Distribution

> How Roko's 36-workspace-member Cargo workspace is packaged, versioned, and distributed to users through
> multiple channels: crates.io, Homebrew, GitHub Releases with prebuilt binaries, and Docker
> images on ghcr.io. This document covers the full release pipeline from commit to installable
> artifact.


> **Implementation**: Specified

---

## Distribution Philosophy

Roko is a Rust workspace producing multiple binaries from a single repository. Different users
need different install paths:

- **Rust developers and CI pipelines** want `cargo install` — compile from source, verify the
  code, integrate into existing Rust toolchains.
- **macOS and Linux users without Rust** want `brew install` — prebuilt binary, zero
  compilation, managed by the system package manager.
- **CI pipelines and Docker deployments** want prebuilt binaries from GitHub Releases — download
  a tarball with SHA256 checksums, no toolchain required.
- **Server deployments and Fly.io** want Docker images from ghcr.io — two variants per service
  (slim and full with web terminal support).
- **Developers who want instant updates** want `cargo binstall` — download prebuilt binaries
  from GitHub Releases instead of compiling from source (~10 seconds instead of ~3 minutes).

The distribution matrix maps products to channels:

| Channel | roko-cli | roko-serve | roko-mcp | mirage-rs |
|---|---|---|---|---|
| `cargo install` | Yes | Yes | Yes | Yes |
| `cargo binstall` | Yes | Yes | Yes | Yes |
| Homebrew | Yes | Yes | Yes | Yes |
| GitHub Releases (prebuilt) | Yes | Yes | Yes | Yes |
| Docker (ghcr.io) | Yes | Yes | No (local tool) | Yes |

The CLI and serve binaries are general-purpose tools — they get the full distribution treatment.
The MCP server is a local process spawned by editors — Docker is unnecessary since it
communicates over stdio. mirage-rs is both a local development tool and a server deployment, so
it gets all channels.

---

## What to Publish to crates.io

The workspace contains 36 workspace members. Most are internal implementation details. Publish only the
products (binaries users install) and their key shared libraries (types other Rust projects
might depend on):

| Crate | crates.io name | Why publish |
|---|---|---|
| `crates/roko-cli` | `roko-cli` | `cargo install roko-cli` — main user-facing binary |
| `crates/roko-serve` | `roko-serve` | `cargo install roko-serve` — HTTP API server |
| `crates/roko-core` | `roko-core` | Library: Engram + 6 Synapse traits, types, config |
| `crates/roko-std` | `roko-std` | Library: default trait implementations, 19 built-in tools |
| `crates/roko-agent` | `roko-agent` | Library: LLM backends, tool dispatch, MCP client |
| `crates/roko-gate` | `roko-gate` | Library: verification pipeline (11+ gates) |
| `crates/roko-compose` | `roko-compose` | Library: prompt assembly, context engineering |
| `crates/roko-index` | `roko-index` | Library: code parsing, symbol graphs, HDC fingerprints |
| `apps/mirage-rs` | `mirage-rs` | `cargo install mirage-rs` — EVM fork simulator |
| `crates/roko-primitives` | `roko-primitives` | Library: HDC vectors, Hamming similarity |

Non-publishable (Docker-only, scaffold, or internal): `roko-chain`,
`roko-conductor`, `roko-daimon`, `roko-dreams`, `roko-neuro`, `roko-learn`, `roko-fs`,
`roko-orchestrator`, `roko-runtime`, all `roko-lang-*` crates, and all `roko-mcp-*` server
crates. These get `publish = false` in their `Cargo.toml`.

### Workspace-Level Publish Defaults

In the root `Cargo.toml`, set `publish = false` as the workspace default so new crates are
non-publishable by default:

```toml
[workspace.package]
publish = false
edition = "2024"
license = "MIT OR Apache-2.0"
# ... existing: authors, repository ...
```

Published crates override this in their own `Cargo.toml`:

```toml
[package]
name = "roko-cli"
version = "0.2.0"
publish = true  # Override workspace default
description = "Cognitive agent toolkit — CLI for building agents that build themselves"
readme = "README.md"
keywords = ["ai", "agent", "orchestrator", "cognitive", "self-improving"]
categories = ["command-line-utilities", "development-tools"]
include = ["src/**/*", "Cargo.toml", "README.md", "LICENSE-*"]
# Inherit from workspace:
edition.workspace = true
license.workspace = true
rust-version.workspace = true
authors.workspace = true
repository.workspace = true
```

The `include` field limits what gets packaged for crates.io. Without it, `cargo publish` bundles
the entire workspace context. With it, only the crate's source, Cargo.toml, and README go into
the package.

### Versioning Strategy

Independent versioning per crate. The CLI may be at 0.2.0, mirage-rs at 0.5.0, roko-core at
0.3.0. They do not move in lockstep. Shared libraries version based on their own API surface.

The justfile includes a `semver` target that runs `cargo-semver-checks`. Run it before any
publish to catch accidental breaking changes in library crates:

```bash
just semver  # Runs cargo-semver-checks on all published library crates
```

Tightly-coupled crates (like `roko-core` and `roko-std`) can share a version group via
release-plz configuration, ensuring they always release together when either changes.

### Publish Order

crates.io requires that dependencies are published before dependents. The publish order for the
Roko workspace:

1. `roko-primitives` (zero internal deps)
2. `roko-core` (depends on `roko-primitives`)
3. `roko-std` (depends on `roko-core`)
4. `roko-agent` (depends on `roko-core`)
5. `roko-gate` (depends on `roko-core`)
6. `roko-compose` (depends on `roko-core`)
7. `roko-index` (depends on `roko-primitives`)
8. `roko-cli` (depends on `roko-core`, `roko-std`, `roko-agent`, `roko-gate`, etc.)
9. `roko-serve` (depends on the same set as roko-cli)
10. `mirage-rs` (minimal internal deps)

For local path dependencies to work alongside crates.io publishing, each Cargo.toml uses the
dual-source pattern:

```toml
[dependencies]
roko-core = { version = "0.3", path = "../../crates/roko-core" }
```

Cargo uses the path for local builds and the version for crates.io resolution. As of Cargo 1.90
(September 2025), `cargo publish --workspace` handles workspace-level publishing natively,
resolving dependency order automatically.

---

## Release Pipeline: release-plz + cargo-dist + git-cliff

The release pipeline chains three tools that are now standard for Rust binary distribution:

- **release-plz** — Runs on every push to main. Compares local packages against the crates.io
  registry, auto-bumps versions based on conventional commits, runs `cargo-semver-checks` to
  detect API breaking changes, generates changelogs via git-cliff, and opens a "Release PR" with
  updated CHANGELOG.md and Cargo.toml files. When merged, it publishes to crates.io and creates
  git tags.
- **cargo-dist** (v0.31+) — Picks up the tags from release-plz and builds binaries for 7+
  platform targets, generates shell and PowerShell installers, creates Homebrew formulae,
  produces npm wrapper packages, calculates SHA256 checksums, creates CycloneDX SBOMs, and
  publishes a GitHub Release.
- **git-cliff** — Generates changelogs from conventional commit messages, referenced by
  release-plz.

The pipeline flow:

```
push to main
  → release-plz detects changes
  → opens Release PR with version bumps + changelog
  → merge PR
  → release-plz publishes to crates.io + creates git tags
  → cargo-dist builds binaries + creates GitHub Release + updates Homebrew tap
```

### Monorepo Release Strategy: Singular Announcements

Each package gets its own tag: `roko-cli-v1.2.0`, `mirage-rs-v0.5.0`, `roko-core-v0.3.0`.
Each tag triggers a separate GitHub Release with platform-specific binaries for that one
package. Multiple tags can be pushed at once to release several packages simultaneously.

release-plz handles this natively via `[[package]]` sections in `release-plz.toml`.
Tightly-coupled crates use `version_group` to share versions. Internal-only crates use
`release = false`.

### Configuration: `release-plz.toml`

```toml
[workspace]
changelog_config = "cliff.toml"
allow_dirty = ["ci"]

# User-facing binaries: publish to crates.io + build binaries
[[package]]
name = "roko-cli"
changelog_include = ["roko-core", "roko-std"]
publish = true

[[package]]
name = "roko-serve"
publish = true

[[package]]
name = "mirage-rs"
publish = true

# Core libraries
[[package]]
name = "roko-core"
publish = true

[[package]]
name = "roko-std"
publish = true

[[package]]
name = "roko-primitives"
publish = true

# Tightly-coupled crates share a version
[[package]]
name = "roko-agent"
version_group = "roko-agent-libs"
publish = true

[[package]]
name = "roko-gate"
publish = true

[[package]]
name = "roko-compose"
publish = true

[[package]]
name = "roko-index"
publish = true

# Internal crates: no release
[[package]]
name = "roko-*"
release = false
publish = false
```

### Configuration: `cliff.toml`

```toml
[changelog]
header = "# Changelog\n\n"
body = """
{% for group, commits in commits | group_by(attribute="group") %}
### {{ group | upper_first }}
{% for commit in commits %}
- {{ commit.message | upper_first }} ({{ commit.id | truncate(length=7, end="") }})\
{% endfor %}
{% endfor %}
"""
trim = true

[git]
conventional_commits = true
filter_unconventional = true
commit_parsers = [
    { message = "^feat", group = "Features" },
    { message = "^fix", group = "Bug Fixes" },
    { message = "^perf", group = "Performance" },
    { message = "^refactor", group = "Refactor" },
    { message = "^doc", group = "Documentation" },
    { message = "^chore", skip = true },
    { message = "^ci", skip = true },
]
```

### Configuration: `dist-workspace.toml`

```toml
[dist]
cargo-dist-version = "0.31.0"
ci = "github"
installers = ["shell", "powershell", "homebrew"]
targets = [
    "x86_64-unknown-linux-gnu",
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-gnu",
    "aarch64-unknown-linux-musl",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
]
tap = "nunchi/homebrew-roko"
publish-jobs = ["homebrew"]
install-path = "~/.cargo/bin"
```

---

## CI Workflows

### Release PR Workflow

`.github/workflows/release-plz.yml` — Runs on push to main, opens Release PRs:

```yaml
name: Release-plz
on:
  push:
    branches: [main]

jobs:
  release-plz:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
        with:
          fetch-depth: 0
      - uses: dtolnay/rust-toolchain@stable
      - uses: MarcoIeni/release-plz-action@v0.5
        with:
          command: release-pr
        env:
          GITHUB_TOKEN: ${{ secrets.RELEASE_PLZ_TOKEN }}
          CARGO_REGISTRY_TOKEN: ${{ secrets.CRATES_IO_TOKEN }}
```

The `RELEASE_PLZ_TOKEN` must be a Personal Access Token, not the default `GITHUB_TOKEN`.
The default token cannot trigger downstream workflows (cargo-dist), so tags created with it
would not produce binary releases.

### Binary Release Workflow

`.github/workflows/release.yml` — Auto-generated by `cargo dist init`. Triggers on tags,
builds binaries for all target platforms, creates GitHub Releases, updates the Homebrew tap.
Do not edit manually — regenerate with `cargo dist generate`.

### Docker Image Workflow

`.github/workflows/docker.yml` — Triggers on tag push. See `03-docker.md` for the full
workflow definition including cross-compilation, multi-arch builds, and slim/full image variants.

---

## Self-Update via axoupdater

axoupdater (the cargo-dist companion for self-update) uses install receipts (JSON metadata
created by the shell/PowerShell installer) to detect when updates are available.

Add `axoupdater` as a dependency to `roko-cli`, `roko-serve`, and `mirage-rs`:

```toml
[dependencies]
axoupdater = { version = "0.8", default-features = false }
```

Implement a soft notification on startup (do not auto-update — print a message):

```rust
// In main.rs, after CLI parsing
if let Ok(updater) = axoupdater::AxoUpdater::new_for("roko-cli") {
    if let Ok(Some(release)) = updater.load_receipt()
        .and_then(|u| u.is_update_needed_sync())
    {
        eprintln!(
            "roko {} available (current: {}). Run: roko update",
            release.version(),
            env!("CARGO_PKG_VERSION")
        );
    }
}
```

Add an explicit `update` subcommand:

```rust
// roko update
fn run_update() -> Result<()> {
    let mut updater = axoupdater::AxoUpdater::new_for("roko-cli")?;
    updater.load_receipt()?.run_sync()?;
    Ok(())
}
```

---

## Homebrew Tap

cargo-dist auto-generates the Homebrew formula and pushes it to the tap repository. The formula
includes SHA256 checksums, SPDX license translation, and `brew style --fix` compliance. Shell
completions are installed automatically as part of the formula (via `clap_complete` output).

```bash
brew tap nunchi/roko
brew install roko-cli
brew install roko-serve
brew install mirage-rs
```

### Shell Completions

All tools use `clap_complete` for shell completions. Each binary provides a `completions <shell>`
subcommand:

```bash
roko completions bash > ~/.local/share/bash-completion/completions/roko
roko completions zsh > ~/.zfunc/_roko
roko completions fish > ~/.config/fish/completions/roko.fish
```

The Homebrew formula installs completions automatically. The shell installer adds a note about
manual completion setup.

---

## Release Flow in Practice

```bash
# 1. Write code, commit with conventional messages
git commit -m "feat(roko-cli): add streaming plan execution"
git commit -m "fix(roko-agent): handle timeout on provider failover"
git push origin main

# 2. release-plz runs in CI, detects changes, opens a Release PR:
#    "Release: roko-cli v0.3.1, roko-agent v0.1.3"
#    The PR updates Cargo.toml versions, CHANGELOG.md for each package

# 3. Review and merge the Release PR

# 4. release-plz publishes to crates.io, creates tags:
#    roko-cli-v0.3.1, roko-agent-v0.1.3

# 5. cargo-dist picks up the tags, builds binaries for all platforms,
#    creates GitHub Releases, updates Homebrew tap

# 6. Users can now:
cargo binstall roko-cli                     # Prebuilt binary (~10 sec)
brew upgrade roko-cli                       # Homebrew
curl ... | sh                               # Shell installer
docker pull ghcr.io/nunchi/roko-cli:0.3.1   # Docker
```

---

## cargo binstall Support

Add binstall metadata to each published binary crate's `Cargo.toml`:

```toml
[package.metadata.binstall]
pkg-url = "{ repo }/releases/download/roko-cli-v{ version }/roko-cli-{ version }-{ target }.tar.gz"
bin-dir = "roko-cli-{ version }-{ target }/{ bin }{ binary-ext }"
pkg-fmt = "tgz"
```

The CI release workflow (cargo-dist) produces tarballs in the expected format. `cargo binstall
roko-cli` downloads the prebuilt binary instead of compiling — approximately 10 seconds instead
of 3+ minutes.

---

## Install UX: What Users Type

```bash
# ── Roko CLI (cognitive agent toolkit) ──────────────────────────────
cargo install roko-cli
roko init --global               # Set up ~/.config/roko/config.toml
cd my-project && roko init       # Set up .roko/ in project

# ── Roko Serve (HTTP API server) ────────────────────────────────────
cargo install roko-serve
roko-serve --port 8080           # Start API server

# ── mirage-rs (EVM fork simulator) ──────────────────────────────────
cargo install mirage-rs
mirage-rs --rpc-url $RPC_URL     # Fork mainnet

# ── Homebrew (macOS / Linux, no Rust required) ──────────────────────
brew tap nunchi/roko
brew install roko-cli mirage-rs

# ── Docker (deployed services) ──────────────────────────────────────
docker compose -f docker/docker-compose.yml up

# ── Fly.io ──────────────────────────────────────────────────────────
./deploy/scripts/fly-deploy.sh all
```

See `08-subscription-configuration.md` for the full config model and `06-cloud-fly-io.md` for
Fly.io deployment details.

---

## Supply Chain Security

Roko's release pipeline incorporates defense-in-depth supply chain security: binary signing, SBOM generation, dependency auditing, and provenance attestations. These protect users from compromised builds, tampered artifacts, and malicious dependency injections.

### Sigstore / cosign Binary Signing

All release binaries are signed using Sigstore's keyless signing via GitHub Actions OIDC. No long-lived private keys exist — each signing event produces an ephemeral certificate from Fulcio, recorded in the Rekor transparency log.

```rust
/// Verify a downloaded binary's Sigstore bundle.
/// Called by `roko update --verify` after downloading a new release.
pub struct SigstoreVerifier {
    /// Expected GitHub workflow identity (certificate SAN)
    pub certificate_identity: String,
    /// Expected OIDC issuer
    pub certificate_issuer: String,
}

impl SigstoreVerifier {
    pub fn new_for_roko() -> Self {
        Self {
            certificate_identity: format!(
                "https://github.com/nunchi/roko/.github/workflows/release.yml@refs/tags/{}",
                env!("CARGO_PKG_VERSION")
            ),
            certificate_issuer: "https://token.actions.githubusercontent.com".to_string(),
        }
    }

    /// Verify a binary against its Sigstore bundle.
    /// Returns Ok(()) if the signature is valid, the certificate identity matches,
    /// and the signing event is recorded in Rekor.
    pub fn verify(&self, binary_path: &Path, bundle_path: &Path) -> Result<()> {
        let bundle = std::fs::read(bundle_path)?;
        let binary = std::fs::read(binary_path)?;

        // Shell out to cosign for verification (cosign verify-blob)
        let status = std::process::Command::new("cosign")
            .args(["verify-blob"])
            .arg(binary_path)
            .args(["--bundle", &bundle_path.to_string_lossy()])
            .args(["--certificate-identity", &self.certificate_identity])
            .args(["--certificate-oidc-issuer", &self.certificate_issuer])
            .status()?;

        if !status.success() {
            anyhow::bail!("Sigstore verification failed — binary may be tampered");
        }
        Ok(())
    }
}
```

CI workflow for signing (added to the cargo-dist release workflow):

```yaml
# .github/workflows/release.yml (additions to cargo-dist generated workflow)
jobs:
  sign-artifacts:
    needs: [build-artifacts]
    runs-on: ubuntu-latest
    permissions:
      id-token: write    # Required for Sigstore OIDC
      contents: write
      attestations: write
    steps:
      - uses: sigstore/cosign-installer@v3

      - name: Sign binaries with cosign (keyless)
        run: |
          for artifact in dist/*.tar.gz dist/*.zip; do
            cosign sign-blob \
              --bundle "${artifact}.sigstore.json" \
              "$artifact"
          done

      - name: Generate SLSA provenance attestation
        uses: actions/attest-build-provenance@v2
        with:
          subject-path: 'dist/*'

      - name: Upload signatures alongside release
        uses: softprops/action-gh-release@v2
        with:
          files: |
            dist/*.sigstore.json
```

Users verify downloaded binaries:

```bash
# Verify a downloaded release binary
cosign verify-blob roko-cli-0.3.0-x86_64-unknown-linux-musl.tar.gz \
  --bundle roko-cli-0.3.0-x86_64-unknown-linux-musl.tar.gz.sigstore.json \
  --certificate-identity="https://github.com/nunchi/roko/.github/workflows/release.yml@refs/tags/roko-cli-v0.3.0" \
  --certificate-oidc-issuer="https://token.actions.githubusercontent.com"
```

### CycloneDX SBOM Generation

Every release includes a CycloneDX SBOM (Software Bill of Materials) listing all transitive Cargo dependencies with versions and licenses. This is enabled in `dist-workspace.toml`:

```toml
[dist]
sbom = true  # Generates bom.xml via cargo-cyclonedx
```

cargo-dist invokes `cargo-cyclonedx` during the build phase. The SBOM is attached to the GitHub Release alongside binaries. The SBOM captures:

- All direct and transitive Cargo dependencies
- Package versions, SPDX license identifiers, and package URLs (purls)
- Build environment metadata (Rust version, target triple)

For runtime binary auditing, `cargo-auditable` embeds a compressed dependency manifest in a `.dep_v0` ELF section:

```toml
# Cargo.toml (published binary crates)
[package.metadata.dist]
cargo-auditable = true
```

Tools like `trivy` and `syft` can extract dependency information directly from the binary:

```bash
# Scan a release binary for known vulnerabilities
trivy fs --scanners vuln target/release/roko-cli

# Extract SBOM from auditable binary
syft target/release/roko-cli -o cyclonedx-json > roko-cli-sbom.json
```

### Dependency Auditing: cargo-deny and cargo-vet

Two complementary tools enforce supply chain policy:

**cargo-deny** checks licenses, advisories, bans, and source restrictions:

```toml
# deny.toml
[advisories]
vulnerability = "deny"
unmaintained = "warn"
yanked = "deny"

[licenses]
allow = [
    "MIT", "Apache-2.0", "Apache-2.0 WITH LLVM-exception",
    "BSD-2-Clause", "BSD-3-Clause", "ISC", "Unicode-DFS-2016",
]
exceptions = [
    { allow = ["LicenseRef-ring"], name = "ring", version = "*" },
]

[bans]
multiple-versions = "warn"

[sources]
unknown-registry = "deny"
unknown-git = "deny"
allow-registry = ["https://github.com/rust-lang/crates.io-index"]
```

**cargo-vet** verifies that every transitive dependency has been audited by a trusted source. Import audit registries from Mozilla, Google, and the Bytecode Alliance:

```toml
# supply-chain/config.toml
[cargo-vet]
version = "0.10"

[imports.mozilla]
url = "https://raw.githubusercontent.com/mozilla/supply-chain/main/audits.toml"

[imports.google]
url = "https://raw.githubusercontent.com/google/supply-chain/main/audits.toml"

[imports.bytecode-alliance]
url = "https://raw.githubusercontent.com/bytecodealliance/supply-chain/main/audits.toml"
```

CI enforcement:

```yaml
# .github/workflows/ci.yml
- name: cargo deny
  uses: EmbarkStudios/cargo-deny-action@v2
  with:
    command: check all

- name: cargo vet
  run: cargo vet check
```

### SLSA Provenance (Level 2+)

cargo-dist v0.30.0+ generates GitHub Artifact Attestations automatically when configured:

```toml
# dist-workspace.toml
[dist]
github-attestations = true
attestations-phase = "announce"
```

This provides SLSA Level 2 provenance — a cryptographically signed statement that a specific artifact was produced by a specific CI workflow from a specific source commit. Users can verify the provenance chain from source commit to binary.

### Supply Chain Security Summary

| Layer | Tool | What It Checks | CI Step |
|---|---|---|---|
| Source dependencies | cargo-deny | Licenses, advisories, bans | `cargo deny check all` |
| Audit trail | cargo-vet | Human audit attestations | `cargo vet check` |
| Binary composition | cargo-auditable | Embedded dep manifest | Build flag |
| Release artifacts | CycloneDX SBOM | Full dependency tree | `cargo-cyclonedx` |
| Binary authenticity | Sigstore/cosign | Keyless signing + Rekor log | `cosign sign-blob` |
| Build provenance | SLSA attestation | Source → binary traceability | `attest-build-provenance` |

---

## Advanced cargo-dist Configuration

### Monorepo Tag Strategy: Singular Announcements

Each package gets its own git tag and GitHub Release. This is the correct strategy for Roko's independent versioning model:

| Tag Format | Behavior |
|---|---|
| `roko-cli-v0.3.0` | Singular: builds only `roko-cli` binaries |
| `mirage-rs-v0.5.0` | Singular: builds only `mirage-rs` binaries |
| `roko-core-v0.3.0` | Singular: publishes library to crates.io (no binary build) |

Multiple tags can be pushed simultaneously to release several packages at once.

### GitHub Actions Pinning (v0.29.0+)

For supply chain hardening of the release workflow itself, cargo-dist v0.29.0+ supports pinning GitHub Actions to commit SHAs instead of mutable tags:

```toml
[dist]
github-actions-pinning = true  # Pin action versions to commit SHAs
```

This prevents a compromised upstream action from injecting malicious steps into the release pipeline.

### Extended dist-workspace.toml

```toml
[dist]
cargo-dist-version = "0.31.0"
ci = ["github"]

targets = [
    "x86_64-unknown-linux-gnu",
    "x86_64-unknown-linux-musl",
    "aarch64-unknown-linux-gnu",
    "aarch64-unknown-linux-musl",
    "x86_64-apple-darwin",
    "aarch64-apple-darwin",
]

# Installers
installers = ["shell", "powershell", "homebrew"]
tap = "nunchi/homebrew-roko"
install-path = "~/.cargo/bin"

# Self-update
install-updater = true
always-use-latest-updater = false

# Supply chain security
sbom = true
github-attestations = true
attestations-phase = "announce"
github-actions-pinning = true

# PR behavior
pr-run-mode = "skip"

# Archive formats
unix-archive = ".tar.gz"
windows-archive = ".zip"
```

### cargo-binstall Metadata

Extended binstall metadata with per-target overrides and minisign signature verification:

```toml
# Cargo.toml (each published binary crate)
[package.metadata.binstall]
pkg-url = "{ repo }/releases/download/roko-cli-v{ version }/roko-cli-{ version }-{ target }.tar.gz"
bin-dir = "roko-cli-{ version }-{ target }/{ bin }{ binary-ext }"
pkg-fmt = "tgz"

[package.metadata.binstall.overrides]
"x86_64-pc-windows-msvc" = { pkg-fmt = "zip" }
"cfg(target_os = \"macos\")" = {
    pkg-url = "{ repo }/releases/download/roko-cli-v{ version }/roko-cli-{ version }-universal-apple-darwin.tar.gz"
}

[package.metadata.binstall.signing]
algorithm = "minisign"
pubkey = "RWRnmBcLmQbXVcEPWo2OOKMI36kki4GiI7gcBgIaPLwvxe14Wtxm9acX"
```

### Test Criteria

```
Supply chain tests:
1. `cargo deny check all` passes with zero denials in CI
2. `cargo vet check` passes (all dependencies audited or exempted)
3. Release binaries have `.sigstore.json` bundles attached
4. `cosign verify-blob` succeeds for each release artifact
5. CycloneDX SBOM is present in GitHub Release assets
6. `trivy fs target/release/roko-cli` reports zero critical/high CVEs
7. `cargo binstall roko-cli` downloads and verifies minisign signature
```
