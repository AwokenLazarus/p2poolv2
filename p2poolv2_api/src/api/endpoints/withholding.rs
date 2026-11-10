// SPDX-FileCopyrightText: 2026 Shoal contributors (AwokenLazarus)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Added by the Shoal project: the public block-withholding statistic. See SHOAL-CHANGES.md.

//! `GET /withholding`: per miner, how many recent confirmed shares met the XBT block target and
//! how many of those blocks this node's XBT header store knows.
//!
//! No pool can stop a miner from keeping back the blocks it finds while it publishes its
//! shares. Shoal shows it instead: a share whose XBT header meets the block target was a block,
//! and an honest miner submits it. A miner with many such shares and none on the XBT chain is
//! withholding. Everything here comes from public data, so any node computes the same numbers
//! (up to which XBT blocks it has seen).

use crate::api::error::ApiError;
use crate::api::server::AppState;
use axum::{
    Json,
    extract::{Query, State},
};
use p2poolv2_lib::address_display::ShareInfoDisplay;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Most shares one request may scan.
pub(crate) const MAX_WITHHOLDING_SHARES: u32 = 20_000;

/// A miner is flagged once it has this many block-target shares and none of them is known.
pub const WITHHOLDING_FLAG_MIN: u32 = 5;

#[derive(Deserialize)]
pub struct WithholdingQuery {
    /// Confirmed shares to scan back from the tip. Default 1,000.
    pub num: Option<u32>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct MinerWithholding {
    /// Shares in the range.
    pub shares: u32,
    /// Of those, shares whose XBT header met the XBT block target.
    pub met_block_target: u32,
    /// Of those, blocks this node's XBT header store knows.
    pub blocks_seen: u32,
    /// True when `met_block_target` >= [`WITHHOLDING_FLAG_MIN`] and `blocks_seen` is 0.
    pub flagged: bool,
}

#[derive(Serialize)]
pub struct WithholdingResponse {
    pub from_height: u32,
    pub to_height: u32,
    /// False when the node has no XBT header store, so `blocks_seen` cannot be counted.
    pub header_store_active: bool,
    /// Keyed by the miner's share chain address.
    pub miners: BTreeMap<String, MinerWithholding>,
}

/// Tally one share. `known` says whether its XBT block is known.
pub(crate) fn tally(entry: &mut MinerWithholding, met: bool, known: bool) {
    entry.shares += 1;
    if met {
        entry.met_block_target += 1;
        if known {
            entry.blocks_seen += 1;
        }
    }
    entry.flagged = entry.met_block_target >= WITHHOLDING_FLAG_MIN && entry.blocks_seen == 0;
}

pub(crate) async fn withholding(
    State(state): State<Arc<AppState>>,
    Query(query): Query<WithholdingQuery>,
) -> Result<Json<WithholdingResponse>, ApiError> {
    let num = query.num.unwrap_or(1_000);
    if !(1..=MAX_WITHHOLDING_SHARES).contains(&num) {
        return Err(ApiError::BadRequest(format!(
            "num must be between 1 and {MAX_WITHHOLDING_SHARES}, got {num}"
        )));
    }
    let chain_store_handle = &state.chain_store_handle;
    let to_height = chain_store_handle
        .get_tip_height()
        .map_err(|error| ApiError::ServerError(format!("Failed to get tip height: {error}")))?
        .ok_or_else(|| ApiError::NotFound("No confirmed chain tip found".to_string()))?;
    // Genesis is not mined by anyone.
    let from_height = to_height.saturating_sub(num - 1).max(1);
    let mut miners = BTreeMap::new();
    let header_store_active = p2poolv2_lib::xbt_chain::is_active();
    if to_height == 0 {
        return Ok(Json(WithholdingResponse {
            from_height,
            to_height,
            header_store_active,
            miners,
        }));
    }
    let store = chain_store_handle.store_handle().store();
    let shares = store
        .query_shares(from_height, to_height)
        .map_err(|error| ApiError::ServerError(format!("Failed to query shares: {error}")))?;
    let display = ShareInfoDisplay::from_share_infos(&shares, Some(state.app_config.network));
    for (share, shown) in shares.iter().zip(display.iter()) {
        let header = match store.get_share_header(&share.blockhash) {
            Ok(Some(header)) => header,
            _ => continue,
        };
        let xbt = &header.bitcoin_header;
        let met = xbt.meets(xbt.target());
        let hash = xbt.block_hash();
        let known = met
            && p2poolv2_lib::xbt_chain::with_store(|s| s.contains(&hash)).unwrap_or(false);
        let entry = miners
            .entry(shown.miner_address.clone())
            .or_insert(MinerWithholding {
                shares: 0,
                met_block_target: 0,
                blocks_seen: 0,
                flagged: false,
            });
        tally(entry, met, known);
    }
    Ok(Json(WithholdingResponse {
        from_height,
        to_height,
        header_store_active,
        miners,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty() -> MinerWithholding {
        MinerWithholding {
            shares: 0,
            met_block_target: 0,
            blocks_seen: 0,
            flagged: false,
        }
    }

    #[test]
    fn honest_miner_is_not_flagged() {
        let mut m = empty();
        for _ in 0..10 {
            tally(&mut m, true, true);
        }
        tally(&mut m, false, false);
        assert_eq!(m.shares, 11);
        assert_eq!(m.met_block_target, 10);
        assert_eq!(m.blocks_seen, 10);
        assert!(!m.flagged);
    }

    #[test]
    fn withholder_is_flagged_only_after_enough_blocks() {
        let mut m = empty();
        for _ in 0..(WITHHOLDING_FLAG_MIN - 1) {
            tally(&mut m, true, false);
        }
        assert!(!m.flagged, "too few to tell");
        tally(&mut m, true, false);
        assert!(m.flagged);
        tally(&mut m, true, true);
        assert!(!m.flagged, "one seen block clears it");
    }

    #[test]
    fn unknown_share_that_missed_block_target_counts_nothing() {
        let mut m = empty();
        tally(&mut m, false, true);
        assert_eq!((m.met_block_target, m.blocks_seen), (0, 0));
    }
}
