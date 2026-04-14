use crate::branch_list::BranchListEntry;
use crate::file_tree::FileTreeEntry;
use crate::launch_wizard::{LaunchWizardAction, LaunchWizardView};
use crate::persistence::{
    CanvasViewport, PersistedWorkspaceState, WindowGeometry, WindowProcessStatus,
};
use crate::preset::WindowPreset;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArrangeMode {
    Tile,
    Stack,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FrontendEvent {
    FrontendReady,
    CreateWindow {
        preset: WindowPreset,
    },
    FocusWindow {
        id: String,
    },
    UpdateViewport {
        viewport: CanvasViewport,
    },
    ArrangeWindows {
        mode: ArrangeMode,
        bounds: WindowGeometry,
    },
    UpdateWindowGeometry {
        id: String,
        geometry: WindowGeometry,
        cols: u16,
        rows: u16,
    },
    CloseWindow {
        id: String,
    },
    TerminalInput {
        id: String,
        data: String,
    },
    LoadFileTree {
        id: String,
        path: Option<String>,
    },
    LoadBranches {
        id: String,
    },
    OpenLaunchWizard {
        id: String,
        branch_name: String,
    },
    LaunchWizardAction {
        action: LaunchWizardAction,
    },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BackendEvent {
    WorkspaceState {
        workspace: PersistedWorkspaceState,
    },
    TerminalOutput {
        id: String,
        data_base64: String,
    },
    TerminalSnapshot {
        id: String,
        data_base64: String,
    },
    TerminalStatus {
        id: String,
        status: WindowProcessStatus,
        detail: Option<String>,
    },
    FileTreeEntries {
        id: String,
        path: String,
        entries: Vec<FileTreeEntry>,
    },
    FileTreeError {
        id: String,
        path: String,
        message: String,
    },
    BranchEntries {
        id: String,
        entries: Vec<BranchListEntry>,
    },
    BranchError {
        id: String,
        message: String,
    },
    LaunchWizardState {
        wizard: Option<LaunchWizardView>,
    },
}
