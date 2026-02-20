//! System resource monitoring (CPU, memory, GPU).

use std::sync::OnceLock;
use sysinfo::{CpuRefreshKind, MemoryRefreshKind, RefreshKind, System};

#[cfg(target_os = "macos")]
use std::sync::Mutex;
#[cfg(target_os = "macos")]
use std::time::{Duration, Instant};

#[cfg(target_os = "windows")]
use serde::Deserialize;
#[cfg(target_os = "windows")]
use wmi::{COMLibrary, WMIConnection};

/// Static GPU information (model name, total VRAM).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GpuStaticInfo {
    pub name: String,
    pub vram_total_bytes: Option<u64>,
}

/// Dynamic GPU information from NVIDIA (usage, VRAM).
#[derive(Debug, Clone, PartialEq)]
pub struct GpuDynamicInfo {
    pub name: String,
    pub usage_percent: f32,
    pub vram_used_bytes: u64,
    pub vram_total_bytes: u64,
}

/// Monitors CPU, memory, and GPU resources.
pub struct SystemMonitor {
    sys: System,
    gpu_static_cache: OnceLock<Vec<GpuStaticInfo>>,
    #[cfg(target_os = "macos")]
    gpu_static_last_failure_at: Mutex<Option<Instant>>,
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
            #[cfg(target_os = "macos")]
            gpu_static_last_failure_at: Mutex::new(None),
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

    /// Detect static GPU info from the local system.
    ///
    /// - macOS: `system_profiler` (with short retry cooldown on failures)
    /// - Windows: WMI `Win32_VideoController`
    /// - Linux/others: no static probe (dynamic NVIDIA info may still be available)
    pub fn gpu_static_infos(&self) -> Vec<GpuStaticInfo> {
        if let Some(cached) = self.gpu_static_cache.get() {
            return cached.clone();
        }

        #[cfg(target_os = "macos")]
        {
            let now = Instant::now();
            {
                let last_failure = match self.gpu_static_last_failure_at.lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
                if let Some(last_failure_at) = *last_failure {
                    if now.duration_since(last_failure_at) < GPU_PROBE_RETRY_COOLDOWN {
                        return Vec::new();
                    }
                }
            }

            let detected = detect_macos_gpus();
            if let Some(infos) = detected {
                let _ = self.gpu_static_cache.set(infos.clone());
                let mut last_failure = match self.gpu_static_last_failure_at.lock() {
                    Ok(guard) => guard,
                    Err(poisoned) => poisoned.into_inner(),
                };
                *last_failure = None;
                return infos;
            }

            let mut last_failure = match self.gpu_static_last_failure_at.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            };
            *last_failure = Some(now);
            Vec::new()
        }

        #[cfg(target_os = "windows")]
        {
            let detected = detect_windows_gpus();
            let _ = self.gpu_static_cache.set(detected.clone());
            detected
        }

        #[cfg(not(any(target_os = "macos", target_os = "windows")))]
        {
            let empty = Vec::new();
            let _ = self.gpu_static_cache.set(empty.clone());
            empty
        }
    }

    /// Dynamic GPU info from NVIDIA (requires `nvidia-gpu` feature on Linux/Windows).
    #[cfg(all(
        feature = "nvidia-gpu",
        any(target_os = "linux", target_os = "windows")
    ))]
    pub fn gpu_dynamic_info(&self) -> Vec<GpuDynamicInfo> {
        let Ok(nvml) = nvml_wrapper::Nvml::init() else {
            return Vec::new();
        };
        let Ok(device_count) = nvml.device_count() else {
            return Vec::new();
        };

        let mut infos = Vec::new();
        for index in 0..device_count {
            let Ok(device) = nvml.device_by_index(index) else {
                continue;
            };
            let Ok(name) = device.name() else {
                continue;
            };
            let Ok(utilization) = device.utilization_rates() else {
                continue;
            };
            let Ok(mem_info) = device.memory_info() else {
                continue;
            };
            infos.push(GpuDynamicInfo {
                name,
                usage_percent: utilization.gpu as f32,
                vram_used_bytes: mem_info.used,
                vram_total_bytes: mem_info.total,
            });
        }
        infos
    }

    /// Stub: always returns empty when NVIDIA GPU feature is not enabled.
    #[cfg(not(all(
        feature = "nvidia-gpu",
        any(target_os = "linux", target_os = "windows")
    )))]
    pub fn gpu_dynamic_info(&self) -> Vec<GpuDynamicInfo> {
        Vec::new()
    }
}

fn normalize_vram_total_bytes(bytes: Option<u64>) -> Option<u64> {
    bytes.filter(|v| *v > 0)
}

#[cfg(any(target_os = "macos", test))]
fn parse_macos_gpu_names(json: &serde_json::Value) -> Vec<String> {
    json.get("SPDisplaysDataType")
        .and_then(serde_json::Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(|entry| entry.get("sppci_model").and_then(serde_json::Value::as_str))
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(target_os = "macos")]
const GPU_PROBE_RETRY_COOLDOWN: Duration = Duration::from_secs(30);

#[cfg(target_os = "macos")]
fn detect_macos_gpus() -> Option<Vec<GpuStaticInfo>> {
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

    let status = wait_with_timeout(&mut child, Duration::from_secs(5));
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
    let names = parse_macos_gpu_names(&json);
    if names.is_empty() {
        return None;
    }

    Some(
        names
            .into_iter()
            .map(|name| GpuStaticInfo {
                name,
                vram_total_bytes: None,
            })
            .collect(),
    )
}

#[cfg(target_os = "windows")]
#[derive(Debug, Deserialize)]
struct Win32VideoController {
    #[serde(rename = "Name")]
    name: Option<String>,
    #[serde(rename = "AdapterRAM")]
    adapter_ram: Option<u64>,
}

#[cfg(target_os = "windows")]
fn detect_windows_gpus() -> Vec<GpuStaticInfo> {
    let Ok(com_library) = COMLibrary::new() else {
        return Vec::new();
    };
    let Ok(wmi) = WMIConnection::new(com_library.into()) else {
        return Vec::new();
    };

    let controllers: Vec<Win32VideoController> =
        match wmi.raw_query("SELECT Name, AdapterRAM FROM Win32_VideoController") {
            Ok(values) => values,
            Err(_) => return Vec::new(),
        };

    controllers
        .into_iter()
        .filter_map(|controller| {
            let name = controller.name?.trim().to_string();
            if name.is_empty() {
                return None;
            }
            Some(GpuStaticInfo {
                name,
                vram_total_bytes: normalize_vram_total_bytes(controller.adapter_ram),
            })
        })
        .collect()
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
    fn gpu_dynamic_info_returns_empty_without_nvidia_feature() {
        let monitor = SystemMonitor::new();
        assert!(monitor.gpu_dynamic_info().is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn gpu_static_infos_returns_entries_on_macos() {
        let monitor = SystemMonitor::new();
        // Headless/virtualized macOS can legitimately return no display model.
        // In that case we skip content assertions and only validate them when available.
        let infos = monitor.gpu_static_infos();
        if infos.is_empty() {
            return;
        }

        for info in infos {
            assert!(!info.name.is_empty(), "GPU name should not be empty");
            // Apple Silicon uses unified memory, so vram_total_bytes should be None
            assert!(
                info.vram_total_bytes.is_none(),
                "Apple Silicon should have vram_total_bytes = None"
            );
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn gpu_static_infos_are_cached() {
        let monitor = SystemMonitor::new();
        let first = monitor.gpu_static_infos();
        let second = monitor.gpu_static_infos();
        assert_eq!(first, second, "gpu_static_infos should be cached");
    }

    #[test]
    fn parse_macos_gpu_json() {
        let json_str = r#"{"SPDisplaysDataType":[{"sppci_model":"Apple M4 Max"},{"sppci_model":"Radeon Pro 5600M"}]}"#;
        let json: serde_json::Value = serde_json::from_str(json_str).unwrap();
        let names = parse_macos_gpu_names(&json);
        assert_eq!(
            names,
            vec!["Apple M4 Max".to_string(), "Radeon Pro 5600M".to_string()]
        );
    }

    #[test]
    fn normalize_vram_total_bytes_treats_zero_as_unknown() {
        assert_eq!(normalize_vram_total_bytes(Some(0)), None);
        assert_eq!(normalize_vram_total_bytes(None), None);
        assert_eq!(normalize_vram_total_bytes(Some(1024)), Some(1024));
    }
}

#[cfg(target_os = "macos")]
fn wait_with_timeout(
    child: &mut std::process::Child,
    timeout: Duration,
) -> Option<std::process::ExitStatus> {
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(_) => return None,
        }
    }
}
