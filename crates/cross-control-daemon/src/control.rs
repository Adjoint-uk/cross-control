//! Local control channel between the running daemon and the CLI.
//!
//! `cross-control stop` uses it to ask the daemon to shut down cleanly
//! (release grabbed input, say goodbye to peers) on every platform —
//! including Windows, which has no SIGTERM. It also keeps a second daemon
//! from starting for the same user and runtime directory.
//!
//! - Unix: a socket in the runtime directory, which only its owner can
//!   enter; the socket itself is owner-only as well.
//! - Windows: a named pipe whose name is derived from the runtime
//!   directory. Remote (network) clients are refused.
//!
//! The protocol is one JSON line each way: the client sends a [`Request`],
//! the daemon answers with a [`Response`] and closes the connection.

use std::fmt;
use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

use crate::daemon::DaemonEvent;

/// Longest request line accepted; real requests are a few dozen bytes.
const MAX_REQUEST: u64 = 4096;
/// How long a client may take to send its request, and the CLI waits for
/// an answer.
const IO_TIMEOUT: Duration = Duration::from_secs(5);

/// What the CLI asks of the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "lowercase")]
pub enum Request {
    /// Is the daemon there? (Answers with its PID.)
    Ping,
    /// Shut down cleanly.
    Stop,
}

/// The daemon's answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    /// The daemon's process id.
    pub pid: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Response {
    fn ok() -> Self {
        Self {
            ok: true,
            pid: std::process::id(),
            error: None,
        }
    }

    fn error(message: String) -> Self {
        Self {
            ok: false,
            pid: std::process::id(),
            error: Some(message),
        }
    }
}

/// Where the daemon listens: a socket path on Unix, a pipe name on Windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint(String);

impl Endpoint {
    /// The endpoint for a runtime directory. Daemons with different runtime
    /// directories (e.g. `CROSS_CONTROL_RUNTIME_DIR` in tests) do not clash.
    #[must_use]
    pub fn for_runtime_dir(dir: &Path) -> Self {
        #[cfg(unix)]
        {
            Self(
                dir.join("cross-control.sock")
                    .to_string_lossy()
                    .into_owned(),
            )
        }
        #[cfg(windows)]
        {
            // Pipe names are global to the machine, so make the name
            // specific to this runtime directory (which is per user).
            let key = dir.to_string_lossy().to_lowercase();
            Self(format!(
                r"\\.\pipe\cross-control-{:016x}",
                fnv1a(key.as_bytes())
            ))
        }
    }

    /// The endpoint of the daemon using this process's runtime directory.
    #[must_use]
    pub fn current() -> Self {
        Self::for_runtime_dir(&crate::setup::runtime_dir())
    }
}

impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// 64-bit FNV-1a: a stable hash (unlike `std`'s), so every build derives
/// the same pipe name.
#[cfg_attr(not(windows), allow(dead_code))]
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// A running control server. Dropping it stops listening (and removes the
/// socket file on Unix).
pub struct ControlServer {
    task: JoinHandle<()>,
    endpoint: Endpoint,
}

impl ControlServer {
    #[must_use]
    pub fn endpoint(&self) -> &Endpoint {
        &self.endpoint
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        self.task.abort();
        #[cfg(unix)]
        let _ = std::fs::remove_file(&self.endpoint.0);
    }
}

/// Start answering control requests at `endpoint`, sending
/// [`DaemonEvent::Shutdown`] to `events` when asked to stop.
///
/// Fails with [`std::io::ErrorKind::AddrInUse`] if another daemon already
/// answers there. Must be called inside a Tokio runtime.
pub fn serve(
    endpoint: Endpoint,
    events: mpsc::Sender<DaemonEvent>,
) -> std::io::Result<ControlServer> {
    let task = platform::serve(&endpoint, events)?;
    info!(endpoint = %endpoint, "control channel listening");
    Ok(ControlServer { task, endpoint })
}

/// Send one request to the daemon at `endpoint` and wait for its answer.
///
/// Fails with [`std::io::ErrorKind::NotFound`] or
/// [`std::io::ErrorKind::ConnectionRefused`] when no daemon is listening.
pub async fn request(endpoint: &Endpoint, request: Request) -> std::io::Result<Response> {
    tokio::time::timeout(IO_TIMEOUT, async {
        let stream = platform::connect(endpoint).await?;
        let (reader, mut writer) = tokio::io::split(stream);
        let mut line = serde_json::to_string(&request)?;
        line.push('\n');
        writer.write_all(line.as_bytes()).await?;
        writer.flush().await?;

        let mut answer = String::new();
        BufReader::new(reader.take(MAX_REQUEST))
            .read_line(&mut answer)
            .await?;
        serde_json::from_str(answer.trim()).map_err(std::io::Error::other)
    })
    .await
    .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "the daemon did not answer"))?
}

/// Answer one connection.
async fn handle<S>(stream: S, events: mpsc::Sender<DaemonEvent>)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let (reader, mut writer) = tokio::io::split(stream);
    let mut line = String::new();
    let read = tokio::time::timeout(
        IO_TIMEOUT,
        BufReader::new(reader.take(MAX_REQUEST)).read_line(&mut line),
    )
    .await;
    if !matches!(read, Ok(Ok(n)) if n > 0) {
        debug!("control client sent nothing");
        return;
    }
    let response = match serde_json::from_str::<Request>(line.trim()) {
        Ok(Request::Ping) => Response::ok(),
        Ok(Request::Stop) => {
            info!("stop requested over the control channel");
            if events.send(DaemonEvent::Shutdown).await.is_ok() {
                Response::ok()
            } else {
                Response::error("the daemon is already stopping".to_string())
            }
        }
        Err(e) => Response::error(format!("invalid request: {e}")),
    };
    let Ok(mut out) = serde_json::to_string(&response) else {
        return;
    };
    out.push('\n');
    if let Err(e) = writer.write_all(out.as_bytes()).await {
        debug!(error = %e, "control client went away");
    }
    let _ = writer.shutdown().await;
}

#[cfg(unix)]
mod platform {
    use std::io::ErrorKind;
    use std::os::unix::fs::PermissionsExt;
    use std::path::Path;

    use tokio::net::{UnixListener, UnixStream};

    use super::{handle, warn, DaemonEvent, Endpoint, JoinHandle};

    pub fn serve(
        endpoint: &Endpoint,
        events: tokio::sync::mpsc::Sender<DaemonEvent>,
    ) -> std::io::Result<JoinHandle<()>> {
        let path = Path::new(&endpoint.0);
        if let Some(dir) = path.parent() {
            crate::setup::create_private_dir(dir)?;
        }
        if path.exists() {
            // A live daemon answers; a dead one left a stale file behind.
            if std::os::unix::net::UnixStream::connect(path).is_ok() {
                return Err(std::io::Error::new(
                    ErrorKind::AddrInUse,
                    format!("another daemon is already running ({})", path.display()),
                ));
            }
            std::fs::remove_file(path)?;
        }
        let listener = UnixListener::bind(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        Ok(tokio::spawn(async move {
            loop {
                match listener.accept().await {
                    Ok((stream, _)) => {
                        tokio::spawn(handle(stream, events.clone()));
                    }
                    Err(e) => {
                        warn!(error = %e, "control channel accept failed");
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                }
            }
        }))
    }

    pub async fn connect(endpoint: &Endpoint) -> std::io::Result<UnixStream> {
        UnixStream::connect(&endpoint.0).await
    }
}

#[cfg(windows)]
mod platform {
    use std::io::ErrorKind;
    use std::time::Duration;

    use tokio::net::windows::named_pipe::{
        ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
    };

    use super::{handle, warn, DaemonEvent, Endpoint, JoinHandle};

    /// `ERROR_PIPE_BUSY`: every server instance is taken; try again.
    const ERROR_PIPE_BUSY: i32 = 231;
    /// `ERROR_ACCESS_DENIED`: what creating a first instance returns when
    /// the pipe already exists.
    const ERROR_ACCESS_DENIED: i32 = 5;

    fn create(name: &str, first: bool) -> std::io::Result<NamedPipeServer> {
        ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create(name)
    }

    pub fn serve(
        endpoint: &Endpoint,
        events: tokio::sync::mpsc::Sender<DaemonEvent>,
    ) -> std::io::Result<JoinHandle<()>> {
        let name = endpoint.0.clone();
        let mut server = create(&name, true).map_err(|e| {
            if e.raw_os_error() == Some(ERROR_ACCESS_DENIED) {
                std::io::Error::new(
                    ErrorKind::AddrInUse,
                    format!("another daemon is already running ({name})"),
                )
            } else {
                e
            }
        })?;
        Ok(tokio::spawn(async move {
            loop {
                if let Err(e) = server.connect().await {
                    warn!(error = %e, "control channel connect failed");
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
                // Open the next instance before serving this one, so a
                // client never finds the pipe missing.
                let next = match create(&name, false) {
                    Ok(next) => next,
                    Err(e) => {
                        warn!(error = %e, "cannot create control pipe instance");
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                };
                let connected = std::mem::replace(&mut server, next);
                tokio::spawn(handle(connected, events.clone()));
            }
        }))
    }

    pub async fn connect(endpoint: &Endpoint) -> std::io::Result<NamedPipeClient> {
        let mut attempts = 0;
        loop {
            match ClientOptions::new().open(&endpoint.0) {
                Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY) && attempts < 50 => {
                    attempts += 1;
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                other => return other,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_are_one_small_json_object() {
        assert_eq!(
            serde_json::to_string(&Request::Stop).unwrap(),
            r#"{"cmd":"stop"}"#
        );
        assert_eq!(
            serde_json::from_str::<Request>(r#"{"cmd":"ping"}"#).unwrap(),
            Request::Ping
        );
        assert!(serde_json::from_str::<Request>(r#"{"cmd":"format-disk"}"#).is_err());
    }

    #[test]
    fn endpoints_differ_by_runtime_dir() {
        let a = Endpoint::for_runtime_dir(Path::new("/run/user/1000"));
        let b = Endpoint::for_runtime_dir(Path::new("/run/user/1001"));
        assert_ne!(a, b);
        assert_eq!(a, Endpoint::for_runtime_dir(Path::new("/run/user/1000")));
    }

    #[test]
    fn fnv1a_is_stable() {
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), 0xaf63_dc4c_8601_ec8c);
    }
}
