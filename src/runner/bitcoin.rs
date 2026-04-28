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

//! Bitcoin runtime: long-poll JSON-RPC against a local Bitcoin Core node.

use base64::Engine as _;
use lcd_odroid::{LcdDisplay, format_lines_bitcoin, group_underscore, info, write_display};
use serde_json::{Value, json};

/// Default Bitcoin Core JSON-RPC HTTP endpoint. Override with `btc_url` in
/// `config.toml`.
pub const HTTP_URL_DEFAULT: &str = "http://127.0.0.1:8332";

/// Runs the Bitcoin display loop.
///
/// Performs an initial render from the current chain tip, then long-polls
/// [`waitfornewblock`](https://developer.bitcoin.org/reference/rpc/waitfornewblock.html)
/// with a 60-second timeout. The display is updated whenever the block hash
/// changes; timeouts that return the same hash are ignored.
pub fn run<L: LcdDisplay>(
    lcd: &mut L,
    btc_url: &str,
    user: &str,
    pass: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Fetch the current chain tip for the initial render.
    let chain_info = rpc(btc_url, "getblockchaininfo", json!([]), user, pass)?;
    let best_hash = chain_info["bestblockhash"]
        .as_str()
        .ok_or("missing bestblockhash")?
        .to_string();
    let tip = rpc(btc_url, "getblock", json!([best_hash, 1]), user, pass)?;
    let init_height = tip["height"].as_u64().ok_or("missing height")?;
    let init_time = tip["time"].as_u64().ok_or("missing time")?;
    render(lcd, btc_url, init_height, &best_hash, init_time, user, pass)?;
    info!("initial render: block #{}", group_underscore(init_height));

    let mut last_hash = best_hash;
    loop {
        // waitfornewblock blocks until a new block arrives or the timeout expires.
        // It always returns the current best, so we compare against last_hash to
        // distinguish a new block from a timeout with no new block.
        let res = rpc(btc_url, "waitfornewblock", json!([60_000]), user, pass)?;
        let new_hash = res["hash"].as_str().ok_or("missing hash")?.to_string();
        if new_hash != last_hash {
            let block = rpc(btc_url, "getblock", json!([new_hash, 1]), user, pass)?;
            let height = block["height"].as_u64().ok_or("missing height")?;
            let timestamp = block["time"].as_u64().ok_or("missing time")?;
            render(lcd, btc_url, height, &new_hash, timestamp, user, pass)?;
            info!("updated lcd to block #{}", group_underscore(height));
            last_hash = new_hash;
        }
    }
}

/// Fetches current fee rate and peer count from the Bitcoin node, formats all
/// four lines, and writes them to the display.
fn render<L: LcdDisplay>(
    lcd: &mut L,
    btc_url: &str,
    height: u64,
    hash: &str,
    timestamp: u64,
    user: &str,
    pass: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Estimate next-block fee rate in sat/vByte from Bitcoin Core's fee estimator.
    // `feerate` is in BTC/kB; multiply by 1e5 to convert to sat/vByte.
    let fee_result = rpc(
        btc_url,
        "estimatesmartfee",
        json!([1, "ECONOMICAL"]),
        user,
        pass,
    )?;
    let fee_sat_vb = if let Some(feerate) = fee_result["feerate"].as_f64() {
        feerate * 1e5
    } else {
        // Insufficient chain data for the estimator (common on fresh nodes).
        // Fall back to the mempool minimum fee; default to 1.0 if also unavailable.
        rpc(btc_url, "getmempoolinfo", json!([]), user, pass)?["mempoolminfee"]
            .as_f64()
            .map(|f| f * 1e5)
            .unwrap_or(1.0)
    };
    let peers = rpc(btc_url, "getnetworkinfo", json!([]), user, pass)?["connections"]
        .as_u64()
        .ok_or("missing connections")?;
    let lines = format_lines_bitcoin(height, hash, timestamp, fee_sat_vb, peers)?;
    write_display(lcd, &lines)
}

/// Sends a JSON-RPC 1.0 request with HTTP Basic auth and returns the
/// `result` field.
fn rpc(
    btc_url: &str,
    method: &str,
    params: Value,
    user: &str,
    pass: &str,
) -> Result<Value, Box<dyn std::error::Error>> {
    let auth = format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"))
    );
    let body = json!({ "jsonrpc": "1.0", "id": 1, "method": method, "params": params });
    let resp: Value = ureq::post(btc_url)
        .header("Authorization", &auth)
        .send_json(body)?
        .body_mut()
        .read_json()?;
    if !resp["error"].is_null() {
        return Err(format!("rpc {method}: {}", resp["error"]).into());
    }
    Ok(resp["result"].clone())
}
