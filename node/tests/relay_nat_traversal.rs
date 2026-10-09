use futures::StreamExt;
use libp2p::{
    Multiaddr, PeerId, SwarmBuilder, autonat, identify,
    identity::Keypair,
    noise, ping, relay,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux,
};
use mesh_core::quota::BandwidthLimiter;
use mesh_node::state::{NodeLifecycleState, NodeState};
use std::time::Duration;
use tempfile::tempdir;

#[derive(NetworkBehaviour)]
pub struct RelayHopBehaviour {
    pub relay: relay::Behaviour,
    pub ping: ping::Behaviour,
    pub identify: identify::Behaviour,
    pub autonat: autonat::Behaviour,
}

#[derive(NetworkBehaviour)]
pub struct NatClientBehaviour {
    pub ping: ping::Behaviour,
    pub identify: identify::Behaviour,
    pub autonat: autonat::Behaviour,
}

#[tokio::test]
async fn test_bandwidth_limiter_state_integration() {
    let tmp = tempdir().expect("tempdir");
    let key = Keypair::generate_ed25519();
    let peer_id = PeerId::from(key.public());
    let mut state = NodeState::with_data_dir(peer_id, tmp.path().to_path_buf(), 1.0);

    // Limit to 50 KB/s with a burst capacity of 100 KB
    let limiter = BandwidthLimiter::new(50 * 1024, 100 * 1024);
    state.bandwidth_limiter = Some(limiter);

    // Try acquiring 60 KB (within burst of 100 KB) -> Should succeed
    assert!(state.check_egress_bandwidth(60 * 1024));

    // Try acquiring another 30 KB (total 90 KB <= 100 KB) -> Should succeed
    assert!(state.check_egress_bandwidth(30 * 1024));

    // Try acquiring 20 KB (90 + 20 = 110 KB > 100 KB) -> Should fail / throttle
    assert!(!state.check_egress_bandwidth(20 * 1024));

    // Simulate time passing (1.1 second = ~55 KB refilled)
    tokio::time::sleep(Duration::from_millis(1100)).await;

    // Now acquiring 40 KB should succeed!
    assert!(state.check_egress_bandwidth(40 * 1024));
}

#[tokio::test]
async fn test_relay_hop_and_autonat_network() {
    let hop_key = Keypair::generate_ed25519();
    let hop_peer_id = PeerId::from(hop_key.public());

    let hop_behaviour = RelayHopBehaviour {
        relay: relay::Behaviour::new(hop_peer_id, relay::Config::default()),
        ping: ping::Behaviour::new(ping::Config::default()),
        identify: identify::Behaviour::new(identify::Config::new(
            "/mesh-storage/1.0.0".into(),
            hop_key.public(),
        )),
        autonat: autonat::Behaviour::new(hop_peer_id, autonat::Config::default()),
    };

    let mut hop_swarm = SwarmBuilder::with_existing_identity(hop_key)
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )
        .expect("tcp")
        .with_behaviour(|_| hop_behaviour)
        .expect("behaviour")
        .build();

    let listen_addr: Multiaddr = "/ip4/127.0.0.1/tcp/0".parse().unwrap();
    hop_swarm.listen_on(listen_addr).expect("listen");

    // Wait for NewListenAddr
    let actual_hop_addr = loop {
        if let SwarmEvent::NewListenAddr { address, .. } = hop_swarm.select_next_some().await {
            break address;
        }
    };

    println!("Hop Relay listening on: {}", actual_hop_addr);

    // Create a client node behind simulated NAT
    let client_key = Keypair::generate_ed25519();
    let client_peer_id = PeerId::from(client_key.public());

    let client_behaviour = NatClientBehaviour {
        ping: ping::Behaviour::new(ping::Config::default()),
        identify: identify::Behaviour::new(identify::Config::new(
            "/mesh-storage/1.0.0".into(),
            client_key.public(),
        )),
        autonat: autonat::Behaviour::new(client_peer_id, autonat::Config::default()),
    };

    let mut client_swarm = SwarmBuilder::with_existing_identity(client_key)
        .with_tokio()
        .with_tcp(
            tcp::Config::default(),
            noise::Config::new,
            yamux::Config::default,
        )
        .expect("tcp")
        .with_behaviour(|_| client_behaviour)
        .expect("behaviour")
        .build();

    client_swarm
        .dial(actual_hop_addr.clone())
        .expect("dial hop");

    // Verify connection established between client and relay hop
    let mut connected = false;
    for _ in 0..20 {
        tokio::select! {
            event = hop_swarm.select_next_some() => {
                if let SwarmEvent::ConnectionEstablished { peer_id, .. } = event
                    && peer_id == client_peer_id
                {
                    connected = true;
                    break;
                }
            }
            event = client_swarm.select_next_some() => {
                if let SwarmEvent::ConnectionEstablished { peer_id, .. } = event
                    && peer_id == hop_peer_id
                {
                    connected = true;
                    break;
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(500)) => {}
        }
    }

    assert!(connected, "Client successfully connected to relay hop!");
}

#[tokio::test]
async fn test_node_nat_and_relay_status_reporting() {
    let tmp = tempdir().expect("tempdir");
    let key = Keypair::generate_ed25519();
    let peer_id = PeerId::from(key.public());
    let mut state = NodeState::with_data_dir(peer_id, tmp.path().to_path_buf(), 2.0);

    // Initial status
    let initial_status = state.get_status();
    assert_eq!(initial_status.nat_status, "Unknown");
    assert!(initial_status.relay_addresses.is_empty());
    assert_eq!(initial_status.bandwidth_limit_kbps, None);
    assert_eq!(initial_status.state, NodeLifecycleState::Active);

    // Update AutoNAT status
    state.set_nat_status("Public (/ip4/198.51.100.1/tcp/4001)".to_string());

    // Add relay circuit address
    let relay_peer = PeerId::random();
    let relay_addr: Multiaddr =
        format!("/ip4/203.0.113.50/tcp/4001/p2p/{}/p2p-circuit", relay_peer)
            .parse()
            .unwrap();
    state.add_relay_address(relay_addr);

    // Set bandwidth limit to 500 KB/s (4 Mbps)
    state.set_bandwidth_limit(Some(500));

    let updated_status = state.get_status();
    assert_eq!(
        updated_status.nat_status,
        "Public (/ip4/198.51.100.1/tcp/4001)"
    );
    assert_eq!(updated_status.relay_addresses.len(), 1);
    assert!(updated_status.relay_addresses[0].contains("/p2p-circuit"));
    assert_eq!(updated_status.bandwidth_limit_kbps, Some(500));

    // Clear bandwidth limit
    state.set_bandwidth_limit(None);
    assert_eq!(state.get_status().bandwidth_limit_kbps, None);
}
