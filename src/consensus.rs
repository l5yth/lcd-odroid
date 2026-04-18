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

//! Consensus-layer display formatting and Beacon Node SSE message parsing.
//!
//! All functions operate on plain Rust values; no hardware or network I/O.

use chrono::DateTime;
use serde_json::Value;

use super::{SECONDS_PER_SLOT, group_underscore};

// ── Consensus-layer display ──────────────────────────────────────────────────

/// Formats the four LCD lines from consensus-layer head data.
///
/// | Row | Content |
/// |-----|---------|
/// | 1   | `"Slot     #21_833_152"` – label left, slot number right-aligned in 15 chars |
/// | 2   | First 20 characters of the block root (includes `0x` prefix), space-padded |
/// | 3   | Slot timestamp (`genesis_time + slot × 12 s`) as `YYYY-MM-DD HH:MM:SSZ` |
/// | 4   | Attestation count and peer count, padded to 20 chars |
///
/// # Errors
/// Returns an error if the slot-to-timestamp arithmetic overflows, if the
/// timestamp value cannot be represented as [`i64`], or if the timestamp is
/// outside the valid [`DateTime`] range.
pub fn format_lines_consensus(
    slot: u64,
    block_root: &str,
    genesis_time: u64,
    att_count: usize,
    peers: u64,
) -> Result<[String; 4], Box<dyn std::error::Error>> {
    // Slot timestamp derived from the chain genesis rather than a field in the
    // event, since the SSE head payload does not include a timestamp.
    // Use checked arithmetic so overflow is an explicit error rather than silent wrap.
    let slot_secs = slot.checked_mul(SECONDS_PER_SLOT).ok_or("slot overflow")?;
    let timestamp = genesis_time
        .checked_add(slot_secs)
        .ok_or("timestamp overflow")?;
    let ts_i64 = i64::try_from(timestamp).map_err(|_| "timestamp out of i64 range")?;

    // Label left-justified in 5 chars; slot number right-justified in 15.
    let line1 = format!(
        "{:<5}{:>15}",
        "Slot",
        format!("#{}", group_underscore(slot))
    );
    // First 20 hex characters of the block root (the "0x" prefix is included).
    // Space-padded to maintain the 20-char invariant if the root is unusually short.
    let line2 = format!("{:<20}", block_root.chars().take(20).collect::<String>());
    // UTC wall-clock time — always exactly 20 characters.
    let line3 = DateTime::from_timestamp(ts_i64, 0)
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

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
        // slot=700B, slot*12=8.4T and genesis+slot*12≈8.4T both fit in u64 and i64,
        // but the resulting timestamp (~year 266_000) exceeds NaiveDateTime::MAX
        // (~year 262_143) so DateTime::from_timestamp returns None → "bad timestamp".
        let (_, root, genesis) = sample_consensus_head();
        assert!(format_lines_consensus(700_000_000_000, root, genesis, 0, 0).is_err());
    }

    #[test]
    fn format_lines_consensus_slot_mul_overflow() {
        // slot * SECONDS_PER_SLOT overflows u64 → checked_mul returns None
        assert!(format_lines_consensus(u64::MAX / 12 + 1, "0xabc", 0, 0, 0).is_err());
    }

    #[test]
    fn format_lines_consensus_timestamp_add_overflow() {
        // genesis_time + slot_secs overflows u64 → checked_add returns None
        assert!(format_lines_consensus(1, "0xabc", u64::MAX - 11, 0, 0).is_err());
    }

    #[test]
    fn format_lines_consensus_timestamp_i64_overflow() {
        // timestamp fits u64 but not i64 → i64::try_from fails
        assert!(format_lines_consensus(0, "0xabc", u64::MAX, 0, 0).is_err());
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
