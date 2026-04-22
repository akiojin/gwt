use std::{fs, path::PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace crates dir")
        .parent()
        .expect("repo root")
        .to_path_buf()
}

#[test]
fn root_repo_compose_uses_home_then_userprofile_for_host_config_mounts() {
    let compose_path = repo_root().join("docker-compose.yml");
    let compose = fs::read_to_string(&compose_path).expect("read root docker-compose.yml");

    assert!(
        compose.contains(
            "- ${HOME:-${USERPROFILE:?HOME or USERPROFILE must be set}}/.claude:/root/.claude-host:ro"
        ),
        "expected HOME/USERPROFILE fallback mount for .claude in {}",
        compose_path.display()
    );
    assert!(
        compose.contains(
            "- ${HOME:-${USERPROFILE:?HOME or USERPROFILE must be set}}/.codex:/root/.codex-host:ro"
        ),
        "expected HOME/USERPROFILE fallback mount for .codex in {}",
        compose_path.display()
    );
}
