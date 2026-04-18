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

//! Core display-logic and data-formatting for the lcd-odroid daemon.
//!
//! Intentionally hardware-free: all functions operate on plain Rust values and
//! the [`LcdDisplay`] trait so that the full rendering pipeline can be unit-
//! tested without a physical I²C bus or a running Ethereum node.

use chrono::DateTime;
use serde_json::Value;

// ── HD44780 row addresses ────────────────────────────────────────────────────

/// DDRAM start address for row 0 of the HD44780 20×4 panel.
pub const LINE1: u8 = 0x00;
/// DDRAM start address for row 1 (second row).
pub const LINE2: u8 = 0x40;
/// DDRAM start address for row 2 (third row).
pub const LINE3: u8 = 0x14;
/// DDRAM start address for row 3 (fourth row).
pub const LINE4: u8 = 0x54;

// ── Timing ───────────────────────────────────────────────────────────────────

/// Maximum time between successive LCD updates.
///
/// Bursts of `newHeads` events (e.g. during a reorg) collapse to the most
/// recent header rather than flooding the display with intermediate blocks.
pub const THROTTLE: std::time::Duration = std::time::Duration::from_secs(1);

/// Slot duration on all mainnet-derived Ethereum networks.
pub const SECONDS_PER_SLOT: u64 = 12;

/// Read-timeout applied to the WebSocket connection.
///
/// Keeping this short lets the event loop check the `pending` slot and drive
/// the throttle without a dedicated timer thread.
pub const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(250);

// ── LCD trait ────────────────────────────────────────────────────────────────

/// Hardware-agnostic interface for a 20×4 character-cell LCD panel.
///
/// The real implementation in `main.rs` wraps [`hd44780_driver::HD44780`]; a
/// lightweight in-memory stub is used in unit tests.
pub trait LcdDisplay {
    /// Write `text` to the display starting at DDRAM address `pos`.
    ///
    /// Callers must ensure `text` is exactly 20 bytes so the LCD controller
    /// does not carry stale characters from the previous frame into the next.
    fn write_line(&mut self, pos: u8, text: &str) -> Result<(), Box<dyn std::error::Error>>;
}

// ── Display helpers ──────────────────────────────────────────────────────────

/// Writes four pre-formatted lines to `lcd` at the four HD44780 row addresses.
///
/// Iterates `lines` paired with [`LINE1`]–[`LINE4`] and stops on the first
/// write error.
///
/// # Errors
/// Propagates the first [`LcdDisplay::write_line`] error encountered.
pub fn write_display(
    lcd: &mut dyn LcdDisplay,
    lines: &[String; 4],
) -> Result<(), Box<dyn std::error::Error>> {
    for (pos, line) in [LINE1, LINE2, LINE3, LINE4].iter().zip(lines.iter()) {
        lcd.write_line(*pos, line)?;
    }
    Ok(())
}

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
    let line2: String = hash.chars().take(20).collect();
    // UTC wall-clock time — "%Y-%m-%d %H:%M:%SZ" is always exactly 20 characters.
    let line3 = DateTime::from_timestamp(timestamp as i64, 0)
        .ok_or("bad timestamp")?
        .format("%Y-%m-%d %H:%M:%SZ")
        .to_string();
    let line4 = format_gas_peers(gas_wei, peers);

    Ok([line1, line2, line3, line4])
}

// ── Consensus-layer display ──────────────────────────────────────────────────

/// Formats the four LCD lines from consensus-layer head data.
///
/// | Row | Content |
/// |-----|---------|
/// | 1   | `"Slot     #21_833_152"` – slot number right-aligned to 20 chars |
/// | 2   | First 20 characters of the block root (includes `0x` prefix) |
/// | 3   | Slot timestamp (`genesis_time + slot × 12 s`) as `YYYY-MM-DD HH:MM:SSZ` |
/// | 4   | Attestation count and peer count, padded to 20 chars |
///
/// # Errors
/// Returns an error if the computed timestamp is outside the valid [`DateTime`]
/// range (requires a slot number far beyond any realistic future value).
pub fn format_lines_consensus(
    slot: u64,
    block_root: &str,
    genesis_time: u64,
    att_count: usize,
    peers: u64,
) -> Result<[String; 4], Box<dyn std::error::Error>> {
    // Slot timestamp derived from the chain genesis rather than a field in the
    // event, since the SSE head payload does not include a timestamp.
    let timestamp = genesis_time + slot * SECONDS_PER_SLOT;

    // Label left-justified in 5 chars; slot number right-justified in 15.
    let line1 = format!(
        "{:<5}{:>15}",
        "Slot",
        format!("#{}", group_underscore(slot))
    );
    // First 20 hex characters of the block root (the "0x" prefix is included).
    let line2: String = block_root.chars().take(20).collect();
    // UTC wall-clock time — always exactly 20 characters.
    let line3 = DateTime::from_timestamp(timestamp as i64, 0)
        .ok_or("bad timestamp")?
        .format("%Y-%m-%d %H:%M:%SZ")
        .to_string();
    let line4 = format_atts_peers(att_count, peers);

    Ok([line1, line2, line3, line4])
}

/// Formats the attestation-count / peer-count status line to exactly 20 characters.
///
/// The left column (11 chars) shows the number of attestations included in the
/// block. The right column (9 chars) shows the connected peer count, right-aligned.
pub fn format_atts_peers(att_count: usize, peers: u64) -> String {
    let att_str = format!("{} atts", att_count);
    let peer_str = format!("{} peers", peers);
    format!("{:<11}{:>9}", att_str, peer_str)
}

// ── SSE message parsing ──────────────────────────────────────────────────────

/// Parses one line from a Beacon Node SSE stream and returns `(slot, block_root)`
/// if the line is a `data:` payload for a `head` event.
///
/// Returns `Ok(None)` for non-`data:` lines (e.g. `event:` or blank separator
/// lines), so callers can feed every line from a [`std::io::BufRead`] iterator directly.
///
/// # Errors
/// Returns an error if the `data:` payload is not valid JSON, or if the
/// expected `slot` or `block` fields are absent or malformed.
pub fn parse_sse_head(line: &str) -> Result<Option<(u64, String)>, Box<dyn std::error::Error>> {
    let json_str = match line.strip_prefix("data: ") {
        Some(s) => s,
        None => return Ok(None),
    };
    let v: Value = serde_json::from_str(json_str)?;
    let slot = v["slot"].as_str().ok_or("missing slot")?.parse::<u64>()?;
    let block_root = v["block"].as_str().ok_or("missing block root")?.to_string();
    Ok(Some((slot, block_root)))
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

// ── Parsing helpers ──────────────────────────────────────────────────────────

/// Parses a `0x`-prefixed (or bare) hexadecimal string into a [`u64`].
///
/// Ethereum JSON-RPC encodes all quantities as lower-case hex strings with a
/// `0x` prefix; this function strips that prefix before forwarding to
/// [`u64::from_str_radix`].
///
/// # Errors
/// Returns [`std::num::ParseIntError`] if the string (after stripping `0x`) is
/// not valid base-16.
pub fn parse_hex_u64(s: &str) -> Result<u64, std::num::ParseIntError> {
    u64::from_str_radix(s.strip_prefix("0x").unwrap_or(s), 16)
}

/// Extracts and parses the `number` field from a JSON block header.
///
/// Returns `0` if the field is absent or cannot be decoded, so a partial or
/// synthetic header does not crash the daemon.
pub fn block_number(header: &Value) -> u64 {
    header["number"]
        .as_str()
        .and_then(|s| parse_hex_u64(s).ok())
        .unwrap_or(0)
}

// ── Formatting helpers ───────────────────────────────────────────────────────

/// Formats `n` with an underscore separator every three digits from the right.
///
/// Used to display large block numbers in a human-readable form, e.g.
/// `21_456_789`. Numbers with fewer than four digits are returned unchanged.
pub fn group_underscore(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, &b) in bytes.iter().enumerate() {
        // Insert a separator before every group of three digits counting from
        // the right, but skip position 0 to avoid a leading underscore.
        let from_end = bytes.len() - i;
        if i > 0 && from_end.is_multiple_of(3) {
            out.push('_');
        }
        out.push(b as char);
    }
    out
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

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── LcdDisplay mocks ─────────────────────────────────────────────────────

    /// Captures every `write_line` call for inspection.
    struct MockLcd {
        writes: Vec<(u8, String)>,
    }

    impl MockLcd {
        fn new() -> Self {
            MockLcd { writes: Vec::new() }
        }
    }

    impl LcdDisplay for MockLcd {
        fn write_line(&mut self, pos: u8, text: &str) -> Result<(), Box<dyn std::error::Error>> {
            self.writes.push((pos, text.to_string()));
            Ok(())
        }
    }

    /// Always returns an error from `write_line`.
    struct FailLcd;

    impl LcdDisplay for FailLcd {
        fn write_line(&mut self, _pos: u8, _text: &str) -> Result<(), Box<dyn std::error::Error>> {
            Err("lcd write error".into())
        }
    }

    // ── parse_hex_u64 ────────────────────────────────────────────────────────

    #[test]
    fn parse_hex_with_0x_prefix() {
        assert_eq!(parse_hex_u64("0x1a").unwrap(), 26);
    }

    #[test]
    fn parse_hex_without_prefix() {
        assert_eq!(parse_hex_u64("1a").unwrap(), 26);
    }

    #[test]
    fn parse_hex_zero() {
        assert_eq!(parse_hex_u64("0x0").unwrap(), 0);
    }

    #[test]
    fn parse_hex_max_u64() {
        assert_eq!(parse_hex_u64("0xffffffffffffffff").unwrap(), u64::MAX);
    }

    #[test]
    fn parse_hex_invalid() {
        assert!(parse_hex_u64("0xgg").is_err());
    }

    #[test]
    fn parse_hex_empty() {
        assert!(parse_hex_u64("").is_err());
    }

    // ── group_underscore ─────────────────────────────────────────────────────

    #[test]
    fn group_under_zero() {
        assert_eq!(group_underscore(0), "0");
    }

    #[test]
    fn group_under_three_digits() {
        assert_eq!(group_underscore(999), "999");
    }

    #[test]
    fn group_under_four_digits() {
        assert_eq!(group_underscore(1_000), "1_000");
    }

    #[test]
    fn group_under_six_digits() {
        assert_eq!(group_underscore(123_456), "123_456");
    }

    #[test]
    fn group_under_seven_digits() {
        assert_eq!(group_underscore(1_234_567), "1_234_567");
    }

    #[test]
    fn group_under_million() {
        assert_eq!(group_underscore(1_000_000), "1_000_000");
    }

    // ── block_number ─────────────────────────────────────────────────────────

    #[test]
    fn block_number_valid() {
        assert_eq!(block_number(&json!({"number": "0x100"})), 256);
    }

    #[test]
    fn block_number_missing_field() {
        assert_eq!(block_number(&json!({})), 0);
    }

    #[test]
    fn block_number_invalid_hex() {
        assert_eq!(block_number(&json!({"number": "not_hex"})), 0);
    }

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
        // Row 1: "Block" + right-justified block number
        assert_eq!(lines[0], "Block    #20_971_520");
        // Row 2: first 20 chars of the hash string
        assert_eq!(lines[1], "0xabcdef1234567890ab");
        // Row 3: ISO-8601 UTC timestamp
        assert_eq!(lines[2].len(), 20);
        assert!(lines[2].ends_with('Z'));
        // Row 4: gas + peers
        assert_eq!(lines[3].len(), 20);
        // All rows exactly 20 chars
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
        // i64::MAX seconds >> NaiveDateTime::MAX (~year 262143) → chrono returns None
        let h = json!({"number": "0x1", "hash": "0xabcdef1234567890ab", "timestamp": "0x7fffffffffffffff"});
        assert!(format_lines(&h, 0, 0).is_err());
    }

    // ── write_display ────────────────────────────────────────────────────────

    #[test]
    fn write_display_correct_positions_and_order() {
        let lines = [
            "line1".to_string(),
            "line2".to_string(),
            "line3".to_string(),
            "line4".to_string(),
        ];
        let mut lcd = MockLcd::new();
        write_display(&mut lcd, &lines).unwrap();
        assert_eq!(lcd.writes.len(), 4);
        assert_eq!(lcd.writes[0], (LINE1, "line1".to_string()));
        assert_eq!(lcd.writes[1], (LINE2, "line2".to_string()));
        assert_eq!(lcd.writes[2], (LINE3, "line3".to_string()));
        assert_eq!(lcd.writes[3], (LINE4, "line4".to_string()));
    }

    #[test]
    fn write_display_propagates_error() {
        let lines = [
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
        ];
        assert!(write_display(&mut FailLcd, &lines).is_err());
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

    // ── format_atts_peers ────────────────────────────────────────────────────

    #[test]
    fn atts_peers_typical() {
        let s = format_atts_peers(128, 42);
        assert_eq!(s.len(), 20);
        assert!(s.starts_with("128 atts"));
        assert!(s.ends_with("42 peers"));
    }

    #[test]
    fn atts_peers_zero_atts() {
        let s = format_atts_peers(0, 0);
        assert_eq!(s.len(), 20);
        assert!(s.starts_with("0 atts"));
    }

    #[test]
    fn atts_peers_post_electra_count() {
        // Post-Electra max attestations per block is 8 (EIP-7549 consolidation)
        let s = format_atts_peers(8, 100);
        assert_eq!(s.len(), 20);
        assert!(s.starts_with("8 atts"));
    }

    // ── format_lines_consensus ───────────────────────────────────────────────

    fn sample_consensus_head() -> (u64, &'static str, u64) {
        (
            21_833_152,                                       // slot
            "0x0123456789abcdef01234567890abcdef01234567890", // block_root (>20 chars)
            1_606_824_023,                                    // genesis_time (mainnet)
        )
    }

    #[test]
    fn format_lines_consensus_happy_path() {
        let (slot, root, genesis) = sample_consensus_head();
        let lines = format_lines_consensus(slot, root, genesis, 128, 42).unwrap();
        assert_eq!(lines[0], "Slot     #21_833_152");
        assert_eq!(lines[1], "0x0123456789abcdef01");
        assert_eq!(lines[2].len(), 20);
        assert!(lines[2].ends_with('Z'));
        assert_eq!(lines[3], "128 atts    42 peers");
        for line in &lines {
            assert_eq!(line.len(), 20, "line not 20 chars: {line:?}");
        }
    }

    #[test]
    fn format_lines_consensus_out_of_range_timestamp() {
        // genesis + slot * 12 > NaiveDateTime::MAX (~year 262143) → error
        let (_, root, genesis) = sample_consensus_head();
        assert!(format_lines_consensus(700_000_000_000, root, genesis, 0, 0).is_err());
    }

    // ── parse_sse_head ───────────────────────────────────────────────────────

    #[test]
    fn parse_sse_head_returns_slot_and_root() {
        let line = r#"data: {"slot":"21833152","block":"0x0123456789abcdef","state":"0xabc","epoch_transition":false}"#;
        let (slot, root) = parse_sse_head(line).unwrap().unwrap();
        assert_eq!(slot, 21_833_152);
        assert_eq!(root, "0x0123456789abcdef");
    }

    #[test]
    fn parse_sse_head_event_line_returns_none() {
        assert!(parse_sse_head("event: head").unwrap().is_none());
    }

    #[test]
    fn parse_sse_head_blank_line_returns_none() {
        assert!(parse_sse_head("").unwrap().is_none());
    }

    #[test]
    fn parse_sse_head_invalid_json() {
        assert!(parse_sse_head("data: not_json{{").is_err());
    }

    #[test]
    fn parse_sse_head_missing_slot() {
        assert!(parse_sse_head(r#"data: {"block":"0x123"}"#).is_err());
    }

    #[test]
    fn parse_sse_head_missing_block() {
        assert!(parse_sse_head(r#"data: {"slot":"123"}"#).is_err());
    }

    #[test]
    fn parse_sse_head_invalid_slot_value() {
        assert!(parse_sse_head(r#"data: {"slot":"not_a_number","block":"0x123"}"#).is_err());
    }
}
