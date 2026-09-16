//! Command-local Playwright evidence captured by the verification executor.

use std::{fs, io, path::PathBuf, process::Command};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeadedE2eEvidence {
    pub chromium_dark_passed: u64,
    pub chromium_light_passed: u64,
    pub failed: u64,
    pub status: String,
}

impl HeadedE2eEvidence {
    pub fn passed(&self) -> bool {
        self.status == "passed"
            && (self.chromium_dark_passed > 0 || self.chromium_light_passed > 0)
            && self.failed == 0
    }
}

pub struct Capture {
    directory: tempfile::TempDir,
    report_path: PathBuf,
}

impl Capture {
    pub fn new() -> io::Result<Self> {
        let directory = tempfile::tempdir()?;
        fs::write(
            directory.path().join("reporter.cjs"),
            include_str!("headed_e2e_reporter.cjs"),
        )?;
        let report_path = directory.path().join("report.json");
        Ok(Self {
            directory,
            report_path,
        })
    }

    pub fn configure(&self, command: &mut Command) {
        command
            .arg("--headed")
            .arg("--trace=on")
            .arg(format!(
                "--reporter=list,{}",
                self.directory.path().join("reporter.cjs").display()
            ))
            .env("GWT_HEADED_E2E_REPORT", &self.report_path);
    }

    pub fn evidence(&self) -> Option<HeadedE2eEvidence> {
        serde_json::from_slice(&fs::read(&self.report_path).ok()?).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_successful_headed_browser_runs_pass() {
        let pass = HeadedE2eEvidence {
            chromium_dark_passed: 1,
            chromium_light_passed: 1,
            failed: 0,
            status: "passed".to_string(),
        };
        assert!(pass.passed());
        assert!(HeadedE2eEvidence {
            chromium_dark_passed: 0,
            ..pass.clone()
        }
        .passed());
        assert!(HeadedE2eEvidence {
            chromium_light_passed: 0,
            ..pass.clone()
        }
        .passed());
        assert!(!HeadedE2eEvidence {
            chromium_dark_passed: 0,
            chromium_light_passed: 0,
            ..pass.clone()
        }
        .passed());
        assert!(!HeadedE2eEvidence {
            failed: 1,
            ..pass.clone()
        }
        .passed());
        assert!(!HeadedE2eEvidence {
            status: "failed".to_string(),
            ..pass
        }
        .passed());
    }

    #[test]
    fn capture_reads_only_its_fresh_report() {
        let capture = Capture::new().unwrap();
        let other = Capture::new().unwrap();
        assert!(capture.evidence().is_none());
        let report =
            br#"{"chromium_dark_passed":1,"chromium_light_passed":1,"failed":0,"status":"passed"}"#;
        fs::write(&capture.report_path, report).unwrap();
        assert!(capture.evidence().unwrap().passed());
        assert!(other.evidence().is_none());
        fs::write(&capture.report_path, "incomplete").unwrap();
        assert!(capture.evidence().is_none());

        let mut command = gwt_core::process::hidden_command("playwright");
        capture.configure(&mut command);
        assert_eq!(command.get_args().next().unwrap(), "--headed");
        assert!(command.get_args().any(|arg| arg == "--trace=on"));
        assert!(command.get_envs().any(|(key, value)| {
            key == "GWT_HEADED_E2E_REPORT" && value == Some(capture.report_path.as_os_str())
        }));
    }
}
