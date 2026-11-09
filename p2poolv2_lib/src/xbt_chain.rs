// SPDX-FileCopyrightText: 2026 Shoal contributors (AwokenLazarus)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Added by the Shoal project: the XBT header store and the "mine on the current XBT chain"
// payout rule, after upstream's design in docs/architecture/mine-on-bitcoin-chain.md
// (issue #652). See SHOAL-CHANGES.md.

//! Pay shares only for work on the XBT chain the paying share extends.
//!
//! Each share names the XBT block it builds on, its *XBT prev*
//! (`bitcoin_header.prev_blockhash`). Without a rule on it, a miner can build on an XBT block
//! that does not exist and still collect PPLNS weight. This module holds the three parts of the
//! fix, following upstream's design (`docs/architecture/mine-on-bitcoin-chain.md`):
//!
//! - **A header store** of XBT headers with heights. The local Knots node feeds it (the GBT
//!   poller walks back from each template's parent), and so do headers carried with shares,
//!   which every node checks itself: they must link to a known header, meet their PoW, carry
//!   exactly the `bits` XBT's difficulty rule requires, and be at most 2 hours ahead of the clock.
//!   The store is kept in a file next to the share store, so a restart keeps headers learned
//!   from peers (orphans the local node never saw).
//! - **Acceptance needs a known XBT prev.** A share from a peer is dropped, not stored, until its
//!   XBT prev is in the store.
//! - **The payout rule.** A share's coinbase pays a window share only if that share's XBT prev
//!   is on the paying share's XBT chain or one block off it ([`Eligibility`]). Ineligible shares
//!   keep their window slot and are paid nothing; uncles follow the same rule.
//!
//! The rule is active once [`init`] has run, which the node does at start-up. Unit tests that
//! never call it see upstream's behaviour.

use bitcoin::hashes::Hash;
use bitcoin::{BlockHash, CompactTarget, Network, Target};
use shoal_header::{XBT_HEADER_SIZE, XbtHeader};
use std::collections::{HashMap, HashSet};
use std::fs::{File, OpenOptions};
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

mod u320 {
    #![allow(clippy::manual_div_ceil, clippy::assign_op_pattern)]
    uint::construct_uint! {
        /// 320-bit unsigned integer for retarget arithmetic (a 256-bit target times a timespan).
        pub struct U320(5);
    }
}
use u320::U320;

/// Most headers one share block may carry (decoder cap).
pub const MAX_CARRIED_HEADERS_PER_SHARE: usize = 100;
/// Most headers one share header batch may carry (decoder cap).
pub const MAX_CARRIED_HEADERS_PER_BATCH: usize = 500;
/// Headers past the first one on the sender's active chain that a share still carries, so a
/// receiver whose node is a few blocks behind can link them.
const CARRY_PAST_ACTIVE_CHAIN: usize = 3;
/// A carried header may be at most this far ahead of the local clock (seconds).
const MAX_FUTURE_SECONDS: u64 = 2 * 60 * 60;
/// Failed carried header hashes remembered, so a repeat costs nothing.
const FAILED_CACHE_SIZE: usize = 4096;

/// XBT's proof-of-work parameters for one network (Knots-BLAKE2b `kernel/chainparams.cpp`).
#[derive(Debug, Clone, Copy)]
pub struct PowParams {
    pub pow_limit: Target,
    pub target_timespan: i64,
    pub target_spacing: i64,
    pub allow_min_difficulty_blocks: bool,
    pub no_retargeting: bool,
    pub enforce_bip94: bool,
    /// First block mined under BLAKE2b; `u32::MAX` when not scheduled.
    pub blake2b_height: u32,
    pub blake2b_target_shift: u32,
}

impl PowParams {
    pub fn for_network(network: Network) -> Self {
        let bitcoin_limit = Target::from_be_bytes(hex_32(
            "00000000ffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        ));
        match network {
            Network::Bitcoin => PowParams {
                pow_limit: bitcoin_limit,
                target_timespan: 14 * 24 * 60 * 60,
                target_spacing: 600,
                allow_min_difficulty_blocks: false,
                no_retargeting: false,
                enforce_bip94: false,
                blake2b_height: 961_640,
                blake2b_target_shift: 22,
            },
            Network::Testnet => PowParams {
                pow_limit: bitcoin_limit,
                target_timespan: 14 * 24 * 60 * 60,
                target_spacing: 600,
                allow_min_difficulty_blocks: true,
                no_retargeting: false,
                enforce_bip94: false,
                blake2b_height: u32::MAX,
                blake2b_target_shift: 20,
            },
            Network::Testnet4 => PowParams {
                pow_limit: bitcoin_limit,
                target_timespan: 14 * 24 * 60 * 60,
                target_spacing: 600,
                allow_min_difficulty_blocks: true,
                no_retargeting: false,
                enforce_bip94: true,
                blake2b_height: 150_308,
                blake2b_target_shift: 20,
            },
            Network::Signet => PowParams {
                pow_limit: Target::from_be_bytes(hex_32(
                    "00000377ae000000000000000000000000000000000000000000000000000000",
                )),
                target_timespan: 14 * 24 * 60 * 60,
                target_spacing: 600,
                allow_min_difficulty_blocks: false,
                no_retargeting: false,
                enforce_bip94: false,
                blake2b_height: u32::MAX,
                blake2b_target_shift: 20,
            },
            // Regtest: no retargeting and min-difficulty blocks, so every block has the limit.
            _ => PowParams {
                pow_limit: Target::from_be_bytes(hex_32(
                    "7fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
                )),
                target_timespan: 24 * 60 * 60,
                target_spacing: 600,
                allow_min_difficulty_blocks: true,
                no_retargeting: true,
                enforce_bip94: false,
                blake2b_height: u32::MAX,
                blake2b_target_shift: 20,
            },
        }
    }

    pub fn interval(&self) -> u32 {
        (self.target_timespan / self.target_spacing) as u32
    }

    fn pow_limit_compact(&self) -> CompactTarget {
        self.pow_limit.to_compact_lossy()
    }
}

fn hex_32(s: &str) -> [u8; 32] {
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).expect("valid hex constant");
    }
    out
}

fn to_u320(target: Target) -> U320 {
    U320::from_big_endian(&target.to_be_bytes())
}

fn from_u320(value: U320) -> Target {
    let bytes = value.to_big_endian();
    let mut out = [0u8; 32];
    out.copy_from_slice(&bytes[bytes.len() - 32..]);
    Target::from_be_bytes(out)
}

/// Knots' `ApplyBlake2bTargetShift`: the one-off easing at the first BLAKE2b block.
fn apply_blake2b_shift(bits: CompactTarget, params: &PowParams) -> CompactTarget {
    let target = to_u320(Target::from_compact(bits));
    let limit = to_u320(params.pow_limit);
    let shifted = if target > (limit >> params.blake2b_target_shift as usize) {
        limit
    } else {
        target << params.blake2b_target_shift as usize
    };
    from_u320(shifted).to_compact_lossy()
}

/// A stored header and its height.
///
/// XBT blocks from before the BLAKE2b activation have Bitcoin's 80-byte header. They come only
/// from the local node (`header` is `None`) and are never carried to peers.
#[derive(Debug, Clone, Copy)]
pub struct HeaderEntry {
    pub hash: BlockHash,
    pub prev: BlockHash,
    pub time: u32,
    pub bits: CompactTarget,
    pub height: u32,
    /// The 164-byte header, for blocks mined under BLAKE2b.
    pub header: Option<XbtHeader>,
}

impl HeaderEntry {
    fn from_xbt(header: XbtHeader, height: u32) -> Self {
        Self {
            hash: header.block_hash(),
            prev: header.prev_blockhash,
            time: header.time,
            bits: header.bits,
            height,
            header: Some(header),
        }
    }

    fn from_legacy(header: &bitcoin::block::Header, height: u32) -> Self {
        Self {
            hash: header.block_hash(),
            prev: header.prev_blockhash,
            time: header.time,
            bits: header.bits,
            height,
            header: None,
        }
    }
}

/// Store file record kinds.
const RECORD_XBT: u8 = 1;
const RECORD_LEGACY: u8 = 2;
const LEGACY_HEADER_SIZE: usize = 80;

/// Why a carried header was refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeaderCheckError {
    UnknownParent,
    WrongBits { required: u32, got: u32 },
    BadProofOfWork,
    TooFarInFuture,
    TimeWarp,
    MissingAncestor,
}

impl std::fmt::Display for HeaderCheckError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HeaderCheckError::UnknownParent => f.write_str("parent header unknown"),
            HeaderCheckError::WrongBits { required, got } => {
                write!(f, "bits {got:08x}, XBT difficulty rule requires {required:08x}")
            }
            HeaderCheckError::BadProofOfWork => f.write_str("proof of work does not meet bits"),
            HeaderCheckError::TooFarInFuture => f.write_str("more than 2 hours in the future"),
            HeaderCheckError::TimeWarp => f.write_str("BIP94 timewarp rule"),
            HeaderCheckError::MissingAncestor => {
                f.write_str("an ancestor the difficulty rule needs is not stored")
            }
        }
    }
}

/// Why eligibility could not be computed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EligibilityError {
    /// The paying XBT prev is not in the store.
    UnknownPayingPrev(BlockHash),
    /// The store does not reach far enough back from the paying XBT prev to place the window.
    StoreTooShallow { paying: BlockHash, reached_height: u32 },
}

impl std::fmt::Display for EligibilityError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EligibilityError::UnknownPayingPrev(hash) => {
                write!(f, "paying XBT prev {hash} is not in the header store")
            }
            EligibilityError::StoreTooShallow {
                paying,
                reached_height,
            } => write!(
                f,
                "XBT header store ends at height {reached_height}, too shallow to place the \
                 window paid on {paying}"
            ),
        }
    }
}

impl std::error::Error for EligibilityError {}

/// The XBT blocks eligible for one paying XBT prev: the paying chain over
/// [`eligibility_depth`] blocks, and every known child of a block on it.
#[derive(Debug, Clone)]
pub struct Eligibility {
    paying: BlockHash,
    eligible: HashSet<BlockHash>,
}

impl Eligibility {
    pub fn paying(&self) -> BlockHash {
        self.paying
    }

    /// Whether work on `btc_prev` is paid by a share whose XBT prev is `paying`.
    pub fn is_eligible(&self, btc_prev: &BlockHash) -> bool {
        self.eligible.contains(btc_prev)
    }

    /// For tests: an eligibility set from a list of hashes.
    #[cfg(any(test, feature = "test-utils"))]
    pub fn from_hashes(paying: BlockHash, hashes: impl IntoIterator<Item = BlockHash>) -> Self {
        Self {
            paying,
            eligible: hashes.into_iter().collect(),
        }
    }
}

/// How far back from the paying XBT prev the paying chain is walked: further than a PPLNS
/// window plus its maturity skip can span, so every node places the same window the same way.
pub fn eligibility_depth(network: Network) -> u32 {
    match network {
        // About 28 days: twice the window's 14 days.
        Network::Bitcoin | Network::Testnet | Network::Testnet4 | Network::Signet => 4_032,
        _ => 1_000,
    }
}

/// How many headers a fresh node loads from its Knots node at start-up: the eligibility walk
/// from a share at the prune depth (~29 days) plus a retarget period, rounded up.
pub fn seed_depth(network: Network) -> u32 {
    match network {
        Network::Bitcoin | Network::Testnet | Network::Testnet4 | Network::Signet => 8_640,
        _ => 1_100,
    }
}

/// The XBT header store.
pub struct HeaderStore {
    network: Network,
    params: PowParams,
    entries: HashMap<BlockHash, HeaderEntry>,
    children: HashMap<BlockHash, Vec<BlockHash>>,
    /// The tip of the local Knots node's active chain, as the GBT poller last saw it.
    local_tip: Option<BlockHash>,
    /// Lowest height in the store; headers below it are not known.
    floor_height: Option<u32>,
    failed: HashSet<BlockHash>,
    file: Option<File>,
    eligibility_cache: Option<Eligibility>,
}

impl HeaderStore {
    pub fn new(network: Network) -> Self {
        Self {
            network,
            params: PowParams::for_network(network),
            entries: HashMap::new(),
            children: HashMap::new(),
            local_tip: None,
            floor_height: None,
            failed: HashSet::new(),
            file: None,
            eligibility_cache: None,
        }
    }

    /// Open (or create) the store file at `path` and load what it holds.
    pub fn open(network: Network, path: &Path) -> std::io::Result<Self> {
        let mut store = Self::new(network);
        if path.exists() {
            let mut reader = BufReader::new(File::open(path)?);
            let mut loaded = 0usize;
            loop {
                let mut prefix = [0u8; 5];
                match reader.read_exact(&mut prefix) {
                    Ok(()) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => break,
                    Err(error) => return Err(error),
                }
                let height = u32::from_le_bytes(prefix[1..].try_into().expect("4 bytes"));
                let entry = match prefix[0] {
                    RECORD_XBT => {
                        let mut bytes = [0u8; XBT_HEADER_SIZE];
                        reader.read_exact(&mut bytes)?;
                        XbtHeader::from_bytes(&bytes)
                            .map(|header| HeaderEntry::from_xbt(header, height))
                            .map_err(|error| error.to_string())
                    }
                    RECORD_LEGACY => {
                        let mut bytes = [0u8; LEGACY_HEADER_SIZE];
                        reader.read_exact(&mut bytes)?;
                        bitcoin::consensus::deserialize::<bitcoin::block::Header>(&bytes)
                            .map(|header| HeaderEntry::from_legacy(&header, height))
                            .map_err(|error| error.to_string())
                    }
                    kind => {
                        warn!("XBT header store has an unknown record kind {kind}; stopping there");
                        break;
                    }
                };
                match entry {
                    Ok(entry) => {
                        store.insert_entry(entry);
                        loaded += 1;
                    }
                    Err(error) => warn!("Skipping unreadable XBT header record: {error}"),
                }
            }
            info!("Loaded {loaded} XBT headers from {}", path.display());
        }
        store.file = Some(OpenOptions::new().create(true).append(true).open(path)?);
        Ok(store)
    }

    pub fn network(&self) -> Network {
        self.network
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn contains(&self, hash: &BlockHash) -> bool {
        self.entries.contains_key(hash)
    }

    pub fn get(&self, hash: &BlockHash) -> Option<&HeaderEntry> {
        self.entries.get(hash)
    }

    pub fn local_tip(&self) -> Option<BlockHash> {
        self.local_tip
    }

    pub fn set_local_tip(&mut self, tip: BlockHash) {
        if self.entries.contains_key(&tip) {
            self.local_tip = Some(tip);
        }
    }

    fn insert_entry(&mut self, entry: HeaderEntry) -> bool {
        if self.entries.contains_key(&entry.hash) {
            return false;
        }
        self.entries.insert(entry.hash, entry);
        self.children.entry(entry.prev).or_default().push(entry.hash);
        self.floor_height = Some(
            self.floor_height
                .map_or(entry.height, |floor| floor.min(entry.height)),
        );
        self.eligibility_cache = None;
        true
    }

    fn persist_record(&mut self, kind: u8, height: u32, bytes: &[u8]) {
        if let Some(file) = self.file.as_mut() {
            let mut record = Vec::with_capacity(5 + bytes.len());
            record.push(kind);
            record.extend_from_slice(&height.to_le_bytes());
            record.extend_from_slice(bytes);
            if let Err(error) = file.write_all(&record) {
                warn!("Failed to persist XBT header: {error}");
            }
        }
    }

    fn persist(&mut self, header: &XbtHeader, height: u32) {
        self.persist_record(RECORD_XBT, height, &header.to_bytes());
    }

    /// Add a header the local Knots node reported, with the height it gave. Not checked: the
    /// local node is trusted for its own chain.
    pub fn insert_trusted(&mut self, header: XbtHeader, height: u32) -> bool {
        let added = self.insert_entry(HeaderEntry::from_xbt(header, height));
        if added {
            self.persist(&header, height);
        }
        added
    }

    /// Add a pre-BLAKE2b (80-byte) header the local Knots node reported.
    pub fn insert_trusted_legacy(&mut self, header: &bitcoin::block::Header, height: u32) -> bool {
        let added = self.insert_entry(HeaderEntry::from_legacy(header, height));
        if added {
            self.persist_record(
                RECORD_LEGACY,
                height,
                &bitcoin::consensus::serialize(header),
            );
        }
        added
    }

    /// The ancestor of `hash` at `height`, walking parent links in the store.
    pub fn ancestor_at(&self, hash: &BlockHash, height: u32) -> Option<&HeaderEntry> {
        let mut entry = self.entries.get(hash)?;
        if entry.height < height {
            return None;
        }
        while entry.height > height {
            entry = self.entries.get(&entry.prev)?;
        }
        Some(entry)
    }

    /// The `bits` XBT's difficulty rule requires of a child of `parent` with time `time`
    /// (Knots-BLAKE2b `GetNextWorkRequired`).
    pub fn required_bits(
        &self,
        parent: &HeaderEntry,
        time: u32,
    ) -> Result<CompactTarget, HeaderCheckError> {
        let params = &self.params;
        let interval = params.interval();
        let next_height = parent.height + 1;
        let limit_compact = params.pow_limit_compact();
        let mut bits;
        if next_height % interval != 0 {
            if params.allow_min_difficulty_blocks {
                if i64::from(time) > i64::from(parent.time) + params.target_spacing * 2 {
                    // Knots returns here, before the BLAKE2b shift.
                    return Ok(limit_compact);
                }
                let mut entry = parent;
                while entry.height % interval != 0 && entry.bits == limit_compact {
                    match self.entries.get(&entry.prev) {
                        Some(previous) => entry = previous,
                        None => break,
                    }
                }
                bits = entry.bits;
            } else {
                bits = parent.bits;
            }
        } else if params.no_retargeting {
            // Knots' CalculateNextWorkRequired returns the parent's bits; no ancestor is needed.
            bits = parent.bits;
        } else {
            let first_height = parent.height + 1 - interval;
            let first = self
                .ancestor_at(&parent.hash, first_height)
                .ok_or(HeaderCheckError::MissingAncestor)?;
            bits = self.calculate_next_work(parent, first);
        }
        if next_height == params.blake2b_height {
            bits = apply_blake2b_shift(bits, params);
        }
        Ok(bits)
    }

    /// Knots' `CalculateNextWorkRequired`.
    fn calculate_next_work(&self, last: &HeaderEntry, first: &HeaderEntry) -> CompactTarget {
        let params = &self.params;
        if params.no_retargeting {
            return last.bits;
        }
        let actual = (i64::from(last.time) - i64::from(first.time))
            .clamp(params.target_timespan / 4, params.target_timespan * 4);
        let base_bits = if params.enforce_bip94 {
            first.bits
        } else {
            last.bits
        };
        let mut target = to_u320(Target::from_compact(base_bits));
        target = target * U320::from(actual as u64);
        target = target / U320::from(params.target_timespan as u64);
        let limit = to_u320(params.pow_limit);
        if target > limit {
            target = limit;
        }
        from_u320(target).to_compact_lossy()
    }

    /// Check one carried header against the store (see the module docs).
    pub fn check_header(&self, header: &XbtHeader, now: u64) -> Result<u32, HeaderCheckError> {
        let parent = self
            .entries
            .get(&header.prev_blockhash)
            .ok_or(HeaderCheckError::UnknownParent)?;
        if u64::from(header.time) > now + MAX_FUTURE_SECONDS {
            return Err(HeaderCheckError::TooFarInFuture);
        }
        let required = self.required_bits(parent, header.time)?;
        if header.bits != required {
            return Err(HeaderCheckError::WrongBits {
                required: required.to_consensus(),
                got: header.bits.to_consensus(),
            });
        }
        let height = parent.height + 1;
        // BIP94: the first block of a period may not be more than 600 s before its parent.
        if self.params.enforce_bip94
            && height % self.params.interval() == 0
            && i64::from(header.time) < i64::from(parent.time) - 600
        {
            return Err(HeaderCheckError::TimeWarp);
        }
        if !header.meets(Target::from_compact(header.bits)) {
            return Err(HeaderCheckError::BadProofOfWork);
        }
        Ok(height)
    }

    /// Check carried headers and add those that pass. Order does not matter: headers are taken
    /// as their parents become known. Returns how many were added.
    pub fn absorb_carried(&mut self, headers: &[XbtHeader], now: u64) -> usize {
        let mut pending: Vec<&XbtHeader> = headers
            .iter()
            .filter(|header| {
                let hash = header.block_hash();
                !self.entries.contains_key(&hash) && !self.failed.contains(&hash)
            })
            .collect();
        let mut added = 0;
        loop {
            let mut progressed = false;
            let mut still_pending = Vec::new();
            for header in pending {
                if !self.entries.contains_key(&header.prev_blockhash) {
                    still_pending.push(header);
                    continue;
                }
                match self.check_header(header, now) {
                    Ok(height) => {
                        if self.insert_entry(HeaderEntry::from_xbt(*header, height)) {
                            self.persist(header, height);
                            added += 1;
                        }
                        progressed = true;
                    }
                    Err(error) => {
                        debug!("Refusing carried XBT header {}: {error}", header.block_hash());
                        if self.failed.len() >= FAILED_CACHE_SIZE {
                            self.failed.clear();
                        }
                        self.failed.insert(header.block_hash());
                    }
                }
            }
            pending = still_pending;
            if !progressed || pending.is_empty() {
                break;
            }
        }
        added
    }

    /// Whether `hash` is on the local Knots node's active chain, as last seen.
    fn on_local_chain(&self, hash: &BlockHash) -> bool {
        let (Some(tip), Some(entry)) = (self.local_tip, self.entries.get(hash)) else {
            return false;
        };
        self.ancestor_at(&tip, entry.height)
            .is_some_and(|ancestor| ancestor.hash == *hash)
    }

    /// The headers a share on `btc_prev` carries, oldest first: the path back from `btc_prev`
    /// to the local active chain, plus a few headers past it.
    pub fn carried_for(&self, btc_prev: &BlockHash) -> Vec<XbtHeader> {
        let mut out = Vec::new();
        let mut current = self.entries.get(btc_prev);
        let mut past_active = None;
        while let Some(entry) = current {
            if out.len() >= MAX_CARRIED_HEADERS_PER_SHARE {
                break;
            }
            // Pre-BLAKE2b headers are never carried; every node's Knots has them.
            let Some(header) = entry.header else {
                break;
            };
            out.push(header);
            if past_active.is_none() && self.on_local_chain(&entry.hash) {
                past_active = Some(0usize);
            }
            if let Some(count) = past_active.as_mut() {
                if *count >= CARRY_PAST_ACTIVE_CHAIN {
                    break;
                }
                *count += 1;
            }
            current = self.entries.get(&entry.prev);
        }
        out.reverse();
        out
    }

    /// The blocks eligible for a share whose XBT prev is `paying`.
    pub fn eligibility(&mut self, paying: &BlockHash) -> Result<Eligibility, EligibilityError> {
        if let Some(cached) = &self.eligibility_cache
            && cached.paying == *paying
        {
            return Ok(cached.clone());
        }
        let mut entry = self
            .entries
            .get(paying)
            .ok_or(EligibilityError::UnknownPayingPrev(*paying))?;
        let depth = eligibility_depth(self.network);
        let mut eligible = HashSet::new();
        let mut walked = 0u32;
        loop {
            let hash = entry.hash;
            eligible.insert(hash);
            if let Some(children) = self.children.get(&hash) {
                eligible.extend(children.iter().copied());
            }
            walked += 1;
            if walked > depth || entry.height == 0 {
                break;
            }
            match self.entries.get(&entry.prev) {
                Some(parent) => entry = parent,
                None => {
                    return Err(EligibilityError::StoreTooShallow {
                        paying: *paying,
                        reached_height: entry.height,
                    });
                }
            }
        }
        let eligibility = Eligibility {
            paying: *paying,
            eligible,
        };
        self.eligibility_cache = Some(eligibility.clone());
        Ok(eligibility)
    }
}

// ---------------------------------------------------------------------------------------------
// The process-wide store. Under `cfg(test)` it is per thread, so unit tests stay independent.
// ---------------------------------------------------------------------------------------------

#[cfg(not(test))]
static STORE: std::sync::OnceLock<std::sync::Mutex<HeaderStore>> = std::sync::OnceLock::new();

#[cfg(test)]
thread_local! {
    static TEST_STORE: std::cell::RefCell<Option<HeaderStore>> = const { std::cell::RefCell::new(None) };
}

/// Run `f` on the store, or return `None` when the rule is not active.
pub fn with_store<R>(f: impl FnOnce(&mut HeaderStore) -> R) -> Option<R> {
    #[cfg(not(test))]
    {
        STORE
            .get()
            .map(|store| f(&mut store.lock().expect("XBT header store lock poisoned")))
    }
    #[cfg(test)]
    {
        TEST_STORE.with(|cell| cell.borrow_mut().as_mut().map(f))
    }
}

/// Whether the rule is active (the node has opened its header store).
pub fn is_active() -> bool {
    with_store(|_| ()).is_some()
}

/// Open the header store at `path` and make the rule active for this process.
pub fn init(network: Network, path: &Path) -> std::io::Result<()> {
    let store = HeaderStore::open(network, path)?;
    #[cfg(not(test))]
    {
        if STORE.set(std::sync::Mutex::new(store)).is_err() {
            warn!("XBT header store already initialised");
        }
    }
    #[cfg(test)]
    {
        TEST_STORE.with(|cell| *cell.borrow_mut() = Some(store));
    }
    Ok(())
}

/// Tests: activate the rule on this thread with an in-memory store.
#[cfg(test)]
pub fn init_for_test(store: HeaderStore) {
    TEST_STORE.with(|cell| *cell.borrow_mut() = Some(store));
}

/// Tests: deactivate the rule on this thread.
#[cfg(test)]
pub fn clear_for_test() {
    TEST_STORE.with(|cell| *cell.borrow_mut() = None);
}

/// The store file for a share store at `store_path`.
pub fn store_file_for(store_path: &str) -> PathBuf {
    let trimmed = store_path.trim_end_matches('/');
    PathBuf::from(format!("{trimmed}.xbt-headers"))
}

fn now_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

/// Whether the XBT prev `hash` is known. True when the rule is not active.
pub fn knows(hash: &BlockHash) -> bool {
    with_store(|store| store.contains(hash)).unwrap_or(true)
}

/// Check and add carried headers. Returns how many were added (0 when inactive).
pub fn absorb(headers: &[XbtHeader]) -> usize {
    if headers.is_empty() {
        return 0;
    }
    let now = now_seconds();
    with_store(|store| store.absorb_carried(headers, now)).unwrap_or(0)
}

/// The headers a share on `btc_prev` carries (none when inactive).
pub fn carried_for(btc_prev: &BlockHash) -> Vec<XbtHeader> {
    with_store(|store| store.carried_for(btc_prev)).unwrap_or_default()
}

/// The headers a batch of shares carries, deduplicated, oldest first.
pub fn carried_for_batch<'a>(btc_prevs: impl IntoIterator<Item = &'a BlockHash>) -> Vec<XbtHeader> {
    with_store(|store| {
        let mut seen_prevs = HashSet::new();
        let mut seen_headers = HashSet::new();
        let mut out: Vec<(u32, XbtHeader)> = Vec::new();
        for prev in btc_prevs {
            if !seen_prevs.insert(*prev) {
                continue;
            }
            for header in store.carried_for(prev) {
                if seen_headers.insert(header.block_hash()) {
                    let height = store.get(&header.block_hash()).map_or(0, |entry| entry.height);
                    out.push((height, header));
                }
            }
            if out.len() >= MAX_CARRIED_HEADERS_PER_BATCH {
                break;
            }
        }
        out.sort_by_key(|(height, _)| *height);
        out.truncate(MAX_CARRIED_HEADERS_PER_BATCH);
        out.into_iter().map(|(_, header)| header).collect()
    })
    .unwrap_or_default()
}

/// The eligibility set for paying XBT prev `paying`. `None` when the rule is not active.
pub fn eligibility(paying: &BlockHash) -> Option<Result<Eligibility, EligibilityError>> {
    with_store(|store| store.eligibility(paying))
}

/// Feed the store from the local Knots node: make sure `tip` and its ancestors (back to a known
/// header, or `seed_depth` blocks on a fresh store) are stored, and record `tip` as the local
/// active chain's tip. Does nothing when the rule is not active.
pub async fn sync_from_node(
    bitcoind: &bitcoindrpc::BitcoindRpcClient,
    tip: &BlockHash,
) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
    let Some((known, empty, network)) =
        with_store(|store| (store.contains(tip), store.is_empty(), store.network()))
    else {
        return Ok(0);
    };
    if known {
        with_store(|store| store.set_local_tip(*tip));
        return Ok(0);
    }
    let verbose: serde_json::Value = bitcoind
        .request("getblockheader", vec![serde_json::json!(tip.to_string()), serde_json::json!(true)])
        .await?;
    let tip_height = verbose
        .get("height")
        .and_then(|height| height.as_u64())
        .ok_or("getblockheader gave no height")? as u32;
    let limit = if empty { seed_depth(network) } else { u32::MAX };
    let mut fetched: Vec<(HeaderEntry, Vec<u8>)> = Vec::new();
    let mut hash = *tip;
    let mut height = tip_height;
    loop {
        if with_store(|store| store.contains(&hash)).unwrap_or(true) {
            break;
        }
        let raw: String = bitcoind
            .request("getblockheader", vec![serde_json::json!(hash.to_string()), serde_json::json!(false)])
            .await?;
        let bytes = hex_bytes(&raw).ok_or("getblockheader gave bad hex")?;
        let entry = match bytes.len() {
            XBT_HEADER_SIZE => {
                let array: [u8; XBT_HEADER_SIZE] = bytes.as_slice().try_into().expect("length");
                let header = XbtHeader::from_bytes(&array).map_err(|error| error.to_string())?;
                HeaderEntry::from_xbt(header, height)
            }
            LEGACY_HEADER_SIZE => {
                let header: bitcoin::block::Header = bitcoin::consensus::deserialize(&bytes)
                    .map_err(|error| error.to_string())?;
                HeaderEntry::from_legacy(&header, height)
            }
            other => return Err(format!("getblockheader gave a {other}-byte header").into()),
        };
        if entry.hash != hash {
            return Err(format!("getblockheader {hash} returned a header hashing elsewhere").into());
        }
        let prev = entry.prev;
        fetched.push((entry, bytes));
        if height == 0 || fetched.len() as u32 >= limit || prev == BlockHash::all_zeros() {
            break;
        }
        hash = prev;
        height -= 1;
    }
    let count = fetched.len();
    with_store(|store| {
        for (entry, _bytes) in fetched.into_iter().rev() {
            match entry.header {
                Some(header) => {
                    store.insert_trusted(header, entry.height);
                }
                None => {
                    if store.insert_entry(entry) {
                        store.persist_record(RECORD_LEGACY, entry.height, &_bytes);
                    }
                }
            }
        }
        store.set_local_tip(*tip);
    });
    if count > 0 {
        debug!("Stored {count} XBT headers from the local node up to {tip}");
    }
    Ok(count)
}

fn hex_bytes(s: &str) -> Option<Vec<u8>> {
    let s = s.trim();
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn regtest_genesis_like() -> XbtHeader {
        let mut header = XbtHeader::regtest_test_header();
        header.prev_blockhash = BlockHash::all_zeros();
        header.bits = CompactTarget::from_consensus(0x207fffff);
        header
    }

    fn child_of(parent: &XbtHeader, time: u32, salt: u32) -> XbtHeader {
        let mut header = *parent;
        header.prev_blockhash = parent.block_hash();
        header.time = time;
        header.nonce = salt;
        header.bits = CompactTarget::from_consensus(0x207fffff);
        assert!(header.grind(Target::from_compact(header.bits), 1_000_000));
        header
    }

    fn store_with_chain(n: usize) -> (HeaderStore, Vec<XbtHeader>) {
        let mut store = HeaderStore::new(Network::Regtest);
        let genesis = regtest_genesis_like();
        store.insert_trusted(genesis, 0);
        let mut chain = vec![genesis];
        for i in 1..n {
            let next = child_of(chain.last().unwrap(), 1_000 + i as u32 * 600, i as u32);
            store.insert_trusted(next, i as u32);
            chain.push(next);
        }
        store.set_local_tip(chain.last().unwrap().block_hash());
        (store, chain)
    }

    #[test]
    fn carried_header_with_known_parent_is_added() {
        let (mut store, chain) = store_with_chain(5);
        let next = child_of(chain.last().unwrap(), 10_000, 99);
        assert_eq!(store.absorb_carried(&[next], 20_000), 1);
        assert_eq!(store.get(&next.block_hash()).unwrap().height, 5);
    }

    #[test]
    fn carried_header_with_unknown_parent_is_refused() {
        let (mut store, _) = store_with_chain(3);
        let mut orphan = regtest_genesis_like();
        orphan.prev_blockhash = BlockHash::from_byte_array([0x5e; 32]);
        assert_eq!(store.absorb_carried(&[orphan], 20_000), 0);
        assert!(!store.contains(&orphan.block_hash()));
    }

    #[test]
    fn carried_header_with_wrong_bits_is_refused() {
        let (store, chain) = store_with_chain(3);
        let mut next = *chain.last().unwrap();
        next.prev_blockhash = chain.last().unwrap().block_hash();
        next.bits = CompactTarget::from_consensus(0x1f7fffff);
        assert!(next.grind(Target::from_compact(next.bits), 10_000_000));
        assert!(matches!(
            store.check_header(&next, 20_000),
            Err(HeaderCheckError::WrongBits { .. })
        ));
    }

    #[test]
    fn carried_header_far_in_the_future_is_refused() {
        let (store, chain) = store_with_chain(3);
        let next = child_of(chain.last().unwrap(), 100_000, 7);
        assert_eq!(
            store.check_header(&next, 100_000 - 3 * 60 * 60),
            Err(HeaderCheckError::TooFarInFuture)
        );
    }

    #[test]
    fn carried_headers_out_of_order_link_up() {
        let (mut store, chain) = store_with_chain(3);
        let a = child_of(chain.last().unwrap(), 5_000, 1);
        let b = child_of(&a, 5_600, 2);
        assert_eq!(store.absorb_carried(&[b, a], 20_000), 2);
    }

    #[test]
    fn eligibility_pays_the_paying_chain_and_one_block_off() {
        let (mut store, chain) = store_with_chain(6);
        // A fork off chain[3]: f1 (one off), f2 (two off).
        let f1 = child_of(&chain[3], 9_000, 501);
        let f2 = child_of(&f1, 9_600, 502);
        assert_eq!(store.absorb_carried(&[f1, f2], 20_000), 2);
        let paying = chain[5].block_hash();
        let eligibility = store.eligibility(&paying).unwrap();
        for header in &chain {
            assert!(eligibility.is_eligible(&header.block_hash()));
        }
        assert!(eligibility.is_eligible(&f1.block_hash()));
        assert!(!eligibility.is_eligible(&f2.block_hash()));
        assert!(!eligibility.is_eligible(&BlockHash::from_byte_array([0x5e; 32])));
    }

    #[test]
    fn eligibility_is_measured_from_the_paying_prev() {
        let (mut store, chain) = store_with_chain(6);
        // Paying on chain[2]: chain[4] and chain[5] are newer and not on that chain; chain[3] is
        // one block off (a child of chain[2]).
        let eligibility = store.eligibility(&chain[2].block_hash()).unwrap();
        assert!(eligibility.is_eligible(&chain[3].block_hash()));
        assert!(!eligibility.is_eligible(&chain[4].block_hash()));
    }

    #[test]
    fn unknown_paying_prev_is_an_error() {
        let (mut store, _) = store_with_chain(2);
        let unknown = BlockHash::from_byte_array([0x5e; 32]);
        assert_eq!(
            store.eligibility(&unknown).unwrap_err(),
            EligibilityError::UnknownPayingPrev(unknown)
        );
    }

    #[test]
    fn shallow_store_is_an_error_not_a_verdict() {
        let mut store = HeaderStore::new(Network::Regtest);
        let genesis = regtest_genesis_like();
        let a = child_of(&genesis, 2_000, 1);
        let b = child_of(&a, 2_600, 2);
        // Seeded from height 10 up, with no genesis: the walk runs off the store.
        store.insert_trusted(a, 10);
        store.insert_trusted(b, 11);
        assert!(matches!(
            store.eligibility(&b.block_hash()),
            Err(EligibilityError::StoreTooShallow { .. })
        ));
    }

    #[test]
    fn carried_for_reaches_back_past_the_local_chain() {
        let (mut store, chain) = store_with_chain(8);
        let o = child_of(&chain[6], 20_000, 77); // an orphan sibling of chain[7]
        store.absorb_carried(&[o], 30_000);
        let carried = store.carried_for(&o.block_hash());
        // The orphan, then chain[6] (on the local chain) and three more below it.
        assert_eq!(carried.last().unwrap().block_hash(), o.block_hash());
        assert_eq!(carried.len(), 1 + 1 + CARRY_PAST_ACTIVE_CHAIN);
        assert_eq!(carried[0].block_hash(), chain[3].block_hash());
    }

    #[test]
    fn store_file_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("h.xbt-headers");
        let (_, chain) = store_with_chain(4);
        {
            let mut store = HeaderStore::open(Network::Regtest, &path).unwrap();
            for (height, header) in chain.iter().enumerate() {
                store.insert_trusted(*header, height as u32);
            }
        }
        let reopened = HeaderStore::open(Network::Regtest, &path).unwrap();
        assert_eq!(reopened.len(), 4);
        assert_eq!(reopened.get(&chain[3].block_hash()).unwrap().height, 3);
    }

    fn mainnet_entry(bits: u32, time: u32, height: u32) -> HeaderEntry {
        let mut header = XbtHeader::regtest_test_header();
        header.bits = CompactTarget::from_consensus(bits);
        header.time = time;
        HeaderEntry::from_xbt(header, height)
    }

    #[test]
    fn mainnet_mid_period_requires_the_parent_bits() {
        let store = HeaderStore::new(Network::Bitcoin);
        let parent = mainnet_entry(0x1703_0ecd, 1_700_000_000, 800_001);
        assert_eq!(
            store.required_bits(&parent, 1_700_000_600).unwrap().to_consensus(),
            0x1703_0ecd
        );
    }

    #[test]
    fn mainnet_blake2b_activation_shifts_the_target() {
        let store = HeaderStore::new(Network::Bitcoin);
        // 961,640 is not a retarget height (961,640 % 2016 = 8), so the parent's bits are
        // shifted 22 bits easier.
        let parent = mainnet_entry(0x1702_3a04, 1_790_000_000, 961_639);
        let required = store.required_bits(&parent, 1_790_000_600).unwrap();
        let expected = from_u320(to_u320(Target::from_compact(CompactTarget::from_consensus(
            0x1702_3a04,
        ))) << 22)
        .to_compact_lossy();
        assert_eq!(required, expected);
        assert_ne!(required.to_consensus(), 0x1702_3a04);
    }

    #[test]
    fn retarget_is_clamped_to_four_times() {
        // Use calculate_next_work directly with a period that took a tiny time.
        let store = HeaderStore::new(Network::Bitcoin);
        let last = mainnet_entry(0x1d00_ffff, 1_000_100, 2015);
        let first = mainnet_entry(0x1d00_ffff, 1_000_000, 0);
        let bits = store.calculate_next_work(&last, &first);
        let expected = from_u320(
            to_u320(Target::from_compact(CompactTarget::from_consensus(0x1d00_ffff))) / U320::from(4u64),
        )
        .to_compact_lossy();
        assert_eq!(bits, expected);
    }

    #[test]
    fn regtest_always_requires_the_limit() {
        let store = HeaderStore::new(Network::Regtest);
        let parent = mainnet_entry(0x207f_ffff, 1_000, 143);
        assert_eq!(store.required_bits(&parent, 1_600).unwrap().to_consensus(), 0x207f_ffff);
    }

    #[test]
    fn share_block_carries_headers_on_the_wire() {
        let (_, chain) = store_with_chain(3);
        let mut block = crate::test_utils::TestShareBlockBuilder::new().build();
        let hash = block.block_hash();
        block.carried_xbt_headers = chain.clone();
        let bytes = bitcoin::consensus::encode::serialize(&block);
        let decoded: crate::shares::share_block::ShareBlock =
            bitcoin::consensus::encode::deserialize(&bytes).unwrap();
        assert_eq!(decoded.carried_xbt_headers, chain);
        // Carried headers are not part of the share's hash.
        assert_eq!(decoded.block_hash(), hash);
    }

    #[test]
    fn share_header_batch_carries_headers_on_the_wire() {
        use crate::node::messages::{Message, ShareHeaderBatch};
        let (_, chain) = store_with_chain(3);
        let block = crate::test_utils::TestShareBlockBuilder::new().build();
        let batch = ShareHeaderBatch::from_headers_with_branches(vec![(block.header, vec![])])
            .with_carried_xbt_headers(chain.clone());
        let bytes = bitcoin::consensus::encode::serialize(&Message::ShareHeaders(batch));
        let decoded: Message = bitcoin::consensus::encode::deserialize(&bytes).unwrap();
        match decoded {
            Message::ShareHeaders(batch) => assert_eq!(batch.carried_xbt_headers(), &chain[..]),
            other => panic!("expected ShareHeaders, got {other}"),
        }
    }

    #[test]
    fn legacy_headers_link_and_are_not_carried() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("l.xbt-headers");
        let legacy = bitcoin::blockdata::constants::genesis_block(Network::Regtest).header;
        let mut first = regtest_genesis_like();
        first.prev_blockhash = legacy.block_hash();
        assert!(first.grind(Target::from_compact(first.bits), 1_000_000));
        {
            let mut store = HeaderStore::open(Network::Regtest, &path).unwrap();
            store.insert_trusted_legacy(&legacy, 0);
            store.insert_trusted(first, 1);
            store.set_local_tip(first.block_hash());
            let carried = store.carried_for(&first.block_hash());
            assert_eq!(carried.len(), 1);
            let next = child_of(&first, 5_000, 3);
            assert_eq!(store.absorb_carried(&[next], 10_000), 1);
            assert!(store.eligibility(&next.block_hash()).is_ok());
        }
        let reopened = HeaderStore::open(Network::Regtest, &path).unwrap();
        assert_eq!(reopened.len(), 3);
        assert!(reopened.get(&legacy.block_hash()).unwrap().header.is_none());
    }

    #[test]
    fn inactive_rule_knows_everything() {
        clear_for_test();
        assert!(!is_active());
        assert!(knows(&BlockHash::from_byte_array([0x5e; 32])));
        assert!(eligibility(&BlockHash::all_zeros()).is_none());
    }
}
