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

//! Execution-layer runtime: HTTP polling for the initial render and
//! WebSocket `newHeads` subscription thereafter.

use std::io;
use std::net::TcpStream;
use std::time::Instant;

use lcd_odroid::{
    LcdDisplay, READ_TIMEOUT, THROTTLE, block_number, extract_new_head, format_lines,
    group_underscore, info, parse_hex_u64, write_display,
};
use serde_json::{Value, json};
use tungstenite::{Message, WebSocket, stream::MaybeTlsStream};

/// Default execution-layer JSON-RPC HTTP endpoint. Override with
/// `el_http_url` in `config.toml`.
pub const HTTP_URL_DEFAULT: &str = "http://127.0.0.1:8545";
/// Default execution-layer JSON-RPC WebSocket endpoint. Override with
/// `el_ws_url` in `config.toml`.
pub const WS_URL_DEFAULT: &str = "ws://127.0.0.1:8546";

/// Runs the execution-layer display loop.
///
/// Performs an initial HTTP fetch and render against `http_url`, then
/// subscribes to `newHeads` over `ws_url`. Incoming headers are latched into
/// a `pending` slot; the display is updated at most once per [`THROTTLE`]
/// second so that burst events (e.g. reorgs) collapse to the latest header.
pub fn run<L: LcdDisplay>(
    lcd: &mut L,
    http_url: &str,
    ws_url: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let initial = rpc_http(http_url, "eth_getBlockByNumber", json!(["latest", false]))?;
    render(lcd, http_url, &initial)?;
    info!(
        "initial render: block #{}",
        group_underscore(block_number(&initial))
    );
    let mut last_render = Instant::now();

    let (mut ws, _) = tungstenite::connect(ws_url)?;
    set_read_timeout(&mut ws, READ_TIMEOUT)?;
    ws.send(Message::text(
        json!({"jsonrpc":"2.0","id":1,"method":"eth_subscribe","params":["newHeads"]}).to_string(),
    ))?;
    info!("subscribed to newHeads at {ws_url}");

    let mut pending: Option<Value> = None;
    loop {
        match ws.read() {
            Ok(Message::Text(t)) => {
                if let Some(header) = extract_new_head(&t)? {
                    info!(
                        "received block #{} from websocket",
                        group_underscore(block_number(&header))
                    );
                    pending = Some(header);
                }
            }
            Ok(Message::Ping(p)) => ws.send(Message::Pong(p))?,
            Ok(Message::Close(_)) => return Err("websocket closed".into()),
            Ok(_) => {}
            // Non-fatal: the socket timed out during the polling window.
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(e) => return Err(e.into()),
        }

        if let Some(header) = pending.take() {
            if last_render.elapsed() >= THROTTLE {
                render(lcd, http_url, &header)?;
                last_render = Instant::now();
                info!(
                    "updated lcd to block #{}",
                    group_underscore(block_number(&header))
                );
            } else {
                // Throttle active: keep the header for the next iteration.
                pending = Some(header);
            }
        }
    }
}

/// Fetches live gas price and peer count, formats all four lines, and writes
/// them to the display.
fn render<L: LcdDisplay>(
    lcd: &mut L,
    http_url: &str,
    header: &Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let gas_wei = parse_hex_u64(
        rpc_http(http_url, "eth_gasPrice", json!([]))?
            .as_str()
            .ok_or("gas")?,
    )?;
    let peers = parse_hex_u64(
        rpc_http(http_url, "net_peerCount", json!([]))?
            .as_str()
            .ok_or("peers")?,
    )?;
    let lines = format_lines(header, gas_wei, peers)?;
    write_display(lcd, &lines)
}

/// Sends a JSON-RPC 2.0 request to `http_url` and returns the `result` field.
fn rpc_http(
    http_url: &str,
    method: &str,
    params: Value,
) -> Result<Value, Box<dyn std::error::Error>> {
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let resp: Value = ureq::post(http_url)
        .send_json(body)?
        .body_mut()
        .read_json()?;
    if let Some(err) = resp.get("error") {
        return Err(format!("rpc {method}: {err}").into());
    }
    Ok(resp["result"].clone())
}

/// Applies `dur` as the read-timeout to the underlying TCP socket of `ws`.
///
/// Only acts when the stream is a plain (non-TLS) connection; TLS streams
/// carry their own timeout machinery and are left untouched.
fn set_read_timeout(
    ws: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    dur: std::time::Duration,
) -> io::Result<()> {
    if let MaybeTlsStream::Plain(s) = ws.get_ref() {
        s.set_read_timeout(Some(dur))?;
    }
    Ok(())
}
