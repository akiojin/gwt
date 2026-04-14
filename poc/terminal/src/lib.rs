pub mod persistence;
pub mod preset;
pub mod workspace;

pub use persistence::{
    default_workspace_state, load_workspace_state, save_workspace_state, workspace_state_path,
    PersistedWindowState, PersistedWorkspaceState, WindowGeometry, WindowProcessStatus,
};
pub use preset::{
    detect_shell_program, resolve_launch_spec, LaunchSpec, PresetResolveError, ShellProgram,
    WindowPreset,
};
pub use workspace::WorkspaceState;
