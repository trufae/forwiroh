//! Local, owner-only daemon control on Unix and Windows.
use std::{io, path::Path};

#[cfg(unix)]
pub use tokio::net::UnixStream as Client;

#[cfg(unix)]
pub struct Listener(tokio::net::UnixListener);

#[cfg(unix)]
impl Listener {
    pub fn bind(path: &Path) -> io::Result<Self> {
        use std::os::unix::fs::PermissionsExt;
        let socket = path.join("control.sock");
        if socket.exists() {
            std::fs::remove_file(&socket)?;
        }
        let listener = tokio::net::UnixListener::bind(&socket)?;
        std::fs::set_permissions(socket, std::fs::Permissions::from_mode(0o600))?;
        Ok(Self(listener))
    }

    pub async fn accept(&mut self) -> io::Result<Client> {
        Ok(self.0.accept().await?.0)
    }
}

#[cfg(unix)]
pub async fn connect(path: &Path) -> io::Result<Client> {
    Client::connect(path.join("control.sock")).await
}

#[cfg(unix)]
pub fn cleanup(path: &Path) -> io::Result<()> {
    std::fs::remove_file(path.join("control.sock"))
}

#[cfg(unix)]
pub async fn terminate() -> io::Result<()> {
    tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?
        .recv()
        .await;
    Ok(())
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::hash::{DefaultHasher, Hash, Hasher};
    use tokio::net::windows::named_pipe::{
        ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
    };

    pub type Client = NamedPipeClient;

    fn name(path: &Path) -> io::Result<String> {
        let mut hash = DefaultHasher::new();
        // Resolve relative paths identically in the daemon and its clients.
        std::fs::canonicalize(path)?.hash(&mut hash);
        Ok(format!(r"\\.\pipe\forwiroh-{:016x}", hash.finish()))
    }

    fn server(name: &str, first: bool) -> io::Result<NamedPipeServer> {
        let security = forwiroh::windows::OwnerSecurity::new()?;
        let mut attributes = security.attributes();
        // SAFETY: the security descriptor and attributes remain valid throughout
        // CreateNamedPipe; Windows copies the descriptor into the pipe object.
        unsafe {
            ServerOptions::new()
                .first_pipe_instance(first)
                .reject_remote_clients(true)
                .create_with_security_attributes_raw(
                    name,
                    std::ptr::from_mut(&mut attributes).cast(),
                )
        }
    }

    pub struct Listener {
        name: String,
        pending: NamedPipeServer,
    }

    impl Listener {
        pub fn bind(path: &Path) -> io::Result<Self> {
            let name = name(path)?;
            let pending = server(&name, true)?;
            Ok(Self { name, pending })
        }

        pub async fn accept(&mut self) -> io::Result<NamedPipeServer> {
            self.pending.connect().await?;
            let next = server(&self.name, false)?;
            Ok(std::mem::replace(&mut self.pending, next))
        }
    }

    pub async fn connect(path: &Path) -> io::Result<Client> {
        let name = name(path)?;
        tokio::time::timeout(forwiroh::wire::SETUP_TIMEOUT, async {
            loop {
                match ClientOptions::new().open(&name) {
                    Ok(client) => return Ok(client),
                    // ERROR_PIPE_BUSY: the server is preparing its next instance.
                    Err(error) if error.raw_os_error() == Some(231) => {
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                    }
                    Err(error) => return Err(error),
                }
            }
        })
        .await?
    }

    pub fn cleanup(_path: &Path) -> io::Result<()> {
        Ok(())
    }

    pub async fn terminate() -> io::Result<()> {
        std::future::pending().await
    }
}

#[cfg(windows)]
pub use windows::*;
