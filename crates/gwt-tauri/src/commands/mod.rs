pub mod agent_config;
pub mod agents;
pub mod branch_suggest;
pub mod branches;
pub mod cleanup;
pub mod docker;
pub mod git_view;
pub mod profiles;
pub mod project;
pub mod sessions;
pub mod settings;
pub mod terminal;

#[cfg(test)]
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Helper for tests that need to manipulate HOME and XDG_CONFIG_HOME.
#[cfg(test)]
pub(crate) struct TestEnvGuard {
    prev_home: Option<std::ffi::OsString>,
    prev_xdg: Option<std::ffi::OsString>,
}

#[cfg(test)]
impl TestEnvGuard {
    pub(crate) fn new(home_path: &std::path::Path) -> Self {
        let prev_home = std::env::var_os("HOME");
        let prev_xdg = std::env::var_os("XDG_CONFIG_HOME");

        std::env::set_var("HOME", home_path);
        std::env::set_var("XDG_CONFIG_HOME", home_path.join(".config"));

        Self {
            prev_home,
            prev_xdg,
        }
    }
}

#[cfg(test)]
impl Drop for TestEnvGuard {
    fn drop(&mut self) {
        match &self.prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match &self.prev_xdg {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
    }
}

#[tauri::command]
pub fn greet(name: &str) -> String {
    format!("Hello, {}! Welcome to gwt.", name)
}
