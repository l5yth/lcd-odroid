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
//!
//! Layer-specific logic lives in submodules that are re-exported here so
//! callers can use a flat `use lcd_odroid::*` import.

pub mod bitcoin;
pub mod consensus;
pub mod execution;

pub use bitcoin::*;
pub use consensus::*;
pub use execution::*;

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

/// Slot duration on Ethereum mainnet.
///
/// This value is hardcoded rather than fetched from `/eth/v1/config/spec`
/// because the daemon targets a single known deployment. Some testnets and
/// devnets use a different slot interval.
pub const SECONDS_PER_SLOT: u64 = 12;

/// Read-timeout applied to the WebSocket connection.
///
/// Keeping this short lets the event loop check the `pending` slot and drive
/// the throttle without a dedicated timer thread.
pub const READ_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(250);

/// Read-timeout applied to the Beacon Node SSE connection.
///
/// If no bytes arrive within this window the SSE stream is considered dead and
/// `run_consensus` returns an error. 60 s matches the Bitcoin `waitfornewblock`
/// poll budget and comfortably survives a run of missed slots before triggering
/// a reconnect via the systemd `Restart=on-failure` supervisor.
pub const SSE_READ_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

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

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(block_number(&serde_json::json!({"number": "0x100"})), 256);
    }

    #[test]
    fn block_number_missing_field() {
        assert_eq!(block_number(&serde_json::json!({})), 0);
    }

    #[test]
    fn block_number_invalid_hex() {
        assert_eq!(block_number(&serde_json::json!({"number": "not_hex"})), 0);
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
}
