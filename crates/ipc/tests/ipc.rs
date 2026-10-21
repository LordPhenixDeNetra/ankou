#![allow(clippy::unwrap_used, clippy::expect_used)]

// Socket Unix ou named pipe Windows, selon la plateforme (`test_socket`).
use ankou_ipc::testing::test_socket;
use ankou_ipc::{Framed, IpcError, IpcListener, connect};
use ankou_proto::ipc::ClientToDaemon;
use ankou_proto::ipc::{DaemonToClient, IPC_PROTOCOL_VERSION};
use tokio::io::AsyncWriteExt;

#[tokio::test]
async fn frames_roundtrip() {
    let (a, b) = tokio::io::duplex(4096);
    let (mut a, mut b) = (Framed::new(a), Framed::new(b));
    for m in [
        ClientToDaemon::GetStatus,
        ClientToDaemon::Subscribe,
        ClientToDaemon::Disconnect,
    ] {
        a.send(&m).await.unwrap();
    }
    drop(a);
    assert_eq!(
        b.recv::<ClientToDaemon>().await.unwrap(),
        Some(ClientToDaemon::GetStatus)
    );
    assert_eq!(
        b.recv::<ClientToDaemon>().await.unwrap(),
        Some(ClientToDaemon::Subscribe)
    );
    assert_eq!(
        b.recv::<ClientToDaemon>().await.unwrap(),
        Some(ClientToDaemon::Disconnect)
    );
    assert_eq!(b.recv::<ClientToDaemon>().await.unwrap(), None);
}

#[tokio::test]
async fn oversized_frame_is_rejected() {
    let (mut a, b) = tokio::io::duplex(64);
    let mut b = Framed::new(b);
    a.write_u32(2 << 20).await.unwrap();
    let err = b.recv::<ClientToDaemon>().await.unwrap_err();
    assert!(matches!(err, IpcError::FrameTooLarge(2_097_152)), "{err:?}");
}

#[tokio::test]
async fn garbage_json_is_an_error() {
    let (mut a, b) = tokio::io::duplex(64);
    let mut b = Framed::new(b);
    a.write_u32(11).await.unwrap();
    a.write_all(b"pas du json").await.unwrap();
    let err = b.recv::<ClientToDaemon>().await.unwrap_err();
    assert!(matches!(err, IpcError::Json(_)), "{err:?}");
}

/// Serveur d'une seule connexion qui répond `reply` au `Hello`.
async fn serve_once(listener: IpcListener, reply: DaemonToClient) {
    let mut conn = listener.accept().await.unwrap();
    let hello = conn.recv::<ClientToDaemon>().await.unwrap();
    assert_eq!(
        hello,
        Some(ClientToDaemon::Hello {
            protocol_version: IPC_PROTOCOL_VERSION
        })
    );
    conn.send(&reply).await.unwrap();
}

#[tokio::test]
async fn connect_does_handshake() {
    let dir = tempfile::tempdir().unwrap();
    let path = test_socket(dir.path());
    let listener = IpcListener::bind(&path).await.unwrap();
    let server = tokio::spawn(serve_once(
        listener,
        DaemonToClient::Welcome {
            daemon_version: "0.1.0".into(),
        },
    ));
    connect(&path).await.unwrap();
    server.await.unwrap();
}

#[tokio::test]
async fn incompatible_daemon_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let path = test_socket(dir.path());
    let listener = IpcListener::bind(&path).await.unwrap();
    let server = tokio::spawn(serve_once(
        listener,
        DaemonToClient::Incompatible {
            protocol_version: 2,
        },
    ));
    let err = connect(&path).await.err().unwrap();
    assert!(matches!(err, IpcError::Incompatible(2)), "{err:?}");
    server.await.unwrap();
}

#[tokio::test]
async fn missing_daemon_is_an_io_error() {
    let dir = tempfile::tempdir().unwrap();
    let err = connect(&test_socket(dir.path())).await.err().unwrap();
    assert!(matches!(err, IpcError::Io(_)), "{err:?}");
}

#[cfg(unix)]
#[tokio::test]
async fn stale_socket_is_replaced() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("s.sock");
    // Listener détruit sans nettoyage, comme après un crash : le fichier reste.
    drop(std::os::unix::net::UnixListener::bind(&path).unwrap());
    assert!(path.exists());
    let _listener = IpcListener::bind(&path).await.unwrap();
}

#[tokio::test]
async fn second_daemon_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = test_socket(dir.path());
    let _first = IpcListener::bind(&path).await.unwrap();
    let err = IpcListener::bind(&path).await.err().unwrap();
    assert!(matches!(err, IpcError::AlreadyRunning), "{err:?}");
}

#[cfg(unix)]
#[tokio::test]
async fn socket_is_group_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let path = test_socket(dir.path());
    let listener = IpcListener::bind(&path).await.unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o660);
    drop(listener);
    assert!(!path.exists(), "socket supprimé à la fermeture");
}
