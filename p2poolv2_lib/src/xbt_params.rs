// SPDX-FileCopyrightText: 2026 Shoal contributors (AwokenLazarus)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Added by the Shoal project: per-network sharechain parameters. See SHOAL-CHANGES.md.

//! Per-network sharechain parameters for Shoal.
//!
//! Upstream uses one easiest-share target (`MAX_POOL_TARGET`, about difficulty 1,164) and one
//! genesis target for every network. That is right for real hashrate, but a regtest chain mined
//! by CPU could never produce a share. Shoal keeps upstream's values for every network except
//! regtest, which gets regtest-easy targets.
//!
//! The network is fixed for the life of a node process, so it is set once at start-up
//! ([`set_network`]), like `sim_overrides`. Unset, upstream's values apply.
//!
//! Under `cfg(test)` the setting is per thread, so unit tests (which share one process) can use
//! regtest parameters without changing them for tests running beside them.

use bitcoin::{CompactTarget, Network};
#[cfg(not(test))]
use std::sync::OnceLock;

#[cfg(not(test))]
static NETWORK: OnceLock<Network> = OnceLock::new();

#[cfg(test)]
thread_local! {
    static TEST_NETWORK: std::cell::Cell<Option<Network>> = const { std::cell::Cell::new(None) };
}

/// The easiest share target on regtest: the same as Bitcoin/XBT regtest's block target.
pub const REGTEST_MAX_POOL_TARGET: u32 = 0x207f_ffff;

/// Genesis (ASERT anchor) target on regtest: about 1/16 of a difficulty-1 share, a few seconds
/// for a CPU miner, so the ASERT loop starts near a 10 s share interval on test hardware.
pub const REGTEST_ANCHOR_TARGET: u32 = 0x1e0f_ffff;

/// Record the node's network. The first call wins; later calls with another network are
/// ignored and return false.
pub fn set_network(network: Network) -> bool {
    #[cfg(not(test))]
    {
        NETWORK.get_or_init(|| network) == &network
    }
    #[cfg(test)]
    {
        TEST_NETWORK.with(|n| match n.get() {
            Some(existing) => existing == network,
            None => {
                n.set(Some(network));
                true
            }
        })
    }
}

fn network() -> Option<Network> {
    #[cfg(not(test))]
    {
        NETWORK.get().copied()
    }
    #[cfg(test)]
    {
        TEST_NETWORK.with(|n| n.get())
    }
}

/// True when this process runs a regtest sharechain.
pub fn is_regtest() -> bool {
    network() == Some(Network::Regtest)
}

/// The easiest target a share may declare on this node's network.
pub fn max_pool_target() -> u32 {
    if is_regtest() {
        REGTEST_MAX_POOL_TARGET
    } else {
        crate::shares::share_block::MAX_POOL_TARGET
    }
}

/// The genesis / ASERT anchor target for `network` when not overridden by the sim feature.
pub fn anchor_target(network: Network) -> CompactTarget {
    match network {
        Network::Regtest => CompactTarget::from_consensus(REGTEST_ANCHOR_TARGET),
        _ => CompactTarget::from_consensus(0x1b41_88f5),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn regtest_targets_are_easier_than_upstream() {
        let upstream = bitcoin::Target::from_compact(CompactTarget::from_consensus(
            crate::shares::share_block::MAX_POOL_TARGET,
        ));
        let regtest_max =
            bitcoin::Target::from_compact(CompactTarget::from_consensus(REGTEST_MAX_POOL_TARGET));
        let regtest_anchor = bitcoin::Target::from_compact(anchor_target(Network::Regtest));
        assert!(regtest_max > upstream);
        assert!(regtest_anchor > upstream && regtest_anchor < regtest_max);
        assert_eq!(anchor_target(Network::Testnet4).to_consensus(), 0x1b41_88f5);
    }

    #[test]
    fn network_setting_is_per_test_thread_and_first_call_wins() {
        assert!(!is_regtest());
        assert_eq!(max_pool_target(), crate::shares::share_block::MAX_POOL_TARGET);
        assert!(set_network(Network::Regtest));
        assert!(is_regtest());
        assert_eq!(max_pool_target(), REGTEST_MAX_POOL_TARGET);
        assert!(!set_network(Network::Testnet4), "a second network is refused");
        assert!(is_regtest());
        let other_thread = std::thread::spawn(is_regtest).join().unwrap();
        assert!(!other_thread, "another test thread keeps upstream values");
    }
}
