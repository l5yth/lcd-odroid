# lcd-odroid

Rust daemon that drives a 20×4 HD44780 character LCD over I²C from an ODROID,
displaying live blockchain state from a local node. Supports Ethereum execution
layer (go-ethereum), Ethereum consensus layer (Lighthouse / any standard BN API),
and Bitcoin Core.

## Configuration

Create `config.toml` next to the binary and set `type` to the desired layer:

```toml
# "execution" (go-ethereum), "consensus" (Beacon Node), or "bitcoin" (Bitcoin Core)
type = "execution"
```

## Display layout

### Execution layer (`type = "execution"`)

```
Block    #15_623_137
0x0123456789abcdef01
2026-04-17 15:23:07Z
12.34 gwei  42 peers
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

### Bitcoin (`type = "bitcoin"`)

```toml
type = "bitcoin"
rpcuser = "alice"
rpcpassword = "s3cr3t"
```

```
Block       #896_969
0x000000000000000000
2026-04-17 15:23:07Z
12.3 sat/vB 42 peers
```

1. Latest block height, thousands grouped with `_`.
2. Block hash truncated to 20 characters (`0x` + first 18 hex chars). Bitcoin hashes
   have many leading zeros that visually reflect the current proof-of-work difficulty.
3. Block timestamp in UTC.
4. Estimated next-block fee rate in sat/vByte (from `estimatesmartfee`) and connected
   peer count.

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
  each new slot (~12 seconds). On stream end or error the daemon exits; run
  under systemd with `Restart=on-failure` for automatic reconnect.
- **Bitcoin**: a Bitcoin Core full node with `server=1`, `rpcuser=`, and
  `rpcpassword=` set in `bitcoin.conf`, exposing JSON-RPC at `127.0.0.1:8332`.
  The daemon performs an initial render on startup, then long-polls
  `waitfornewblock` (60-second timeout) and refreshes the display on each new
  block (~10-minute average). See `contrib/bitcoind/` for a reference
  `bitcoin.conf` and systemd unit. Note: the reference config uses
  `assumevalid=0` to force full script verification from genesis — initial block
  download will take several days.
- **Security**: `config.toml` contains the Bitcoin RPC password in plaintext;
  `chmod 600 config.toml` after writing it.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).

<!--
SPDX-License-Identifier: Apache-2.0
Copyright 2026 l5y
-->
