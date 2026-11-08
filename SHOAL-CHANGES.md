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

Known open items (tracked in Shoal's plan.md, milestone M1): v2 fields from the real Stratum job,
an XBT sharechain genesis, regenerated test fixtures, and Sia-dialect Stratum V1 for BLAKE2b ASICs.
