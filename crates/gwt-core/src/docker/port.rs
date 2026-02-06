//! Port allocation and conflict resolution (SPEC-f5f5657e)
//!
//! Provides utilities for finding available ports and resolving port conflicts
//! when running multiple Docker containers.

use std::collections::HashMap;
use std::net::TcpListener;
use tracing::debug;

/// Default port range start
const DEFAULT_PORT_RANGE_START: u16 = 10000;
/// Default port range end
const DEFAULT_PORT_RANGE_END: u16 = 65535;
/// Maximum attempts to find an available port
const MAX_PORT_SEARCH_ATTEMPTS: u16 = 100;

/// Port allocator for finding available ports
#[derive(Debug, Clone)]
pub struct PortAllocator {
    /// Start of the port range to search
    range_start: u16,
    /// End of the port range to search
    range_end: u16,
}

impl Default for PortAllocator {
    fn default() -> Self {
        Self::new()
    }
}

impl PortAllocator {
    /// Create a new PortAllocator with default range (10000-65535)
    pub fn new() -> Self {
        Self {
            range_start: DEFAULT_PORT_RANGE_START,
            range_end: DEFAULT_PORT_RANGE_END,
        }
    }

    /// Create a PortAllocator with a custom range
    pub fn with_range(start: u16, end: u16) -> Self {
        Self {
            range_start: start,
            range_end: end,
        }
    }

    /// Check if a specific port is currently in use
    ///
    /// Attempts to bind to the port on localhost. If successful, the port is available.
    pub fn is_port_in_use(port: u16) -> bool {
        // Bind to loopback for reliable "is this port available locally?" checks.
        // On some platforms, binding 0.0.0.0 may not conflict with an existing
        // 127.0.0.1 bind, which would make this check flaky in tests and in practice.
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(_) => {
                debug!(category = "docker", port = port, "Port is available");
                false
            }
            Err(_) => {
                debug!(category = "docker", port = port, "Port is in use");
                true
            }
        }
    }

    /// Find an available port starting from the given base port
    ///
    /// If the base port is in use, searches incrementally within the configured range.
    /// Returns None if no available port is found within MAX_PORT_SEARCH_ATTEMPTS.
    pub fn find_available_port(&self, base_port: u16) -> Option<u16> {
        let start = base_port.max(self.range_start);
        let end = self.range_end;

        for offset in 0..MAX_PORT_SEARCH_ATTEMPTS {
            let port = start.saturating_add(offset);
            if port > end {
                break;
            }

            if !Self::is_port_in_use(port) {
                debug!(
                    category = "docker",
                    base_port = base_port,
                    allocated_port = port,
                    "Found available port"
                );
                return Some(port);
            }
        }

        debug!(
            category = "docker",
            base_port = base_port,
            "No available port found in range"
        );
        None
    }

    /// Allocate ports for a set of environment variable names
    ///
    /// For each env var name, finds an available port and returns a mapping
    /// of environment variable name to allocated port.
    pub fn allocate_ports(&self, port_env_vars: &[(&str, u16)]) -> HashMap<String, u16> {
        let mut allocated = HashMap::new();
        let mut used_ports: Vec<u16> = Vec::new();

        for (env_name, base_port) in port_env_vars {
            // Start searching from the base port, but skip already allocated ports
            let mut current_port = *base_port;

            while let Some(port) = self.find_available_port(current_port) {
                if !used_ports.contains(&port) {
                    allocated.insert(env_name.to_string(), port);
                    used_ports.push(port);
                    break;
                }
                // Port was allocated in this batch, try next
                current_port = port + 1;
            }
        }

        debug!(
            category = "docker",
            count = allocated.len(),
            "Allocated ports for environment variables"
        );

        allocated
    }

    /// Convert allocated ports to environment variable format
    ///
    /// Returns a HashMap suitable for passing to docker compose.
    pub fn ports_to_env(&self, allocated: &HashMap<String, u16>) -> HashMap<String, String> {
        allocated
            .iter()
            .map(|(k, v)| (k.clone(), v.to_string()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // T-301: Available port detection test
    #[test]
    fn test_find_available_port() {
        let allocator = PortAllocator::new();
        let port = allocator.find_available_port(10000);
        assert!(port.is_some());
        // The returned port should be >= 10000
        assert!(port.unwrap() >= 10000);
    }

    // T-302: Port in use check test
    #[test]
    fn test_is_port_in_use_available() {
        // Find a port that's available
        let allocator = PortAllocator::new();
        if let Some(port) = allocator.find_available_port(20000) {
            // Port should not be in use
            assert!(!PortAllocator::is_port_in_use(port));
        }
    }

    #[test]
    fn test_is_port_in_use_occupied() {
        // Bind to a port, then check if it's in use
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        // Port should be in use while listener is alive
        assert!(PortAllocator::is_port_in_use(port));

        // After dropping, port should be available again
        drop(listener);
        // Note: There might be a small delay before the port is released
    }

    // T-303: Range allocation test
    #[test]
    fn test_find_available_port_in_range() {
        let allocator = PortAllocator::with_range(30000, 30100);
        let port = allocator.find_available_port(30000);
        assert!(port.is_some());
        let port = port.unwrap();
        assert!((30000..=30100).contains(&port));
    }

    // T-304: Multiple port allocation test
    #[test]
    fn test_allocate_multiple_ports() {
        let allocator = PortAllocator::new();
        let ports = allocator.allocate_ports(&[("PORT_A", 40000), ("PORT_B", 40000)]);

        assert_eq!(ports.len(), 2);
        assert!(ports.contains_key("PORT_A"));
        assert!(ports.contains_key("PORT_B"));

        // The two ports should be different
        let port_a = ports.get("PORT_A").unwrap();
        let port_b = ports.get("PORT_B").unwrap();
        assert_ne!(port_a, port_b);
    }

    #[test]
    fn test_ports_to_env() {
        let allocator = PortAllocator::new();
        let mut allocated = HashMap::new();
        allocated.insert("WEB_PORT".to_string(), 8080u16);
        allocated.insert("API_PORT".to_string(), 3000u16);

        let env = allocator.ports_to_env(&allocated);
        assert_eq!(env.get("WEB_PORT"), Some(&"8080".to_string()));
        assert_eq!(env.get("API_PORT"), Some(&"3000".to_string()));
    }

    #[test]
    fn test_port_allocator_default() {
        let allocator = PortAllocator::default();
        assert_eq!(allocator.range_start, DEFAULT_PORT_RANGE_START);
        assert_eq!(allocator.range_end, DEFAULT_PORT_RANGE_END);
    }
}
