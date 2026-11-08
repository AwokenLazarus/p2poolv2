// SPDX-FileCopyrightText: 2024-2026 P2Poolv2 Developers (see AUTHORS)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Modified by the Shoal project (AwokenLazarus and Shoal contributors), 2026:
// adapted for XBT (Bitcoin on BLAKE2b, 164-byte header v2). See SHOAL-CHANGES.md.

pub mod block_template;
pub mod coinbase;
pub mod difficulty;
pub(crate) mod error;
pub mod gbt;
pub mod notify;
pub mod prepared_notify;
pub mod tracker;
pub mod xbt;
