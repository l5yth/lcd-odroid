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

//! Bitcoin display formatting.
//!
//! All functions operate on plain Rust values; no hardware or network I/O.

use super::{format_hex_line, format_label_number, format_status_line, format_timestamp_line};

// ── Bitcoin display ──────────────────────────────────────────────────────────

/// Number of trailing hash characters shown on LCD row 2.
///
/// The row is 20 columns; the `"0x.."` prefix takes four of them, leaving 16.
/// The tail is shown rather than the head because a Bitcoin block hash opens
/// with a proof-of-work zero run — 19 nibbles or more at present — that is
/// wider than the row and therefore identical on every block.
const HASH_TAIL_CHARS: usize = 16;

/// Formats the four LCD lines from a Bitcoin block.
///
/// | Row | Content |
/// |-----|---------|
/// | 1   | `"Block     #1_623_137"` – block height right-aligned to 20 chars |
/// | 2   | `"0x.."` followed by the last 16 characters of the block hash |
/// | 3   | Block timestamp formatted as `YYYY-MM-DD HH:MM:SSZ` (exactly 20 chars) |
/// | 4   | Fee rate in sat/vByte and peer count, padded to 20 chars |
///
/// # Errors
/// Returns an error if [`format_timestamp_line`] rejects `timestamp`.
pub fn format_lines_bitcoin(
    height: u64,
    hash: &str,
    timestamp: u64,
    fee_sat_vb: f64,
    peers: u64,
) -> Result<[String; 4], Box<dyn std::error::Error>> {
    let line1 = format_label_number("Block", height);
    // Bitcoin hashes are bare hex and, at current PoW difficulty, open with a
    // run of 19 or more zero nibbles — wider than the row itself, so a leading
    // window renders the same constant string for every block. Take the
    // low-order tail instead, which is the part that identifies the block, and
    // mark the elided middle with "..". `chars()` rather than byte slicing so a
    // malformed non-ASCII hash cannot panic on a char boundary.
    let skip = hash.chars().count().saturating_sub(HASH_TAIL_CHARS);
    let tail: String = hash.chars().skip(skip).collect();
    // "0x" + ".." + 16 = exactly 20, so format_hex_line neither truncates nor
    // pads a well-formed hash; it only pads a short, malformed one.
    let line2 = format_hex_line(&format!("0x..{tail}"));
    let line3 = format_timestamp_line(timestamp)?;
    let line4 = format_fee_peers(fee_sat_vb, peers);

    Ok([line1, line2, line3, line4])
}

/// Formats the fee-rate / peer-count status line to exactly 20 characters.
///
/// The left column (11 chars) shows the estimated next-block fee rate in sat/vByte:
/// - below 100 sat/vB: one decimal place (`"12.3 sat/vB"` = 11 chars)
/// - 100–9 999 sat/vB: no decimal places (`"9999 sat/vB"` = 11 chars)
/// - ≥ 10 000 sat/vB: displayed in ksat/vB (`"10k sat/vB"` = 10 chars); values
///   above ~999 000 sat/vB (≈10 BTC/vByte) would overflow but are not physically
///   achievable on mainnet.
///
/// The right column (9 chars) shows the connected peer count, right-aligned.
pub fn format_fee_peers(fee_sat_vb: f64, peers: u64) -> String {
    // Three tiers to keep the left column ≤ 11 characters in all realistic scenarios.
    let fee_str = if fee_sat_vb >= 10_000.0 {
        format!("{:.0}k sat/vB", fee_sat_vb / 1_000.0)
    } else if fee_sat_vb >= 100.0 {
        format!("{:.0} sat/vB", fee_sat_vb)
    } else {
        format!("{:.1} sat/vB", fee_sat_vb)
    };
    let peer_str = format!("{} peers", peers);
    format_status_line(&fee_str, &peer_str)
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── format_fee_peers ─────────────────────────────────────────────────────

    #[test]
    fn fee_peers_below_100_sat_vb() {
        let s = format_fee_peers(12.3, 42);
        assert_eq!(s.len(), 20);
        assert!(s.starts_with("12.3 sat/vB"));
        assert!(s.ends_with("42 peers"));
    }

    #[test]
    fn fee_peers_exactly_100_sat_vb() {
        let s = format_fee_peers(100.0, 0);
        assert_eq!(s.len(), 20);
        assert!(s.starts_with("100 sat/vB"));
    }

    #[test]
    fn fee_peers_above_100_sat_vb() {
        let s = format_fee_peers(500.0, 3);
        assert_eq!(s.len(), 20);
        assert!(s.starts_with("500 sat/vB"));
        assert!(s.ends_with("3 peers"));
    }

    #[test]
    fn fee_peers_low_fee() {
        // Minimum relay fee (~1 sat/vB)
        let s = format_fee_peers(1.0, 8);
        assert_eq!(s.len(), 20);
        assert!(s.starts_with("1.0 sat/vB"));
    }

    #[test]
    fn fee_peers_high_fee_kilo_tier() {
        // ≥ 10_000 sat/vB switches to k-unit to stay within 11 chars
        let s = format_fee_peers(10_000.0, 5);
        assert_eq!(s.len(), 20);
        assert!(s.starts_with("10k sat/vB"));
        assert!(s.ends_with("5 peers"));
    }

    #[test]
    fn fee_peers_high_fee_large_kilo() {
        // 100_000 sat/vB → "100k sat/vB" = 11 chars — still fits
        let s = format_fee_peers(100_000.0, 1);
        assert_eq!(s.len(), 20);
        assert!(s.starts_with("100k sat/vB"));
    }

    // ── format_lines_bitcoin ─────────────────────────────────────────────────

    fn sample_block() -> (u64, &'static str, u64) {
        // Real mainnet block 800 000: a full 64-character hash with the
        // 19-nibble leading-zero run that current PoW difficulty produces.
        (
            800_000,                                                            // height
            "00000000000000000002a7c4c1e48d76c5a37902165a270156b7a8d72728a054", // hash
            1_690_168_629,                                                      // timestamp
        )
    }

    #[test]
    fn format_lines_bitcoin_happy_path() {
        let (height, hash, ts) = sample_block();
        let lines = format_lines_bitcoin(height, hash, ts, 12.3, 42).unwrap();
        assert_eq!(lines[0], "Block       #800_000");
        // "0x.." + the last 16 chars of the hash; the elided middle is the
        // leading-zero run, which is identical on every block at this difficulty.
        assert_eq!(lines[1], "0x..56b7a8d72728a054");
        assert_eq!(lines[2].len(), 20);
        assert!(lines[2].ends_with('Z'));
        assert_eq!(lines[3].len(), 20);
        for line in &lines {
            assert_eq!(line.len(), 20, "line not 20 chars: {line:?}");
        }
    }

    #[test]
    fn format_lines_bitcoin_row2_identifies_the_block() {
        // REGRESSION: row 2 used to render "0x" + the *first* 18 hash chars.
        // Mainnet hashes carry a 19-nibble leading-zero run at current PoW
        // difficulty, so that window fell entirely inside the zeros and every
        // block rendered the identical constant "0x000000000000000000".
        // Row 2 must vary with the block. Three real mainnet hashes:
        let blocks = [
            "00000000000000000002a7c4c1e48d76c5a37902165a270156b7a8d72728a054",
            "0000000000000000000320283a032748cef8227873ff4872689bf23f1cda83a5",
            "00000000000000000001f9ee4f69cbc75ce61db5178afdfbeb3d1da4ca9eb01c",
        ];
        let mut rows: Vec<String> = blocks
            .iter()
            .map(|hash| {
                let lines = format_lines_bitcoin(800_000, hash, 1_690_168_629, 12.3, 42).unwrap();
                let row2 = lines[1].clone();
                assert_eq!(row2.len(), 20, "row 2 not 20 chars: {row2:?}");
                row2
            })
            .collect();
        assert_eq!(rows[0], "0x..56b7a8d72728a054");
        assert_eq!(rows[1], "0x..689bf23f1cda83a5");
        assert_eq!(rows[2], "0x..eb3d1da4ca9eb01c");
        rows.sort();
        rows.dedup();
        assert_eq!(rows.len(), 3, "row 2 does not distinguish blocks: {rows:?}");
    }

    #[test]
    fn format_lines_bitcoin_row2_short_hash_padded() {
        // A truncated or malformed hash has fewer than 16 chars to elide to.
        // The tail is whatever exists and format_hex_line pads to 20, so the
        // "every line is exactly 20 characters" invariant still holds.
        let lines = format_lines_bitcoin(1, "abc", 1_690_168_629, 1.0, 0).unwrap();
        assert_eq!(lines[1], "0x..abc             ");
        assert_eq!(lines[1].len(), 20);
    }

    #[test]
    fn format_lines_bitcoin_out_of_range_timestamp() {
        // Timestamp far beyond NaiveDateTime::MAX (~year 262_143) → chrono returns None.
        // u64::MAX / 2 == i64::MAX, which converts cleanly but exceeds DateTime range.
        let (height, hash, _) = sample_block();
        assert!(format_lines_bitcoin(height, hash, u64::MAX / 2, 1.0, 0).is_err());
    }

    #[test]
    fn format_lines_bitcoin_timestamp_i64_overflow() {
        // timestamp fits u64 but not i64 → i64::try_from fails
        let (height, hash, _) = sample_block();
        assert!(format_lines_bitcoin(height, hash, u64::MAX, 1.0, 0).is_err());
    }
}
