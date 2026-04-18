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

use chrono::DateTime;

use super::group_underscore;

// ── Bitcoin display ──────────────────────────────────────────────────────────

/// Formats the four LCD lines from a Bitcoin block.
///
/// | Row | Content |
/// |-----|---------|
/// | 1   | `"Block   #1_623_137"` – block height right-aligned to 20 chars |
/// | 2   | `"0x"` followed by the first 18 characters of the block hash, space-padded |
/// | 3   | Block timestamp formatted as `YYYY-MM-DD HH:MM:SSZ` (exactly 20 chars) |
/// | 4   | Fee rate in sat/vByte and peer count, padded to 20 chars |
///
/// # Errors
/// Returns an error if `timestamp` is outside the valid [`DateTime`] range.
pub fn format_lines_bitcoin(
    height: u64,
    hash: &str,
    timestamp: u64,
    fee_sat_vb: f64,
    peers: u64,
) -> Result<[String; 4], Box<dyn std::error::Error>> {
    // Label left-justified in 5 chars; height right-justified in the remaining 15.
    let line1 = format!(
        "{:<5}{:>15}",
        "Block",
        format!("#{}", group_underscore(height))
    );
    // "0x" prefix (2 chars) + first 18 hex chars of the hash = 20 chars total.
    // Space-padded in case the hash is unusually short so the 20-char invariant holds.
    // Note: Bitcoin hashes have many leading zeros that visually reflect PoW difficulty.
    let line2 = format!(
        "{:<20}",
        format!("0x{}", hash.chars().take(18).collect::<String>())
    );
    // Bitcoin block `time` is a Unix u32 (safely fits in i64; ~year 2106 max).
    let line3 = DateTime::from_timestamp(timestamp as i64, 0)
        .ok_or("bad timestamp")?
        .format("%Y-%m-%d %H:%M:%SZ")
        .to_string();
    let line4 = format_fee_peers(fee_sat_vb, peers);

    Ok([line1, line2, line3, line4])
}

/// Formats the fee-rate / peer-count status line to exactly 20 characters.
///
/// The left column (11 chars) shows the estimated next-block fee rate in sat/vByte:
/// one decimal place when below 100 sat/vB, no decimal places at or above.
/// The right column (9 chars) shows the connected peer count, right-aligned.
pub fn format_fee_peers(fee_sat_vb: f64, peers: u64) -> String {
    // Omit decimals at high values so the column stays within 11 characters.
    // Max: "99.9 sat/vB" = 11 chars; "9999 sat/vB" = 11 chars.
    let fee_str = if fee_sat_vb >= 100.0 {
        format!("{:.0} sat/vB", fee_sat_vb)
    } else {
        format!("{:.1} sat/vB", fee_sat_vb)
    };
    let peer_str = format!("{} peers", peers);
    format!("{:<11}{:>9}", fee_str, peer_str)
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

    // ── format_lines_bitcoin ─────────────────────────────────────────────────

    fn sample_block() -> (u64, &'static str, u64) {
        (
            896_969,                                                           // height
            "000000000000000000029e6aa02cd33459c76d32b786eba3eb3e1ea9af4e469", // hash
            1_745_000_000, // timestamp (approx 2025)
        )
    }

    #[test]
    fn format_lines_bitcoin_happy_path() {
        let (height, hash, ts) = sample_block();
        let lines = format_lines_bitcoin(height, hash, ts, 12.3, 42).unwrap();
        assert_eq!(lines[0], "Block       #896_969");
        // "0x" + first 18 chars of the hash (all zeros for this block)
        assert_eq!(lines[1], "0x000000000000000000");
        assert_eq!(lines[2].len(), 20);
        assert!(lines[2].ends_with('Z'));
        assert_eq!(lines[3].len(), 20);
        for line in &lines {
            assert_eq!(line.len(), 20, "line not 20 chars: {line:?}");
        }
    }

    #[test]
    fn format_lines_bitcoin_out_of_range_timestamp() {
        // Timestamp far beyond NaiveDateTime::MAX (~year 262_143) → chrono returns None
        let (height, hash, _) = sample_block();
        assert!(format_lines_bitcoin(height, hash, u64::MAX / 2, 1.0, 0).is_err());
    }
}
