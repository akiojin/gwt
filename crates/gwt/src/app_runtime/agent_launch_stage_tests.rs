//! SPEC-2809 (revised) — Tests for the Launch Wizard -> Console
//! `agent` tab stage emission. Confirms that `emit_agent_launch_stage`
//! pushes a banner line to the ProcessConsoleHub under the
//! `AgentBootstrap` kind so the Console window surfaces the launch
//! pipeline before the PTY pane takes over.
use super::{emit_agent_launch_stage, next_agent_launch_stage_id};
use gwt_core::process_console::{ProcessConsoleHub, ProcessKind, ProcessStream};

fn drain_lines(hub: &ProcessConsoleHub) -> Vec<String> {
    hub.snapshot_kind(ProcessKind::AgentBootstrap)
        .into_iter()
        .map(|line| line.message)
        .collect()
}

#[test]
fn launch_stage_ids_are_unique_per_caller() {
    let a = next_agent_launch_stage_id();
    let b = next_agent_launch_stage_id();
    assert!(b > a, "stage ids must strictly increase: {a} -> {b}");
}

#[test]
fn emit_agent_launch_stage_pushes_a_banner_line_to_global_hub() {
    // The global hub is installed lazily by `gwt_core::logging::init`
    // in production, but tests run without that bootstrap. Install
    // a hub before exercising the emit helper so the snapshot read
    // observes the same instance the helper writes to. `set_global`
    // succeeds at most once per process; ignore the result so this
    // test cooperates with peers that also install the hub.
    let _ = gwt_core::process_console::set_global(ProcessConsoleHub::new());
    let spawn_id = next_agent_launch_stage_id();
    emit_agent_launch_stage(spawn_id, "resolve_binary", "claude");
    let hub = gwt_core::process_console::global();
    let recent = hub.snapshot_kind(ProcessKind::AgentBootstrap);
    assert!(
        recent.iter().any(|line| line.spawn_id == spawn_id
            && line.message == "[resolve_binary] claude"
            && line.stream == ProcessStream::Stdout),
        "expected a banner for the resolve_binary stage, got: {recent:?}",
    );
}

#[test]
fn launch_stage_banner_includes_stage_label_in_message() {
    let hub = ProcessConsoleHub::new();
    for stage in ["prepare_env", "spawn_pty", "ready"] {
        hub.push(gwt_core::process_console::ProcessLine::new(
            ProcessKind::AgentBootstrap,
            42,
            ProcessStream::Stdout,
            format!("[{stage}] codex"),
        ));
    }
    let lines = drain_lines(&hub);
    assert_eq!(
        lines,
        vec![
            "[prepare_env] codex".to_string(),
            "[spawn_pty] codex".to_string(),
            "[ready] codex".to_string(),
        ]
    );
}
