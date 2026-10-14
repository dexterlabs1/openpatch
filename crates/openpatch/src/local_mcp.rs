//! Same-user Unix socket discovery and attachment for live desktop sessions.
use crate::control::SharedSession;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Read, Write};
use std::net::Shutdown;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const MAX_CLIENTS: usize = 8;
const MAX_DESCRIPTOR: u64 = 16 * 1024;
const MAX_ENTRIES: usize = 256;
const DISCOVERY_BUDGET: Duration = Duration::from_secs(2);
const SOCKET: &str = "rpc.sock";
const DESCRIPTOR: &str = "session.json";

#[derive(Clone, Debug)]
pub struct SessionInfo {
    pub id: String,
    pub workspace: PathBuf,
    pub pid: u32,
}

impl SessionInfo {
    pub fn to_json(&self) -> Value {
        json!({"id": self.id, "workspace": self.workspace, "pid": self.pid})
    }
}

fn uid() -> libc::uid_t {
    unsafe { libc::geteuid() }
}

fn registry() -> PathBuf {
    // Keep below macOS's small sockaddr_un limit, independently of TMPDIR.
    PathBuf::from(format!("/tmp/opm-{}", uid()))
}

fn private_dir(path: &Path, create: bool) -> Result<(), String> {
    if create {
        match fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(format!("create local MCP directory: {e}")),
        }
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|e| format!("inspect local MCP directory {}: {e}", path.display()))?;
    if !metadata.is_dir() || metadata.uid() != uid() || metadata.mode() & 0o7777 != 0o700 {
        return Err(format!(
            "local MCP directory {} must be an owned, nonsymlink directory with mode 0700",
            path.display()
        ));
    }
    Ok(())
}

fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn random_id() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|e| format!("generate local MCP session ID: {e}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

#[derive(Clone)]
struct Identity {
    path: PathBuf,
    dev: u64,
    ino: u64,
}

impl Identity {
    fn read(path: &Path) -> Result<Self, String> {
        let m = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
        Ok(Self {
            path: path.to_owned(),
            dev: m.dev(),
            ino: m.ino(),
        })
    }

    fn still_owned(&self) -> bool {
        fs::symlink_metadata(&self.path)
            .is_ok_and(|m| m.dev() == self.dev && m.ino() == self.ino && m.uid() == uid())
    }
}

struct InstanceFiles {
    directory: Identity,
    files: Vec<Identity>,
}

impl Drop for InstanceFiles {
    fn drop(&mut self) {
        if !self.directory.still_owned() {
            return;
        }
        for file in &self.files {
            if file.still_owned() {
                let _ = fs::remove_file(&file.path);
            }
        }
        // Never recursively delete: unknown files, including stale entries,
        // remain untouched even if another same-user process puts them here.
        let _ = fs::remove_dir(&self.directory.path);
    }
}

#[derive(Default)]
struct Connections {
    stopping: bool,
    next_id: u64,
    streams: HashMap<u64, UnixStream>,
}

pub struct LocalServer {
    id: String,
    connections: Arc<Mutex<Connections>>,
    _files: InstanceFiles,
}

impl LocalServer {
    pub fn start(session: SharedSession, workspace: &Path) -> Result<Self, String> {
        let workspace = workspace.canonicalize().map_err(|e| e.to_string())?;
        if workspace.to_str().is_none() {
            return Err("local MCP workspace must be valid UTF-8".into());
        }
        let root = registry();
        private_dir(&root, true)?;
        let (id, directory) = (0..8)
            .find_map(|_| {
                let id = match random_id() {
                    Ok(id) => id,
                    Err(e) => return Some(Err(e)),
                };
                let directory = root.join(&id);
                match fs::DirBuilder::new().mode(0o700).create(&directory) {
                    Ok(()) => Some(Ok((id, directory))),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => None,
                    Err(e) => Some(Err(e.to_string())),
                }
            })
            .unwrap_or_else(|| Err("could not allocate a unique local MCP session".into()))?;
        let mut files = InstanceFiles {
            directory: Identity::read(&directory)?,
            files: Vec::new(),
        };
        private_dir(&directory, false)?;
        let socket_path = directory.join(SOCKET);
        let listener =
            UnixListener::bind(&socket_path).map_err(|e| format!("bind local MCP socket: {e}"))?;
        files.files.push(Identity::read(&socket_path)?);
        fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let descriptor = json!({"version": 1, "id": id, "workspace": workspace, "pid": std::process::id(), "socket": SOCKET}).to_string();
        if descriptor.len() as u64 > MAX_DESCRIPTOR {
            return Err("local MCP workspace exceeds descriptor size limit".into());
        }
        let temporary = directory.join("session.tmp");
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        files.files.push(Identity::read(&temporary)?);
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        file.write_all(descriptor.as_bytes())
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        drop(file);
        fs::rename(&temporary, directory.join(DESCRIPTOR)).map_err(|e| e.to_string())?;
        files.files.last_mut().unwrap().path = directory.join(DESCRIPTOR);

        let connections = Arc::new(Mutex::new(Connections::default()));
        let state = connections.clone();
        thread::Builder::new()
            .name("openpatch-mcp-listener".into())
            .spawn(move || {
                loop {
                    if state.lock().unwrap_or_else(|e| e.into_inner()).stopping {
                        break;
                    }
                    match listener.accept() {
                        Ok((stream, _)) => {
                            if same_user(&stream).is_err() || stream.set_nonblocking(false).is_err()
                            {
                                continue;
                            }
                            let tracked = match stream.try_clone() {
                                Ok(stream) => stream,
                                Err(_) => continue,
                            };
                            let client_id = {
                                let mut clients = state.lock().unwrap_or_else(|e| e.into_inner());
                                if clients.stopping || clients.streams.len() >= MAX_CLIENTS {
                                    continue;
                                }
                                clients.next_id = clients.next_id.wrapping_add(1);
                                let id = clients.next_id;
                                clients.streams.insert(id, tracked);
                                id
                            };
                            let worker_state = state.clone();
                            let worker_session = session.clone();
                            if thread::Builder::new()
                                .name("openpatch-mcp-client".into())
                                .spawn(move || {
                                    // Cleanup runs on errors and unwinding as well as EOF.
                                    struct ClientGuard(Arc<Mutex<Connections>>, u64);
                                    impl Drop for ClientGuard {
                                        fn drop(&mut self) {
                                            self.0
                                                .lock()
                                                .unwrap_or_else(|e| e.into_inner())
                                                .streams
                                                .remove(&self.1);
                                        }
                                    }
                                    let _guard = ClientGuard(worker_state, client_id);
                                    let _ = crate::mcp::run_socket(stream, worker_session);
                                })
                                .is_err()
                            {
                                state
                                    .lock()
                                    .unwrap_or_else(|e| e.into_inner())
                                    .streams
                                    .remove(&client_id);
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                            let mut poll = libc::pollfd {
                                fd: listener.as_raw_fd(),
                                events: libc::POLLIN,
                                revents: 0,
                            };
                            // Incoming connections wake this immediately; an idle
                            // desktop pays only four stop checks per second.
                            let result = unsafe { libc::poll(&mut poll, 1, 250) };
                            if result < 0
                                && std::io::Error::last_os_error().kind()
                                    != std::io::ErrorKind::Interrupted
                            {
                                break;
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break,
                    }
                }
            })
            .map_err(|e| format!("start local MCP listener: {e}"))?;
        Ok(Self {
            id,
            connections,
            _files: files,
        })
    }

    pub fn session_id(&self) -> &str {
        &self.id
    }

    pub fn client_count(&self) -> usize {
        self.connections
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .streams
            .len()
    }
}

impl Drop for LocalServer {
    fn drop(&mut self) {
        let mut clients = self.connections.lock().unwrap_or_else(|e| e.into_inner());
        clients.stopping = true;
        for stream in clients.streams.values() {
            let _ = stream.shutdown(Shutdown::Both);
        }
        clients.streams.clear();
        // Do not join workers: a bounded offline render may still be finishing.
        // The detached admission thread observes stopping within 250 ms.
    }
}

fn safe_file(m: &Metadata, socket: bool) -> bool {
    m.uid() == uid()
        && m.mode() & 0o7777 == 0o600
        && if socket {
            m.file_type().is_socket()
        } else {
            m.is_file()
        }
}

fn read_descriptor(root: &Path, id: &str) -> Result<SessionInfo, String> {
    if !valid_id(id) {
        return Err("invalid local MCP session ID".into());
    }
    let directory = root.join(id);
    private_dir(&directory, false)?;
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(directory.join(DESCRIPTOR))
        .map_err(|e| e.to_string())?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !safe_file(&metadata, false) || metadata.len() > MAX_DESCRIPTOR {
        return Err("invalid local MCP descriptor ownership, permissions, or size".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_DESCRIPTOR + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_DESCRIPTOR {
        return Err("local MCP descriptor is too large".into());
    }
    let value: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    let object = value.as_object().ok_or("invalid local MCP descriptor")?;
    if object.len() != 5
        || value["version"].as_u64() != Some(1)
        || value["id"] != id
        || value["socket"] != SOCKET
    {
        return Err("invalid local MCP descriptor version, ID, or socket basename".into());
    }
    let pid = value["pid"]
        .as_u64()
        .filter(|p| *p > 0 && *p <= u32::MAX as u64)
        .ok_or("invalid local MCP PID")? as u32;
    let workspace = PathBuf::from(
        value["workspace"]
            .as_str()
            .ok_or("invalid local MCP workspace")?,
    );
    if !workspace.is_absolute() {
        return Err("local MCP workspace must be absolute".into());
    }
    let socket_metadata =
        fs::symlink_metadata(directory.join(SOCKET)).map_err(|e| e.to_string())?;
    if !safe_file(&socket_metadata, true) {
        return Err("invalid local MCP socket ownership or permissions".into());
    }
    Ok(SessionInfo {
        id: id.to_owned(),
        workspace,
        pid,
    })
}

fn same_user(stream: &UnixStream) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    let peer_uid = {
        let mut peer_uid = 0;
        let mut peer_gid = 0;
        if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut peer_uid, &mut peer_gid) } != 0 {
            return Err(io::Error::last_os_error());
        }
        peer_uid
    };
    #[cfg(target_os = "linux")]
    let peer_uid = {
        let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&mut credentials as *mut libc::ucred).cast(),
                &mut len,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        if len as usize != std::mem::size_of::<libc::ucred>() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid local MCP peer credentials",
            ));
        }
        credentials.uid
    };
    if peer_uid != uid() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "local MCP peer belongs to another user",
        ));
    }
    Ok(())
}

fn connect_socket(path: &Path) -> Result<UnixStream, String> {
    connect_socket_timeout(path, Duration::from_millis(150))
        .map_err(|e| format!("connect local MCP socket: {e}"))
}

fn connect_socket_timeout(path: &Path, timeout: Duration) -> io::Result<UnixStream> {
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    let bytes = path.as_os_str().as_bytes();
    if bytes.len() >= address.sun_path.len() || bytes.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid local MCP socket path",
        ));
    }
    for (target, source) in address.sun_path.iter_mut().zip(bytes) {
        *target = *source as libc::c_char;
    }
    #[cfg(target_os = "macos")]
    {
        address.sun_len = std::mem::size_of::<libc::sockaddr_un>() as u8;
    }
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0
        || unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, libc::O_NONBLOCK) } < 0
    {
        return Err(io::Error::last_os_error());
    }
    let result = unsafe {
        libc::connect(
            fd.as_raw_fd(),
            (&address as *const libc::sockaddr_un).cast(),
            std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
        )
    };
    if result < 0 {
        let e = std::io::Error::last_os_error();
        // Linux EAGAIN means a full Unix listen backlog: there is no pending
        // connection to poll, so fail promptly and let a future attach retry.
        if e.raw_os_error() != Some(libc::EINPROGRESS) {
            return Err(e);
        }
        let mut poll = libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLOUT,
            revents: 0,
        };
        let millis = timeout.as_millis().clamp(1, 150) as libc::c_int;
        match unsafe { libc::poll(&mut poll, 1, millis) } {
            0 => {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "local MCP connection timed out",
                ))
            }
            n if n < 0 => return Err(io::Error::last_os_error()),
            _ => {}
        }
        let mut error: libc::c_int = 0;
        let mut len = std::mem::size_of_val(&error) as libc::socklen_t;
        if unsafe {
            libc::getsockopt(
                fd.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_ERROR,
                (&mut error as *mut libc::c_int).cast(),
                &mut len,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        if error != 0 {
            return Err(io::Error::from_raw_os_error(error));
        }
    }
    let stream = UnixStream::from(fd);
    stream.set_nonblocking(false)?;
    same_user(&stream)?;
    Ok(stream)
}

pub fn sessions() -> Result<Vec<SessionInfo>, String> {
    Ok(discover(None, false)?
        .into_iter()
        .map(|(info, _)| info)
        .collect())
}

fn discover(
    workspace: Option<&Path>,
    retain_stream: bool,
) -> Result<Vec<(SessionInfo, Option<UnixStream>)>, String> {
    discover_with(workspace, retain_stream, connect_socket_timeout)
}

fn discover_with(
    workspace: Option<&Path>,
    retain_stream: bool,
    mut probe: impl FnMut(&Path, Duration) -> io::Result<UnixStream>,
) -> Result<Vec<(SessionInfo, Option<UnixStream>)>, String> {
    let deadline = Instant::now() + DISCOVERY_BUDGET;
    let root = registry();
    if !root.try_exists().map_err(|e| e.to_string())? {
        // symlink_metadata also catches dangling symlinks, which must fail closed.
        if fs::symlink_metadata(&root).is_ok() {
            return Err("invalid local MCP registry".into());
        }
        return Ok(Vec::new());
    }
    private_dir(&root, false)?;
    let mut result = Vec::new();
    for (index, entry) in fs::read_dir(&root).map_err(|e| e.to_string())?.enumerate() {
        if Instant::now() >= deadline {
            return Err(
                "local MCP discovery exceeded its 2-second deadline; specify --session".into(),
            );
        }
        if index >= MAX_ENTRIES {
            return Err("local MCP registry has too many entries".into());
        }
        let entry = entry.map_err(|e| e.to_string())?;
        let name = entry.file_name();
        let Some(id) = name.to_str().filter(|id| valid_id(id)) else {
            continue;
        };
        if let Ok(info) = read_descriptor(&root, id) {
            if workspace.is_some_and(|path| info.workspace != path) {
                continue;
            }
            let timeout = deadline
                .checked_duration_since(Instant::now())
                .ok_or("local MCP discovery exceeded its 2-second deadline; specify --session")?;
            match probe(&root.join(id).join(SOCKET), timeout) {
                Ok(stream) => {
                    // Attachment reuses this connection. Reopening would race
                    // our own probe for the last available client slot.
                    result.push((info, if retain_stream { Some(stream) } else { None }));
                }
                Err(e) if matches!(e.raw_os_error(), Some(libc::ENOENT | libc::ECONNREFUSED)) => {}
                Err(e) => return Err(format!("cannot determine whether local MCP session {id} is live: {e}; specify --session")),
            }
        }
    }
    if Instant::now() >= deadline {
        return Err("local MCP discovery exceeded its 2-second deadline; specify --session".into());
    }
    result.sort_by(|a, b| a.0.id.cmp(&b.0.id));
    Ok(result)
}

pub fn connect(session_id: Option<&str>, workspace: Option<&Path>) -> Result<UnixStream, String> {
    if session_id.is_some_and(|id| !valid_id(id)) {
        return Err(
            "invalid local MCP session ID (expected 32 lowercase hexadecimal characters)".into(),
        );
    }
    let workspace = workspace
        .map(Path::canonicalize)
        .transpose()
        .map_err(|e| format!("resolve local MCP workspace: {e}"))?;
    if let Some(id) = session_id {
        let root = registry();
        private_dir(&root, false)?;
        let info = read_descriptor(&root, id)?;
        if workspace
            .as_ref()
            .is_some_and(|path| info.workspace != *path)
        {
            return Err("local MCP session does not match the requested workspace".into());
        }
        return connect_socket(&root.join(id).join(SOCKET));
    }
    let mut candidates = discover(workspace.as_deref(), true)?;
    match candidates.len() {
        0 => Err(
            "no matching live OpenPatch session; open a desktop window or run openpatch sessions"
                .into(),
        ),
        1 => Ok(candidates.pop().unwrap().1.unwrap()),
        _ => Err(format!(
            "multiple live OpenPatch sessions match; choose --session from: {}",
            candidates
                .iter()
                .map(|(s, _)| s.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::Application;

    static REGISTRY_TEST: Mutex<()> = Mutex::new(());

    fn temporary() -> PathBuf {
        let path = PathBuf::from("/tmp").join(format!("opm-test-{}", random_id().unwrap()));
        fs::create_dir(&path).unwrap();
        path
    }

    fn eventually(mut check: impl FnMut() -> bool) {
        for _ in 0..100 {
            if check() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(check(), "local MCP lifecycle did not settle");
    }

    #[test]
    fn private_directory_rejects_permissions_and_symlinks() {
        let path = temporary();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(private_dir(&path, false).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        private_dir(&path, false).unwrap();
        let link = path.with_extension("link");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(private_dir(&link, true).is_err());
        fs::remove_file(link).unwrap();
        fs::remove_dir(path).unwrap();
    }

    #[test]
    fn multiple_windows_are_ambiguous_and_drop_disconnects() {
        let _serial = REGISTRY_TEST.lock().unwrap();
        let workspace = temporary();
        let session = Application::new(&workspace).unwrap().into_shared();
        let first = LocalServer::start(session.clone(), &workspace).unwrap();
        let second = LocalServer::start(session, &workspace).unwrap();
        assert_ne!(first.session_id(), second.session_id());
        assert!(connect(None, Some(&workspace))
            .unwrap_err()
            .contains("multiple"));
        let mut stream = connect(Some(first.session_id()), Some(&workspace)).unwrap();
        eventually(|| first.client_count() == 1);
        let directory = registry().join(first.session_id());
        stream
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        drop(first);
        assert_eq!(stream.read(&mut [0]).unwrap(), 0);
        assert!(!directory.exists());
        let stream = connect(None, Some(&workspace)).unwrap();
        drop(stream);
        eventually(|| second.client_count() == 0);
        drop(second);
        fs::remove_dir(workspace).unwrap();
    }

    #[test]
    fn eight_clients_are_bounded_and_disconnect_releases_capacity() {
        let _serial = REGISTRY_TEST.lock().unwrap();
        let workspace = temporary();
        let session = Application::new(&workspace).unwrap().into_shared();
        let server = LocalServer::start(session, &workspace).unwrap();
        let path = registry().join(server.session_id()).join(SOCKET);
        let mut streams = Vec::new();
        for _ in 0..MAX_CLIENTS - 1 {
            streams.push(connect(Some(server.session_id()), Some(&workspace)).unwrap());
        }
        eventually(|| server.client_count() == MAX_CLIENTS - 1);
        streams.push(connect(Some(server.session_id()), Some(&workspace)).unwrap());
        eventually(|| server.client_count() == MAX_CLIENTS);
        let mut overflow = connect_socket(&path).unwrap();
        overflow
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        assert_eq!(overflow.read(&mut [0]).unwrap(), 0);
        streams.pop();
        eventually(|| server.client_count() == MAX_CLIENTS - 1);
        // Workspace selection must reuse its discovery probe for the eighth
        // connection rather than consume that slot and reopen another socket.
        streams.push(connect(None, Some(&workspace)).unwrap());
        eventually(|| server.client_count() == MAX_CLIENTS);
        drop(streams);
        eventually(|| server.client_count() == 0);
        drop(server);
        fs::remove_dir(workspace).unwrap();
    }

    #[test]
    fn discovery_ignores_and_preserves_stale_descriptors() {
        let _serial = REGISTRY_TEST.lock().unwrap();
        let root = registry();
        private_dir(&root, true).unwrap();
        let id = random_id().unwrap();
        let directory = root.join(&id);
        private_dir(&directory, true).unwrap();
        let socket = directory.join(SOCKET);
        drop(UnixListener::bind(&socket).unwrap());
        fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
        let descriptor = directory.join(DESCRIPTOR);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&descriptor)
            .unwrap();
        write!(
            file,
            "{}",
            json!({"version":1,"id":id,"workspace":"/tmp","pid":std::process::id(),"socket":SOCKET})
        )
        .unwrap();
        drop(file);
        assert!(!sessions().unwrap().iter().any(|info| info.id == id));
        assert!(descriptor.exists());
        assert!(socket.exists());
        fs::remove_file(descriptor).unwrap();
        fs::remove_file(socket).unwrap();
        fs::remove_dir(directory).unwrap();
    }

    #[test]
    fn discovery_does_not_select_one_window_when_another_probe_is_indeterminate() {
        let _serial = REGISTRY_TEST.lock().unwrap();
        let workspace = temporary().canonicalize().unwrap();
        let session = Application::new(&workspace).unwrap().into_shared();
        let first = LocalServer::start(session.clone(), &workspace).unwrap();
        let second = LocalServer::start(session, &workspace).unwrap();
        for errno in [
            None,
            Some(libc::EAGAIN),
            Some(libc::EACCES),
            Some(libc::EMFILE),
        ] {
            let mut probes = 0;
            let mut uncertain_id = String::new();
            let error = discover_with(Some(&workspace), true, |path, timeout| {
                probes += 1;
                if probes == 2 {
                    uncertain_id = path
                        .parent()
                        .unwrap()
                        .file_name()
                        .unwrap()
                        .to_str()
                        .unwrap()
                        .to_owned();
                    Err(errno.map(io::Error::from_raw_os_error).unwrap_or_else(|| {
                        io::Error::new(io::ErrorKind::TimedOut, "injected probe timeout")
                    }))
                } else {
                    connect_socket_timeout(path, timeout)
                }
            })
            .unwrap_err();
            assert_eq!(probes, 2);
            assert!(error.contains(&uncertain_id));
            assert!(error.contains("specify --session"));
        }
        drop(first);
        drop(second);
        fs::remove_dir(workspace).unwrap();
    }

    #[test]
    fn descriptors_reject_arbitrary_socket_targets_and_are_bounded() {
        let root = temporary();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        let id = random_id().unwrap();
        let dir = root.join(&id);
        private_dir(&dir, true).unwrap();
        let descriptor = dir.join(DESCRIPTOR);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&descriptor)
            .unwrap();
        write!(
            file,
            "{}",
            json!({"version":1,"id":id,"workspace":"/tmp","pid":1,"socket":"/tmp/other.sock"})
        )
        .unwrap();
        assert!(read_descriptor(&root, &id).is_err());
        file.set_len(MAX_DESCRIPTOR + 1).unwrap();
        assert!(read_descriptor(&root, &id).is_err());
        assert!(read_descriptor(&root, "../escape").is_err());
        drop(file);
        fs::remove_file(descriptor).unwrap();
        fs::remove_dir(dir).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn cleanup_preserves_replaced_or_unknown_files() {
        let root = temporary();
        let path = root.join("own");
        fs::write(&path, "own").unwrap();
        let guard = InstanceFiles {
            directory: Identity::read(&root).unwrap(),
            files: vec![Identity::read(&path).unwrap()],
        };
        fs::rename(&path, root.join("old")).unwrap();
        fs::write(&path, "replacement").unwrap();
        drop(guard);
        assert_eq!(fs::read_to_string(&path).unwrap(), "replacement");
        fs::remove_file(path).unwrap();
        fs::remove_file(root.join("old")).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
