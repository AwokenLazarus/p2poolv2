// SPDX-FileCopyrightText: 2024-2026 P2Poolv2 Developers (see AUTHORS)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Modified by the Shoal project (AwokenLazarus and Shoal contributors), 2026:
// adapted for XBT (Bitcoin on BLAKE2b, 164-byte header v2). See SHOAL-CHANGES.md.

use crate::stratum::error::Error;
use crate::stratum::messages::SimpleRequest;
use crate::stratum::work::tracker::JobDetails;
use bitcoin::consensus::Decodable;
use hex::FromHex;
use shoal_header::XbtHeader as Header;
use tracing::{debug, info};

/// Share validation result
///
/// Captures the header and coinbase, with a boolean to signal if block meets bitcoin difficulty
pub struct ValidationResult {
    /// The block header built from the share components
    pub header: Header,
    /// The coinbase transaction built from share components
    pub coinbase: bitcoin::Transaction,
    /// Does the block meet bitcoin difficulty
    pub meets_bitcoin_difficulty: bool,
}

/// Build coinbase from the submitted share and block template components.
pub fn build_coinbase_from_components(
    coinbase1: &str,
    enonce1: &str,
    enonce2: &str,
    coinbase2: &str,
) -> Result<bitcoin::Transaction, Error> {
    // Add detailed logging to debug the issue
    let complete_tx = format!("{coinbase1}{enonce1}{enonce2}{coinbase2}");

    let tx_bytes = Vec::from_hex(&complete_tx)
        .map_err(|_| Error::InvalidParams("Invalid coinbase hex".into()))?;
    bitcoin::Transaction::consensus_decode(&mut std::io::Cursor::new(tx_bytes))
        .map_err(|_e| Error::InvalidParams("Failed to decode coinbase transaction".into()))
}

/// Applies version mask received with submission, if the version_bits is compatible with version_mask from session
/// If params don't include version_bits, it returns the original header version
// Shoal: unused on XBT (Sia-profile miners do no version rolling); kept for upstream parity.
#[allow(dead_code)]
fn apply_version_mask(
    header_version: i32,
    version_mask: i32,
    params: &[Option<String>],
) -> Result<i32, Error> {
    if params.len() > 5 {
        let version_bits_hex = params[5]
            .as_ref()
            .ok_or_else(|| Error::InvalidParams("Missing version bits".into()))?;
        let bits = i32::from_be_bytes(
            hex::decode(version_bits_hex)
                .map_err(|_| Error::InvalidParams("Failed to decode version bits hex".into()))?
                .as_slice()
                .try_into()
                .map_err(|_| Error::InvalidParams("Invalid version bits length".into()))?,
        );
        Ok((header_version & !version_mask) | (bits & version_mask))
    } else {
        Ok(header_version)
    }
}

/// Validate the difficulty of a submitted share against the block template
///
/// We build the block header from received submission and the corresponding block template.
/// Then we check if the header's difficulty meets the target specified in the block template.
///
/// Returns ValidationResult if validation was error free, else the error.
pub fn validate_bitcoin_difficulty(
    job: &JobDetails,
    submission: &SimpleRequest<'_>,
    enonce1_hex: &str,
    version_mask: i32,
) -> Result<ValidationResult, Error> {
    let compact_target = bitcoin::CompactTarget::from_unprefixed_hex(&job.blocktemplate.bits)
        .map_err(|_| Error::InvalidParams("Failed to parse compact target".into()))?;
    let target = bitcoin::Target::from_compact(compact_target);

    let enonce2 = submission.params[2]
        .as_ref()
        .ok_or_else(|| Error::InvalidParams("Missing enonce2".into()))?
        .as_str();

    // Shoal: on XBT the extranonce is in the header, so the coinbase is built with a zero
    // extranonce, exactly as the job's notify was.
    let _ = enonce2;
    let coinbase = build_coinbase_from_components(
        &job.coinbase1,
        "00000000",
        "0000000000000000",
        &job.coinbase2,
    )
    .map_err(|_| Error::InvalidParams("Failed to build coinbase transaction".into()))?;

    debug!("Coinbase transaction: {:?}", coinbase);

    // Shoal: rebuild the XBT header the miner hashed: the job header (merkle root over the
    // zero-extranonce coinbase) plus this session's extranonce1 and the submitted
    // extranonce2, nonces and rolled ntime. Sia-profile miners do no version rolling, so the
    // version mask is not applied.
    let _ = version_mask;
    let job_header = crate::stratum::work::xbt::job_header(
        &job.blocktemplate,
        &coinbase,
        &job.template_merkle_branches,
    )
    .map_err(|error| Error::InvalidParams(error.message))?;
    let enonce1: [u8; shoal_stratum::EXTRANONCE1_SIZE] = Vec::from_hex(enonce1_hex)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| Error::InvalidParams("Bad enonce1".into()))?;
    let sia_submission = shoal_stratum::Submission::parse(
        submission.params[2].as_deref(),
        submission.params[3].as_deref(),
        submission.params[4].as_deref(),
    )
    .map_err(|error| Error::InvalidParams(error.to_string()))?;
    let header: Header = shoal_stratum::apply_submission(&job_header, &enonce1, &sia_submission);

    let meets_bitcoin_difficulty = match header.validate_pow(target) {
        Ok(_) => {
            info!("Header meets current bitcoin target");
            true
        }
        Err(e) => {
            debug!("Header does not meet the bitcoin target: {}", e);
            false
        }
    };

    Ok(ValidationResult {
        header,
        coinbase,
        meets_bitcoin_difficulty,
    })
}
