// SPDX-FileCopyrightText: 2026 Shoal contributors (AwokenLazarus)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Added by the Shoal project: regenerates test fixtures for XBT. See SHOAL-CHANGES.md.

//! Fixture tools (tests only, `#[ignore]`, run by hand).
//!
//! Upstream's fixtures carry 80-byte Bitcoin headers mined with SHA-256d. These tools rewrite
//! them for XBT: each bitcoin header becomes a 164-byte v2 header (v2 fields zero) and is
//! BLAKE2b-mined again to the target its share declares, so every proof of work in the fixture
//! is real. Fixtures whose point is an unmeetable target are rewritten without mining.
//!
//! ```text
//! cargo test -p p2poolv2_lib xbt_fixtures -- --ignored --nocapture
//! ```

#[cfg(test)]
mod tests {
    use crate::shares::share_block::ShareHeader;
    use bitcoin::Target;
    use serde_json::Value;

    const VALIDATION_HEADERS: &str = "../p2poolv2_tests/test_data/validation/share_headers.json";

    /// Add the v2 fields to a fixture's 80-byte bitcoin header, in place.
    fn add_v2_fields(header: &mut Value) {
        let bitcoin_header = header
            .get_mut("bitcoin_header")
            .and_then(Value::as_object_mut)
            .expect("fixture header has a bitcoin_header object");
        if !bitcoin_header.contains_key("v2") {
            bitcoin_header.insert(
                "v2".to_string(),
                serde_json::to_value(shoal_header::V2Fields::default()).unwrap(),
            );
        }
    }

    /// Re-mine `header`'s bitcoin header so its BLAKE2b PoW meets the share's declared target.
    /// Returns false when the target is out of reach (left unmined on purpose).
    fn remine(header: &mut ShareHeader) -> bool {
        let target = Target::from_compact(header.bits);
        // A share that declares an impossibly tight target is a negative fixture.
        if target < Target::from_compact(bitcoin::CompactTarget::from_consensus(0x1d00_ffff)) {
            return false;
        }
        header.bitcoin_header.nonce = 0;
        header.bitcoin_header.grind(target, u32::MAX)
    }

    #[test]
    #[ignore = "rewrites fixtures; run by hand"]
    fn regenerate_validation_share_headers() {
        let text = std::fs::read_to_string(VALIDATION_HEADERS).unwrap();
        let mut fixtures: serde_json::Map<String, Value> = serde_json::from_str(&text).unwrap();
        for (name, value) in fixtures.iter_mut() {
            add_v2_fields(value);
            let mut header: ShareHeader = serde_json::from_value(value.clone()).unwrap();
            let mined = remine(&mut header);
            println!(
                "{name}: bits {:#x} mined={mined} nonce={} bitcoin hash {}",
                header.bits.to_consensus(),
                header.bitcoin_header.nonce,
                header.bitcoin_header.block_hash()
            );
            *value = serde_json::to_value(&header).unwrap();
        }
        let out = serde_json::to_string_pretty(&fixtures).unwrap() + "\n";
        std::fs::write(VALIDATION_HEADERS, out).unwrap();
    }

    const SYNC_BLOCKS: &str = "../p2poolv2_tests/test_data/share_sync/share_blocks.json";
    const SYNC_HEADERS: &str = "../p2poolv2_tests/test_data/share_sync/share_headers.json";

    /// Export the first confirmed shares (genesis included) of a real Shoal regtest run's store
    /// as the share_sync fixtures. The chain must have been mined with regtest parameters, as
    /// `e2e/m1-single-miner.sh` does; the fixture tests then run in regtest mode.
    ///
    /// `SHOAL_STORE=<run>/shoal/store.db SHOAL_FIXTURE_SHARES=5 cargo test -p p2poolv2_lib \
    ///   xbt_fixtures::tests::export_share_sync_from_store -- --ignored --nocapture`
    #[test]
    #[ignore = "rewrites fixtures from a regtest run's store; run by hand"]
    fn export_share_sync_from_store() {
        let path = std::env::var("SHOAL_STORE").expect("set SHOAL_STORE to a run's store.db");
        let count: u32 = std::env::var("SHOAL_FIXTURE_SHARES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(5);
        let store = crate::store::Store::new(path, true).expect("open store read-only");
        let mut blocks = Vec::new();
        for height in 0..count {
            let hash = store
                .get_confirmed_at_height(height)
                .unwrap_or_else(|e| panic!("no confirmed share at height {height}: {e}"));
            let block = store.get_share(&hash).expect("share body");
            println!(
                "height {height}: {hash} bits {:#x} bitcoin {}",
                block.header.bits.to_consensus(),
                block.header.bitcoin_header.block_hash()
            );
            blocks.push(block);
        }
        let headers: Vec<ShareHeader> = blocks.iter().map(|b| b.header.clone()).collect();
        std::fs::write(SYNC_BLOCKS, serde_json::to_string_pretty(&blocks).unwrap() + "\n").unwrap();
        std::fs::write(SYNC_HEADERS, serde_json::to_string_pretty(&headers).unwrap() + "\n").unwrap();
    }
}
