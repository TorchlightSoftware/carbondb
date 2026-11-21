// SPDX-License-Identifier: MIT OR Apache-2.0

//! A node that already shares a space with a peer, then subscribes to a second space the peer
//! holds too, should sync it at once: it knows where the peer is.
//!
//! Subscribing resets the discovery walkers, and a reset walk starts from a bootstrap node. Without
//! bootstrap nodes the walk picks any known node, the peer, and the second space syncs at once.
//! With a bootstrap node that holds neither space, both walkers visit it, learn nothing new and back
//! off for 1-5s before the walk reaches the peer.
//!
//! The nodes find each other only through a relay, with mDNS off, and let discovery settle after
//! the first space syncs, as nodes that have run a while have. Needs
//! `iroh-relay = { version = "1.4.1", features = ["server"] }` as a dev-dependency of the `p2panda`
//! crate. Run with `cargo test -p p2panda --test second_space -- --nocapture`.

use std::net::{Ipv4Addr, SocketAddr};
use std::time::{Duration, Instant};

use iroh_relay::server::{RelayConfig, Server, ServerConfig};
use p2panda::network::MdnsDiscoveryMode;
use p2panda::spaces::{AccessLevel, SpaceSubscription};
use p2panda::streams::StreamEvent;
use p2panda::{NetworkId, NodeBuilder, NodeId, RelayUrl, Topic};
use p2panda_core::test_utils::setup_logging;
use tokio_stream::StreamExt;

/// How long the nodes run after the first space syncs, before penguin subscribes to the second.
const SETTLE: Duration = Duration::from_secs(3);

/// A relay on localhost serving plain HTTP, and its URL.
async fn relay() -> (Server, RelayUrl) {
    let mut config = ServerConfig::default();
    config.relay = Some(RelayConfig::new(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))));
    let server = Server::spawn(config).await.unwrap();
    let url = format!("http://{}", server.http_addr().unwrap())
        .parse()
        .unwrap();
    (server, url)
}

/// A node behind the relay, with mDNS off.
fn node(network_id: NetworkId, relay: &RelayUrl) -> NodeBuilder {
    p2panda::builder()
        .network_id(network_id)
        .mdns_mode(MdnsDiscoveryMode::Disabled)
        .relay_url(relay.clone())
}

/// Wait for a message the given node wrote.
async fn message_from(subscription: &mut SpaceSubscription<String>, author: NodeId) {
    while let Some(event) = subscription.next().await {
        if let StreamEvent::Processed { operation, .. } = event
            && operation.author() == author
        {
            return;
        }
    }
}

/// How long penguin waits for the second space's message, with or without a third node both
/// panda and penguin use as a bootstrap.
async fn second_space_wait(with_bootstrap_node: bool) -> Duration {
    setup_logging();

    let (_relay, relay_url) = relay().await;
    let network_id: NetworkId = Topic::random().into();
    // A third node, holding neither space.
    let seal = node(network_id, &relay_url).spawn().await.unwrap();
    let bootstrap = |builder: NodeBuilder| {
        if with_bootstrap_node {
            builder.bootstrap(seal.id(), relay_url.clone())
        } else {
            builder
        }
    };
    let panda = bootstrap(node(network_id, &relay_url))
        .spawn()
        .await
        .unwrap();
    let penguin = bootstrap(node(network_id, &relay_url))
        .spawn()
        .await
        .unwrap();

    // Panda learns penguin's key bundle and address over a side channel.
    panda
        .register_member(penguin.me().await.unwrap())
        .await
        .unwrap();
    panda
        .insert_bootstrap(penguin.id(), relay_url.clone())
        .await
        .unwrap();

    // Panda holds two spaces, penguin a member of both, each with a message.
    let (first, _first_rx) = panda.create_space::<String>().await.unwrap();
    let (second, _second_rx) = panda.create_space::<String>().await.unwrap();
    for space in [&first, &second] {
        space.add(penguin.id(), AccessLevel::Write).await.unwrap();
        space
            .publish("hello".to_string())
            .await
            .unwrap()
            .await
            .unwrap();
    }

    // Penguin syncs the first space: from here on it knows where panda is.
    let (_first_pending, mut first_rx) = penguin.space::<String>(first.id()).await.unwrap();
    message_from(&mut first_rx, panda.id()).await;
    tokio::time::sleep(SETTLE).await;

    // Then penguin subscribes to the second space.
    let started = Instant::now();
    let (_second_pending, mut second_rx) = penguin.space::<String>(second.id()).await.unwrap();
    message_from(&mut second_rx, panda.id()).await;
    let waited = started.elapsed();
    println!("penguin waited {waited:?} for the second space");
    waited
}

#[tokio::test]
async fn a_second_space_syncs_at_once_with_a_peer_already_known() {
    let waited = second_space_wait(false).await;
    assert!(waited < Duration::from_millis(500), "waited {waited:?}");
}

#[tokio::test]
async fn a_second_space_syncs_at_once_with_a_peer_already_known_despite_a_bootstrap_node() {
    let waited = second_space_wait(true).await;
    assert!(waited < Duration::from_millis(500), "waited {waited:?}");
}
