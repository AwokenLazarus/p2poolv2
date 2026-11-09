# Shoal changes to P2Poolv2

This repository is a fork of [P2Poolv2](https://github.com/p2poolv2/p2poolv2), maintained by the
[Shoal](https://github.com/AwokenLazarus/Shoal) project. Shoal adapts it for XBT (Bitcoin on
BLAKE2b, run by Bitcoin Knots-BLAKE2b): a 164-byte block header (v2) and BLAKE2b proof of work.

**Not affiliated with or endorsed by the P2Poolv2 developers.** Report problems with this fork to
Shoal, not upstream.

## Licence
Unchanged: MIT OR Apache-2.0, at your option ([LICENSE-MIT](LICENSE-MIT),
[LICENSE-APACHE](LICENSE-APACHE)). Upstream's copyright notices, AUTHORS and SPDX headers are
kept as they were. Every file Shoal modified carries a "Modified by the Shoal project" notice
under its original header. Shoal's own changes are offered under the same terms.

## Base
Upstream tag `v0.16.3` (`a6f39146`). Shoal's work lives on `shoal/*` branches; `main` follows
upstream unchanged.

## Changes (branch `shoal/xbt-header`)
- The bitcoin header inside every share is `shoal_header::XbtHeader` (164 bytes, BLAKE2b PoW),
  from the Shoal repository, in place of rust-bitcoin's 80-byte `block::Header`. Found blocks are
  `shoal_header::XbtBlock`.
- The share header encoder/decoder carries the 84 header-v2 bytes after the legacy fields.
- `bitcoindrpc::submit_block` takes any consensus-encodable block.
- Tests: header literals gain a `v2` field; tests that used Bitcoin's regtest genesis header use
  `XbtHeader::regtest_test_header()`.

## Changes (milestone M1: XBT mining)
- Stratum speaks the Sia-profile dialect XBT ASICs use (via `shoal-stratum`): notify carries
  `000000 ‖ h2 ‖ 00000000` as coinbase1, with coinbase2, branches and version empty; submit carries
  an 8-byte extranonce2, ntime (`time_offset ‖ nonce3`) and nonce (`nonce ‖ nonce2`). New file
  `p2poolv2_lib/src/stratum/work/xbt.rs`.
- The extranonce is in the XBT header, so each job's coinbase keeps P2Poolv2's layout with the
  12 extranonce bytes fixed at zero. Padding, the share commitment and the midstate proof are
  unchanged.
- `getblocktemplate` requests the `blake2b` rule. The template's `version` arrives with the
  header-v2 flag set and is stripped on parse.
- Regtest genesis (`shares/genesis/regtest.dat`) anchors to a deterministic XBT v2 block made by
  Shoal's `cargo run -p shoal-header --example regtest_genesis`.

- Per-network sharechain parameters (new `p2poolv2_lib/src/xbt_params.rs`, set once at node
  start): regtest gets an easy share floor (`0x207fffff`), ASERT ceiling and genesis anchor
  (`0x1e0fffff`), so CPU miners can produce shares on regtest. Every other network keeps
  upstream's `MAX_POOL_TARGET`, ASERT ceiling and anchor unchanged.

- No built-in DNS seeds: upstream's `seed.testnet4.p2poolv2.org` is a Bitcoin P2Poolv2 host that
  an XBT node must never query. Shoal seeds come with its first public testnet; until then peers
  are configured with `dial_peers`.
- Testnet4 and mainnet keep upstream's Bitcoin genesis anchors on purpose: an XBT node refuses
  them (v1 header), so Shoal cannot run on those networks until XBT anchors are chosen.

## Changes (milestone M2: three nodes)
- Regtest PPLNS window of 12 shares with a maturity of 4 (`xbt_params::pplns_window_shares` and
  `coinbase_maturity`), so a test run of a few dozen shares slides the window and exercises the
  maturity skip. Every other network keeps upstream's 120,960 and 6,048.
- On regtest the PPLNS window cache keeps at least 64 entries past the window
  (`REGTEST_MIN_WINDOW_BUFFER`), so the short window still pays for forks a few shares deep.
  Other networks keep upstream's 1% buffer.

## Changes (milestone M4: latency and uncles)
- On regtest, stratum difficulty is in units of 1/1024 of difficulty 1
  (`xbt_params::stratum_difficulty_divisor`): `mining.set_difficulty` sends the fractional value
  (difficulty 1 goes out as 0.0009765625) and the submit check scales the hash's difficulty to
  match. CPU miners can then find 10 s shares. Every other network has a divisor of 1 and sends
  upstream's integers unchanged.
- A regtest node can take its genesis share's timestamp from `SHOAL_REGTEST_GENESIS_TIME`
  (`xbt_params::init_regtest_genesis_time_from_env`, read once at start). The genesis time is
  the ASERT anchor, and the fixed regtest genesis is months old, which pins the share target at
  its easiest. Test runs set it to their start time. `SHOAL_REGTEST_GENESIS_BITS` likewise sets
  the genesis (ASERT anchor) target, so a run with several CPU miners starts near 10 s shares
  instead of climbing from the single-miner default. Both ignored on every other network.

## Changes (milestone M5: mine on the current XBT chain)
Upstream's design `docs/architecture/mine-on-bitcoin-chain.md` (issue #652), implemented for XBT:
- `p2poolv2_lib/src/xbt_chain.rs` (new): the XBT header store and the payout rule. The store is
  fed by the GBT poller (each template's parent, walked back to a stored header; a fresh store
  loads `seed_depth` headers) and by headers carried with shares, and is kept in
  `<store path>.xbt-headers`. A carried header must link to a stored one, meet its PoW, have
  exactly the `bits` XBT's difficulty rule gives (Knots-BLAKE2b `GetNextWorkRequired`: Bitcoin's
  2,016-block retarget clamped at 4x, min-difficulty blocks on test networks, BIP94 on testnet4,
  and the one-off `Blake2bTargetShift` at the BLAKE2b activation height; regtest does not
  retarget) and be at most 2 hours ahead of the clock.
- `ShareBlock` and `ShareHeaderBatch` carry XBT headers (`carried_xbt_headers`, raw 164-byte
  headers, capped at 100 and 500), outside every share hash. Senders attach the path from each
  share's XBT prev back to their node's active chain plus 3 headers.
- Acceptance: `handle_share_block` and `handle_share_headers` check carried headers, then drop a
  share whose XBT prev is still unknown (a batch stops at the first such share). The genesis
  share is exempt.
- Payout rule: `PplnsWindow::get_distribution_for_paying` pays a window share, or uncle, only if
  its XBT prev is on the paying share's XBT chain or one block off it. Ineligible shares keep
  their slot and weigh 0; a nephew gets no bonus for an ineligible uncle. A window with shares
  but none eligible is `NothingEligible`, a consensus failure in `validate_bitcoin_payout`. The
  job builder passes the template's XBT prev (`PayoutDistribution::set_paying_btc_prev`), the
  validator the share's own.
- Active once the node opens the header store at start-up; unit tests that do not open one see
  upstream's behaviour. The attack test build does not open it (it plays a node without the rule).

## Attack test builds (milestone M5)
- Cargo feature `shoal-attack-dead-template` (lib and node): every job builds on a fabricated XBT
  parent block, for the regtest test that shows what the sharechain pays for work that can never
  become a block. Refuses to run off regtest. Never enabled by default or in a release build.
- Cargo feature `shoal-attack` (lib and node), mode from `SHOAL_ATTACK` (`p2poolv2_lib/src/attack.rs`):
  `withhold` (never submit blocks), `wrong-split` (coinbase pays all to `SHOAL_ATTACK_ADDRESS`, own
  payout check skipped), `copy` (rebroadcast each share with a changed coinbase and under the
  attacker's address) and `private-fork` (ignore peers and hold own shares back between
  `SHOAL_ATTACK_PRIVATE_FROM` and `_UNTIL` seconds, then release). Regtest only; never default.
- Sharechain reorg alarm (`store/organise/confirmed.rs`): a reorg replacing 6 or more confirmed
  shares logs `SHARECHAIN REORG ALARM` at warn level. Active in every build.

## Test fixtures (XBT)
Regenerated with `p2poolv2_lib/src/xbt_fixtures.rs` (ignored tests, run by hand) and Shoal's
`remine_session` example. Every proof of work in them is real BLAKE2b:
- `validation/share_headers.json`: each bitcoin header converted to v2 and re-mined to the target
  its share declares (the deliberately unmeetable `tight_target_header` is not mined).
- `share_sync/*.json`: the first six shares of a real Shoal regtest run (M2: three nodes and
  three CPU miners on xbt-lab, 2026-11-08), mined with regtest sharechain parameters, including
  the 12-share window and maturity 4; their tests run in regtest mode.
- `validation/stratum/{a,b,with_version_rolling}/submit.json`: re-mined Sia-dialect submissions
  for each recorded session's job (template, coinbase halves and extranonce1 unchanged).
- Notify tests assert the XBT dialect and read the coinbase halves from the tracked job.
- Two DNS-seed tests are ignored with a reason (no Shoal seeds yet).
- Other test ports: stratum submit tests send XBT-format fields (bad-nonce and missing-version-bit
  cases assert XBT behaviour); `build_pool_job_submit` and the session share builder in
  `test_utils.rs`, and the `sim` share builder, assemble XBT headers; the three-node share-sync
  integration test runs regtest parameters for its regtest fixture chain.
- `p2poolv2_lib` re-exports `shoal_header`, so dependent crates use one version.

Test status (2026-11-09, xbt-lab): `cargo test --workspace` 1,613 passed, 0 failed, 6 ignored
(fixture tools and DNS-seed tests), with Shoal crates fetched from GitHub at the pinned rev.

Known open items: XBT testnet4 anchor and Shoal DNS seeds (phase 3).
