# Trading examples

Walks through exercising the roko trading surface from the CLI. See
[`plans/P08-trading-surface.md`](../../plans/P08-trading-surface.md) for the
full design.

## Prerequisites

```
cargo build --workspace
```

## List registered strategies

```
cargo run --bin roko -- trading strategies list
```

Prints every strategy in `roko-strategy::registry::StrategyRegistry`.

## Inspect the Perpetual Agent Jobs registry

```
cargo run --bin roko -- jobs list
cargo run --bin roko -- jobs info oracle_updater
```

The list + info output mirrors `hl jobs list` / `hl jobs info` from
[`Nunchi-trade/offchainservices-agent`](https://github.com/Nunchi-trade/offchainservices-agent).

## Place a single mock order

```
cargo run --bin roko -- trading trade \
  --venue mock --instrument ETH-PERP --side buy --size 0.01 --price 2500 --tif ioc
```

The `mock` venue is the deterministic in-memory backend from
`roko-venue::mock::MockVenue`. It auto-fills at the submitted price and is
the path CI runs. HL + Nunchi venue construction binds in once
`roko-chain`'s alloy submitter lands — tracked as the T12 close-out item in
the PRD.

## Run a strategy against the mock venue

```
cargo run --bin roko -- trading run --strategy simple_mm --venue mock --mock
```

Reports "Would run …" today; the full loop (tick scheduler + engine +
custody guard + persistence) binds in the T12 close-out once the
venue-factory + keystore wiring is in place.

## Inspect APEX presets

```
cargo run --bin roko -- trading apex presets
cargo run --bin roko -- trading apex once --preset default
```

## Unit test everything

```
cargo test --workspace
```

Every new crate ships its own tests; the suite is fast and has no network
or chain dependencies.
