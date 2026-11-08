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
        std::fs::write(
            SYNC_BLOCKS,
            serde_json::to_string_pretty(&blocks).unwrap() + "\n",
        )
        .unwrap();
        std::fs::write(
            SYNC_HEADERS,
            serde_json::to_string_pretty(&headers).unwrap() + "\n",
        )
        .unwrap();
    }

    /// Re-mine the recorded stratum sessions' `submit.json` for XBT. Each session's template,
    /// job coinbase halves (in `notify.json`) and extranonce1 (in `authorize_response.json`) are
    /// kept; the submission becomes a Sia-dialect one (8-byte extranonce2, ntime and nonce)
    /// whose BLAKE2b PoW meets the template's own target, so it is a real XBT block for that job.
    /// A sixth param (version bits) is kept as recorded: XBT miners do not roll versions, but
    /// the test still exercises the six-param path.
    ///
    /// `cargo test -p p2poolv2_lib xbt_fixtures::tests::remine_stratum_sessions -- --ignored --nocapture`
    #[test]
    #[ignore = "rewrites fixtures; run by hand"]
    fn remine_stratum_sessions() {
        use crate::stratum::work::xbt::{job_header, zero_extranonce_coinbase};
        for dir in ["a", "b", "with_version_rolling"] {
            let path = format!("../p2poolv2_tests/test_data/validation/stratum/{dir}");
            let (template, notify, _submit, authorize) =
                crate::test_utils::load_valid_stratum_work_components(&path);
            let coinbase =
                zero_extranonce_coinbase(&notify.params.coinbase1, &notify.params.coinbase2)
                    .unwrap();
            let job = job_header(&template, &coinbase, &[]).unwrap();
            let target = Target::from_compact(job.bits);
            let en1_hex = authorize.result.unwrap()[1].as_str().unwrap().to_string();
            let en1: [u8; 4] = hex::decode(&en1_hex).unwrap().try_into().unwrap();
            let mut found = None;
            'search: for en2 in 0u64.. {
                for nonce in 0u32..(1 << 20) {
                    let sub = shoal_stratum::Submission {
                        extranonce2: en2.to_le_bytes(),
                        ntime: [0u8; 8],
                        nonce: {
                            let mut n = [0u8; 8];
                            n[..4].copy_from_slice(&nonce.to_le_bytes());
                            n
                        },
                    };
                    let header = shoal_stratum::apply_submission(&job, &en1, &sub);
                    if header.meets(target) {
                        found = Some((sub, header));
                        break 'search;
                    }
                }
            }
            let (sub, header) = found.unwrap();
            let submit_path = format!("{path}/submit.json");
            let mut submit: Value =
                serde_json::from_str(&std::fs::read_to_string(&submit_path).unwrap()).unwrap();
            let params = submit["params"].as_array_mut().unwrap();
            params[2] = Value::String(hex::encode(sub.extranonce2));
            params[3] = Value::String(hex::encode(sub.ntime));
            params[4] = Value::String(hex::encode(sub.nonce));
            println!(
                "{dir}: job {} bits {:#x} -> block {}",
                notify.params.job_id,
                job.bits.to_consensus(),
                header.block_hash()
            );
            std::fs::write(
                &submit_path,
                serde_json::to_string_pretty(&submit).unwrap() + "\n",
            )
            .unwrap();
        }
    }
}
