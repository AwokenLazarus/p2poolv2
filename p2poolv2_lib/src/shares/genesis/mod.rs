// SPDX-FileCopyrightText: 2024-2026 P2Poolv2 Developers (see AUTHORS)
//
// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Modified by the Shoal project (AwokenLazarus and Shoal contributors), 2026:
// adapted for XBT (Bitcoin on BLAKE2b, 164-byte header v2). See SHOAL-CHANGES.md.

use std::error::Error;

/// Genesis block data captures the first block in the share chain
///
/// It captures the bitcoin block the work was done, and the work done
/// with certain difficulty.
///
/// This genesis block does _not_ mean the share block met bitcoin
/// difficulty. It just captures where we started from and keeps all
/// nodes build on the same chain.
pub struct GenesisData {
    /// The public key hex string used to derive the miner's bitcoin
    /// address for the genesis share block coinbase
    pub public_key: &'static str,
    /// The consensus serialized bitcoin block for the share, embedded
    /// from a raw `.dat` file, e.g. the output of
    /// `bitcoin-cli getblock <hash> 0 | xxd -r -p`
    pub bitcoin_block: &'static [u8],
    /// Bitcoin header height
    pub bitcoin_height: u64,
    /// Unix timestamp for the genesis share block
    pub timestamp: u32,
}

// Bitcoin signet genesis block (height 0), hash
// 00000008819873e925422c1ff0f99f7cc9bbb232af63a077a480a3633bee1ef6.
// Shoal: an XBT v2 block; see test_signet_genesis_block_hash.
const SIGNET_GENESIS_DATA: GenesisData = GenesisData {
    public_key: "02ac493f2130ca56cb5c3a559860cef9a84f90b5a85dfe4ec6e6067eeee17f4d2d",
    bitcoin_block: include_bytes!("signet.dat"),
    bitcoin_height: 0,
    timestamp: 1776855600,
};

// XBT testnet4 block 154,600, hash
// 00000000dd483f2a5ed2a9f8ff481d9022827703a64df8686e8853a5ab67047f.
// Shoal: a real post-fork block (BLAKE2b, v2 header), the anchor of the trial testnet4 sharechain.
const TESTNET4_GENESIS_DATA: GenesisData = GenesisData {
    public_key: "02ac493f2130ca56cb5c3a559860cef9a84f90b5a85dfe4ec6e6067eeee17f4d2d",
    bitcoin_block: include_bytes!("testnet4.dat"),
    bitcoin_height: 154600,
    timestamp: 1794272400, // trial launch; set when the run starts
};

// Mainnet block hash 000000000000000000006fa29653a523d531657ad20d6f946d0fef88c97cd935,
// mined on previousblockhash 00000000000000000001414ee2ca686bd8dc13a40f02d94ff96cb1279dac64aa.
// Shoal: still upstream's Bitcoin anchor, deliberately: an XBT node refuses it (v1 header), so
// Shoal cannot run on mainnet until an XBT mainnet anchor is chosen.
const MAINNET_GENESIS_DATA: GenesisData = GenesisData {
    public_key: "02ac493f2130ca56cb5c3a559860cef9a84f90b5a85dfe4ec6e6067eeee17f4d2d",
    bitcoin_block: include_bytes!("main.dat"),
    bitcoin_height: 920527,
    timestamp: 1776855600,
};

// Bitcoin regtest genesis block (height 0), hash
// 0f9188f13cb7b2c71f2a335e3a4fc328bf5beb436012afca590b1a11466e2206.
// Used only as a fixed anchor for the share chain on regtest; it does
// not need to match the connected bitcoind's current tip.
// Shoal: an XBT v2 block; see test_regtest_genesis_block_hash.
const REGTEST_GENESIS_DATA: GenesisData = GenesisData {
    public_key: "02ac493f2130ca56cb5c3a559860cef9a84f90b5a85dfe4ec6e6067eeee17f4d2d",
    bitcoin_block: include_bytes!("regtest.dat"),
    bitcoin_height: 0,
    timestamp: 1776855600,
};

/// Get the genesis data for a given network
pub fn genesis_data(network: bitcoin::Network) -> Result<GenesisData, Box<dyn Error>> {
    match network {
        bitcoin::Network::Signet => Ok(SIGNET_GENESIS_DATA),
        bitcoin::Network::Testnet4 => Ok(TESTNET4_GENESIS_DATA),
        bitcoin::Network::Bitcoin => Ok(MAINNET_GENESIS_DATA),
        bitcoin::Network::Regtest => Ok(REGTEST_GENESIS_DATA),
        _ => Err("Unsupported network".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bitcoin::consensus::deserialize;
    use bitcoin::{Block, BlockHash};
    use std::str::FromStr;

    /// Shoal: XBT has no signet; Shoal's signet sharechain (used by upstream's tests) anchors to
    /// a deterministic XBT v2 block built like the regtest one, from Bitcoin's signet genesis
    /// coinbase, time and bits (`regtest_genesis -- signet.dat signet`).
    #[test]
    fn test_signet_genesis_block_hash() {
        let genesis = genesis_data(bitcoin::Network::Signet).unwrap();
        let block = deserialize::<shoal_header::XbtBlock>(genesis.bitcoin_block).unwrap();

        assert_eq!(
            block.block_hash(),
            BlockHash::from_str("00000058ff90205bf876b495ec72ead257297a64c3bd8478b4eda0b9a8b15bbc")
                .unwrap()
        );
        assert!(block.check_merkle_root());
        assert!(block.header.validate_pow(block.header.target()).is_ok());
    }

    #[test]
    fn test_testnet4_genesis_block_hash() {
        let genesis = genesis_data(bitcoin::Network::Testnet4).unwrap();
        let block = deserialize::<shoal_header::XbtBlock>(genesis.bitcoin_block).unwrap();

        assert_eq!(
            block.block_hash(),
            BlockHash::from_str("00000000dd483f2a5ed2a9f8ff481d9022827703a64df8686e8853a5ab67047f")
                .unwrap()
        );
        assert!(block.check_merkle_root());
        assert!(block.header.validate_pow(block.header.target()).is_ok());
    }

    #[test]
    fn test_mainnet_genesis_block_hash() {
        let genesis = genesis_data(bitcoin::Network::Bitcoin).unwrap();
        let block = deserialize::<Block>(genesis.bitcoin_block).unwrap();

        assert_eq!(
            block.block_hash(),
            BlockHash::from_str("000000000000000000006fa29653a523d531657ad20d6f946d0fef88c97cd935")
                .unwrap()
        );
        assert!(block.check_merkle_root());
    }

    /// Shoal: regtest anchors to a deterministic XBT v2 block (Bitcoin's regtest genesis
    /// coinbase under a BLAKE2b-mined 164-byte header), made by Shoal's
    /// `cargo run -p shoal-header --example regtest_genesis`.
    #[test]
    fn test_regtest_genesis_block_hash() {
        let genesis = genesis_data(bitcoin::Network::Regtest).unwrap();
        let block = deserialize::<shoal_header::XbtBlock>(genesis.bitcoin_block).unwrap();

        assert_eq!(
            block.block_hash(),
            BlockHash::from_str("6b25bc7efc91c1eafd86d97618dd024a868849889b7bd9c1008d72f0510dc77c")
                .unwrap()
        );
        assert!(block.check_merkle_root());
        assert!(block.header.validate_pow(block.header.target()).is_ok());
    }
}
