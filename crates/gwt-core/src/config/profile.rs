//! Profile management for environment variables

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Environment profile
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Profile {
    /// Profile name
    pub name: String,
    /// Environment variables
    pub env: HashMap<String, String>,
    /// Description
    #[serde(default)]
    pub description: String,
}

impl Profile {
    /// Create a new profile
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            env: HashMap::new(),
            description: String::new(),
        }
    }

    /// Add an environment variable
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Apply profile to current environment
    pub fn apply(&self) {
        for (key, value) in &self.env {
            std::env::set_var(key, value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_profile_builder() {
        let profile = Profile::new("test")
            .with_env("FOO", "bar")
            .with_env("BAZ", "qux");

        assert_eq!(profile.name, "test");
        assert_eq!(profile.env.get("FOO"), Some(&"bar".to_string()));
        assert_eq!(profile.env.get("BAZ"), Some(&"qux".to_string()));
    }
}
