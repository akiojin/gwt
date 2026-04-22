use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;

const RELEASE_CONTRACT_JSON: &str = include_str!("../../../assets/release-assets.json");

#[derive(Debug, Deserialize)]
struct ReleaseContract {
    bundle_binaries: BTreeMap<String, Vec<String>>,
    portable_assets: BTreeMap<String, String>,
    installer_assets: BTreeMap<String, String>,
}

fn contract() -> &'static ReleaseContract {
    static CONTRACT: OnceLock<ReleaseContract> = OnceLock::new();
    CONTRACT.get_or_init(|| {
        serde_json::from_str(RELEASE_CONTRACT_JSON)
            .expect("assets/release-assets.json must be valid JSON")
    })
}

fn normalize_os(os: &str) -> &str {
    match os {
        "darwin" => "macos",
        "win32" => "windows",
        other => other,
    }
}

fn normalize_arch(arch: &str) -> &str {
    match arch {
        "arm64" => "aarch64",
        "x64" => "x86_64",
        other => other,
    }
}

pub fn bundle_binary_names(os: &str) -> Option<Vec<String>> {
    contract().bundle_binaries.get(normalize_os(os)).cloned()
}

pub fn portable_asset_name(os: &str, arch: &str) -> Option<String> {
    let key = format!("{}-{}", normalize_os(os), normalize_arch(arch));
    contract().portable_assets.get(&key).cloned()
}

pub fn installer_asset_name(os: &str) -> Option<String> {
    contract().installer_assets.get(normalize_os(os)).cloned()
}

#[cfg(test)]
mod tests {
    use super::{bundle_binary_names, installer_asset_name, portable_asset_name};

    #[test]
    fn release_contract_reads_shared_assets() {
        assert_eq!(
            portable_asset_name("windows", "x86_64").as_deref(),
            Some("gwt-windows-x86_64.zip")
        );
        assert_eq!(
            installer_asset_name("windows").as_deref(),
            Some("gwt-windows-x86_64.msi")
        );
        assert_eq!(
            bundle_binary_names("windows").expect("bundle binaries"),
            vec!["gwt.exe".to_string(), "gwtd.exe".to_string()]
        );
    }

    #[test]
    fn release_contract_normalizes_node_platform_and_arch_names() {
        assert_eq!(
            portable_asset_name("win32", "x64").as_deref(),
            Some("gwt-windows-x86_64.zip")
        );
        assert_eq!(
            bundle_binary_names("darwin").expect("bundle binaries"),
            vec!["gwt".to_string(), "gwtd".to_string()]
        );
    }
}
