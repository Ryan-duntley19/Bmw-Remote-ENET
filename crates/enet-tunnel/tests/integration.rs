//! Additional integration-style tests for disconnects and bursts.

use bytes::Bytes;
use enet_tunnel::{EthernetPort, SimulatedEthernet, TunnelEngine, TunnelOptions};
use std::net::{Ipv4Addr, SocketAddr};
use std::time::Duration;
use tokio::net::UdpSocket;

async fn bind_local() -> SocketAddr {
    let s = UdpSocket::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))
        .await
        .unwrap();
    let addr = s.local_addr().unwrap();
    drop(s);
    addr
}

#[tokio::test]
async fn burst_and_flap() {
    let (agent_eth, car) = SimulatedEthernet::pair("a", "car");
    let (gw_eth, tools) = SimulatedEthernet::pair("g", "tools");
    let gw_addr = bind_local().await;

    let gw = TunnelEngine::new(
        TunnelOptions {
            bind: gw_addr,
            peer: None,
            allowed_cidrs: vec![],
            crypto: None,
            require_crypto: false,
            keepalive_interval_ms: 200,
            peer_timeout_ms: 2000,
            role: "gateway".into(),
            version: "test".into(),
        },
        gw_eth,
    )
    .run()
    .await
    .unwrap();

    let agent = TunnelEngine::new(
        TunnelOptions {
            bind: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
            peer: Some(gw_addr),
            allowed_cidrs: vec![],
            crypto: None,
            require_crypto: false,
            keepalive_interval_ms: 200,
            peer_timeout_ms: 2000,
            role: "agent".into(),
            version: "test".into(),
        },
        agent_eth,
    )
    .run()
    .await
    .unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    for i in 0..100u16 {
        let mut frame = vec![0xffu8; 14];
        frame.extend_from_slice(&i.to_be_bytes());
        car.send(Bytes::from(frame)).await.unwrap();
    }

    let mut got = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline && got < 100 {
        if tokio::time::timeout(Duration::from_millis(50), tools.recv())
            .await
            .ok()
            .and_then(|r| r.ok())
            .is_some()
        {
            got += 1;
        }
    }
    assert!(got >= 90, "expected most burst frames, got {got}");

    car.set_link(false);
    tokio::time::sleep(Duration::from_millis(100)).await;
    car.set_link(true);
    car.send(Bytes::from_static(b"\xff\xff\xff\xff\xff\xff\x01\x02\x03\x04\x05\x06\x08\x00WAKE"))
        .await
        .unwrap();
    let wake = tokio::time::timeout(Duration::from_secs(2), tools.recv())
        .await
        .expect("wake timeout")
        .unwrap();
    assert!(wake.windows(4).any(|w| w == b"WAKE"));

    agent.stop();
    gw.stop();
}

#[tokio::test]
async fn encrypted_tunnel_roundtrip() {
    let key = enet_protocol::derive_key_from_password("lab-secret");
    let crypto_a = enet_protocol::SessionCrypto::from_key(key);
    let crypto_b = enet_protocol::SessionCrypto::from_key(key);

    let (agent_eth, car) = SimulatedEthernet::pair("a", "car");
    let (gw_eth, tools) = SimulatedEthernet::pair("g", "tools");
    let gw_addr = bind_local().await;

    let gw = TunnelEngine::new(
        TunnelOptions {
            bind: gw_addr,
            peer: None,
            allowed_cidrs: vec![],
            crypto: Some(crypto_b),
            require_crypto: true,
            keepalive_interval_ms: 300,
            peer_timeout_ms: 3000,
            role: "gateway".into(),
            version: "test".into(),
        },
        gw_eth,
    )
    .run()
    .await
    .unwrap();

    let agent = TunnelEngine::new(
        TunnelOptions {
            bind: SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
            peer: Some(gw_addr),
            allowed_cidrs: vec![],
            crypto: Some(crypto_a),
            require_crypto: true,
            keepalive_interval_ms: 300,
            peer_timeout_ms: 3000,
            role: "agent".into(),
            version: "test".into(),
        },
        agent_eth,
    )
    .run()
    .await
    .unwrap();

    tokio::time::sleep(Duration::from_millis(80)).await;
    let frame = Bytes::from_static(b"\xff\xff\xff\xff\xff\xff\xaa\xbb\xcc\xdd\xee\xff\x08\x00SECRET");
    car.send(frame.clone()).await.unwrap();
    let got = tokio::time::timeout(Duration::from_secs(2), tools.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(got, frame);

    agent.stop();
    gw.stop();
}
