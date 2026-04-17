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
use std::io;
use std::net::TcpStream;
use std::time::{Duration, Instant};

use chrono::DateTime;
use hd44780_driver::{Cursor, CursorBlink, Display, DisplayMode, HD44780, bus::DataBus};
use linux_embedded_hal::{Delay, I2cdev};
use serde_json::{Value, json};
use tungstenite::{Message, WebSocket, stream::MaybeTlsStream};

const CONFIG_PATH: &str = "config.toml";

const HTTP_URL: &str = "http://127.0.0.1:8545";
const WS_URL: &str = "ws://127.0.0.1:8546";

const LINE1: u8 = 0x00;
const LINE2: u8 = 0x40;
const LINE3: u8 = 0x14;
const LINE4: u8 = 0x54;

const THROTTLE: Duration = Duration::from_secs(1);
const READ_TIMEOUT: Duration = Duration::from_millis(250);

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cfg: toml::Value = toml::from_str(&fs::read_to_string(CONFIG_PATH)?)?;
    let layer = cfg
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or("config.toml: missing or non-string 'type' field")?;

    let i2c = I2cdev::new("/dev/i2c-0")?;
    let mut delay = Delay;

    let mut lcd = HD44780::new_i2c(i2c, 0x27, &mut delay).map_err(|_| "lcd init")?;
    lcd.reset(&mut delay).map_err(|_| "reset")?;
    lcd.clear(&mut delay).map_err(|_| "clear")?;
    lcd.set_display_mode(
        DisplayMode {
            display: Display::On,
            cursor_visibility: Cursor::Invisible,
            cursor_blink: CursorBlink::Off,
        },
        &mut delay,
    )
    .map_err(|_| "mode")?;

    match layer {
        "execution" => run_execution(&mut lcd, &mut delay),
        other => Err(format!("config.toml: unsupported type {other:?}").into()),
    }
}

fn run_execution<B: DataBus>(
    lcd: &mut HD44780<B>,
    delay: &mut Delay,
) -> Result<(), Box<dyn std::error::Error>> {
    let initial = rpc_http("eth_getBlockByNumber", json!(["latest", false]))?;
    render(lcd, delay, &initial)?;
    let mut last_render = Instant::now();

    let (mut ws, _) = tungstenite::connect(WS_URL)?;
    set_read_timeout(&mut ws, READ_TIMEOUT)?;
    ws.send(Message::Text(
        json!({"jsonrpc":"2.0","id":1,"method":"eth_subscribe","params":["newHeads"]})
            .to_string(),
    ))?;

    let mut pending: Option<Value> = None;
    loop {
        match ws.read() {
            Ok(Message::Text(t)) => {
                let v: Value = serde_json::from_str(&t)?;
                if v.get("method").and_then(|m| m.as_str()) == Some("eth_subscription") {
                    if let Some(header) = v["params"].get("result") {
                        pending = Some(header.clone());
                    }
                }
            }
            Ok(Message::Ping(p)) => ws.send(Message::Pong(p))?,
            Ok(Message::Close(_)) => return Err("websocket closed".into()),
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {}
            Err(e) => return Err(e.into()),
        }

        if let Some(header) = pending.take() {
            if last_render.elapsed() >= THROTTLE {
                render(lcd, delay, &header)?;
                last_render = Instant::now();
            } else {
                pending = Some(header);
            }
        }
    }
}

fn render<B: DataBus>(
    lcd: &mut HD44780<B>,
    delay: &mut Delay,
    header: &Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let number = parse_hex_u64(header["number"].as_str().ok_or("number")?)?;
    let hash = header["hash"].as_str().ok_or("hash")?;
    let timestamp = parse_hex_u64(header["timestamp"].as_str().ok_or("timestamp")?)?;

    let gas_wei = parse_hex_u64(rpc_http("eth_gasPrice", json!([]))?.as_str().ok_or("gas")?)?;
    let peers = parse_hex_u64(rpc_http("net_peerCount", json!([]))?.as_str().ok_or("peers")?)?;

    let line1 = format!("{:<5}{:>15}", "Block", format!("#{}", group_underscore(number)));
    let line2: String = hash.chars().take(20).collect();
    let line3 = DateTime::from_timestamp(timestamp as i64, 0)
        .ok_or("bad timestamp")?
        .format("%Y-%m-%d %H:%M:%SZ")
        .to_string();
    let line4 = format_gas_peers(gas_wei, peers);

    write_line(lcd, delay, LINE1, &line1)?;
    write_line(lcd, delay, LINE2, &line2)?;
    write_line(lcd, delay, LINE3, &line3)?;
    write_line(lcd, delay, LINE4, &line4)?;
    Ok(())
}

fn write_line<B: DataBus>(
    lcd: &mut HD44780<B>,
    delay: &mut Delay,
    pos: u8,
    text: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    lcd.set_cursor_pos(pos, delay).map_err(|_| "cursor")?;
    lcd.write_str(text, delay).map_err(|_| "write")?;
    Ok(())
}

fn rpc_http(method: &str, params: Value) -> Result<Value, Box<dyn std::error::Error>> {
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    let resp: Value = ureq::post(HTTP_URL).send_json(body)?.into_json()?;
    if let Some(err) = resp.get("error") {
        return Err(format!("rpc {method}: {err}").into());
    }
    Ok(resp["result"].clone())
}

fn set_read_timeout(
    ws: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    dur: Duration,
) -> io::Result<()> {
    if let MaybeTlsStream::Plain(s) = ws.get_ref() {
        s.set_read_timeout(Some(dur))?;
    }
    Ok(())
}

fn parse_hex_u64(s: &str) -> Result<u64, std::num::ParseIntError> {
    u64::from_str_radix(s.strip_prefix("0x").unwrap_or(s), 16)
}

fn group_underscore(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, &b) in bytes.iter().enumerate() {
        let from_end = bytes.len() - i;
        if i > 0 && from_end % 3 == 0 {
            out.push('_');
        }
        out.push(b as char);
    }
    out
}

fn format_gas_peers(gas_wei: u64, peers: u64) -> String {
    let gwei = gas_wei as f64 / 1e9;
    let gas_str = if gwei >= 100.0 {
        format!("{:.0} gwei", gwei)
    } else {
        format!("{:.2} gwei", gwei)
    };
    let peer_str = format!("{} peers", peers);
    format!("{:<11}{:>9}", gas_str, peer_str)
}
