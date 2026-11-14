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

/// Default genesis (ASERT anchor) target on regtest: difficulty 1/4096 (4,096 times easier than
/// a difficulty-1 share), so a single slow CPU miner still finds shares. A run with more hashrate
/// sets its own with [`REGTEST_GENESIS_BITS_ENV`].
pub const REGTEST_ANCHOR_TARGET: u32 = 0x1e0f_ffff;

/// The easiest share target on XBT testnet4: difficulty 1, the same as testnet4's
/// minimum-difficulty blocks, so testers with CPUs or small miners can take part. ASERT (half-life
/// 10 minutes) raises it within seconds when an ASIC joins. Upstream's floor (difficulty about
/// 1,164) stays on every other network but regtest.
pub const TESTNET4_MAX_POOL_TARGET: u32 = 0x1d00_ffff;

/// Genesis / ASERT anchor target on XBT testnet4 (the public test sharechain): the floor, so
/// whoever mines first finds shares at once and ASERT sets the pace from there.
pub const TESTNET4_ANCHOR_TARGET: u32 = TESTNET4_MAX_POOL_TARGET;

/// Genesis time of the public XBT testnet4 sharechain (Unix seconds), built in so a tester's node
/// joins it without settings. [`TESTNET4_GENESIS_TIME_ENV`] still overrides it for a private
/// trial chain.
pub const TESTNET4_GENESIS_TIME: u32 = TESTNET4_LAUNCH;

/// Launch time of the current public testnet4 sharechain.
const TESTNET4_LAUNCH: u32 = 1_794_628_800; // 2026-11-14 04:00:00 UTC

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

/// The network set with [`set_network`], if any.
pub fn current_network() -> Option<Network> {
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
    current_network() == Some(Network::Regtest)
}

/// The easiest target a share may declare on this node's network.
pub fn max_pool_target() -> u32 {
    match current_network() {
        Some(Network::Regtest) => REGTEST_MAX_POOL_TARGET,
        Some(Network::Testnet4) => TESTNET4_MAX_POOL_TARGET,
        _ => crate::shares::share_block::MAX_POOL_TARGET,
    }
}

/// The genesis / ASERT anchor target for `network` when not overridden by the sim feature.
pub fn anchor_target(network: Network) -> CompactTarget {
    match network {
        Network::Regtest => CompactTarget::from_consensus(
            regtest_genesis_bits().unwrap_or(REGTEST_ANCHOR_TARGET),
        ),
        Network::Testnet4 => CompactTarget::from_consensus(
            regtest_genesis_bits().unwrap_or(TESTNET4_ANCHOR_TARGET),
        ),
        _ => CompactTarget::from_consensus(0x1b41_88f5),
    }
}

/// Environment variable that sets the regtest genesis share's timestamp (Unix seconds).
///
/// The genesis time is the ASERT anchor time. A fixed regtest genesis from months ago makes ASERT
/// see the chain as far behind schedule and pins the share target at its easiest, so a test run
/// sets this to its start time. All nodes of one regtest sharechain must use the same value: it
/// changes the genesis hash, and nodes on different genesis shares do not peer.
pub const REGTEST_GENESIS_TIME_ENV: &str = "SHOAL_REGTEST_GENESIS_TIME";

/// Environment variable that sets the regtest genesis share's target (compact bits, hex, e.g.
/// `1d07ffff`). It is the ASERT anchor target: a run sets it near the difficulty its miners find
/// a share at every 10 s, so ASERT does not spend the run climbing from the default. Same rules
/// as [`REGTEST_GENESIS_TIME_ENV`]: one value per sharechain, it changes the genesis hash.
pub const REGTEST_GENESIS_BITS_ENV: &str = "SHOAL_REGTEST_GENESIS_BITS";

/// Testnet4's counterpart of [`REGTEST_GENESIS_TIME_ENV`]: a trial testnet4 sharechain sets its
/// launch time here instead of in the binary. Same rules: every node of the sharechain uses the
/// same value, and it changes the genesis hash.
pub const TESTNET4_GENESIS_TIME_ENV: &str = "SHOAL_TESTNET4_GENESIS_TIME";

/// Testnet4's counterpart of [`REGTEST_GENESIS_BITS_ENV`] (default [`TESTNET4_ANCHOR_TARGET`]).
pub const TESTNET4_GENESIS_BITS_ENV: &str = "SHOAL_TESTNET4_GENESIS_BITS";

/// True on the networks whose genesis time and bits may come from the environment.
fn genesis_overridable() -> bool {
    matches!(current_network(), Some(Network::Regtest | Network::Testnet4))
}

/// The environment variable names for the genesis time and bits on this node's network.
fn genesis_env_names() -> (&'static str, &'static str) {
    if current_network() == Some(Network::Testnet4) {
        (TESTNET4_GENESIS_TIME_ENV, TESTNET4_GENESIS_BITS_ENV)
    } else {
        (REGTEST_GENESIS_TIME_ENV, REGTEST_GENESIS_BITS_ENV)
    }
}

#[cfg(not(test))]
static REGTEST_GENESIS_TIME: OnceLock<u32> = OnceLock::new();
#[cfg(not(test))]
static REGTEST_GENESIS_BITS: OnceLock<u32> = OnceLock::new();

/// On a regtest node, read [`REGTEST_GENESIS_TIME_ENV`] and [`REGTEST_GENESIS_BITS_ENV`] once
/// (call after [`set_network`]). Returns the overrides in force. Ignored on other networks.
/// Bits must be no easier than [`REGTEST_MAX_POOL_TARGET`].
pub fn init_regtest_genesis_from_env() -> Result<(Option<u32>, Option<u32>), String> {
    if !genesis_overridable() {
        return Ok((None, None));
    }
    let (time_env, bits_env) = genesis_env_names();
    if let Ok(value) = std::env::var(time_env) {
        let time: u32 = value
            .trim()
            .parse()
            .map_err(|e| format!("{time_env}={value:?}: {e}"))?;
        #[cfg(not(test))]
        REGTEST_GENESIS_TIME.get_or_init(|| time);
        let _ = time;
    }
    if let Ok(value) = std::env::var(bits_env) {
        let bits = u32::from_str_radix(value.trim().trim_start_matches("0x"), 16)
            .map_err(|e| format!("{bits_env}={value:?}: {e}"))?;
        let target = bitcoin::Target::from_compact(CompactTarget::from_consensus(bits));
        let easiest =
            bitcoin::Target::from_compact(CompactTarget::from_consensus(max_pool_target()));
        if target > easiest || target == bitcoin::Target::ZERO {
            return Err(format!(
                "{bits_env}={value:?}: must be a nonzero target no easier than {:#x}",
                max_pool_target()
            ));
        }
        #[cfg(not(test))]
        REGTEST_GENESIS_BITS.get_or_init(|| bits);
    }
    Ok((regtest_genesis_time(), regtest_genesis_bits()))
}

/// The regtest genesis bits override, if set (never under `cfg(test)`).
pub fn regtest_genesis_bits() -> Option<u32> {
    #[cfg(not(test))]
    {
        if genesis_overridable() {
            return REGTEST_GENESIS_BITS.get().copied();
        }
    }
    None
}

/// The genesis time override, if set: the environment variable, or on testnet4 the built-in
/// [`TESTNET4_GENESIS_TIME`] (never under `cfg(test)`).
pub fn regtest_genesis_time() -> Option<u32> {
    #[cfg(not(test))]
    {
        if genesis_overridable() {
            let set = REGTEST_GENESIS_TIME.get().copied();
            if set.is_none() && current_network() == Some(Network::Testnet4) {
                return Some(TESTNET4_GENESIS_TIME);
            }
            return set;
        }
    }
    None
}

/// On regtest, stratum difficulty 1 means 1/1024 of a real difficulty-1 share, so CPU miners
/// submit often enough for 10 s shares and uncles. The wire value is fractional (0.0009765625).
pub const REGTEST_STRATUM_DIFFICULTY_DIVISOR: u64 = 1024;

/// The divisor between pool-unit stratum difficulty and what the miner is told: 1 on every
/// network but regtest.
pub fn stratum_difficulty_divisor() -> u64 {
    if is_regtest() && !cfg!(feature = "sim") {
        REGTEST_STRATUM_DIFFICULTY_DIVISOR
    } else {
        1
    }
}

/// PPLNS window on regtest, in shares: short, so a test run of a few dozen shares slides it.
pub const REGTEST_PPLNS_WINDOW_SHARES: usize = 12;

/// Share depth before a share is paid on regtest (upstream's `COINBASE_MATURITY`, 6,048).
pub const REGTEST_COINBASE_MATURITY: usize = 4;

/// PPLNS window on XBT testnet4, in shares: one day at 10 s shares (upstream: 14 days), so a
/// tester sees the window move within a test.
pub const TESTNET4_PPLNS_WINDOW_SHARES: usize = 8_640;

/// Share depth before a share is paid on XBT testnet4: one hour at 10 s shares (upstream: 6,048,
/// about 17 hours), so a new tester's first payout comes within the hour.
pub const TESTNET4_COINBASE_MATURITY: usize = 360;

/// Shares a found block pays on `network`.
///
/// Every node of one sharechain runs the same network, so they all compute the same window.
pub fn pplns_window_shares(network: Network) -> usize {
    use crate::accounting::payout::sharechain_pplns::pplns_window::PPLNS_WINDOW_SHARES;
    if cfg!(feature = "sim") {
        return PPLNS_WINDOW_SHARES;
    }
    match network {
        Network::Regtest => REGTEST_PPLNS_WINDOW_SHARES,
        Network::Testnet4 => TESTNET4_PPLNS_WINDOW_SHARES,
        _ => PPLNS_WINDOW_SHARES,
    }
}

/// Depth a share must reach before a found block pays it on `network`.
pub fn coinbase_maturity(network: Network) -> usize {
    use crate::accounting::payout::sharechain_pplns::pplns_window::COINBASE_MATURITY;
    if cfg!(feature = "sim") {
        return COINBASE_MATURITY;
    }
    match network {
        Network::Regtest => REGTEST_COINBASE_MATURITY,
        Network::Testnet4 => TESTNET4_COINBASE_MATURITY,
        _ => COINBASE_MATURITY,
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
        assert_eq!(anchor_target(Network::Bitcoin).to_consensus(), 0x1b41_88f5);
        assert_eq!(anchor_target(Network::Testnet4).to_consensus(), TESTNET4_ANCHOR_TARGET);
    }

    #[test]
    fn testnet4_floor_is_difficulty_one() {
        assert!(set_network(Network::Testnet4));
        assert_eq!(max_pool_target(), 0x1d00_ffff);
        let floor = bitcoin::Target::from_compact(CompactTarget::from_consensus(max_pool_target()));
        let upstream = bitcoin::Target::from_compact(CompactTarget::from_consensus(
            crate::shares::share_block::MAX_POOL_TARGET,
        ));
        assert!(floor > upstream, "testnet4 shares may be easier than upstream's floor");
        assert_eq!(anchor_target(Network::Testnet4).to_consensus(), max_pool_target());
    }

    #[test]
    fn only_regtest_divides_stratum_difficulty() {
        assert_eq!(stratum_difficulty_divisor(), 1);
        assert!(set_network(Network::Regtest));
        assert_eq!(stratum_difficulty_divisor(), REGTEST_STRATUM_DIFFICULTY_DIVISOR);
        let message = crate::stratum::messages::SetDifficultyNotification::new(2048);
        assert_eq!(
            serde_json::to_string(&message).unwrap(),
            r#"{"method":"mining.set_difficulty","params":[2.0]}"#
        );
    }

    #[test]
    fn only_test_networks_get_short_windows() {
        use crate::accounting::payout::sharechain_pplns::pplns_window::{
            COINBASE_MATURITY, PPLNS_WINDOW_SHARES,
        };
        assert_eq!(pplns_window_shares(Network::Regtest), REGTEST_PPLNS_WINDOW_SHARES);
        assert_eq!(coinbase_maturity(Network::Regtest), REGTEST_COINBASE_MATURITY);
        assert_eq!(pplns_window_shares(Network::Testnet4), TESTNET4_PPLNS_WINDOW_SHARES);
        assert_eq!(coinbase_maturity(Network::Testnet4), TESTNET4_COINBASE_MATURITY);
        for network in [Network::Bitcoin, Network::Signet] {
            assert_eq!(pplns_window_shares(network), PPLNS_WINDOW_SHARES);
            assert_eq!(coinbase_maturity(network), COINBASE_MATURITY);
        }
    }

    #[test]
    fn network_setting_is_per_test_thread_and_first_call_wins() {
        assert!(!is_regtest());
        assert_eq!(
            max_pool_target(),
            crate::shares::share_block::MAX_POOL_TARGET
        );
        assert!(set_network(Network::Regtest));
        assert!(is_regtest());
        assert_eq!(max_pool_target(), REGTEST_MAX_POOL_TARGET);
        assert!(
            !set_network(Network::Testnet4),
            "a second network is refused"
        );
        assert!(is_regtest());
        let other_thread = std::thread::spawn(is_regtest).join().unwrap();
        assert!(!other_thread, "another test thread keeps upstream values");
    }
}
