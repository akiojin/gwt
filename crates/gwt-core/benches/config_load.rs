//! Configuration loading benchmarks

use criterion::{criterion_group, criterion_main, Criterion};
use gwt_core::config::Settings;
use std::process::Command;
use tempfile::TempDir;

fn create_test_repo() -> TempDir {
    let temp = TempDir::new().unwrap();
    Command::new("git")
        .args(["init"])
        .current_dir(temp.path())
        .output()
        .unwrap();
    temp
}

fn config_load_no_file_benchmark(c: &mut Criterion) {
    let temp = create_test_repo();
    let path = temp.path().to_path_buf();

    c.bench_function("config_load_no_file", |b| {
        b.iter(|| Settings::load(&path).unwrap())
    });
}

fn config_load_with_file_benchmark(c: &mut Criterion) {
    let temp = create_test_repo();
    let path = temp.path().to_path_buf();

    // Create a config file
    let config_content = r#"
protected_branches = ["main", "master", "develop", "release"]
default_base_branch = "develop"
worktree_root = ".worktrees"
debug = false
log_retention_days = 14

[web]
port = 3000
address = "127.0.0.1"
cors = true

[agent]
default_agent = "claude-code"
"#;
    std::fs::write(path.join(".gwt.toml"), config_content).unwrap();

    c.bench_function("config_load_with_file", |b| {
        b.iter(|| Settings::load(&path).unwrap())
    });
}

fn settings_default_benchmark(c: &mut Criterion) {
    c.bench_function("settings_default", |b| b.iter(Settings::default));
}

criterion_group!(
    benches,
    config_load_no_file_benchmark,
    config_load_with_file_benchmark,
    settings_default_benchmark,
);
criterion_main!(benches);
