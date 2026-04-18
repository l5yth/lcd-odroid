# lcd-odroid

Rust daemon that drives a 20×4 HD44780 character LCD over I²C from an ODROID,
displaying live Ethereum state from a local node. Supports both the execution
layer (go-ethereum) and the consensus layer (Lighthouse / any standard BN API).

## Configuration

Create `config.toml` next to the binary and set `type` to the desired layer:

```toml
# "execution" (go-ethereum) or "consensus" (Beacon Node)
type = "execution"
```

## Display layout

### Execution layer (`type = "execution"`)

```
Block    #15_623_137
0x0123456789abcdef01
2026-04-17 15:23:07Z
12.34 gwei   42 peers
```

1. Latest execution-layer block number, thousands grouped with `_`.
2. Block hash truncated to 20 characters (includes the `0x` prefix).
3. Block timestamp in UTC.
4. Current gas price in gwei and connected peer count.

### Consensus layer (`type = "consensus"`)

```
Slot     #21_833_152
0x0123456789abcdef01
2026-04-17 15:23:07Z
128 atts    42 peers
```

1. Latest beacon-chain slot number, thousands grouped with `_`.
2. Block root truncated to 20 characters (includes the `0x` prefix).
3. Slot timestamp derived from `genesis_time + slot × 12 s`, shown in UTC.
4. Attestation count included in the block body and connected peer count.

## Build and run

```
cargo build --release
sudo ./target/release/lcd-odroid
```

### Requirements

- HD44780-compatible LCD on I²C bus `/dev/i2c-0`, address `0x27`. Both are
  hardcoded in `src/main.rs`; change them for a different wiring.
- Read/write access to the I²C device node (root, or membership in the `i2c`
  group).
- **Execution layer**: a local go-ethereum node started with `--http --ws`,
  exposing JSON-RPC at `127.0.0.1:8545` and WebSocket at `127.0.0.1:8546`.
  The daemon performs an initial render via HTTP, then subscribes to `newHeads`
  over WebSocket and refreshes the display on each new block, throttled to one
  update per second.
- **Consensus layer**: a Beacon Node (Lighthouse or any BN REST API-compatible
  client) exposing the standard REST API at `127.0.0.1:5052`. The daemon
  performs an initial render on startup, then subscribes to `head` events over
  the SSE endpoint (`/eth/v1/events?topics=head`) and refreshes the display on
  each new slot (~12 seconds).

## License

Licensed under the [Apache License, Version 2.0](LICENSE).

<!--
SPDX-License-Identifier: Apache-2.0
Copyright 2026 l5y
-->
