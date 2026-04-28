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

use std::fs;
use std::io::{self, BufRead, BufReader};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use base64::Engine as _;
use embedded_hal::blocking::delay::{DelayMs, DelayUs};
use embedded_hal::blocking::i2c::Write as I2cWrite02;
use embedded_hal_1::i2c::I2c as _;
use hd44780_driver::{Cursor, CursorBlink, Display, DisplayMode, HD44780, bus::DataBus};
use lcd_odroid::{
    LcdDisplay, READ_TIMEOUT, SSE_READ_TIMEOUT, THROTTLE, block_number, extract_new_head,
    format_lines, format_lines_bitcoin, format_lines_consensus, group_underscore, parse_hex_u64,
    parse_sse_head, write_display,
};
use linux_embedded_hal::{I2CError, I2cdev};
use serde_json::{Value, json};
use tungstenite::{Message, WebSocket, stream::MaybeTlsStream};
use ureq::Agent;

/// Path to the TOML configuration file, relative to the working directory.
const CONFIG_PATH: &str = "config.toml";
/// HTTP JSON-RPC endpoint for the local go-ethereum node.
const HTTP_URL: &str = "http://127.0.0.1:8545";
/// WebSocket endpoint used for `eth_subscribe` / `newHeads`.
const WS_URL: &str = "ws://127.0.0.1:8546";
/// Beacon Node REST API endpoint (Lighthouse default port).
const CL_HTTP_URL: &str = "http://127.0.0.1:5052";
/// Bitcoin Core JSON-RPC HTTP endpoint.
const BTC_HTTP_URL: &str = "http://127.0.0.1:8332";

/// Logs a timestamped INFO line to stderr.
macro_rules! info {
    ($($arg:tt)*) => {{
        eprintln!(
            "[{}] INFO  {}",
            chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ"),
            format_args!($($arg)*)
        );
    }};
}

/// Adapter that wraps [`linux_embedded_hal::I2cdev`] and re-exposes it through
/// the embedded-hal 0.2 [`I2cWrite02`] trait that `hd44780-driver` 0.4 requires.
///
/// `linux-embedded-hal` 0.4 dropped its embedded-hal 0.2 impls when it migrated
/// to 1.0; this shim forwards each 0.2 `write` call to the underlying 1.0 `I2c`
/// implementation so the existing driver keeps working unchanged.
struct I2cAdapter(I2cdev);

impl I2cWrite02 for I2cAdapter {
    type Error = I2CError;

    fn write(&mut self, address: u8, bytes: &[u8]) -> Result<(), Self::Error> {
        self.0.write(address, bytes)
    }
}

/// Adapter that re-exposes [`linux_embedded_hal::Delay`] through the
/// embedded-hal 0.2 [`DelayUs`]/[`DelayMs`] traits that `hd44780-driver` 0.4
/// requires. The driver only ever asks for `u16` µs and `u8` ms delays, so
/// only those two width specialisations are implemented.
struct DelayAdapter;

impl DelayUs<u16> for DelayAdapter {
    fn delay_us(&mut self, us: u16) {
        std::thread::sleep(Duration::from_micros(u64::from(us)));
    }
}

impl DelayMs<u8> for DelayAdapter {
    fn delay_ms(&mut self, ms: u8) {
        std::thread::sleep(Duration::from_millis(u64::from(ms)));
    }
}

/// Concrete LCD implementation that wraps [`HD44780`] and a [`DelayAdapter`].
///
/// Owns both so callers can use a single `&mut I2cLcd<B>` rather than threading
/// two separate mutable references through every call.
struct I2cLcd<B: DataBus> {
    lcd: HD44780<B>,
    delay: DelayAdapter,
}

impl<B: DataBus> LcdDisplay for I2cLcd<B> {
    fn write_line(&mut self, pos: u8, text: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.lcd
            .set_cursor_pos(pos, &mut self.delay)
            .map_err(|_| "cursor")?;
        self.lcd
            .write_str(text, &mut self.delay)
            .map_err(|_| "write")?;
        Ok(())
    }
}

/// Fetches live gas price and peer count, formats all four lines, and writes
/// them to the display.
///
/// All LCD rows are exactly 20 characters, so no full clear is needed between
/// updates — each render overwrites the previous content in place.
///
/// # Errors
/// Returns an error if any JSON-RPC HTTP request fails, a response field is
/// missing or malformed, [`format_lines`] returns an error, or the LCD write
/// fails.
fn render<B: DataBus>(
    lcd: &mut I2cLcd<B>,
    header: &Value,
) -> Result<(), Box<dyn std::error::Error>> {
    // Fetch ancillary data over HTTP for each render cycle.
    let gas_wei = parse_hex_u64(rpc_http("eth_gasPrice", json!([]))?.as_str().ok_or("gas")?)?;
    let peers = parse_hex_u64(
        rpc_http("net_peerCount", json!([]))?
            .as_str()
            .ok_or("peers")?,
    )?;
    let lines = format_lines(header, gas_wei, peers)?;
    write_display(lcd, &lines)
}

/// Runs the execution-layer display loop.
///
/// Performs an initial HTTP fetch and render, then subscribes to `newHeads`
/// over WebSocket. Incoming headers are latched into a `pending` slot; the
/// display is updated at most once per [`THROTTLE`] second so that burst events
/// (e.g. reorgs) collapse to the latest header.
fn run_execution<B: DataBus>(lcd: &mut I2cLcd<B>) -> Result<(), Box<dyn std::error::Error>> {
    let initial = rpc_http("eth_getBlockByNumber", json!(["latest", false]))?;
    render(lcd, &initial)?;
    info!(
        "initial render: block #{}",
        group_underscore(block_number(&initial))
    );
    let mut last_render = Instant::now();

    let (mut ws, _) = tungstenite::connect(WS_URL)?;
    set_read_timeout(&mut ws, READ_TIMEOUT)?;
    ws.send(Message::text(
        json!({"jsonrpc":"2.0","id":1,"method":"eth_subscribe","params":["newHeads"]}).to_string(),
    ))?;
    info!("subscribed to newHeads at {}", WS_URL);

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
                render(lcd, &header)?;
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

/// Sends a JSON-RPC request over HTTP and returns the `result` field.
///
/// # Errors
/// Returns an error if the HTTP request fails, the response is not valid JSON,
/// or the response contains a JSON-RPC `error` object.
fn rpc_http(method: &str, params: Value) -> Result<Value, Box<dyn std::error::Error>> {
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let resp: Value = ureq::post(HTTP_URL)
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
/// Only acts when the stream is a plain (non-TLS) connection; TLS streams carry
/// their own timeout machinery and are left untouched.
fn set_read_timeout(
    ws: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    dur: std::time::Duration,
) -> io::Result<()> {
    if let MaybeTlsStream::Plain(s) = ws.get_ref() {
        s.set_read_timeout(Some(dur))?;
    }
    Ok(())
}

/// Sends a GET request to the Beacon Node REST API and returns the parsed JSON.
///
/// # Errors
/// Returns an error if the HTTP request fails or the response is not valid JSON.
fn cl_get(path: &str) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(ureq::get(&format!("{CL_HTTP_URL}{path}"))
        .call()?
        .body_mut()
        .read_json()?)
}

/// Sends a JSON-RPC 1.0 request to the Bitcoin Core node and returns the `result` field.
///
/// Credentials are passed as HTTP Basic Auth; the `Authorization` header is
/// constructed from `user` and `pass` at call time.
///
/// # Errors
/// Returns an error if the HTTP request fails, the response is not valid JSON,
/// or the response contains a non-null `error` object.
fn rpc_btc(
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
    let resp: Value = ureq::post(BTC_HTTP_URL)
        .header("Authorization", &auth)
        .send_json(body)?
        .body_mut()
        .read_json()?;
    if !resp["error"].is_null() {
        return Err(format!("rpc {method}: {}", resp["error"]).into());
    }
    Ok(resp["result"].clone())
}

/// Fetches current fee rate and peer count from the Bitcoin node, formats all
/// four lines, and writes them to the display.
///
/// # Errors
/// Returns an error if any Bitcoin Core RPC request fails, a required response
/// field is absent, [`format_lines_bitcoin`] returns an error, or the LCD write fails.
fn render_bitcoin<B: DataBus>(
    lcd: &mut I2cLcd<B>,
    height: u64,
    hash: &str,
    timestamp: u64,
    user: &str,
    pass: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Estimate next-block fee rate in sat/vByte from Bitcoin Core's fee estimator.
    // `feerate` is in BTC/kB; multiply by 1e5 to convert to sat/vByte.
    let fee_result = rpc_btc("estimatesmartfee", json!([1, "ECONOMICAL"]), user, pass)?;
    let fee_sat_vb = if let Some(feerate) = fee_result["feerate"].as_f64() {
        feerate * 1e5
    } else {
        // Insufficient chain data for the estimator (common on fresh nodes).
        // Fall back to the mempool minimum fee; default to 1.0 if also unavailable.
        rpc_btc("getmempoolinfo", json!([]), user, pass)?["mempoolminfee"]
            .as_f64()
            .map(|f| f * 1e5)
            .unwrap_or(1.0)
    };
    let peers = rpc_btc("getnetworkinfo", json!([]), user, pass)?["connections"]
        .as_u64()
        .ok_or("missing connections")?;
    let lines = format_lines_bitcoin(height, hash, timestamp, fee_sat_vb, peers)?;
    write_display(lcd, &lines)
}

/// Runs the Bitcoin display loop.
///
/// Performs an initial render from the current chain tip, then long-polls
/// [`waitfornewblock`](https://developer.bitcoin.org/reference/rpc/waitfornewblock.html)
/// with a 60-second timeout. The display is updated whenever the block hash changes;
/// timeouts that return the same hash are ignored.
fn run_bitcoin<B: DataBus>(
    lcd: &mut I2cLcd<B>,
    user: &str,
    pass: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    // Fetch the current chain tip for the initial render.
    let info = rpc_btc("getblockchaininfo", json!([]), user, pass)?;
    let best_hash = info["bestblockhash"]
        .as_str()
        .ok_or("missing bestblockhash")?
        .to_string();
    let tip = rpc_btc("getblock", json!([best_hash, 1]), user, pass)?;
    let init_height = tip["height"].as_u64().ok_or("missing height")?;
    let init_time = tip["time"].as_u64().ok_or("missing time")?;
    render_bitcoin(lcd, init_height, &best_hash, init_time, user, pass)?;
    info!("initial render: block #{}", group_underscore(init_height));

    let mut last_hash = best_hash;
    loop {
        // waitfornewblock blocks until a new block arrives or the timeout expires.
        // It always returns the current best, so we compare against last_hash to
        // distinguish a new block from a timeout with no new block.
        let res = rpc_btc("waitfornewblock", json!([60_000]), user, pass)?;
        let new_hash = res["hash"].as_str().ok_or("missing hash")?.to_string();
        if new_hash != last_hash {
            let block = rpc_btc("getblock", json!([new_hash, 1]), user, pass)?;
            let height = block["height"].as_u64().ok_or("missing height")?;
            let timestamp = block["time"].as_u64().ok_or("missing time")?;
            render_bitcoin(lcd, height, &new_hash, timestamp, user, pass)?;
            info!("updated lcd to block #{}", group_underscore(height));
            last_hash = new_hash;
        }
    }
}

/// Fetches attestation count and peer count from the Beacon Node, formats all
/// four lines, and writes them to the display.
///
/// # Errors
/// Returns an error if either Beacon Node HTTP request fails, the peer-count
/// field is absent or cannot be parsed as [`u64`], [`format_lines_consensus`]
/// returns an error, or the LCD write fails.
fn render_consensus<B: DataBus>(
    lcd: &mut I2cLcd<B>,
    slot: u64,
    block_root: &str,
    genesis_time: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    // Count attestations from the block body.
    let block = cl_get(&format!("/eth/v2/beacon/blocks/{block_root}"))?;
    let att_count = block["data"]["message"]["body"]["attestations"]
        .as_array()
        .map_or(0, |a| a.len());
    // Peer count is a decimal string (unlike EL hex quantities).
    let peers: u64 = cl_get("/eth/v1/node/peer_count")?["data"]["connected"]
        .as_str()
        .ok_or("missing peer count")?
        .parse()?;
    let lines = format_lines_consensus(slot, block_root, genesis_time, att_count, peers)?;
    write_display(lcd, &lines)
}

/// Runs the consensus-layer display loop.
///
/// Fetches the chain genesis time and current head once on startup, then
/// subscribes to `head` events over the Beacon Node SSE endpoint. Each `data:`
/// line triggers an immediate render — no throttle is needed because Ethereum
/// slots are 12 seconds apart and burst events do not occur on the consensus
/// layer the way reorgs can on the execution layer.
///
/// Returns an error on stream end or I/O failure. There is no internal reconnect
/// loop; run the daemon under systemd with `Restart=on-failure` for automatic
/// recovery.
fn run_consensus<B: DataBus>(lcd: &mut I2cLcd<B>) -> Result<(), Box<dyn std::error::Error>> {
    // Genesis time is needed to convert slot numbers to wall-clock timestamps.
    let genesis_time: u64 = cl_get("/eth/v1/config/genesis")?["data"]["genesis_time"]
        .as_str()
        .ok_or("missing genesis_time")?
        .parse()?;
    info!("genesis_time={genesis_time}");

    // Initial render from the current chain head.
    let head = cl_get("/eth/v1/beacon/headers/head")?;
    let init_slot: u64 = head["data"]["header"]["message"]["slot"]
        .as_str()
        .ok_or("missing slot")?
        .parse()?;
    let init_root = head["data"]["root"]
        .as_str()
        .ok_or("missing root")?
        .to_string();
    render_consensus(lcd, init_slot, &init_root, genesis_time)?;
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
        .get(&format!("{CL_HTTP_URL}/eth/v1/events?topics=head"))
        .call()?;
    info!("subscribed to head SSE at {CL_HTTP_URL}");

    let reader = BufReader::new(resp.into_body().into_reader());
    for line_result in reader.lines() {
        let line = line_result?;
        if let Some((slot, block_root)) = parse_sse_head(&line)? {
            info!("received slot #{} from SSE", group_underscore(slot));
            render_consensus(lcd, slot, &block_root, genesis_time)?;
            info!("updated lcd to slot #{}", group_underscore(slot));
        }
    }

    Err("SSE stream ended".into())
}

/// Entry point: reads config, initialises the LCD, then delegates to the
/// appropriate layer loop.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg: toml::Value = toml::from_str(&fs::read_to_string(CONFIG_PATH)?)?;
    let layer = cfg
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or("config.toml: missing or non-string 'type' field")?;
    info!("loaded {} (layer={})", CONFIG_PATH, layer);

    let i2c = I2cAdapter(I2cdev::new("/dev/i2c-0")?);
    let mut delay = DelayAdapter;
    let mut lcd_inner = HD44780::new_i2c(i2c, 0x27, &mut delay).map_err(|_| "lcd init")?;
    lcd_inner.reset(&mut delay).map_err(|_| "reset")?;
    lcd_inner.clear(&mut delay).map_err(|_| "clear")?;
    lcd_inner
        .set_display_mode(
            DisplayMode {
                display: Display::On,
                cursor_visibility: Cursor::Invisible,
                cursor_blink: CursorBlink::Off,
            },
            &mut delay,
        )
        .map_err(|_| "mode")?;
    info!("LCD initialized on /dev/i2c-0 @ 0x27");

    let mut lcd = I2cLcd {
        lcd: lcd_inner,
        delay,
    };

    match layer {
        "execution" => run_execution(&mut lcd),
        "consensus" => run_consensus(&mut lcd),
        "bitcoin" => {
            let user = cfg
                .get("rpcuser")
                .and_then(|v| v.as_str())
                .ok_or("config.toml: missing 'rpcuser'")?;
            let pass = cfg
                .get("rpcpassword")
                .and_then(|v| v.as_str())
                .ok_or("config.toml: missing 'rpcpassword'")?;
            run_bitcoin(&mut lcd, user, pass)
        }
        other => Err(format!("config.toml: unsupported type {other:?}").into()),
    }
}
