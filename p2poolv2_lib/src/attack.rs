// SPDX-FileCopyrightText: 2026 Shoal contributors (AwokenLazarus)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Added by the Shoal project: regtest attack test modes (milestone M5). See SHOAL-CHANGES.md.

//! Attack modes for Shoal's regtest attack tests.
//!
//! Built only with the `shoal-attack` cargo feature, which is never on by default or in a
//! release build. The mode comes from the `SHOAL_ATTACK` environment variable, and every mode
//! refuses to run off regtest. Without the feature, [`mode`] is always `None` and the hooks
//! compile away.
//!
//! - `withhold`: never submit a block to the XBT node. Shares are published as usual.
//! - `wrong-split`: every job's coinbase pays the whole reward to `SHOAL_ATTACK_ADDRESS`, and
//!   this node skips its own payout check, so it publishes those shares.
//! - `copy`: every share this node broadcasts is followed by two copies: one under
//!   `SHOAL_ATTACK_ADDRESS` as miner, one with its coinbase changed. Monero P2Pool's
//!   GHSA-fm6j-gf38-p925 was one share copied ~12,000 times.
//! - `private-fork`: from `SHOAL_ATTACK_PRIVATE_FROM` to `SHOAL_ATTACK_PRIVATE_UNTIL` (seconds
//!   after start-up), ignore every share from peers and hold back this node's own, then
//!   release them all at once.

/// An attack mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Withhold,
    WrongSplit,
    Copy,
    PrivateFork,
}

#[cfg(feature = "shoal-attack")]
mod imp {
    use super::Mode;
    use std::sync::OnceLock;
    use std::time::Instant;

    pub(super) struct State {
        pub mode: Option<Mode>,
        pub address: Option<bitcoin::Address>,
        pub start: Instant,
        pub private_from: u64,
        pub private_until: u64,
    }

    pub(super) static STATE: OnceLock<State> = OnceLock::new();

    pub(super) fn state() -> &'static State {
        STATE.get_or_init(|| {
            let mode = match std::env::var("SHOAL_ATTACK").as_deref() {
                Ok("withhold") => Some(Mode::Withhold),
                Ok("wrong-split") => Some(Mode::WrongSplit),
                Ok("copy") => Some(Mode::Copy),
                Ok("private-fork") => Some(Mode::PrivateFork),
                Ok("") | Err(_) => None,
                Ok(other) => panic!("unknown SHOAL_ATTACK mode {other}"),
            };
            if mode.is_some() && !crate::xbt_params::is_regtest() {
                panic!("SHOAL_ATTACK is a regtest test mode");
            }
            let address = std::env::var("SHOAL_ATTACK_ADDRESS").ok().map(|text| {
                text.parse::<bitcoin::Address<_>>()
                    .expect("SHOAL_ATTACK_ADDRESS is an address")
                    .require_network(bitcoin::Network::Regtest)
                    .expect("SHOAL_ATTACK_ADDRESS is a regtest address")
            });
            let seconds = |name: &str, default: u64| {
                std::env::var(name)
                    .ok()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(default)
            };
            if let Some(mode) = mode {
                tracing::warn!("SHOAL ATTACK TEST MODE {mode:?} (regtest only)");
            }
            State {
                mode,
                address,
                start: Instant::now(),
                private_from: seconds("SHOAL_ATTACK_PRIVATE_FROM", 120),
                private_until: seconds("SHOAL_ATTACK_PRIVATE_UNTIL", 420),
            }
        })
    }
}

/// The attack mode this process runs, if any.
pub fn mode() -> Option<Mode> {
    #[cfg(feature = "shoal-attack")]
    {
        imp::state().mode
    }
    #[cfg(not(feature = "shoal-attack"))]
    {
        None
    }
}

/// Whether this process runs attack mode `wanted`.
pub fn is(wanted: Mode) -> bool {
    mode() == Some(wanted)
}

/// The attacker's own address (`SHOAL_ATTACK_ADDRESS`).
pub fn address() -> Option<bitcoin::Address> {
    #[cfg(feature = "shoal-attack")]
    {
        imp::state().address.clone()
    }
    #[cfg(not(feature = "shoal-attack"))]
    {
        None
    }
}

/// Start the attack clock (call at start-up).
pub fn start() {
    let _ = mode();
}

/// In `private-fork` mode: whether the private period is running now.
pub fn private_now() -> bool {
    #[cfg(feature = "shoal-attack")]
    {
        let state = imp::state();
        if state.mode != Some(Mode::PrivateFork) {
            return false;
        }
        let elapsed = state.start.elapsed().as_secs();
        elapsed >= state.private_from && elapsed < state.private_until
    }
    #[cfg(not(feature = "shoal-attack"))]
    {
        false
    }
}

/// In `private-fork` mode: whether the private period is over (time to release).
pub fn private_over() -> bool {
    #[cfg(feature = "shoal-attack")]
    {
        let state = imp::state();
        state.mode == Some(Mode::PrivateFork)
            && state.start.elapsed().as_secs() >= state.private_until
    }
    #[cfg(not(feature = "shoal-attack"))]
    {
        false
    }
}
