//! `UpdateCliOps` trait + production wiring (`RealUpdateCliOps`).
//!
//! Split from `cli/update.rs` for SPEC-1942 SC-027 file size budget. The
//! parent `cli::update::mod` consumes the trait via `super::ops::*`.

use std::{
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
};

use gwt_core::update::{is_ci, InstallerKind, PreparedPayload, UpdateManager, UpdateState};

pub(super) trait UpdateCliOps {
    fn is_ci(&self) -> bool;
    fn current_exe(&self) -> io::Result<PathBuf>;
    fn current_args(&self) -> Vec<String>;
    fn read_line(&mut self, line: &mut String) -> io::Result<usize>;
    fn write_stdout(&mut self, text: &str);
    fn write_stderr(&mut self, text: &str);
    fn flush_stdout(&mut self) -> io::Result<()>;
    fn check_for_executable(&mut self, force: bool, current_exe: Option<&Path>) -> UpdateState;
    fn prepare_update(&mut self, latest: &str, asset_url: &str) -> Result<PreparedPayload, String>;
    fn write_restart_args_file(&mut self, path: &Path, args: Vec<String>) -> Result<(), String>;
    fn make_helper_copy(&mut self, current_exe: &Path, latest: &str) -> Result<PathBuf, String>;
    fn spawn_internal_apply_update(
        &mut self,
        helper_exe: &Path,
        old_pid: u32,
        current_exe: &Path,
        payload: &Path,
        args_file: &Path,
    ) -> Result<(), String>;
    fn spawn_internal_run_installer(
        &mut self,
        helper_exe: &Path,
        old_pid: u32,
        current_exe: &Path,
        installer: &Path,
        kind: InstallerKind,
        args_file: &Path,
    ) -> Result<(), String>;
}

pub(super) struct RealUpdateCliOps {
    mgr: UpdateManager,
}

impl Default for RealUpdateCliOps {
    fn default() -> Self {
        Self {
            mgr: UpdateManager::new(),
        }
    }
}

impl UpdateCliOps for RealUpdateCliOps {
    fn is_ci(&self) -> bool {
        is_ci()
    }

    fn current_exe(&self) -> io::Result<PathBuf> {
        std::env::current_exe()
    }

    fn current_args(&self) -> Vec<String> {
        std::env::args().skip(1).collect()
    }

    fn read_line(&mut self, line: &mut String) -> io::Result<usize> {
        io::stdin().lock().read_line(line)
    }

    fn write_stdout(&mut self, text: &str) {
        let _ = write!(io::stdout(), "{text}");
    }

    fn write_stderr(&mut self, text: &str) {
        let _ = write!(io::stderr(), "{text}");
    }

    fn flush_stdout(&mut self) -> io::Result<()> {
        io::stdout().flush()
    }

    fn check_for_executable(&mut self, force: bool, current_exe: Option<&Path>) -> UpdateState {
        self.mgr.check_for_executable(force, current_exe)
    }

    fn prepare_update(&mut self, latest: &str, asset_url: &str) -> Result<PreparedPayload, String> {
        self.mgr.prepare_update(latest, asset_url)
    }

    fn write_restart_args_file(&mut self, path: &Path, args: Vec<String>) -> Result<(), String> {
        self.mgr.write_restart_args_file(path, args)
    }

    fn make_helper_copy(&mut self, current_exe: &Path, latest: &str) -> Result<PathBuf, String> {
        self.mgr.make_helper_copy(current_exe, latest)
    }

    fn spawn_internal_apply_update(
        &mut self,
        helper_exe: &Path,
        old_pid: u32,
        current_exe: &Path,
        payload: &Path,
        args_file: &Path,
    ) -> Result<(), String> {
        self.mgr
            .spawn_internal_apply_update(helper_exe, old_pid, current_exe, payload, args_file)
    }

    fn spawn_internal_run_installer(
        &mut self,
        helper_exe: &Path,
        old_pid: u32,
        current_exe: &Path,
        installer: &Path,
        kind: InstallerKind,
        args_file: &Path,
    ) -> Result<(), String> {
        self.mgr.spawn_internal_run_installer(
            helper_exe,
            old_pid,
            current_exe,
            installer,
            kind,
            args_file,
        )
    }
}
