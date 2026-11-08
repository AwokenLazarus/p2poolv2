// SPDX-FileCopyrightText: 2026 Shoal contributors (AwokenLazarus)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Added by the Shoal project: XBT (Bitcoin on BLAKE2b) jobs. See SHOAL-CHANGES.md.

//! XBT jobs for the Sia-profile Stratum dialect.
//!
//! An XBT miner's extranonce lives in the 164-byte header, not in the coinbase, so the coinbase
//! for a job is `coinbase1 ‖ zero extranonce ‖ coinbase2` for every miner on it. The coinbase
//! layout P2Poolv2 builds (padding, share commitment last, midstate proof) is unchanged; only
//! the twelve extranonce bytes are fixed at zero.

use crate::stratum::work::block_template::BlockTemplate;
use crate::stratum::work::error::WorkError;
use crate::stratum::work::gbt::compute_merkle_root_from_branches;
use bitcoin::consensus::Decodable;
use bitcoin::hashes::hex::FromHex;
use shoal_header::XbtHeader;
use shoal_stratum::{COINBASE_EXTRANONCE_FILL, JobTemplate};
use std::str::FromStr;

/// The job's coinbase: `coinbase1`, the zero extranonce, then `coinbase2`.
pub fn zero_extranonce_coinbase(
    coinbase1: &str,
    coinbase2: &str,
) -> Result<bitcoin::Transaction, WorkError> {
    let hex = format!(
        "{coinbase1}{}{coinbase2}",
        "00".repeat(COINBASE_EXTRANONCE_FILL)
    );
    let bytes = Vec::<u8>::from_hex(&hex).map_err(|error| WorkError {
        message: format!("Invalid coinbase hex: {error}"),
    })?;
    bitcoin::Transaction::consensus_decode(&mut bytes.as_slice()).map_err(|error| WorkError {
        message: format!("Failed to decode coinbase: {error}"),
    })
}

/// The header every miner on this job starts from (nonces and extranonce zero).
pub fn job_header(
    template: &BlockTemplate,
    coinbase: &bitcoin::Transaction,
    merkle_branches: &[bitcoin::TxMerkleNode],
) -> Result<XbtHeader, WorkError> {
    let merkle_root = compute_merkle_root_from_branches(coinbase.compute_txid(), merkle_branches);
    let prev_blockhash =
        bitcoin::BlockHash::from_str(&template.previousblockhash).map_err(|error| WorkError {
            message: format!("Invalid previousblockhash: {error}"),
        })?;
    let bits =
        bitcoin::CompactTarget::from_unprefixed_hex(&template.bits).map_err(|error| WorkError {
            message: format!("Invalid bits: {error}"),
        })?;
    let txcount = u16::try_from(template.transactions.len() + 1).map_err(|_| WorkError {
        message: "Template has more transactions than an XBT header can count".to_string(),
    })?;
    Ok(shoal_stratum::job_header(&JobTemplate {
        version: template.version,
        prev_blockhash,
        merkle_root,
        curtime: template.curtime,
        bits,
        height: template.height,
        txcount,
    }))
}

/// Test helper: the XBT notify a job must produce, rebuilt from the job's tracked coinbase.
/// Asserts the notify is the Sia-profile dialect and commits to exactly that coinbase.
#[cfg(test)]
pub(crate) fn assert_notify_matches_job(
    notify: &crate::stratum::messages::Notify,
    job: &crate::stratum::work::tracker::JobDetails,
) {
    let coinbase = zero_extranonce_coinbase(&job.coinbase1, &job.coinbase2).unwrap();
    let header = job_header(&job.blocktemplate, &coinbase, &job.template_merkle_branches).unwrap();
    let p = &notify.params;
    assert_eq!(p.coinbase1, shoal_stratum::coinb1_hex(&header), "coinb1 carries h2 of the job");
    assert_eq!(p.coinbase1.len(), 78, "39-byte Sia coinb1");
    assert!(p.coinbase1.starts_with("000000") && p.coinbase1.ends_with("00000000"));
    assert_eq!(p.coinbase2, "", "XBT notify has no coinbase2");
    assert!(p.merkle_branches.is_empty(), "XBT notify has no merkle branches");
    assert_eq!(p.version, "", "XBT notify sends no version");
    assert_eq!(p.prevhash, shoal_stratum::prevhash_hex(&header));
    assert_eq!(p.nbits, shoal_stratum::nbits_hex(&header));
    assert_eq!(p.ntime, "0000000000000000");
}

/// The XBT header-v2 flag in a block version.
const VERSION_V2_FLAG: u32 = 0x8000_0000;

/// Deserialize `getblocktemplate`'s `version`, which XBT reports with the header-v2 flag set
/// (e.g. `0xa0000000`, above `i32::MAX`). The flag is not part of the BIP9 version; it is
/// stripped, and the XBT header encoder always sets it.
pub fn deserialize_template_version<'de, D>(deserializer: D) -> Result<i32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = <i64 as serde::Deserialize>::deserialize(deserializer)?;
    let raw = u32::try_from(raw)
        .or_else(|_| i32::try_from(raw).map(|v| v as u32))
        .map_err(|_| serde::de::Error::custom(format!("block version {raw} out of range")))?;
    Ok((raw & !VERSION_V2_FLAG) as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(serde::Deserialize)]
    struct V {
        #[serde(deserialize_with = "deserialize_template_version")]
        version: i32,
    }

    fn parse(json: &str) -> Result<i32, serde_json::Error> {
        serde_json::from_str::<V>(json).map(|v| v.version)
    }

    #[test]
    fn xbt_template_version_has_its_v2_flag_stripped() {
        assert_eq!(parse(r#"{"version": 2684354560}"#).unwrap(), 0x2000_0000);
        assert_eq!(parse(r#"{"version": 536870912}"#).unwrap(), 0x2000_0000);
    }

    #[test]
    fn out_of_range_template_version_is_refused() {
        assert!(parse(r#"{"version": 8589934592}"#).is_err());
    }
}
