//! System resource monitoring (CPU, memory, GPU).

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
        Self { sys }
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

    /// Attempt to detect GPU model from system components.
    ///
    /// The `sysinfo` "component" feature is not enabled, so this always returns `None`.
    /// On NVIDIA systems, GPU info comes from `gpu_dynamic_info()` instead.
    pub fn gpu_static_info(&self) -> Option<GpuStaticInfo> {
        None
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
}
