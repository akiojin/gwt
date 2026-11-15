// Compile the real front door with the test dev-dependency graph.
#[cfg(not(test))]
include!("../src/bin/gwtd.rs");

// `cargo test --all-targets` must not start a daemon.
#[cfg(test)]
fn main() {}
