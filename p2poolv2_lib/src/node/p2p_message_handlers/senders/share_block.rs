// SPDX-FileCopyrightText: 2024-2026 P2Poolv2 Developers (see AUTHORS)
//
// SPDX-License-Identifier: MIT OR Apache-2.0

use crate::node::messages::Message;
use crate::node::request_response_handler::peer_block_knowledge::PeerBlockKnowledge;
use crate::node::request_sender::RequestSender;
use crate::shares::share_block::ShareBlock;
use libp2p::PeerId;
use tracing::debug;

/// Broadcast a full ShareBlock to all connected peers.
///
/// Peers that are already known to have the block (tracked in
/// `peer_block_knowledge`) are skipped. Each recipient is recorded
/// in `peer_block_knowledge` so that subsequent broadcasts of the
/// same block are suppressed.
///
/// Sends directly via the swarm to avoid deadlocking the actor's
/// select loop (which would happen if we awaited on the bounded
/// swarm_tx channel that the actor also drains).
pub fn send_share_block_broadcast(
    mut share_block: ShareBlock,
    connected_peers: &[PeerId],
    peer_block_knowledge: &mut PeerBlockKnowledge,
    request_sender: &mut impl RequestSender,
) {
    let block_hash = share_block.block_hash();
    // Shoal: carry the XBT headers a receiver needs to place this share's XBT prev.
    if share_block.carried_xbt_headers.is_empty() {
        share_block.carried_xbt_headers =
            crate::xbt_chain::carried_for(&share_block.header.bitcoin_header.prev_blockhash);
    }

    // Shoal attack test (`private-fork`): hold shares back during the private period, then
    // release them all, oldest first.
    if crate::attack::is(crate::attack::Mode::PrivateFork) {
        let mut held = PRIVATE_HELD.lock().expect("held shares lock");
        if crate::attack::private_now() {
            held.push(share_block);
            return;
        }
        if !held.is_empty() {
            tracing::warn!("SHOAL ATTACK: releasing {} private shares", held.len());
            for block in held.drain(..) {
                for peer_id in connected_peers {
                    request_sender.send_request(peer_id, Message::ShareBlock(block.clone()));
                }
            }
        }
    }
    // Shoal attack test (`copy`): a bad copy of each share first (same share, changed
    // coinbase), and a copy under the attacker's address after it.
    let copies = if crate::attack::is(crate::attack::Mode::Copy) {
        let mut changed_coinbase = share_block.clone();
        if let Some(output) = changed_coinbase.bitcoin_coinbase.output.first_mut() {
            output.value = bitcoin::Amount::from_sat(output.value.to_sat().saturating_sub(1));
        }
        for peer_id in connected_peers {
            request_sender.send_request(peer_id, Message::ShareBlock(changed_coinbase.clone()));
        }
        crate::attack::address().map(|address| {
            let mut renamed = share_block.clone();
            renamed.header.miner_bitcoin_address = address;
            renamed
        })
    } else {
        None
    };

    for peer_id in connected_peers {
        if peer_block_knowledge.peer_knows_block(peer_id, &block_hash) {
            debug!("Skipping broadcast to peer {peer_id}: already knows block {block_hash}");
        } else {
            debug!("Broadcasting ShareBlock {block_hash} to peer {peer_id}");
            request_sender.send_request(peer_id, Message::ShareBlock(share_block.clone()));
            peer_block_knowledge.record_block_known(peer_id, block_hash);
        }
    }
    if let Some(renamed) = copies {
        for peer_id in connected_peers {
            request_sender.send_request(peer_id, Message::ShareBlock(renamed.clone()));
        }
    }
}

/// Shoal attack test (`private-fork`): shares held back during the private period.
static PRIVATE_HELD: std::sync::Mutex<Vec<ShareBlock>> = std::sync::Mutex::new(Vec::new());

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::request_sender::MockRequestSender;
    use crate::test_utils::TestShareBlockBuilder;

    #[test]
    fn test_broadcast_to_all_peers() {
        let peer_a = PeerId::random();
        let peer_b = PeerId::random();
        let peer_c = PeerId::random();
        let share_block = TestShareBlockBuilder::new().build();
        let block_hash = share_block.block_hash();
        let mut knowledge = PeerBlockKnowledge::default();

        let mut mock_sender = MockRequestSender::new();
        mock_sender
            .expect_send_request()
            .times(3)
            .returning(|_, _| ());

        let connected = vec![peer_a, peer_b, peer_c];
        send_share_block_broadcast(share_block, &connected, &mut knowledge, &mut mock_sender);

        assert!(knowledge.peer_knows_block(&peer_a, &block_hash));
        assert!(knowledge.peer_knows_block(&peer_b, &block_hash));
        assert!(knowledge.peer_knows_block(&peer_c, &block_hash));
    }

    #[test]
    fn test_broadcast_filters_known_peers() {
        let peer_a = PeerId::random();
        let peer_b = PeerId::random();
        let share_block = TestShareBlockBuilder::new().build();
        let block_hash = share_block.block_hash();

        let mut knowledge = PeerBlockKnowledge::default();
        knowledge.record_block_known(&peer_a, block_hash);

        let mut mock_sender = MockRequestSender::new();
        mock_sender
            .expect_send_request()
            .withf(move |sent_peer_id, _| *sent_peer_id == peer_b)
            .times(1)
            .returning(|_, _| ());

        let connected = vec![peer_a, peer_b];
        send_share_block_broadcast(share_block, &connected, &mut knowledge, &mut mock_sender);

        assert!(knowledge.peer_knows_block(&peer_b, &block_hash));
    }

    #[test]
    fn test_broadcast_skips_all_known_peers() {
        let peer_a = PeerId::random();
        let peer_b = PeerId::random();
        let share_block = TestShareBlockBuilder::new().build();
        let block_hash = share_block.block_hash();

        let mut knowledge = PeerBlockKnowledge::default();
        knowledge.record_block_known(&peer_a, block_hash);
        knowledge.record_block_known(&peer_b, block_hash);

        let mut mock_sender = MockRequestSender::new();
        mock_sender.expect_send_request().never();

        let connected = vec![peer_a, peer_b];
        send_share_block_broadcast(share_block, &connected, &mut knowledge, &mut mock_sender);
    }

    #[test]
    fn test_broadcast_no_connected_peers() {
        let share_block = TestShareBlockBuilder::new().build();
        let mut knowledge = PeerBlockKnowledge::default();

        let mut mock_sender = MockRequestSender::new();
        mock_sender.expect_send_request().never();

        send_share_block_broadcast(share_block, &[], &mut knowledge, &mut mock_sender);
    }
}
