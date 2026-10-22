//! GUI event adapter for the shared Docker launch output mirror.

use gwt::project_runtime::launch_output_mirror::DockerLaunchOutputMirror;

use super::{AppEventProxy, UserEvent};

/// Forward preparation output through the existing terminal output event.
pub(crate) fn start(proxy: AppEventProxy, window_id: String) -> DockerLaunchOutputMirror {
    DockerLaunchOutputMirror::start_with_hub(gwt_core::process_console::global(), move |data| {
        proxy.send(UserEvent::LaunchTerminalOutput {
            window_id: window_id.clone(),
            data,
        });
    })
}
