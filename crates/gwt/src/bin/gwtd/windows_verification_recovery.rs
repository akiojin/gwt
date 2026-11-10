//! Move measured verification out of opaque GUI Jobs without relaxing admission
//! or CPU-cap observation. The Windows process service starts the private role;
//! its original caller owns a fresh kill-on-close Job before dispatch begins.

use std::{
    ffi::OsString,
    fs::{self, File},
    io,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        io::AsRawHandle,
    },
    path::{Path, PathBuf},
    process::{Child, Stdio},
    time::{Duration, Instant},
};

use base64::Engine;
use serde::{Deserialize, Serialize};
use windows::{
    core::{Owned, PCWSTR},
    Win32::{
        Foundation::{LocalFree, FILETIME, HANDLE, HLOCAL, WAIT_OBJECT_0, WAIT_TIMEOUT},
        Security::Cryptography::{
            CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        },
        System::{
            JobObjects::IsProcessInJob,
            Threading::{
                GetCurrentProcess, GetExitCodeProcess, GetProcessTimes, OpenProcess,
                TerminateProcess, WaitForSingleObject, PROCESS_QUERY_LIMITED_INFORMATION,
                PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
            },
        },
    },
};

const PRIVATE_ROLE: &str = "__gwt-windows-verification-recovery";
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_millis(100);
type Result<T> = std::result::Result<T, String>;

#[derive(Serialize, Deserialize)]
struct Context {
    parent_pid: u32,
    parent_birth: u64,
    session_id: u32,
    nonce: String,
    cwd: PathBuf,
    environment: Vec<(Vec<u16>, Vec<u16>)>,
    input: String,
}

#[derive(Serialize, Deserialize, PartialEq, Eq)]
struct Receipt {
    pid: u32,
    birth: u64,
    nonce: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    source: Option<String>,
}

impl Receipt {
    fn matches(&self, pid: u32, birth: u64, nonce: &str) -> bool {
        self.pid == pid && self.birth == birth && self.nonce == nonce
    }
}

/// This route runs before ordinary startup and cannot recurse into forwarding.
pub(super) fn run_private_role(argv: &[String], dispatch: impl FnOnce(&str) -> i32) -> Option<i32> {
    if argv.get(1).map(String::as_str) != Some(PRIVATE_ROLE) {
        return None;
    }
    Some(match argv {
        [_, _, ciphertext] => {
            let ciphertext = Path::new(ciphertext);
            let stdio = ciphertext
                .parent()
                .ok_or_else(|| "missing recovery directory".to_string())
                .and_then(redirect_stdio);
            match stdio {
                Ok(_stdio) => match worker(ciphertext, dispatch) {
                    Ok(code) => code,
                    Err(error) => {
                        eprintln!("verify: Windows Job recovery refused: {error}");
                        1
                    }
                },
                Err(_) => 1,
            }
        }
        _ => 2,
    })
}

pub(super) fn forward_if_needed(input: &str) -> Option<i32> {
    let result = match gwt::cli::verification_record::measurement::windows_recovery_reason(input) {
        Ok(None) => return None,
        Ok(Some(_reason)) => recover(input),
        Err(error) => Err(error),
    };
    Some(match result {
        Ok(code) => code,
        Err(error) => {
            println!(
                "{}",
                serde_json::json!({
                    "exit_code": 1, "ok": false, "operation": "verify.run",
                "output": format!("ENVIRONMENT_INCOMPATIBLE: Windows Job recovery failed: {error}. Keep the GUI open. Inspect verify.status before retrying the identical registered matrix with the same Session after repairing the reported environment. This recovery error claims no passing result.\n")
                })
            );
            1
        }
    })
}

fn recover(input: &str) -> Result<i32> {
    let directory = tempfile::Builder::new()
        .prefix("gwt-verification-recovery-")
        .tempdir()
        .map_err(|_| "create private recovery directory")?;
    let ciphertext = directory.path().join("context.dpapi");
    let context = Context {
        parent_pid: std::process::id(),
        parent_birth: birth(unsafe { GetCurrentProcess() })?,
        session_id: session_id(std::process::id())?,
        nonce: uuid::Uuid::new_v4().to_string(),
        cwd: std::env::current_dir().map_err(|_| "read verification directory")?,
        environment: std::env::vars_os()
            .map(|(key, value)| (key.encode_wide().collect(), value.encode_wide().collect()))
            .collect(),
        input: input.to_string(),
    };
    let mut plaintext = serde_json::to_vec(&context).map_err(|_| "encode recovery context")?;
    let encrypted = protect(&plaintext);
    plaintext.fill(0);
    fs::write(&ciphertext, encrypted?).map_err(|_| "write encrypted recovery context")?;
    let result = (|| {
        let pid = spawn_worker(&ciphertext, &context.cwd)?;
        // Until the nonce and native creation time agree, this could be a
        // recycled PID. An unverified handle must never arm termination.
        let process = open_process(pid, true)?;
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        let receipt = loop {
            if let Some(receipt) = read_receipt(&directory.path().join("ready.json"))? {
                if !receipt.matches(pid, birth(*process)?, &context.nonce) {
                    return Err("worker process identity changed".into());
                }
                break receipt;
            }
            if exited(*process)? || Instant::now() >= deadline {
                return Err("worker did not establish an outside-Job startup within 30s".into());
            }
            std::thread::sleep(POLL_INTERVAL);
        };
        let process = WorkerProcess(process);
        // Native readback is independent of the worker's receipt. Holding this
        // exact process handle also pins its identity while attach opens its PID.
        if session_id(pid)? != context.session_id || in_job(*process.0)? {
            return Err("worker is not outside Jobs in the caller's Windows session".into());
        }
        let job = gwt_core::process_tree::WindowsJobObject::attach_running(pid)
            .map_err(|_| "attach recovery worker containment")?;
        // Keep this query-only handle in the original caller for the entire
        // run. Descendants must prove native membership and this live caller's
        // creation time; the serialized descriptor grants nothing by itself.
        let source = gwt::process::windows_recovery::RecoverySource::new(&job)
            .map_err(|_| "retain native recovery source containment")?;
        let attached = Receipt {
            source: Some(source.descriptor()),
            ..receipt
        };
        write_receipt(&directory.path().join("attached.json"), &attached)?;
        while !exited(*process.0)? {
            std::thread::sleep(POLL_INTERVAL);
        }
        let mut code = 1;
        // SAFETY: this exact owned process is signaled and the output is live.
        unsafe { GetExitCodeProcess(*process.0, &mut code) }
            .map_err(|_| "read recovery worker exit status")?;
        drop(source);
        drop(job); // Reclaim any descendants before returning the result.
        Ok(code.min(255) as i32)
    })();
    // The closure's native owner reclaims the helper on every error path.
    for (name, stderr) in [("stdout.log", false), ("stderr.log", true)] {
        if let Ok(mut file) = File::open(directory.path().join(name)) {
            let copied = if stderr {
                io::copy(&mut file, &mut io::stderr().lock())
            } else {
                io::copy(&mut file, &mut io::stdout().lock())
            };
            copied.map_err(|_| "relay recovery worker output")?;
        }
    }
    result
}

fn worker(ciphertext: &Path, dispatch: impl FnOnce(&str) -> i32) -> Result<i32> {
    let directory = ciphertext.parent().ok_or("missing recovery directory")?;
    let encrypted = fs::read(ciphertext).map_err(|_| "read encrypted recovery context")?;
    let mut plaintext = unprotect(&encrypted)?;
    let context = serde_json::from_slice::<Context>(&plaintext);
    plaintext.fill(0);
    fs::remove_file(ciphertext).map_err(|_| "remove consumed recovery context")?;
    let context = context.map_err(|_| "decode recovery context")?;
    if !serde_json::from_str::<serde_json::Value>(&context.input).is_ok_and(|request| {
        request.get("operation").and_then(serde_json::Value::as_str) == Some("verify.run")
    }) {
        return Err("private recovery accepts verify.run only".into());
    }
    let parent = open_process(context.parent_pid, false)?;
    if birth(*parent)? != context.parent_birth
        || session_id(std::process::id())? != context.session_id
        || session_id(context.parent_pid)? != context.session_id
        || exited(*parent)?
    {
        return Err("recovery caller identity changed".into());
    }
    // Private startup is single-threaded. No credentials occur in argv, scripts,
    // receipts, or plaintext files; only this same-user DPAPI context carries them.
    for (key, _) in std::env::vars_os().collect::<Vec<_>>() {
        std::env::remove_var(key);
    }
    for (key, value) in &context.environment {
        std::env::set_var(OsString::from_wide(key), OsString::from_wide(value));
    }
    std::env::set_current_dir(&context.cwd).map_err(|_| "restore verification directory")?;
    let current = unsafe { GetCurrentProcess() };
    if in_job(current)? {
        return Err("Windows process service worker still belongs to a Job".into());
    }
    let receipt = Receipt {
        pid: std::process::id(),
        birth: birth(current)?,
        nonce: context.nonce,
        source: None,
    };
    write_receipt(&directory.join("ready.json"), &receipt)?;
    let deadline = Instant::now() + STARTUP_TIMEOUT;
    loop {
        if exited(*parent)? || Instant::now() >= deadline {
            return Err("caller exited or containment startup exceeded 30s".into());
        }
        if let Some(attached) = read_receipt(&directory.join("attached.json"))? {
            if !attached.matches(receipt.pid, receipt.birth, &receipt.nonce) {
                return Err("containment receipt identity changed".into());
            }
            let caps = gwt_core::process_tree::job_cpu::observe(receipt.pid)?;
            if !in_job(current)?
                || !caps.iter().any(|cap| cap.owner_pid == context.parent_pid)
                || caps.iter().any(|cap| cap.hard_cap_percent.is_some())
            {
                return Err("registered uncapped worker containment is not observable".into());
            }
            // Private startup is still single-threaded. The descriptor is
            // inherited by verification commands but always checked against
            // the kernel and the exact live originating process at use time.
            std::env::set_var(
                gwt::process::windows_recovery::SOURCE_ENV,
                attached
                    .source
                    .ok_or("missing recovery source descriptor")?,
            );
            // Capture before dispatch: later environment removal/replacement
            // must not erase the service worker's original source locator.
            let source = gwt::process::windows_recovery::capture_source()
                .ok_or("missing captured recovery source descriptor")?;
            if !gwt::process::checked_runtime_descendant_of(
                receipt.pid,
                context.parent_pid,
                Some(source),
            )
            .unwrap_or(false)
            {
                return Err("native recovery source containment is not observable".into());
            }
            break;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
    eprintln!(
        "verify: Windows Job recovery — worker {} outside Jobs; containment established",
        receipt.pid
    );
    Ok(dispatch(&context.input))
}

fn spawn_worker(ciphertext: &Path, cwd: &Path) -> Result<u32> {
    let executable = std::env::current_exe().map_err(|_| "resolve recovery executable")?;
    let executable = executable
        .to_str()
        .ok_or("recovery executable path is not Unicode")?;
    let ciphertext = ciphertext
        .to_str()
        .ok_or("recovery context path is not Unicode")?;
    let cwd = cwd
        .to_str()
        .ok_or("verification directory is not Unicode")?;
    let command_line = format!("\"{executable}\" {PRIVATE_ROLE} \"{ciphertext}\"");
    // Fixed inline script, containing paths only. Do not change ExecutionPolicy
    // or enable Job-wide breakaway; the service's result is checked natively.
    let script = format!(
        "$ErrorActionPreference='Stop';try {{$startup=New-CimInstance -ClassName Win32_ProcessStartup -ClientOnly -Property @{{CreateFlags=[uint32]150994944;ShowWindow=[uint16]0;WinstationDesktop='winsta0\\default'}};$result=Invoke-CimMethod -ClassName Win32_Process -MethodName Create -Arguments @{{CommandLine='{}';CurrentDirectory='{}';ProcessStartupInformation=$startup}};if($result.ReturnValue -ne 0){{exit 1}};[Console]::Write($result.ProcessId)}}catch{{exit 1}}",
        command_line.replace('\'', "''"), cwd.replace('\'', "''")
    );
    let encoded = base64::engine::general_purpose::STANDARD.encode(
        script
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    let mut command = gwt_core::process::resolved_command(
        gwt_core::process::ProcessPlanRequest::new("powershell.exe"),
    )
    .map_err(|_| "resolve hidden Windows process service launcher")?;
    let mut child = command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-EncodedCommand",
            &encoded,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "invoke Windows process service")?;
    wait_launcher(&mut child, Instant::now() + STARTUP_TIMEOUT)?;
    let output = child
        .wait_with_output()
        .map_err(|_| "collect Windows process service result")?;
    if !output.status.success() {
        return Err("Windows process service refused recovery startup".into());
    }
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<u32>()
        .map_err(|_| "Windows process service returned no worker PID".into())
}

fn wait_launcher(child: &mut Child, deadline: Instant) -> Result<()> {
    let error = loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(POLL_INTERVAL),
            Ok(None) => break "Windows process service launcher exceeded 30s",
            Err(_) => break "observe Windows process service launcher",
        }
    };
    // Kill only the exact launcher Child. A service-created worker whose PID
    // we never verified self-exits at its bounded pre-containment handshake.
    let _ = child.kill();
    let _ = child.wait();
    Err(error.into())
}

fn write_receipt(path: &Path, receipt: &Receipt) -> Result<()> {
    let bytes = serde_json::to_vec(receipt).map_err(|_| "encode worker receipt")?;
    gwt_core::atomic_file::write_atomic(path, &bytes).map_err(|_| "publish worker receipt".into())
}

fn read_receipt(path: &Path) -> Result<Option<Receipt>> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|_| "invalid worker receipt".into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err("read worker receipt".into()),
    }
}

fn open_process(pid: u32, terminate: bool) -> Result<Owned<HANDLE>> {
    let mut rights = PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SYNCHRONIZE;
    if terminate {
        rights |= PROCESS_TERMINATE;
    }
    // SAFETY: only query/synchronize and the exact helper's cleanup right.
    let handle = unsafe { OpenProcess(rights, false, pid) }.map_err(|_| "open recovery process")?;
    Ok(unsafe { Owned::new(handle) })
}

fn birth(handle: HANDLE) -> Result<u64> {
    let (mut created, mut exited, mut kernel, mut user) = (
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
        FILETIME::default(),
    );
    // SAFETY: the live process handle and exact writable buffers are valid.
    unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) }
        .map_err(|_| "query recovery process incarnation")?;
    Ok((u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime))
}

fn in_job(handle: HANDLE) -> Result<bool> {
    let mut result = windows::core::BOOL::default();
    // SAFETY: None requests native membership in any Job for this live handle.
    unsafe { IsProcessInJob(handle, None, &mut result) }
        .map_err(|_| "query native Job membership")?;
    Ok(result.as_bool())
}

fn exited(handle: HANDLE) -> Result<bool> {
    // SAFETY: all caller handles include SYNCHRONIZE and remain live.
    match unsafe { WaitForSingleObject(handle, 0) } {
        WAIT_OBJECT_0 => Ok(true),
        WAIT_TIMEOUT => Ok(false),
        _ => Err("observe recovery process lifetime".into()),
    }
}

struct WorkerProcess(Owned<HANDLE>);
impl Drop for WorkerProcess {
    fn drop(&mut self) {
        // SAFETY: this is the exact helper handle, never a re-opened PID. After
        // containment, closing the Job also reclaims every helper descendant.
        unsafe {
            let _ = TerminateProcess(*self.0, 1);
            let _ = WaitForSingleObject(*self.0, 5000);
        }
    }
}

#[link(name = "kernel32")]
extern "system" {
    fn ProcessIdToSessionId(pid: u32, session: *mut u32) -> i32;
    fn SetStdHandle(kind: u32, handle: *mut std::ffi::c_void) -> i32;
}

fn session_id(pid: u32) -> Result<u32> {
    let mut session = 0;
    // SAFETY: only writes the supplied session identifier buffer.
    if unsafe { ProcessIdToSessionId(pid, &mut session) } == 0 {
        return Err("query Windows process session".into());
    }
    Ok(session)
}

fn redirect_stdio(directory: &Path) -> Result<(File, File)> {
    let stdout = File::create(directory.join("stdout.log")).map_err(|_| "open recovery stdout")?;
    let stderr = File::create(directory.join("stderr.log")).map_err(|_| "open recovery stderr")?;
    // SAFETY: these owned files remain live throughout the private role; this
    // happens before Rust stdio or ordinary application startup is initialized.
    if unsafe { SetStdHandle(-11i32 as u32, stdout.as_raw_handle()) } == 0
        || unsafe { SetStdHandle(-12i32 as u32, stderr.as_raw_handle()) } == 0
    {
        return Err("redirect recovery output".into());
    }
    Ok((stdout, stderr))
}

fn protect(bytes: &[u8]) -> Result<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: input stays live, output receives DPAPI-owned memory. Omitting
    // LOCAL_MACHINE binds the ciphertext to CurrentUser; dialogs are forbidden.
    unsafe {
        CryptProtectData(
            &input,
            PCWSTR::null(),
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    }
    .map_err(|_| "encrypt recovery context with CurrentUser DPAPI")?;
    Ok(take_dpapi_blob(output))
}

fn unprotect(bytes: &[u8]) -> Result<Vec<u8>> {
    let input = CRYPT_INTEGER_BLOB {
        cbData: bytes.len() as u32,
        pbData: bytes.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // SAFETY: only same-user DPAPI data can decrypt; no credential dialog opens.
    unsafe {
        CryptUnprotectData(
            &input,
            None,
            None,
            None,
            None,
            CRYPTPROTECT_UI_FORBIDDEN,
            &mut output,
        )
    }
    .map_err(|_| "decrypt recovery context with CurrentUser DPAPI")?;
    Ok(take_dpapi_blob(output))
}

fn take_dpapi_blob(output: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    let bytes = if output.cbData == 0 {
        Vec::new()
    } else {
        // SAFETY: successful DPAPI calls supply this exact readable allocation.
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize) }.to_vec()
    };
    // SAFETY: DPAPI allocates this memory with LocalAlloc; free it exactly once.
    unsafe { LocalFree(Some(HLOCAL(output.pbData.cast()))) };
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_context_is_encrypted_for_the_same_user() {
        let plaintext = br#"{"input":"verify.run","session_token":"private-test-value"}"#;
        let ciphertext = protect(plaintext).expect("encrypt recovery context");
        assert!(!ciphertext
            .windows(plaintext.len())
            .any(|part| part == plaintext));
        assert_eq!(unprotect(&ciphertext).unwrap(), plaintext);
    }

    #[test]
    fn plaintext_or_damaged_context_is_refused() {
        assert!(unprotect(b"plaintext request and environment").is_err());
        let mut ciphertext = protect(b"private-test-value").expect("encrypt recovery context");
        ciphertext[0] ^= 0xff;
        assert!(unprotect(&ciphertext).is_err());
    }

    #[test]
    fn startup_receipt_requires_nonce_pid_and_native_incarnation() {
        let receipt = Receipt {
            pid: 7,
            birth: 11,
            nonce: "test-nonce".into(),
            source: None,
        };
        assert!(receipt.matches(7, 11, "test-nonce"));
        assert!(!receipt.matches(7, 12, "test-nonce"));
        assert!(!receipt.matches(8, 11, "test-nonce"));
        assert!(!receipt.matches(7, 11, "wrong-nonce"));
    }

    #[test]
    fn expired_launcher_is_killed_and_reaped() {
        let mut child = gwt_core::process::hidden_command("cmd.exe")
            .args(["/C", "pause"])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        assert!(wait_launcher(&mut child, Instant::now()).is_err());
        assert!(child.try_wait().unwrap().is_some());
    }
}
