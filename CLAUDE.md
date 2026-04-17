<!--
SPDX-License-Identifier: Apache-2.0
Copyright 2026 l5y
-->

# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

Rust daemon that drives a 20×4 HD44780 character LCD over I²C from an ODROID SBC and shows live execution-layer state pulled from a local go-ethereum node. Single-file program (`src/main.rs`) using `hd44780-driver` on top of `linux-embedded-hal`, with `ureq` for JSON-RPC over HTTP, `tungstenite` for WebSocket subscriptions, and `chrono` for timestamp formatting. Edition 2024, Apache-2.0 licensed.

## Commands

- Build: `cargo build` (release: `cargo build --release`)
- Run: `cargo run` — must execute on the ODROID (or wherever `/dev/i2c-0` exists) with access to the I²C device node (root or `i2c` group), and needs geth reachable at `127.0.0.1:8545` (HTTP) and `127.0.0.1:8546` (WS).
- Check without linking: `cargo check`
- Tests: `cargo test` (currently none defined)

CI runs `cargo build` and `cargo test` on every push/PR via `.github/workflows/ci.yml`. A separate `coverage.yml` workflow runs `cargo-llvm-cov` and uploads to Codecov; `codecov.yml` sets a 100% target with a 10% threshold. Dependabot watches the cargo manifest weekly.

## Hardware and node coupling

The program opens a hardcoded I²C bus (`/dev/i2c-0`) and addresses the LCD at `0x27`. RPC endpoints are also hardcoded (`HTTP_URL`, `WS_URL` constants in `src/main.rs`). Code cannot be meaningfully exercised without both the LCD and a running geth — `cargo check` / `cargo build` are the only feedback loops available off-device.

## Runtime behavior

Startup does one HTTP fetch of the latest block and renders all four lines, so the display is populated immediately. Then the daemon subscribes to `newHeads` over WebSocket, latches the most recent header into a `pending` slot, and renders at most once per second; bursts (e.g. reorgs) collapse to the latest header. Per-render the program also re-fetches `eth_gasPrice` and `net_peerCount` over HTTP. All four lines are formatted to exactly 20 characters so the LCD doesn't need a clear between updates.

## Licensing

All `.rs` source files carry the full Apache-2.0 header. All other text files (configs, workflows, README) carry a 2-line SPDX notice (`# SPDX-License-Identifier: Apache-2.0` + `# Copyright 2026 l5y`, or the HTML-comment equivalent for Markdown). When adding new files, follow the same convention. `Cargo.lock` is intentionally left without a notice because cargo regenerates it.
