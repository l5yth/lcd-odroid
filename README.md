# lcd-odroid

Rust daemon that drives a 20×4 HD44780 character LCD over I²C from an ODROID,
displaying live execution-layer state from a local go-ethereum node.

## Display layout

```
Block    #15_623_137
0x0123456789abcdef01
2026-04-17 15:23:07Z
12.34 gwei   42 peers
```

1. Latest execution-layer block number, thousands grouped with `_`.
2. Block hash truncated to `0x` plus the first 9 bytes (18 hex chars).
3. Block timestamp in UTC.
4. Current gas price in gwei and connected peer count.

## Build and run

```
cargo build --release
sudo ./target/release/lcd-odroid
```

### Requirements

- HD44780-compatible LCD on I²C bus `/dev/i2c-0`, address `0x27`. Both are
  hardcoded in `src/main.rs`; change them for a different wiring.
- A local go-ethereum node started with `--http --ws`, exposing JSON-RPC at
  `127.0.0.1:8545` and WebSocket at `127.0.0.1:8546`.
- Read/write access to the I²C device node (root, or membership in the `i2c`
  group).

The daemon performs an initial render via HTTP, then subscribes to `newHeads`
over WebSocket and refreshes the display on each new block, throttled to one
update per second.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).

<!--
SPDX-License-Identifier: Apache-2.0
Copyright 2026 l5y
-->
