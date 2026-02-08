//! Integration test: QUIC transport roundtrip on loopback.

use std::net::SocketAddr;

use cross_control_types::{
    ControlMessage, DeviceCapability, DeviceId, DeviceInfo, InputEvent, InputMessage, KeyCode,
    MachineId, ScreenGeometry, PROTOCOL_VERSION,
};

use cross_control_types::ButtonState;

#[tokio::test]
async fn hello_welcome_handshake_on_loopback() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    // Generate a cert for the server
    let cert = cross_control_certgen::generate_certificate("localhost").unwrap();

    // Bind server
    let bind_addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
    let transport =
        cross_control_protocol::QuicTransport::bind(bind_addr, &cert.cert_pem, &cert.key_pem)
            .unwrap();
    let server_addr = transport.local_addr().unwrap();

    // Spawn accept task
    let server = tokio::spawn(async move {
        let conn = transport.accept().await.unwrap();
        let (mut tx, mut rx) = conn.accept_control_stream().await.unwrap();

        // Receive Hello
        let hello: ControlMessage = rx.recv().await.unwrap().unwrap();
        match hello {
            ControlMessage::Hello {
                version,
                machine_id: _,
                name,
                screen: _,
            } => {
                assert_eq!(version, PROTOCOL_VERSION);
                assert_eq!(name, "test-client");
            }
            other => panic!("expected Hello, got {other:?}"),
        }

        // Send Welcome
        let welcome = ControlMessage::Welcome {
            version: PROTOCOL_VERSION,
            machine_id: MachineId::new(),
            name: "test-server".to_string(),
            screen: ScreenGeometry::new(2560, 1440),
        };
        tx.send(&welcome).await.unwrap();

        // Receive DeviceAnnounce
        let announce: ControlMessage = rx.recv().await.unwrap().unwrap();
        match announce {
            ControlMessage::DeviceAnnounce(info) => {
                assert_eq!(info.name, "Test Keyboard");
            }
            other => panic!("expected DeviceAnnounce, got {other:?}"),
        }

        // Accept input stream and read one message
        let mut input_rx = conn.accept_input_stream().await.unwrap();
        let input: InputMessage = input_rx.recv().await.unwrap().unwrap();
        assert_eq!(input.events.len(), 1);
        assert_eq!(
            input.events[0],
            InputEvent::Key {
                code: KeyCode::KeyA,
                state: ButtonState::Pressed,
            }
        );

        transport.close();
    });

    // Client side
    let client_cert = cross_control_certgen::generate_certificate("localhost").unwrap();
    let mut client_transport = cross_control_protocol::QuicTransport::bind(
        "127.0.0.1:0".parse().unwrap(),
        &client_cert.cert_pem,
        &client_cert.key_pem,
    )
    .unwrap();

    let conn = client_transport
        .connect(server_addr, "localhost")
        .await
        .unwrap();

    // Send Hello
    let (mut tx, mut rx) = conn.open_control_stream().await.unwrap();
    let hello = ControlMessage::Hello {
        version: PROTOCOL_VERSION,
        machine_id: MachineId::new(),
        name: "test-client".to_string(),
        screen: ScreenGeometry::new(1920, 1080),
    };
    tx.send(&hello).await.unwrap();

    // Receive Welcome
    let welcome: ControlMessage = rx.recv().await.unwrap().unwrap();
    match welcome {
        ControlMessage::Welcome { name, .. } => {
            assert_eq!(name, "test-server");
        }
        other => panic!("expected Welcome, got {other:?}"),
    }

    // Send DeviceAnnounce
    let device = DeviceInfo {
        id: DeviceId(1),
        name: "Test Keyboard".to_string(),
        capabilities: vec![DeviceCapability::Keyboard],
    };
    tx.send(&ControlMessage::DeviceAnnounce(device))
        .await
        .unwrap();

    // Open input stream and send a key event
    let mut input_tx = conn.open_input_stream().await.unwrap();
    let input_msg = InputMessage {
        device_id: DeviceId(1),
        timestamp_us: 12345,
        events: vec![InputEvent::Key {
            code: KeyCode::KeyA,
            state: ButtonState::Pressed,
        }],
    };
    input_tx.send(&input_msg).await.unwrap();

    // Wait for server to finish
    server.await.unwrap();

    client_transport.close();
}

#[tokio::test]
async fn ping_pong_roundtrip() {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let cert = cross_control_certgen::generate_certificate("localhost").unwrap();
    let transport = cross_control_protocol::QuicTransport::bind(
        "127.0.0.1:0".parse().unwrap(),
        &cert.cert_pem,
        &cert.key_pem,
    )
    .unwrap();
    let server_addr = transport.local_addr().unwrap();

    // Use a channel to synchronize server and client
    let (done_tx, done_rx) = tokio::sync::oneshot::channel::<()>();

    let server = tokio::spawn(async move {
        let conn = transport.accept().await.unwrap();
        let (mut tx, mut rx) = conn.accept_control_stream().await.unwrap();

        let msg: ControlMessage = rx.recv().await.unwrap().unwrap();
        match msg {
            ControlMessage::Ping { seq } => {
                tx.send(&ControlMessage::Pong { seq }).await.unwrap();
            }
            other => panic!("expected Ping, got {other:?}"),
        }

        // Wait for client to signal it's done
        let _ = done_rx.await;
    });

    let client_cert = cross_control_certgen::generate_certificate("localhost").unwrap();
    let mut client = cross_control_protocol::QuicTransport::bind(
        "127.0.0.1:0".parse().unwrap(),
        &client_cert.cert_pem,
        &client_cert.key_pem,
    )
    .unwrap();

    let conn = client.connect(server_addr, "localhost").await.unwrap();
    let (mut tx, mut rx) = conn.open_control_stream().await.unwrap();

    tx.send(&ControlMessage::Ping { seq: 42 }).await.unwrap();
    let reply: ControlMessage = rx.recv().await.unwrap().unwrap();
    match reply {
        ControlMessage::Pong { seq } => assert_eq!(seq, 42),
        other => panic!("expected Pong, got {other:?}"),
    }

    // Signal server we're done
    let _ = done_tx.send(());
    server.await.unwrap();
}
