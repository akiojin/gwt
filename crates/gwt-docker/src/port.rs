//! Port management utilities.
//!
//! Provides port availability checks and an allocator for finding free host
//! ports when running Docker containers.

use std::net::TcpListener;

use tracing::debug;

/// A mapping between a host port and a container port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortMapping {
    /// Port on the host machine.
    pub host_port: u16,
    /// Port inside the container.
    pub container_port: u16,
    /// Protocol (e.g. "tcp", "udp").
    pub protocol: String,
}

impl PortMapping {
    pub fn tcp(host: u16, container: u16) -> Self {
        Self {
            host_port: host,
            container_port: container,
            protocol: "tcp".to_string(),
        }
    }
}

/// Check whether a port is available for binding on localhost.
pub fn check_port_available(port: u16) -> bool {
    TcpListener::bind(("127.0.0.1", port)).is_ok()
}

/// Allocator that finds available host ports in a configurable range.
#[derive(Debug, Clone)]
pub struct PortAllocator {
    range_start: u16,
    range_end: u16,
}

impl Default for PortAllocator {
    fn default() -> Self {
        Self::new()
    }
}

const DEFAULT_RANGE_START: u16 = 10000;
const DEFAULT_RANGE_END: u16 = 65535;
const MAX_SEARCH_ATTEMPTS: u16 = 100;

impl PortAllocator {
    /// Create an allocator with the default range (10000-65535).
    pub fn new() -> Self {
        Self {
            range_start: DEFAULT_RANGE_START,
            range_end: DEFAULT_RANGE_END,
        }
    }

    /// Create an allocator with a custom range.
    pub fn with_range(start: u16, end: u16) -> Self {
        Self {
            range_start: start,
            range_end: end,
        }
    }

    /// Find an available port starting from `base_port`.
    ///
    /// Searches incrementally within the configured range, up to
    /// `MAX_SEARCH_ATTEMPTS` tries.
    pub fn find_available(&self, base_port: u16) -> Option<u16> {
        let start = base_port.max(self.range_start);
        for offset in 0..MAX_SEARCH_ATTEMPTS {
            let port = start.saturating_add(offset);
            if port > self.range_end {
                break;
            }
            if check_port_available(port) {
                debug!(category = "docker", port = port, "found available port");
                return Some(port);
            }
        }
        debug!(
            category = "docker",
            base_port = base_port,
            "no available port found"
        );
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_mapping_tcp() {
        let pm = PortMapping::tcp(8080, 80);
        assert_eq!(pm.host_port, 8080);
        assert_eq!(pm.container_port, 80);
        assert_eq!(pm.protocol, "tcp");
    }

    #[test]
    fn check_port_available_ephemeral() {
        // Port 0 asks the OS for any available port — should succeed.
        assert!(check_port_available(0));
    }

    #[test]
    fn check_port_in_use() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(!check_port_available(port));
        drop(listener);
    }

    #[test]
    fn find_available_in_default_range() {
        let alloc = PortAllocator::new();
        let port = alloc.find_available(10000);
        assert!(port.is_some());
        assert!(port.unwrap() >= 10000);
    }

    #[test]
    fn find_available_in_custom_range() {
        let alloc = PortAllocator::with_range(30000, 30100);
        let port = alloc.find_available(30000);
        assert!(port.is_some());
        let p = port.unwrap();
        assert!((30000..=30100).contains(&p));
    }

    #[test]
    fn default_allocator() {
        let alloc = PortAllocator::default();
        assert_eq!(alloc.range_start, DEFAULT_RANGE_START);
        assert_eq!(alloc.range_end, DEFAULT_RANGE_END);
    }
}
