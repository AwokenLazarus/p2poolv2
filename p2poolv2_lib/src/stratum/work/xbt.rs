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
