#!/usr/bin/env bash
# mirage-rs + roko-chain-watcher end-to-end demo
#
# Forks Ethereum mainnet at 50ms blocks, launches 18 roko agents that analyze
# REAL chain data, and serves the browser dashboard on :8080.
#
# No synthetic data — every insight and pheromone comes from actual chain
# activity observed by the agents.
#
# Usage:
#   ./quickstart.sh                    # uses public RPC as fallback
#   ETH_RPC_URL="https://..." ./quickstart.sh   # use your own RPC
#
# Stop with Ctrl+C (all processes killed cleanly).

set -euo pipefail

DEMO_ROOT="$(cd "$(dirname "$0")" && pwd)"
ROKO_ROOT="$(cd "$DEMO_ROOT/.." && pwd)"
REPO_ROOT="$(cd "$ROKO_ROOT/.." && pwd)"

# Load .env if present (walk up to repo root)
for envfile in "$REPO_ROOT/.env" "$ROKO_ROOT/.env" "$DEMO_ROOT/.env"; do
    if [[ -f "$envfile" ]]; then
        set -a
        # shellcheck disable=SC1090
        source "$envfile"
        set +a
        break
    fi
done

# Upstream Ethereum RPC (for real block data)
# Prefer ETH_RPC_URL → MIRAGE_RPC_URL → public node as fallback.
ETH_RPC_URL="${ETH_RPC_URL:-${MIRAGE_RPC_URL:-}}"

# Validate the URL before we fork — fail fast with a useful message.
probe_rpc() {
    local url="$1"
    local body
    body=$(curl -s --max-time 6 -X POST -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"eth_blockNumber","id":1}' "$url" 2>/dev/null || echo "")
    echo "$body" | grep -q '"result"'
}

if [[ -z "$ETH_RPC_URL" ]] || ! probe_rpc "$ETH_RPC_URL"; then
    echo "==> configured ETH_RPC_URL unreachable/expired — trying public nodes"
    for url in \
        "https://ethereum-rpc.publicnode.com" \
        "https://eth.llamarpc.com" \
        "https://cloudflare-eth.com"
    do
        if probe_rpc "$url"; then
            ETH_RPC_URL="$url"
            echo "==> using public RPC: $url"
            break
        fi
    done
fi

if [[ -z "$ETH_RPC_URL" ]] || ! probe_rpc "$ETH_RPC_URL"; then
    echo "ERROR: no reachable Ethereum RPC. Set ETH_RPC_URL to a working mainnet endpoint."
    exit 1
fi

echo "==> using upstream ETH RPC: ${ETH_RPC_URL:0:48}..."
echo "==> building mirage-rs + roko-chain-watcher (release)"
(cd "$ROKO_ROOT" && cargo build --release --bin mirage-rs --features binary,chain,roko 2>&1 | tail -3)
(cd "$ROKO_ROOT" && cargo build --release -p roko-chain-watcher 2>&1 | tail -3)

# Find binaries — check common target dirs
find_bin() {
    local name="$1"
    for dir in \
        "$REPO_ROOT/.mori/cache/cargo-target/release" \
        "$ROKO_ROOT/target/release" \
        "$REPO_ROOT/target/release"
    do
        if [[ -x "$dir/$name" ]]; then echo "$dir/$name"; return 0; fi
    done
    # Last resort: ask cargo
    (cd "$ROKO_ROOT" && cargo build --release --bin "$name" --message-format=json 2>/dev/null \
        | grep -o "\"executable\":\"[^\"]*\"" | head -1 | cut -d'"' -f4)
}

MIRAGE_BIN="$(find_bin mirage-rs)"
WATCHER_BIN="$(find_bin roko-chain-watcher)"

if [[ -z "$MIRAGE_BIN" ]] || [[ ! -x "$MIRAGE_BIN" ]]; then
    echo "ERROR: could not find mirage-rs binary"
    exit 1
fi
if [[ -z "$WATCHER_BIN" ]] || [[ ! -x "$WATCHER_BIN" ]]; then
    echo "ERROR: could not find roko-chain-watcher binary"
    exit 1
fi
echo "==> mirage binary: $MIRAGE_BIN"
echo "==> watcher binary: $WATCHER_BIN"

PIDS=()
cleanup() {
    echo
    echo "==> shutting down ${#PIDS[@]} processes"
    for p in "${PIDS[@]}"; do kill "$p" 2>/dev/null || true; done
    wait 2>/dev/null || true
    echo "==> goodbye"
}
trap cleanup EXIT INT TERM

echo "==> starting mirage-rs (fork mainnet, 50ms local blocks, chain ext enabled)"
ROKO_LOG=info "$MIRAGE_BIN" \
    --host 127.0.0.1 --port 8545 \
    --rpc-url "$ETH_RPC_URL" \
    --block-interval-ms 50 \
    --chain-id 1 \
    --cache-size 50000 \
    --enable-hdc --enable-knowledge --enable-stigmergy \
    > /tmp/mirage-demo-mirage.log 2>&1 &
MIRAGE_PID=$!
PIDS+=($MIRAGE_PID)

# Wait for mirage to be ready
for i in {1..60}; do
    if curl -s -X POST -H "Content-Type: application/json" \
         -d '{"jsonrpc":"2.0","method":"eth_blockNumber","id":1}' \
         http://127.0.0.1:8545 2>/dev/null | grep -q result; then
        TIP_HEX=$(curl -s -X POST -H "Content-Type: application/json" -d '{"jsonrpc":"2.0","method":"eth_blockNumber","id":1}' http://127.0.0.1:8545 | sed 's/.*"result":"\([^"]*\)".*/\1/')
        TIP_DEC=$((16#${TIP_HEX#0x}))
        echo "==> mirage ready · fork tip = block $TIP_DEC ($TIP_HEX)"
        break
    fi
    sleep 0.5
done

echo "==> launching 18 roko agents (real chain analysis, no mock data)"

launch_watcher() {
    local name="$1"; shift
    ROKO_LOG=info "$WATCHER_BIN" \
        --rpc-url http://127.0.0.1:8545 \
        --eth-rpc-url "$ETH_RPC_URL" \
        --watcher-id "$name" \
        "$@" \
        > "/tmp/mirage-demo-watcher-${name}.log" 2>&1 &
    PIDS+=($!)
    echo "    $name (pid $!)"
}

# === BLOCK OBSERVERS (analyze real chain data, post grounded insights) ===

# Fast gas analyst — high frequency, light blocks
launch_watcher roko-alpha-gas \
    --block-poll-interval-ms 2000 --block-backfill 12 --max-reactions-per-min 80

# Whale/MEV detector — full tx analysis
launch_watcher roko-gamma-whale \
    --block-poll-interval-ms 4000 --block-backfill 8 \
    --fetch-full-txs --max-reactions-per-min 50

# DEX activity — full tx, focused query
launch_watcher roko-delta-dex \
    --block-poll-interval-ms 3000 --block-backfill 6 \
    --fetch-full-txs --max-reactions-per-min 60 \
    --query "swap liquidity pool uniswap sushiswap curve"

# Lending monitor — full tx
launch_watcher roko-iota-lending \
    --block-poll-interval-ms 3500 --block-backfill 6 \
    --fetch-full-txs --max-reactions-per-min 50 \
    --query "aave compound borrow supply lending liquidation"

# Stablecoin flows — full tx
launch_watcher roko-theta-stables \
    --block-poll-interval-ms 3500 --block-backfill 8 \
    --fetch-full-txs --max-reactions-per-min 50 \
    --query "USDT USDC DAI stablecoin depeg transfer volume"

# NFT/marketplace — full tx, slower poll
launch_watcher roko-eta-nft \
    --block-poll-interval-ms 6000 --block-backfill 5 \
    --fetch-full-txs --max-reactions-per-min 30 \
    --query "ERC721 ERC1155 transfer mint burn NFT marketplace"

# Bridge/L2 — full tx
launch_watcher roko-kappa-bridge \
    --block-poll-interval-ms 5000 --block-backfill 6 \
    --fetch-full-txs --max-reactions-per-min 40 \
    --query "bridge arbitrum optimism polygon zksync L2 cross-chain"

# Contract deploy/upgrade watcher
launch_watcher roko-zeta-risk \
    --block-poll-interval-ms 5000 --block-backfill 10 \
    --fetch-full-txs --max-reactions-per-min 45 \
    --query "deploy proxy upgrade governance selfdestruct"

# Liquid staking monitor
launch_watcher roko-lambda-lst \
    --block-poll-interval-ms 5000 --block-backfill 6 \
    --fetch-full-txs --max-reactions-per-min 35 \
    --query "lido rocketpool steth reth liquid staking"

# Fast block-structure analyst (empty blocks, reorgs, timing)
launch_watcher roko-mu-blocks \
    --block-poll-interval-ms 1500 --block-backfill 15 --max-reactions-per-min 90

# === PURE REACTORS (no block observer — confirm/challenge/synthesize) ===

# Broad consensus builder
launch_watcher roko-beta-consensus \
    --disable-block-observer --poll-interval-ms 3000 --poll-k 30 \
    --query "gas congestion whale threat opportunity arbitrage swap" \
    --max-reactions-per-min 40

# DEX-focused reactor
launch_watcher roko-nu-dex-react \
    --disable-block-observer --poll-interval-ms 4000 --poll-k 20 \
    --query "swap uniswap sushiswap curve dex router liquidity pool" \
    --max-reactions-per-min 30

# Gas/congestion reactor
launch_watcher roko-xi-gas-react \
    --disable-block-observer --poll-interval-ms 3000 --poll-k 25 \
    --query "gas gwei saturation congestion base fee spike low" \
    --max-reactions-per-min 35

# Whale/MEV reactor
launch_watcher roko-omicron-whale-react \
    --disable-block-observer --poll-interval-ms 4000 --poll-k 20 \
    --query "whale transfer large ETH MEV sandwich tip priority" \
    --max-reactions-per-min 25

# Stablecoin reactor
launch_watcher roko-pi-stable-react \
    --disable-block-observer --poll-interval-ms 5000 --poll-k 20 \
    --query "stablecoin USDC USDT DAI depeg velocity" \
    --max-reactions-per-min 25

# Cross-domain synthesizer
launch_watcher roko-rho-synth \
    --disable-block-observer --poll-interval-ms 3500 --poll-k 40 \
    --query "threat opportunity wisdom convergence pattern trend" \
    --max-reactions-per-min 35

# Oracle/price reactor
launch_watcher roko-epsilon-oracle \
    --disable-block-observer --poll-interval-ms 5000 --poll-k 15 \
    --query "oracle chainlink price feed stale deviation" \
    --max-reactions-per-min 20

# Skeptic — higher challenge rate
launch_watcher roko-sigma-skeptic \
    --disable-block-observer --poll-interval-ms 6000 --poll-k 30 \
    --query "confirmed warning threat wrong bug incorrect" \
    --max-reactions-per-min 20

sleep 3

echo "==> serving browser demo at http://127.0.0.1:8080"
(cd "$DEMO_ROOT" && python3 -m http.server 8080 --bind 127.0.0.1) > /dev/null 2>&1 &
PIDS+=($!)
sleep 1

echo ""
echo "=========================================================="
echo "  DEMO RUNNING — visit http://127.0.0.1:8080"
echo ""
echo "  mirage-rs:      fork of mainnet @ 50ms local blocks"
echo "                  http://127.0.0.1:8545"
echo "                  REST API at http://127.0.0.1:8545/api/"
echo ""
echo "  18 roko agents analyzing REAL ethereum data:"
echo ""
echo "  Block observers (10):"
echo "    alpha-gas        fast gas/fee trends"
echo "    gamma-whale      whale transfers, MEV"
echo "    delta-dex        DEX swaps, liquidity"
echo "    iota-lending     Aave/Compound activity"
echo "    theta-stables    stablecoin velocity"
echo "    eta-nft          NFT marketplace trades"
echo "    kappa-bridge     cross-chain bridges"
echo "    zeta-risk        contract upgrades, anomalies"
echo "    lambda-lst       liquid staking (Lido, Rocket Pool)"
echo "    mu-blocks        block structure, timing"
echo ""
echo "  Reactors (8):"
echo "    beta-consensus   broad consensus builder"
echo "    nu-dex-react     DEX signal amplifier"
echo "    xi-gas-react     gas signal amplifier"
echo "    omicron-whale    whale signal amplifier"
echo "    pi-stable-react  stablecoin signal amplifier"
echo "    rho-synth        cross-domain synthesizer"
echo "    epsilon-oracle   oracle/price reactor"
echo "    sigma-skeptic    contrarian challenger"
echo ""
echo "  logs: tail -f /tmp/mirage-demo-mirage.log"
echo "        tail -f /tmp/mirage-demo-watcher-roko-*.log"
echo ""
echo "  Ctrl+C to stop everything"
echo "=========================================================="
echo ""

wait $MIRAGE_PID
