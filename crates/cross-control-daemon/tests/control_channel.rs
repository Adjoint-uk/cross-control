//! The local control channel: ping, stop, single instance, cleanup.

use cross_control_daemon::control::{self, Endpoint, Request};
use cross_control_daemon::DaemonEvent;
use tokio::sync::mpsc;

fn scratch_endpoint() -> Endpoint {
    let dir = std::env::temp_dir().join(format!("cc-ctl-{}", uuid::Uuid::new_v4().simple()));
    Endpoint::for_runtime_dir(&dir)
}

#[tokio::test]
async fn ping_and_stop_reach_the_daemon() {
    let endpoint = scratch_endpoint();
    let (tx, mut rx) = mpsc::channel(4);
    let server = control::serve(endpoint.clone(), tx).unwrap();

    let pong = control::request(&endpoint, Request::Ping).await.unwrap();
    assert!(pong.ok);
    assert_eq!(pong.pid, std::process::id());
    assert!(rx.try_recv().is_err(), "ping must not stop anything");

    let stopped = control::request(&endpoint, Request::Stop).await.unwrap();
    assert!(stopped.ok);
    assert!(matches!(rx.recv().await, Some(DaemonEvent::Shutdown)));
    drop(server);
}

#[tokio::test]
async fn a_second_daemon_is_refused() {
    let endpoint = scratch_endpoint();
    let (tx, _rx) = mpsc::channel(4);
    let _first = control::serve(endpoint.clone(), tx.clone()).unwrap();
    let second = control::serve(endpoint.clone(), tx);
    assert_eq!(
        second.err().map(|e| e.kind()),
        Some(std::io::ErrorKind::AddrInUse)
    );
}

#[tokio::test]
async fn no_daemon_means_an_error_not_a_hang() {
    let endpoint = scratch_endpoint();
    assert!(control::request(&endpoint, Request::Ping).await.is_err());
}

#[tokio::test]
async fn the_endpoint_is_freed_when_the_server_stops() {
    let endpoint = scratch_endpoint();
    let (tx, _rx) = mpsc::channel(4);
    drop(control::serve(endpoint.clone(), tx.clone()).unwrap());
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(control::request(&endpoint, Request::Ping).await.is_err());
    // And a new daemon can take it over.
    let _again = control::serve(endpoint, tx).unwrap();
}

#[tokio::test]
async fn garbage_gets_an_error_answer() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let endpoint = scratch_endpoint();
    let (tx, mut rx) = mpsc::channel(4);
    let _server = control::serve(endpoint.clone(), tx).unwrap();
    #[cfg(unix)]
    let stream = tokio::net::UnixStream::connect(endpoint.to_string())
        .await
        .unwrap();
    #[cfg(windows)]
    let stream = tokio::net::windows::named_pipe::ClientOptions::new()
        .open(endpoint.to_string())
        .unwrap();
    let (reader, mut writer) = tokio::io::split(stream);
    writer.write_all(b"not json\n").await.unwrap();
    let mut answer = String::new();
    BufReader::new(reader).read_line(&mut answer).await.unwrap();
    let response: control::Response = serde_json::from_str(answer.trim()).unwrap();
    assert!(!response.ok);
    assert!(rx.try_recv().is_err());
}
