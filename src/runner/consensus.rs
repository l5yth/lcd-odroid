// Copyright 2026 l5y
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Consensus-layer runtime: Beacon Node REST + SSE driver.

use std::io::{BufRead, BufReader};
use std::time::Duration;

use lcd_odroid::{
    LcdDisplay, SSE_READ_TIMEOUT, format_lines_consensus, group_underscore, info, parse_sse_head,
    write_display,
};
use serde_json::Value;
use ureq::Agent;

/// Default Beacon Node REST API endpoint. Override with `cl_url` in
/// `config.toml`. Matches the Lighthouse default port.
pub const HTTP_URL_DEFAULT: &str = "http://127.0.0.1:5052";
/// Per-request timeout for non-streaming Beacon Node REST calls. Keeps a
/// misrouted or unresponsive endpoint from hanging the daemon silently.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

/// Runs the consensus-layer display loop.
///
/// Fetches the chain genesis time and current head once on startup, then
/// subscribes to `head` events over the Beacon Node SSE endpoint. Each
/// `data:` line triggers an immediate render — no throttle is needed because
/// Ethereum slots are 12 seconds apart and burst events do not occur on the
/// consensus layer the way reorgs can on the execution layer.
///
/// Returns an error on stream end or I/O failure. There is no internal
/// reconnect loop; run the daemon under systemd with `Restart=on-failure`
/// for automatic recovery.
pub fn run<L: LcdDisplay>(lcd: &mut L, cl_url: &str) -> Result<(), Box<dyn std::error::Error>> {
    // Genesis time is needed to convert slot numbers to wall-clock timestamps.
    let genesis_time: u64 = cl_get(cl_url, "/eth/v1/beacon/genesis")?["data"]["genesis_time"]
        .as_str()
        .ok_or("missing genesis_time")?
        .parse()?;
    info!("genesis_time={genesis_time}");

    // Initial render from the current chain head.
    let head = cl_get(cl_url, "/eth/v1/beacon/headers/head")?;
    let init_slot: u64 = head["data"]["header"]["message"]["slot"]
        .as_str()
        .ok_or("missing slot")?
        .parse()?;
    let init_root = head["data"]["root"]
        .as_str()
        .ok_or("missing root")?
        .to_string();
    render(lcd, cl_url, init_slot, &init_root, genesis_time)?;
    info!("initial render: slot #{}", group_underscore(init_slot));

    // Subscribe to head events via the Beacon Node SSE endpoint.
    // `timeout_recv_body` (the ureq 3 successor of `timeout_read`) ensures a
    // silent/hung connection is detected promptly; the daemon exits with an
    // error and relies on systemd to restart it.
    let agent: Agent = Agent::config_builder()
        .timeout_recv_body(Some(SSE_READ_TIMEOUT))
        .build()
        .into();
    let resp = agent
        .get(&format!("{cl_url}/eth/v1/events?topics=head"))
        .call()?;
    info!("subscribed to head SSE at {cl_url}");

    let reader = BufReader::new(resp.into_body().into_reader());
    for line_result in reader.lines() {
        let line = line_result?;
        if let Some((slot, block_root)) = parse_sse_head(&line)? {
            info!("received slot #{} from SSE", group_underscore(slot));
            render(lcd, cl_url, slot, &block_root, genesis_time)?;
            info!("updated lcd to slot #{}", group_underscore(slot));
        }
    }

    Err("SSE stream ended".into())
}

/// Fetches attestation count and peer count from the Beacon Node, formats
/// all four lines, and writes them to the display.
fn render<L: LcdDisplay>(
    lcd: &mut L,
    cl_url: &str,
    slot: u64,
    block_root: &str,
    genesis_time: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    // Count attestations from the block body.
    let block = cl_get(cl_url, &format!("/eth/v2/beacon/blocks/{block_root}"))?;
    let att_count = block["data"]["message"]["body"]["attestations"]
        .as_array()
        .map_or(0, |a| a.len());
    // Peer count is a decimal string (unlike EL hex quantities).
    let peers: u64 = cl_get(cl_url, "/eth/v1/node/peer_count")?["data"]["connected"]
        .as_str()
        .ok_or("missing peer count")?
        .parse()?;
    let lines = format_lines_consensus(slot, block_root, genesis_time, att_count, peers)?;
    write_display(lcd, &lines)
}

/// Sends a GET request to the Beacon Node REST API rooted at `cl_url`.
///
/// Bound by [`REQUEST_TIMEOUT`] so a misrouted endpoint or unresponsive
/// server fails loudly instead of hanging the daemon forever.
fn cl_get(cl_url: &str, path: &str) -> Result<Value, Box<dyn std::error::Error>> {
    let url = format!("{cl_url}{path}");
    info!("GET {url}");
    let agent: Agent = Agent::config_builder()
        .timeout_recv_response(Some(REQUEST_TIMEOUT))
        .timeout_recv_body(Some(REQUEST_TIMEOUT))
        .build()
        .into();
    Ok(agent.get(&url).call()?.body_mut().read_json()?)
}
