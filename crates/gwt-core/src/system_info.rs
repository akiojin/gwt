//! System resource monitoring (CPU, memory, GPU).

use std::sync::OnceLock;
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

/// Static GPU information (model name, total VRAM).
#[derive(Debug, Clone)]
pub struct GpuStaticInfo {
    pub name: String,
    pub vram_total_bytes: Option<u64>,
}

/// Dynamic GPU information from NVIDIA (usage, VRAM).
#[derive(Debug, Clone)]
pub struct GpuDynamicInfo {
    pub usage_percent: f32,
    pub vram_used_bytes: u64,
    pub vram_total_bytes: u64,
}

/// Monitors CPU, memory, and GPU resources.
pub struct SystemMonitor {
    sys: System,
    gpu_static_cache: OnceLock<GpuStaticInfo>,
}

impl Default for SystemMonitor {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemMonitor {
    /// Create a new monitor with CPU and memory tracking enabled.
    pub fn new() -> Self {
        let sys = System::new_with_specifics(
            RefreshKind::nothing()
                .with_cpu(CpuRefreshKind::everything())
                .with_memory(MemoryRefreshKind::everything()),
        );
        Self {
            sys,
            gpu_static_cache: OnceLock::new(),
        }
    }

    /// Refresh CPU and memory readings.
    pub fn refresh(&mut self) {
        self.sys.refresh_cpu_usage();
        self.sys.refresh_memory();
    }

    /// Current global CPU usage as a percentage (0.0..=100.0).
    pub fn cpu_usage(&self) -> f32 {
        self.sys.global_cpu_usage()
    }

    /// Current memory usage: `(used_bytes, total_bytes)`.
    pub fn memory_info(&self) -> (u64, u64) {
        (self.sys.used_memory(), self.sys.total_memory())
    }

    /// Detect GPU model name from the system.
    ///
    /// On macOS, uses `system_profiler` to detect Apple Silicon GPU.
    /// On NVIDIA systems, dynamic GPU info comes from `gpu_dynamic_info()` instead.
    /// Successful detection is cached after the first success.
    /// Failed probes are retried on subsequent calls.
    pub fn gpu_static_info(&self) -> Option<GpuStaticInfo> {
        if let Some(cached) = self.gpu_static_cache.get() {
            return Some(cached.clone());
        }

        #[cfg(target_os = "macos")]
        {
            let detected = detect_macos_gpu()?;
            let _ = self.gpu_static_cache.set(detected.clone());
            Some(detected)
        }
        #[cfg(not(target_os = "macos"))]
        {
            None
        }
    }

    /// Dynamic GPU info from NVIDIA (requires `nvidia-gpu` feature on Linux/Windows).
    #[cfg(feature = "nvidia-gpu")]
    pub fn gpu_dynamic_info(&self) -> Option<GpuDynamicInfo> {
        match nvml_wrapper::Nvml::init() {
            Ok(nvml) => {
                let device = nvml.device_by_index(0).ok()?;
                let utilization = device.utilization_rates().ok()?;
                let mem_info = device.memory_info().ok()?;
                Some(GpuDynamicInfo {
                    usage_percent: utilization.gpu as f32,
                    vram_used_bytes: mem_info.used,
                    vram_total_bytes: mem_info.total,
                })
            }
            Err(_) => None,
        }
    }

    /// Stub: always returns `None` when NVIDIA GPU feature is not enabled.
    #[cfg(not(feature = "nvidia-gpu"))]
    pub fn gpu_dynamic_info(&self) -> Option<GpuDynamicInfo> {
        None
    }
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_gpu_name(json: &serde_json::Value) -> Option<String> {
    json.get("SPDisplaysDataType")?
        .as_array()?
        .first()?
        .get("sppci_model")?
        .as_str()
        .map(|s| s.to_string())
}

#[cfg(target_os = "macos")]
fn detect_macos_gpu() -> Option<GpuStaticInfo> {
    let mut child = crate::process::command("/usr/sbin/system_profiler")
        .args(["SPDisplaysDataType", "-json"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;

    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    };
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stdout, &mut buf);
        buf
    });

    let status = wait_with_timeout(&mut child, std::time::Duration::from_secs(5));
    let status = match status {
        Some(status) => status,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return None;
        }
    };

    if !status.success() {
        let _ = reader.join();
        return None;
    }

    let output = reader.join().ok()?;
    let json: serde_json::Value = serde_json::from_slice(&output).ok()?;
    let name = parse_macos_gpu_name(&json)?;
    Some(GpuStaticInfo {
        name,
        vram_total_bytes: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_monitor_new_succeeds() {
        let _monitor = SystemMonitor::new();
    }

    #[test]
    fn cpu_usage_in_range_after_refresh() {
        let mut monitor = SystemMonitor::new();
        monitor.refresh();
        // sysinfo may return 0.0 on first call, but it should still be in range
        let cpu = monitor.cpu_usage();
        assert!(
            (0.0..=100.0).contains(&cpu),
            "cpu_usage {} out of range",
            cpu
        );
    }

    #[test]
    fn memory_total_is_positive() {
        let mut monitor = SystemMonitor::new();
        monitor.refresh();
        let (_used, total) = monitor.memory_info();
        assert!(total > 0, "total memory should be > 0, got {}", total);
    }

    #[test]
    fn gpu_dynamic_info_returns_none_without_nvidia_feature() {
        let monitor = SystemMonitor::new();
        // On macOS without nvidia-gpu feature, this should always be None
        assert!(monitor.gpu_dynamic_info().is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn gpu_static_info_returns_some_on_macos() {
        let monitor = SystemMonitor::new();
        // Headless/virtualized macOS can legitimately return no display model.
        // In that case we skip content assertions and only validate them when available.
        let Some(info) = monitor.gpu_static_info() else {
            return;
        };
        assert!(!info.name.is_empty(), "GPU name should not be empty");
        // Apple Silicon uses unified memory, so vram_total_bytes should be None
        assert!(
            info.vram_total_bytes.is_none(),
            "Apple Silicon should have vram_total_bytes = None"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn gpu_static_info_is_cached() {
        let monitor = SystemMonitor::new();
        let first = monitor.gpu_static_info();
        let second = monitor.gpu_static_info();
        assert_eq!(
            first.as_ref().map(|i| &i.name),
            second.as_ref().map(|i| &i.name),
            "gpu_static_info should return the same result on repeated calls"
        );
    }

    #[test]
    fn parse_macos_gpu_json() {
        let json_str = r#"{"SPDisplaysDataType":[{"sppci_model":"Apple M4 Max"}]}"#;
        let json: serde_json::Value = serde_json::from_str(json_str).unwrap();
        let name = parse_macos_gpu_name(&json);
        assert_eq!(name, Some("Apple M4 Max".to_string()));
    }
}

#[cfg(target_os = "macos")]
fn wait_with_timeout(
    child: &mut std::process::Child,
    timeout: std::time::Duration,
) -> Option<std::process::ExitStatus> {
    let start = std::time::Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    return None;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    }
}
