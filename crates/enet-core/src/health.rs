//! Host health sampling (CPU / memory).

use sysinfo::System;

/// Samples host resource usage for the GUI and flash-safety gate.
#[derive(Debug, Default)]
pub struct HealthMonitor {
    sys: System,
}

impl HealthMonitor {
    /// Create and perform an initial refresh.
    pub fn new() -> Self {
        let mut sys = System::new();
        sys.refresh_cpu_usage();
        sys.refresh_memory();
        Self { sys }
    }

    /// Refresh and return global CPU usage percent (0–100).
    pub fn cpu_pct(&mut self) -> f64 {
        self.sys.refresh_cpu_usage();
        self.sys.global_cpu_usage() as f64
    }

    /// Refresh and return used memory bytes.
    pub fn memory_used_bytes(&mut self) -> u64 {
        self.sys.refresh_memory();
        self.sys.used_memory()
    }

    /// Total memory bytes.
    pub fn memory_total_bytes(&mut self) -> u64 {
        self.sys.refresh_memory();
        self.sys.total_memory()
    }
}
