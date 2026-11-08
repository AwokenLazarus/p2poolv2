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

Known open items (Shoal plan.md, M1): test fixtures regenerated as XBT shares, and the
Bitcoin-format stratum and notify tests rewritten for the XBT dialect.
