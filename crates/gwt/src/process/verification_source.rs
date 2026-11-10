//! Authenticated caller ancestry for daemon-launched verification commands.
//!
//! The locator grants nothing: both ends authenticate the kernel peer PID.
//! Only the command root acknowledged by the daemon, while alive in the same
//! incarnation, can borrow its original caller's ancestry.
use std::{
    ffi::{OsStr, OsString},
    io::{self, Read, Write},
    os::unix::{
        io::AsRawFd,
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    sync::{Arc, Condvar, Mutex, OnceLock},
    thread::{self, JoinHandle},
    time::Duration,
};

pub(crate) const SOURCE_ENV: &str = "GWT_VERIFICATION_DAEMON_SOURCE";
const TIMEOUT: Duration = Duration::from_secs(3);

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Descriptor {
    pid: u32,
    birth: u64,
    socket: PathBuf,
}

#[derive(Clone, Copy)]
enum CommandState {
    Pending,
    Running(u32, u64),
    Revoked,
}

pub(crate) struct Source {
    descriptor: Descriptor,
    state: Arc<(Mutex<CommandState>, Condvar)>,
    worker: Option<JoinHandle<()>>,
    _directory: tempfile::TempDir,
}

impl Source {
    pub(crate) fn new() -> io::Result<Self> {
        // macOS sockaddr_un paths are limited to 104 bytes. A private directory
        // also prevents another user from replacing the socket pathname.
        let directory = tempfile::Builder::new()
            .prefix("gwt-source-")
            .tempdir_in("/tmp")?;
        let socket = directory.path().join("source");
        let listener = UnixListener::bind(&socket)?;
        let descriptor = Descriptor {
            pid: std::process::id(),
            birth: process_birth(std::process::id())
                .ok_or_else(|| io::Error::other("verification caller identity unavailable"))?,
            socket,
        };
        let state = Arc::new((Mutex::new(CommandState::Pending), Condvar::new()));
        let shared = state.clone();
        let worker = thread::Builder::new()
            .name("verification-source".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { break };
                    let (lock, changed) = &*shared;
                    let state = lock.lock().unwrap_or_else(|e| e.into_inner());
                    if matches!(*state, CommandState::Revoked) {
                        break;
                    }
                    // A command can reach doctor before VerificationAccepted reaches
                    // its caller. Wait for registration, never deny this valid race.
                    let (state, _) = changed
                        .wait_timeout_while(state, TIMEOUT, |state| {
                            matches!(state, CommandState::Pending)
                        })
                        .unwrap_or_else(|e| e.into_inner());
                    let allowed = match *state {
                        CommandState::Running(root, birth) => {
                            peer_pid(&stream).ok().is_some_and(|peer| {
                                let live = || process_birth(root) == Some(birth);
                                live()
                                    && (peer == root
                                        || super::is_descendant_of(peer, root).unwrap_or(false))
                                    && live()
                            })
                        }
                        _ => false,
                    };
                    drop(state);
                    let _ = stream.set_write_timeout(Some(TIMEOUT));
                    if stream.write_all(&[u8::from(allowed)]).is_ok() {
                        // macOS cannot query LOCAL_PEERPID after its peer has
                        // closed. Hold the connection through client revalidation.
                        let _ = stream.set_read_timeout(Some(TIMEOUT));
                        let _ = stream.read_exact(&mut [0]);
                    }
                }
            })?;
        Ok(Self {
            descriptor,
            state,
            worker: Some(worker),
            _directory: directory,
        })
    }

    pub(crate) fn register(&self, pid: u32) {
        let (lock, changed) = &*self.state;
        let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
        if matches!(*state, CommandState::Pending) {
            *state = process_birth(pid)
                .map(|birth| CommandState::Running(pid, birth))
                .unwrap_or(CommandState::Revoked);
        }
        changed.notify_all();
    }

    pub(crate) fn revoke(&self) {
        let (lock, changed) = &*self.state;
        *lock.lock().unwrap_or_else(|e| e.into_inner()) = CommandState::Revoked;
        changed.notify_all();
    }

    pub(crate) fn descriptor(&self) -> String {
        serde_json::to_string(&self.descriptor).expect("fixed verification source descriptor")
    }
}

impl Drop for Source {
    fn drop(&mut self) {
        self.revoke();
        // Wake accept; no global process or signal manipulation is needed.
        let _ = UnixStream::connect(&self.descriptor.socket);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub(crate) fn capture_source() -> Option<&'static OsStr> {
    static SOURCE: OnceLock<Option<OsString>> = OnceLock::new();
    SOURCE
        .get_or_init(|| std::env::var_os(SOURCE_ENV))
        .as_deref()
}

pub(crate) fn checked_descendant_of(pid: u32, ancestor: u32, source: &OsStr) -> io::Result<bool> {
    let denied = || {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            "daemon verification source is unproven",
        )
    };
    if pid != std::process::id() || ancestor == 0 {
        return Err(denied());
    }
    let descriptor: Descriptor =
        serde_json::from_slice(source.as_encoded_bytes()).map_err(|_| denied())?;
    let mut stream = UnixStream::connect(&descriptor.socket)?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    let authenticate = |stream: &UnixStream| -> io::Result<bool> {
        if peer_pid(stream)? != descriptor.pid || descriptor.pid == 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "daemon source peer identity mismatch",
            ));
        }
        if process_birth(descriptor.pid) != Some(descriptor.birth) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "daemon source origin expired",
            ));
        }
        Ok(descriptor.pid == ancestor
            || super::is_descendant_of(descriptor.pid, ancestor).map_err(io::Error::other)?)
    };
    let related = authenticate(&stream)?;
    let mut reply = [0];
    stream.read_exact(&mut reply)?;
    if reply != [1] {
        return Err(denied());
    }
    let authenticated = authenticate(&stream)? && related;
    stream.write_all(&[1])?;
    Ok(authenticated)
}

// Keep the full native creation identity for the new delegation boundary.
fn process_birth(pid: u32) -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        let pid = i32::try_from(pid).ok().filter(|pid| *pid > 0)?;
        // SAFETY: proc_pidinfo receives exactly its native proc_bsdinfo buffer.
        let mut info: libc::proc_bsdinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of_val(&info) as i32;
        let count = unsafe {
            libc::proc_pidinfo(pid, libc::PROC_PIDTBSDINFO, 0, (&raw mut info).cast(), size)
        };
        (count == size && info.pbi_start_tvsec > 0)
            .then(|| info.pbi_start_tvsec * 1_000_000 + info.pbi_start_tvusec)
    }
    #[cfg(not(target_os = "macos"))]
    {
        super::host_process_start_time(pid)
    }
}

fn peer_pid(stream: &UnixStream) -> io::Result<u32> {
    #[cfg(target_os = "macos")]
    {
        let mut pid: libc::pid_t = 0;
        let mut len = std::mem::size_of_val(&pid) as libc::socklen_t;
        // SAFETY: getsockopt writes only the supplied, correctly sized buffer.
        let result = unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_LOCAL,
                libc::LOCAL_PEERPID,
                (&raw mut pid).cast(),
                &mut len,
            )
        };
        if result == 0 && len as usize == std::mem::size_of_val(&pid) && pid > 0 {
            return Ok(pid as u32);
        }
        Err(io::Error::last_os_error())
    }
    #[cfg(target_os = "linux")]
    {
        // SAFETY: zeroed ucred is valid and getsockopt receives its exact size.
        let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of_val(&cred) as libc::socklen_t;
        let result = unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                (&raw mut cred).cast(),
                &mut len,
            )
        };
        if result == 0 && len as usize == std::mem::size_of_val(&cred) && cred.pid > 0 {
            return Ok(cred.pid as u32);
        }
        Err(io::Error::last_os_error())
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = stream;
        Err(io::ErrorKind::Unsupported.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command, Stdio};

    #[test]
    fn member_fixture() {
        let Ok(descriptor) = std::env::var("GWT_DAEMON_SOURCE_FIXTURE") else {
            return;
        };
        let ancestor = std::env::var("GWT_DAEMON_SOURCE_ANCESTOR")
            .unwrap()
            .parse()
            .unwrap();
        let result = checked_descendant_of(
            std::process::id(),
            ancestor,
            std::ffi::OsStr::new(&descriptor),
        );
        eprintln!("daemon source fixture result: {result:?}");
        let accepted = result.unwrap_or(false);
        assert_eq!(
            accepted,
            std::env::var("GWT_DAEMON_SOURCE_EXPECT").unwrap() == "true"
        );
        if std::env::var_os("GWT_DAEMON_SOURCE_HOLD").is_some() {
            println!("GWT_SOURCE_READY");
            io::stdout().flush().unwrap();
            // The parent releases this exact process after testing a foreign
            // client. EOF also releases it if the parent assertion unwinds.
            let mut release = [0];
            if io::stdin().read_exact(&mut release).is_ok() {
                assert!(
                    checked_descendant_of(
                        std::process::id(),
                        ancestor,
                        std::ffi::OsStr::new(&descriptor)
                    )
                    .is_err(),
                    "revoked member retained authority"
                );
            }
        }
    }

    fn start_probe(source: &Source, descriptor: &str, member: bool, hold: bool) -> Child {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "process::verification_source::tests::member_fixture",
                "--nocapture",
            ])
            .env("GWT_DAEMON_SOURCE_FIXTURE", descriptor)
            .env("GWT_DAEMON_SOURCE_ANCESTOR", std::process::id().to_string())
            .env("GWT_DAEMON_SOURCE_EXPECT", member.to_string())
            .stdin(Stdio::null());
        if hold {
            command
                .env("GWT_DAEMON_SOURCE_HOLD", "1")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped());
        }
        // SAFETY: setsid is async-signal-safe; detach from the caller tree's
        // process group just like the daemon's verification spawn.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    Err(io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
        let child = command.spawn().unwrap();
        if member {
            source.register(child.id());
        }
        child
    }

    fn probe(source: &Source, descriptor: &str) {
        assert!(start_probe(source, descriptor, false, false)
            .wait()
            .unwrap()
            .success());
    }

    #[test]
    fn daemon_source_accepts_only_registered_command_and_live_origin() {
        let source = Source::new().unwrap();
        let descriptor = source.descriptor();
        let mut root = start_probe(&source, &descriptor, true, true);
        let mut output = io::BufReader::new(root.stdout.take().unwrap());
        loop {
            let mut line = String::new();
            assert_ne!(
                output.read_line(&mut line).unwrap(),
                0,
                "root exited before its membership check"
            );
            if line.trim() == "GWT_SOURCE_READY" {
                break;
            }
        }
        assert!(root.try_wait().unwrap().is_none());
        // A copied descriptor must reject another caller while the registered
        // command is alive, rather than merely reject an expired root.
        probe(&source, &descriptor);
        let mut forged: serde_json::Value = serde_json::from_str(&descriptor).unwrap();
        forged["birth"] = serde_json::json!(0);
        probe(&source, &forged.to_string());
        let mut forged: serde_json::Value = serde_json::from_str(&descriptor).unwrap();
        forged["pid"] = serde_json::json!(root.id());
        probe(&source, &forged.to_string());
        source.revoke();
        probe(&source, &descriptor);
        root.stdin.take().unwrap().write_all(&[1]).unwrap();
        assert!(root.wait().unwrap().success());
    }
    #[test]
    fn connection_before_registration_waits_for_the_acknowledged_root() {
        let source = Source::new().unwrap();
        let mut stream = UnixStream::connect(&source.descriptor.socket).unwrap();
        stream.set_read_timeout(Some(TIMEOUT)).unwrap();
        source.register(std::process::id());
        let mut reply = [0];
        stream.read_exact(&mut reply).unwrap();
        assert_eq!(reply, [1]);
        stream.write_all(&[1]).unwrap();
    }
}
