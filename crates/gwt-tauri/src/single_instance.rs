use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    net::{TcpStream, ToSocketAddrs},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

const LOCK_FILE_NAME: &str = "app-instance.lock";
const FOCUS_COMMAND: &[u8] = b"focus\n";

#[derive(Debug)]
pub enum AcquireOutcome {
    Acquired(SingleInstanceGuard),
    AlreadyRunning(RunningInstance),
}

#[derive(Debug, Clone)]
pub struct RunningInstance {
    pub pid: Option<u32>,
    pub focus_port: Option<u16>,
}

#[derive(Debug)]
pub struct SingleInstanceGuard {
    lock_file: File,
    lock_path: PathBuf,
    owner_pid: u32,
    started_at_millis: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LockMetadata {
    pid: u32,
    started_at_millis: i64,
    #[serde(default)]
    focus_port: Option<u16>,
}

impl SingleInstanceGuard {
    pub fn set_focus_port(&self, focus_port: Option<u16>) -> std::io::Result<()> {
        let metadata = LockMetadata {
            pid: self.owner_pid,
            started_at_millis: self.started_at_millis,
            focus_port,
        };
        write_metadata(&self.lock_file, &metadata)
    }
}

impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        let _ = self.lock_file.unlock();
        info!(
            category = "single_instance",
            pid = self.owner_pid,
            lock_path = %self.lock_path.display(),
            "Released single-instance lock"
        );
    }
}

pub fn try_acquire_single_instance() -> std::io::Result<AcquireOutcome> {
    let path = default_lock_path()?;
    try_acquire_at(&path)
}

pub fn notify_existing_instance_focus() -> std::io::Result<()> {
    let lock_path = default_lock_path()?;
    notify_existing_instance_focus_at(&lock_path)
}

fn default_lock_path() -> std::io::Result<PathBuf> {
    let home = dirs::home_dir().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, "home directory not found")
    })?;
    Ok(home.join(".gwt").join(LOCK_FILE_NAME))
}

fn try_acquire_at(lock_path: &Path) -> std::io::Result<AcquireOutcome> {
    ensure_parent_dir(lock_path)?;
    let lock_file = open_lock_file(lock_path)?;

    match lock_file.try_lock_exclusive() {
        Ok(()) => {
            let guard = create_guard(lock_file, lock_path.to_path_buf())?;
            info!(
                category = "single_instance",
                pid = guard.owner_pid,
                lock_path = %guard.lock_path.display(),
                "Acquired single-instance lock"
            );
            Ok(AcquireOutcome::Acquired(guard))
        }
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
            let metadata = read_metadata(lock_path).ok().flatten();
            if let Some(meta) = metadata.as_ref() {
                if !process_is_alive(meta.pid) {
                    warn!(
                        category = "single_instance",
                        stale_pid = meta.pid,
                        lock_path = %lock_path.display(),
                        "Detected stale single-instance metadata; retrying lock"
                    );
                    std::thread::sleep(Duration::from_millis(50));
                    if lock_file.try_lock_exclusive().is_ok() {
                        let guard = create_guard(lock_file, lock_path.to_path_buf())?;
                        return Ok(AcquireOutcome::Acquired(guard));
                    }
                }
            }

            Ok(AcquireOutcome::AlreadyRunning(RunningInstance {
                pid: metadata.as_ref().map(|m| m.pid),
                focus_port: metadata.and_then(|m| m.focus_port),
            }))
        }
        Err(err) => Err(err),
    }
}

fn notify_existing_instance_focus_at(lock_path: &Path) -> std::io::Result<()> {
    let mut last_err: Option<std::io::Error> = None;
    for _ in 0..20 {
        let metadata = match read_metadata(lock_path) {
            Ok(Some(meta)) => meta,
            Ok(None) => {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            Err(err) => {
                last_err = Some(err);
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
        };

        let Some(port) = metadata.focus_port else {
            std::thread::sleep(Duration::from_millis(50));
            continue;
        };

        let addr = ("127.0.0.1", port)
            .to_socket_addrs()?
            .next()
            .ok_or_else(|| std::io::Error::other("failed to resolve focus endpoint"))?;
        match TcpStream::connect_timeout(&addr, Duration::from_millis(500)) {
            Ok(mut stream) => {
                stream.set_write_timeout(Some(Duration::from_millis(500)))?;
                stream.write_all(FOCUS_COMMAND)?;
                stream.flush()?;
                return Ok(());
            }
            Err(err) => {
                last_err = Some(err);
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }

    Err(last_err.unwrap_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "failed to notify existing instance",
        )
    }))
}

fn create_guard(lock_file: File, lock_path: PathBuf) -> std::io::Result<SingleInstanceGuard> {
    let owner_pid = std::process::id();
    let started_at_millis = now_millis();
    let guard = SingleInstanceGuard {
        lock_file,
        lock_path,
        owner_pid,
        started_at_millis,
    };
    guard.set_focus_port(None)?;
    Ok(guard)
}

fn open_lock_file(lock_path: &Path) -> std::io::Result<File> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(lock_path)
}

fn ensure_parent_dir(lock_path: &Path) -> std::io::Result<()> {
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(())
}

fn write_metadata(file: &File, metadata: &LockMetadata) -> std::io::Result<()> {
    let mut writer = file.try_clone()?;
    writer.set_len(0)?;
    writer.seek(SeekFrom::Start(0))?;
    let data = serde_json::to_vec(metadata)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    writer.write_all(&data)?;
    writer.flush()?;
    Ok(())
}

fn read_metadata(lock_path: &Path) -> std::io::Result<Option<LockMetadata>> {
    if !lock_path.exists() {
        return Ok(None);
    }

    let mut content = String::new();
    let mut file = File::open(lock_path)?;
    file.read_to_string(&mut content)?;
    if content.trim().is_empty() {
        return Ok(None);
    }
    let metadata = serde_json::from_str::<LockMetadata>(&content)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    Ok(Some(metadata))
}

#[cfg(unix)]
fn process_is_alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };

    match unsafe { libc::kill(pid, 0) } {
        0 => true,
        -1 => matches!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(err) if err == libc::EPERM
        ),
        _ => false,
    }
}

#[cfg(windows)]
fn process_is_alive(pid: u32) -> bool {
    use std::ffi::c_void;

    // Windows API constants
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259; // STATUS_PENDING

    extern "system" {
        fn OpenProcess(desired_access: u32, inherit_handle: i32, process_id: u32) -> *mut c_void;
        fn GetExitCodeProcess(process: *mut c_void, exit_code: *mut u32) -> i32;
        fn CloseHandle(object: *mut c_void) -> i32;
    }

    // SAFETY: calling well-known Windows API with valid arguments.
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle.is_null() {
            return false;
        }
        let mut exit_code: u32 = 0;
        let ok = GetExitCodeProcess(handle, &mut exit_code);
        CloseHandle(handle);
        ok != 0 && exit_code == STILL_ACTIVE
    }
}

#[cfg(not(any(unix, windows)))]
fn process_is_alive(_pid: u32) -> bool {
    false
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn acquire_fails_when_second_instance_is_running() {
        let tmp = TempDir::new().unwrap();
        let lock_path = tmp.path().join("app-instance.lock");

        let first = try_acquire_at(&lock_path).unwrap();
        let AcquireOutcome::Acquired(first_guard) = first else {
            panic!("first acquire should succeed");
        };

        let second = try_acquire_at(&lock_path).unwrap();
        match second {
            AcquireOutcome::Acquired(_) => panic!("second acquire must fail"),
            AcquireOutcome::AlreadyRunning(running) => {
                assert!(running.pid.is_some());
            }
        }

        drop(first_guard);
    }

    #[test]
    fn acquire_succeeds_after_guard_drop() {
        let tmp = TempDir::new().unwrap();
        let lock_path = tmp.path().join("app-instance.lock");

        let first = try_acquire_at(&lock_path).unwrap();
        let AcquireOutcome::Acquired(first_guard) = first else {
            panic!("first acquire should succeed");
        };
        drop(first_guard);

        let second = try_acquire_at(&lock_path).unwrap();
        match second {
            AcquireOutcome::Acquired(_) => {}
            AcquireOutcome::AlreadyRunning(_) => panic!("lock should be released after drop"),
        }
    }

    #[test]
    fn stale_metadata_without_lock_is_recovered() {
        let tmp = TempDir::new().unwrap();
        let lock_path = tmp.path().join("app-instance.lock");
        ensure_parent_dir(&lock_path).unwrap();
        let stale = LockMetadata {
            pid: 999_999,
            started_at_millis: 0,
            focus_port: Some(12345),
        };
        let mut file = open_lock_file(&lock_path).unwrap();
        file.set_len(0).unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        file.write_all(serde_json::to_string(&stale).unwrap().as_bytes())
            .unwrap();
        file.flush().unwrap();

        let outcome = try_acquire_at(&lock_path).unwrap();
        match outcome {
            AcquireOutcome::Acquired(_) => {}
            AcquireOutcome::AlreadyRunning(_) => {
                panic!("stale metadata should not block acquisition")
            }
        }
    }
}
