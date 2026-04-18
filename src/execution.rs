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

//! Execution-layer display formatting and WebSocket message parsing.
//!
//! All functions operate on plain Rust values; no hardware or network I/O.

use chrono::DateTime;
use serde_json::Value;

use super::{group_underscore, parse_hex_u64};

// ── Execution-layer display ──────────────────────────────────────────────────

/// Formats the four LCD lines from a block `header` plus current network state.
///
/// | Row | Content |
/// |-----|---------|
/// | 1   | `"Block   #21_000_000"` – block number right-aligned to 20 chars |
/// | 2   | First 20 characters of the block hash (includes `0x` prefix) |
/// | 3   | Block timestamp formatted as `YYYY-MM-DD HH:MM:SSZ` (exactly 20 chars) |
/// | 4   | Gas price in gwei and peer count, padded to 20 chars |
///
/// # Errors
/// Returns an error if `header` is missing the `number`, `hash`, or `timestamp`
/// field, if any field cannot be hex-decoded, or if the timestamp is outside
/// the valid [`DateTime`] range.
pub fn format_lines(
    header: &Value,
    gas_wei: u64,
    peers: u64,
) -> Result<[String; 4], Box<dyn std::error::Error>> {
    let number = parse_hex_u64(header["number"].as_str().ok_or("number")?)?;
    let hash = header["hash"].as_str().ok_or("hash")?;
    let timestamp = parse_hex_u64(header["timestamp"].as_str().ok_or("timestamp")?)?;

    // Label left-justified in 5 chars; number right-justified in the remaining 15.
    let line1 = format!(
        "{:<5}{:>15}",
        "Block",
        format!("#{}", group_underscore(number))
    );
    // First 20 hex characters of the block hash (the "0x" prefix is included in the count).
    // Padded with spaces if the hash is somehow shorter than 20 chars so the LCD line
    // length invariant is maintained and stale characters are not left on the display.
    let line2 = format!("{:<20}", hash.chars().take(20).collect::<String>());
    // UTC wall-clock time — "%Y-%m-%d %H:%M:%SZ" is always exactly 20 characters.
    let line3 = DateTime::from_timestamp(timestamp as i64, 0)
        .ok_or("bad timestamp")?
        .format("%Y-%m-%d %H:%M:%SZ")
        .to_string();
    let line4 = format_gas_peers(gas_wei, peers);

    Ok([line1, line2, line3, line4])
}

/// Formats the gas-price / peer-count status line to exactly 20 characters.
///
/// The left column (11 chars) shows the current gas price in gwei:
/// two decimal places when below 100 gwei, no decimal places at or above.
/// The right column (9 chars) shows the connected peer count, right-aligned.
pub fn format_gas_peers(gas_wei: u64, peers: u64) -> String {
    let gwei = gas_wei as f64 / 1e9;
    // Omit decimals at high values so the column stays within 11 characters.
    let gas_str = if gwei >= 100.0 {
        format!("{:.0} gwei", gwei)
    } else {
        format!("{:.2} gwei", gwei)
    };
    let peer_str = format!("{} peers", peers);
    format!("{:<11}{:>9}", gas_str, peer_str)
}

// ── WebSocket message parsing ────────────────────────────────────────────────

/// Parses a JSON WebSocket message and returns the block header embedded in an
/// `eth_subscription` notification, if present.
///
/// Returns `Ok(None)` for any message that is not an `eth_subscription`
/// notification — most importantly the subscription-confirmation reply that
/// geth sends immediately after the `eth_subscribe` call.
///
/// # Errors
/// Returns `Err` if `text` is not valid JSON.
pub fn extract_new_head(text: &str) -> Result<Option<Value>, serde_json::Error> {
    let v: Value = serde_json::from_str(text)?;
    if v.get("method").and_then(|m| m.as_str()) == Some("eth_subscription") {
        // `params.result` holds the new block header; absent on malformed messages.
        Ok(v["params"].get("result").cloned())
    } else {
        Ok(None)
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── format_gas_peers ─────────────────────────────────────────────────────

    #[test]
    fn gas_peers_below_100_gwei() {
        let s = format_gas_peers(1_500_000_000, 42);
        assert_eq!(s.len(), 20);
        assert!(s.starts_with("1.50 gwei"));
        assert!(s.ends_with("42 peers"));
    }

    #[test]
    fn gas_peers_exactly_100_gwei() {
        let s = format_gas_peers(100_000_000_000, 0);
        assert_eq!(s.len(), 20);
        assert!(s.starts_with("100 gwei"));
    }

    #[test]
    fn gas_peers_above_100_gwei() {
        let s = format_gas_peers(200_000_000_000, 5);
        assert_eq!(s.len(), 20);
        assert!(s.starts_with("200 gwei"));
        assert!(s.ends_with("5 peers"));
    }

    // ── format_lines ─────────────────────────────────────────────────────────

    fn sample_header() -> Value {
        json!({
            "number": "0x1400000",    // 20_971_520
            "hash":   "0xabcdef1234567890abcdef1234567890abcdef12",
            "timestamp": "0x67b9a480" // 2025-02-22 (approx)
        })
    }

    #[test]
    fn format_lines_happy_path() {
        let lines = format_lines(&sample_header(), 1_500_000_000, 42).unwrap();
        assert_eq!(lines[0], "Block    #20_971_520");
        assert_eq!(lines[1], "0xabcdef1234567890ab");
        assert_eq!(lines[2].len(), 20);
        assert!(lines[2].ends_with('Z'));
        assert_eq!(lines[3].len(), 20);
        for line in &lines {
            assert_eq!(line.len(), 20, "line not 20 chars: {line:?}");
        }
    }

    #[test]
    fn format_lines_missing_number() {
        let h = json!({"hash": "0xabc", "timestamp": "0x1"});
        assert!(format_lines(&h, 0, 0).is_err());
    }

    #[test]
    fn format_lines_invalid_number_hex() {
        let h = json!({"number": "xyz", "hash": "0xabc", "timestamp": "0x1"});
        assert!(format_lines(&h, 0, 0).is_err());
    }

    #[test]
    fn format_lines_missing_hash() {
        let h = json!({"number": "0x1", "timestamp": "0x1"});
        assert!(format_lines(&h, 0, 0).is_err());
    }

    #[test]
    fn format_lines_missing_timestamp() {
        let h = json!({"number": "0x1", "hash": "0xabc"});
        assert!(format_lines(&h, 0, 0).is_err());
    }

    #[test]
    fn format_lines_invalid_timestamp_hex() {
        let h = json!({"number": "0x1", "hash": "0xabc", "timestamp": "xyz"});
        assert!(format_lines(&h, 0, 0).is_err());
    }

    #[test]
    fn format_lines_out_of_range_timestamp() {
        // i64::MAX seconds >> NaiveDateTime::MAX (~year 262_143) → chrono returns None
        let h = json!({
            "number": "0x1",
            "hash": "0xabcdef1234567890ab",
            "timestamp": "0x7fffffffffffffff"
        });
        assert!(format_lines(&h, 0, 0).is_err());
    }

    // ── extract_new_head ─────────────────────────────────────────────────────

    #[test]
    fn extract_new_head_returns_header() {
        let msg = r#"{"jsonrpc":"2.0","method":"eth_subscription","params":{"subscription":"0x1","result":{"number":"0x100"}}}"#;
        let head = extract_new_head(msg).unwrap().unwrap();
        assert_eq!(head["number"], "0x100");
    }

    #[test]
    fn extract_new_head_subscription_no_result() {
        // Malformed subscription with no result field → Ok(None)
        let msg =
            r#"{"jsonrpc":"2.0","method":"eth_subscription","params":{"subscription":"0x1"}}"#;
        assert!(extract_new_head(msg).unwrap().is_none());
    }

    #[test]
    fn extract_new_head_non_subscription_returns_none() {
        // Subscription-confirmation reply, not a notification
        let msg = r#"{"jsonrpc":"2.0","id":1,"result":"0xsub123"}"#;
        assert!(extract_new_head(msg).unwrap().is_none());
    }

    #[test]
    fn extract_new_head_invalid_json() {
        assert!(extract_new_head("not valid json {{").is_err());
    }
}
