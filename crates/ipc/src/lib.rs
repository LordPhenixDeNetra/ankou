//! Transport local entre les clients (CLI, UI) et `ankoud` (spec §6.7) : trames
//! longueur `u32` big-endian + JSON, poignée de main `Hello` → `Welcome`.

use ankou_proto::ipc::{ClientToDaemon, DaemonToClient, IPC_PROTOCOL_VERSION, MAX_FRAME};
use interprocess::local_socket::tokio::{Listener, Stream};
use interprocess::local_socket::traits::tokio::{Listener as _, Stream as _};
use interprocess::local_socket::{GenericFilePath, ListenerOptions, Name, ToFsName};
use serde::Serialize;
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("trame trop grande : {0} octets")]
    FrameTooLarge(u32),
    #[error("message invalide : {0}")]
    Json(String),
    #[error("version de protocole incompatible : {0}")]
    Incompatible(u32),
    #[error("connexion fermée")]
    Closed,
    #[error("un autre service écoute déjà sur ce socket")]
    AlreadyRunning,
}

/// Flux à trames : chaque message est une longueur `u32` puis son JSON.
pub struct Framed<S> {
    stream: S,
}

impl<S> Framed<S> {
    pub fn new(stream: S) -> Self {
        Self { stream }
    }

    /// Flux brut, par exemple pour le couper en deux avec `tokio::io::split` : `recv`
    /// n'est pas annulable sans perte, une lecture et une écriture concurrentes passent
    /// donc par deux moitiés.
    pub fn into_inner(self) -> S {
        self.stream
    }
}

impl<S: AsyncWrite + Unpin> Framed<S> {
    pub async fn send<T: Serialize>(&mut self, msg: &T) -> Result<(), IpcError> {
        let body = serde_json::to_vec(msg).map_err(|e| IpcError::Json(e.to_string()))?;
        let len = u32::try_from(body.len())
            .ok()
            .filter(|&n| n <= MAX_FRAME)
            .ok_or(IpcError::FrameTooLarge(u32::MAX))?;
        self.stream.write_u32(len).await?;
        self.stream.write_all(&body).await?;
        self.stream.flush().await?;
        Ok(())
    }
}

impl<S: AsyncRead + Unpin> Framed<S> {
    /// `Ok(None)` : l'autre côté a fermé proprement, entre deux trames.
    pub async fn recv<T: DeserializeOwned>(&mut self) -> Result<Option<T>, IpcError> {
        let len = match self.stream.read_u32().await {
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        // Refusée avant d'allouer : un client ne peut pas faire réserver 4 Gio.
        if len > MAX_FRAME {
            return Err(IpcError::FrameTooLarge(len));
        }
        let mut body = vec![0; len as usize];
        self.stream.read_exact(&mut body).await.map_err(|e| {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                IpcError::Closed
            } else {
                e.into()
            }
        })?;
        serde_json::from_slice(&body)
            .map(Some)
            .map_err(|e| IpcError::Json(e.to_string()))
    }
}

pub type Connection = Framed<Stream>;

/// Socket du service (spec §6.7) : named pipe sous Windows.
pub fn default_socket() -> PathBuf {
    if cfg!(windows) {
        r"\\.\pipe\ankou".into()
    } else if cfg!(target_os = "macos") {
        "/var/run/ankou/ankoud.sock".into()
    } else {
        "/run/ankou/ankoud.sock".into()
    }
}

/// Droits du named pipe (Windows) : SYSTEM et administrateurs, tout ; utilisateurs de la
/// session (interactifs), lecture et écriture.
pub const PIPE_SDDL: &str = "D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GRGW;;;IU)";

#[cfg(feature = "testing")]
pub mod testing {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    /// Socket propre à un test : un fichier de `dir` (Unix), un named pipe unique (Windows).
    pub fn test_socket(dir: &Path) -> PathBuf {
        if cfg!(windows) {
            static NEXT: AtomicU32 = AtomicU32::new(0);
            let n = NEXT.fetch_add(1, Ordering::Relaxed);
            PathBuf::from(format!(r"\\.\pipe\ankou-test-{}-{n}", std::process::id()))
        } else {
            dir.join("ankoud.sock")
        }
    }
}

fn name(path: &Path) -> std::io::Result<Name<'_>> {
    path.to_fs_name::<GenericFilePath>()
}

/// Se connecte au service et fait la poignée de main.
pub async fn connect(path: &Path) -> Result<Connection, IpcError> {
    let mut conn = Framed::new(Stream::connect(name(path)?).await?);
    conn.send(&ClientToDaemon::Hello {
        protocol_version: IPC_PROTOCOL_VERSION,
    })
    .await?;
    match conn.recv::<DaemonToClient>().await? {
        Some(DaemonToClient::Welcome { .. }) => Ok(conn),
        Some(DaemonToClient::Incompatible { protocol_version }) => {
            Err(IpcError::Incompatible(protocol_version))
        }
        Some(other) => Err(IpcError::Json(format!("réponse inattendue : {other:?}"))),
        None => Err(IpcError::Closed),
    }
}

/// Socket d'écoute du service ; le fichier est supprimé à la fermeture.
pub struct IpcListener {
    listener: Listener,
    _path: PathBuf,
}

impl IpcListener {
    /// Un service répond déjà → `AlreadyRunning`. Unix : un socket mort (crash) est supprimé
    /// puis recréé, droits `0660` (propriétaire et groupe). Windows : ACL `PIPE_SDDL`.
    pub async fn bind(path: &Path) -> Result<Self, IpcError> {
        if Stream::connect(name(path)?).await.is_ok() {
            return Err(IpcError::AlreadyRunning);
        }
        #[cfg(unix)]
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        let options = ListenerOptions::new().name(name(path)?);
        #[cfg(windows)]
        let options = {
            use interprocess::os::windows::local_socket::ListenerOptionsExt;
            use interprocess::os::windows::security_descriptor::SecurityDescriptor;
            let sddl = widestring::U16CString::from_str(PIPE_SDDL)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
            options.security_descriptor(SecurityDescriptor::deserialize(&sddl)?)
        };
        let listener = options.create_tokio()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o660))?;
        }
        Ok(Self {
            listener,
            _path: path.to_owned(),
        })
    }

    pub async fn accept(&self) -> Result<Connection, IpcError> {
        Ok(Framed::new(self.listener.accept().await?))
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::PIPE_SDDL;
    use interprocess::os::windows::security_descriptor::SecurityDescriptor;

    #[test]
    fn pipe_security_descriptor_parses() {
        let sddl = widestring::U16CString::from_str(PIPE_SDDL).unwrap();
        assert!(SecurityDescriptor::deserialize(&sddl).is_ok());
    }
}
