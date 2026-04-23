use axum::{
    http::header,
    response::{Html, IntoResponse},
};

pub(crate) fn index_html() -> &'static str {
    include_str!("../web/index.html")
}

pub(crate) fn app_js() -> &'static str {
    include_str!("../web/app.js")
}

pub(crate) async fn index_handler() -> Html<&'static str> {
    Html(index_html())
}

pub(crate) async fn app_js_handler() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        app_js(),
    )
}

#[cfg(test)]
mod tests {
    use super::{app_js, index_html};

    fn frontend_bundle_source() -> &'static str {
        concat!(
            include_str!("../web/index.html"),
            "\n",
            include_str!("../web/app.js")
        )
    }

    #[test]
    fn embedded_web_terminal_copy_shortcut_uses_ctrl_shift_c() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("function installTerminalCopyHandlers"),
            "expected web terminal copy handler bootstrap in embedded html",
        );
        assert!(
            html.contains("terminal.attachCustomKeyEventHandler"),
            "expected xterm custom key handler for copy shortcut",
        );
        assert!(
            html.contains("event.ctrlKey"),
            "expected Ctrl modifier handling in embedded html",
        );
        assert!(
            html.contains("event.shiftKey"),
            "expected Shift modifier handling in embedded html",
        );
    }

    #[test]
    fn embedded_web_terminal_drag_selection_copies_on_mouseup() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("terminalRoot.addEventListener(\"mousedown\""),
            "expected drag selection tracking in embedded html",
        );
        assert!(
            html.contains("window.addEventListener(\"mouseup\"") && html.contains("handleMouseUp"),
            "expected mouse release copy handling in embedded html",
        );
        assert!(
            html.contains("function copyTerminalSelection"),
            "expected clipboard copy helper in embedded html",
        );
        assert!(
            html.contains("navigator.clipboard.writeText"),
            "expected clipboard write path in embedded html",
        );
    }

    #[test]
    fn embedded_web_terminal_clipboard_fallback_restores_terminal_focus() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("restoreFocus"),
            "expected clipboard fallback to invoke restoreFocus after textarea copy",
        );
        assert!(
            html.contains("writeClipboardText(selection")
                && html.contains("runtime.terminal.focus()"),
            "expected selection copy path to pass terminal focus restoration",
        );
    }

    #[test]
    fn embedded_web_terminal_writes_refresh_viewport_after_xterm_parse() {
        let html = frontend_bundle_source();
        let streaming_write = regex::Regex::new(
            r"runtime\.terminal\.write\(\s*decoder\.decode\(decodeBase64\(base64\),\s*\{\s*stream:\s*true\s*\}\),\s*\(\)\s*=>\s*\{\s*scheduleTerminalViewportRefresh\(windowId\);\s*\}\s*\);",
        )
        .expect("valid regex");
        let snapshot_write = regex::Regex::new(
            r"runtime\.terminal\.write\(\s*decoder\.decode\(decodeBase64\(base64\)\),\s*\(\)\s*=>\s*\{\s*scheduleTerminalViewportRefresh\(windowId\);\s*\}\s*\);",
        )
        .expect("valid regex");
        let hidden_geometry_sync = regex::Regex::new(
            r"if\s*\(\s*!canRefreshTerminalViewport\(windowId\)\s*\)\s*\{\s*if\s*\(\s*persist\s*\)\s*\{\s*sendGeometry\(windowId,\s*runtime\.terminal\.cols,\s*runtime\.terminal\.rows\);\s*\}\s*return;\s*\}",
        )
        .expect("valid regex");

        assert!(
            html.contains("function scheduleTerminalViewportRefresh(windowId)"),
            "expected terminal viewport refresh scheduling helper",
        );
        assert!(
            html.contains("viewportRefreshFrame"),
            "expected terminal runtime to debounce viewport refreshes",
        );
        assert!(
            streaming_write.is_match(html),
            "expected streaming terminal output to refresh viewport after xterm parses it",
        );
        assert!(
            snapshot_write.is_match(html),
            "expected terminal snapshots to refresh viewport after xterm parses them",
        );
        assert!(
            html.contains("cancelAnimationFrame(runtime.viewportRefreshFrame)"),
            "expected pending terminal viewport refresh frames to be cancelled during cleanup",
        );
        assert!(
            html.contains("if (runtime && runtime.viewportRefreshFrame !== null)"),
            "expected terminal cleanup to guard non-terminal windows before cancelling refresh frames",
        );
        assert!(
            html.contains("function canRefreshTerminalViewport(windowId)")
                && html.contains("!workspaceWindowById(windowId)?.minimized"),
            "expected terminal viewport refresh to skip minimized windows",
        );
        assert!(
            hidden_geometry_sync.is_match(html),
            "expected persisted terminal fit to sync geometry even while hidden",
        );
        assert!(
            html.contains("const wasMinimized = element.classList.contains(\"minimized\")")
                && html.contains(
                    "const shouldPersistTerminalGeometry = wasMinimized && !windowData.minimized",
                )
                && html.contains("fitTerminal(windowData.id, shouldPersistTerminalGeometry)"),
            "expected restored terminals to persist fitted geometry after becoming visible",
        );
    }

    #[test]
    fn embedded_web_repo_browser_scroll_surfaces_block_canvas_pan_at_edges() {
        let html = frontend_bundle_source();
        let scroll_gate = regex::Regex::new(
            r"if\s*\(\s*!event\.ctrlKey\s*&&\s*!event\.metaKey\s*&&\s*nativeWheelScrollSurface\s*\)\s*\{\s*return;\s*\}",
        )
        .expect("valid regex");

        assert!(
            html.contains("function findNativeWheelScrollSurface"),
            "expected embedded html to define a repo browser wheel routing helper",
        );
        assert!(
            html.contains(".branch-scroll") && html.contains(".file-tree-scroll"),
            "expected embedded html to reference repo browser scroll containers",
        );
        assert!(
            html.contains(
                "const nativeWheelScrollSurface = findNativeWheelScrollSurface(event.target);",
            ),
            "expected plain wheel handling to route repo browser surfaces through the shared helper",
        );
        assert!(
            scroll_gate.is_match(html),
            "expected plain wheel input on repo browser surfaces to stay within the window even at scroll edges",
        );
        assert!(
            !html.contains("canScrollSurfaceConsumeWheelDelta"),
            "expected repo browser wheel routing to stop using edge fallback heuristics",
        );
    }

    #[test]
    fn embedded_web_canvas_wheel_routing_is_installed_through_named_handler() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("function handleCanvasWheelEvent(event)"),
            "expected canvas wheel routing to live in a named handler",
        );
        assert!(
            html.contains("function installCanvasWheelRouting()"),
            "expected wheel routing bootstrap to be isolated behind an installer",
        );
        assert!(
            html.contains("document.addEventListener(\"wheel\", handleCanvasWheelEvent, { capture: true, passive: false })"),
            "expected capture-phase wheel routing to be installed through the named handler",
        );
    }

    #[test]
    fn embedded_web_canvas_stage_keeps_transform_layer_hint_opt_in() {
        let html = index_html();

        assert!(
            html.contains(".canvas-stage"),
            "expected embedded html to define the canvas stage surface",
        );
        assert!(
            !html.contains("will-change: transform"),
            "expected canvas stage css to avoid pinning the transform layer hint",
        );
    }

    #[test]
    fn embedded_web_canvas_apply_viewport_debounces_raster_hint_reset() {
        let js = app_js();
        let apply_viewport = regex::Regex::new(
            r#"(?s)function applyViewport\(\)\s*\{\s*stage\.style\.transform = `translate\(\$\{viewport\.x\}px, \$\{viewport\.y\}px\) scale\(\$\{viewport\.zoom\}\)`;\s*stage\.style\.willChange = "transform";\s*if \(viewportRasterTimer !== null\) \{\s*clearTimeout\(viewportRasterTimer\);\s*\}\s*viewportRasterTimer = setTimeout\(\(\) => \{\s*stage\.style\.willChange = "auto";\s*viewportRasterTimer = null;\s*\}, 300\);\s*\}"#,
        )
        .expect("valid regex");

        assert!(
            js.contains("let viewportRasterTimer = null;"),
            "expected viewport apply flow to keep a dedicated raster debounce timer",
        );
        assert!(
            apply_viewport.is_match(js),
            "expected viewport application to opt into the transform layer only during motion and reset it after 300ms",
        );
    }

    #[test]
    fn embedded_web_window_status_chip_uses_running_waiting_stopped_error_variants() {
        let html = index_html();

        assert!(
            html.contains(".status-chip.waiting .status-dot"),
            "expected embedded html to define a waiting variant for window status chips",
        );
        assert!(
            html.contains(".status-chip.stopped .status-dot"),
            "expected embedded html to define a stopped variant for window status chips",
        );
        assert!(
            !html.contains(".status-chip.ready .status-dot")
                && !html.contains(".status-chip.exited .status-dot"),
            "expected embedded html to stop styling legacy ready/exited status chip variants",
        );
    }

    #[test]
    fn embedded_web_window_state_visualization_normalizes_runtime_state_and_separates_geometry() {
        let js = app_js();

        assert!(
            js.contains("function normalizeWindowRuntimeState(status, preset)"),
            "expected embedded js to expose a runtime-state normalization helper",
        );
        assert!(
            js.contains("function windowGeometryLabel(windowData)"),
            "expected embedded js to expose a dedicated geometry label helper",
        );
        assert!(
            js.contains("function windowRuntimeLabel(status)"),
            "expected embedded js to expose a dedicated runtime label helper",
        );
        assert!(
            js.contains("const geometryLabel = windowGeometryLabel(entry);")
                && js.contains("const runtimeState = runtimeStateForWindow(entry);")
                && js.contains("const runtimeLabel = windowRuntimeLabel(runtimeState);"),
            "expected window list rendering to derive geometry and runtime labels through separate helpers",
        );
        assert!(
            !js.contains("function windowStateLabel(windowData)"),
            "expected embedded js to stop reusing one helper for both geometry and runtime labels",
        );
    }

    #[test]
    fn embedded_web_agent_color_styles_define_palette_and_accent_surfaces() {
        let html = index_html();

        assert!(
            html.contains("--agent-claude")
                && html.contains("--agent-codex")
                && html.contains("--agent-gemini")
                && html.contains("--agent-opencode")
                && html.contains("--agent-copilot")
                && html.contains("--agent-custom"),
            "expected embedded html to define the AgentColor palette variables",
        );
        assert!(
            html.contains("[data-agent-color=\"yellow\"]")
                && html.contains("[data-agent-color=\"cyan\"]")
                && html.contains("[data-agent-color=\"magenta\"]")
                && html.contains("[data-agent-color=\"green\"]")
                && html.contains("[data-agent-color=\"blue\"]")
                && html.contains("[data-agent-color=\"gray\"]"),
            "expected embedded html to map serialized AgentColor values to the shared CSS variable",
        );
        assert!(
            html.contains(".workspace-window[data-agent-color]::before")
                && html.contains(".window-list-row[data-agent-color]::before"),
            "expected embedded html to expose agent-color accent bars for workspace windows and the window list",
        );
        assert!(
            html.contains(".agent-dot"),
            "expected embedded html to style the shared agent color dot surface",
        );
    }

    #[test]
    fn embedded_web_agent_color_is_bound_for_windows_wizard_and_board() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("if (windowData.agent_color)")
                && html.contains("element.dataset.agentColor = windowData.agent_color"),
            "expected embedded bundle to bind workspace window colors from windowData.agent_color",
        );
        assert!(
            html.contains("row.dataset.agentColor = entry.agent_color"),
            "expected embedded bundle to bind window list rows from entry.agent_color",
        );
        assert!(
            html.contains("card.dataset.agentColor = entry.agent_color"),
            "expected embedded bundle to bind board cards from entry.agent_color",
        );
        assert!(
            html.contains("if (entry.agent_color)")
                && html.contains("createNode(\"span\", \"agent-dot\")"),
            "expected embedded bundle to render board entry agent dots when agent_color is present",
        );
        assert!(
            html.contains("if (option.color)")
                && html.contains("button.dataset.agentColor = option.color"),
            "expected embedded bundle to bind launch wizard agent colors from option.color",
        );
    }

    #[test]
    fn embedded_web_shell_windows_do_not_render_waiting_status() {
        let js = app_js();

        assert!(
            js.contains("function presetSupportsWaitingStatus(preset)"),
            "expected embedded js to isolate the waiting-capable preset contract",
        );
        assert!(
            js.contains(
                "if (!presetSupportsWaitingStatus(preset) && normalizedState === \"waiting\")"
            ) && js.contains("return \"running\";"),
            "expected embedded js to downgrade waiting to running for shell-like presets",
        );
    }

    #[test]
    fn embedded_web_apply_status_keeps_window_list_and_badges_in_sync() {
        let js = app_js();
        let apply_status = regex::Regex::new(
            r#"(?s)function applyStatus\(windowId,\s*status,\s*detail\)\s*\{.*?const runtimeState = normalizeWindowRuntimeState\(status,\s*windowData\?\.preset\);.*?windowRuntimeStateMap\.set\(windowId,\s*runtimeState\);.*?label\.textContent = windowRuntimeLabel\(runtimeState\);.*?renderWindowList\(\);"#,
        )
        .expect("valid regex");

        assert!(
            js.contains("const windowRuntimeStateMap = new Map();"),
            "expected embedded js to keep a shared runtime-state map for badges and the window list",
        );
        assert!(
            apply_status.is_match(js),
            "expected applyStatus to normalize runtime state once, update the shared map, and re-render the window list",
        );
    }

    #[test]
    fn embedded_web_window_list_selection_keeps_focus_center_and_restore_contract() {
        let js = app_js();

        assert!(
            js.contains("focusWindowRemotely(entry.id, { center: true });"),
            "expected window list selection to keep centering the chosen window",
        );
        assert!(
            js.contains("if (entry.minimized) {")
                && js.contains("send({ kind: \"restore_window\", id: entry.id });"),
            "expected window list selection to keep restoring minimized windows after focus",
        );
    }

    #[test]
    fn embedded_web_socket_protocol_wiring_uses_named_handlers() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("function handleSocketOpen()"),
            "expected socket open flow to live in a named handler",
        );
        assert!(
            html.contains("function handleSocketMessage(event)"),
            "expected socket message flow to live in a named handler",
        );
        assert!(
            html.contains("function handleSocketClose()"),
            "expected socket close flow to live in a named handler",
        );
        assert!(
            html.contains("function installSocketEventHandlers(activeSocket)"),
            "expected socket listener registration to be isolated behind an installer",
        );
        assert!(
            html.contains("activeSocket.addEventListener(\"open\", handleSocketOpen)")
                && html.contains("activeSocket.addEventListener(\"message\", handleSocketMessage)")
                && html.contains("activeSocket.addEventListener(\"close\", handleSocketClose)"),
            "expected socket listeners to be registered through named handlers",
        );
    }

    #[test]
    fn embedded_web_socket_open_replays_frontend_ready_before_flushing_pending_messages() {
        let html = frontend_bundle_source();
        let open_flow = regex::Regex::new(
            r#"function handleSocketOpen\(\)\s*\{\s*setConnectionState\(true\);\s*send\(\{\s*kind:\s*"frontend_ready"\s*\}\);\s*while\s*\(\s*pendingMessages\.length\s*>\s*0\s*\)\s*\{\s*socket\.send\(JSON\.stringify\(pendingMessages\.shift\(\)\)\);\s*\}\s*\}"#,
        )
        .expect("valid regex");

        assert!(
            html.contains("function connectSocket()"),
            "expected socket transport bootstrap helper in embedded html",
        );
        assert!(
            html.contains("socket = new WebSocket(websocketUrl());")
                && html.contains("setConnectionState(false);")
                && html.contains("installSocketEventHandlers(socket);"),
            "expected socket bootstrap to create the websocket, reset connection state, and install handlers",
        );
        assert!(
            open_flow.is_match(html),
            "expected socket open flow to announce frontend readiness before replaying queued messages",
        );
    }

    #[test]
    fn embedded_web_websocket_contract_stays_host_neutral_for_browser_and_native_modes() {
        let html = frontend_bundle_source();
        let websocket_url = regex::Regex::new(
            r#"function websocketUrl\(\)\s*\{\s*const url = new URL\(window\.location\.href\);\s*url\.protocol = url\.protocol === "https:" \? "wss:" : "ws:";\s*url\.pathname = "/ws";\s*url\.search = "";\s*url\.hash = "";\s*return url\.toString\(\);\s*\}"#,
        )
        .expect("valid regex");

        assert!(
            websocket_url.is_match(html),
            "expected embedded bundle to derive the websocket endpoint from window.location without host-specific branches",
        );
        assert!(
            !html.contains("__TAURI__")
                && !html.contains("window.chrome.webview")
                && !html.contains("webkit.messageHandlers"),
            "expected websocket transport to avoid native-host-specific frontend branches",
        );
    }

    #[test]
    fn embedded_web_workspace_state_renders_active_workspace_through_app_state_helper() {
        let html = frontend_bundle_source();
        let workspace_state_flow = regex::Regex::new(
            r#"case\s*"workspace_state":\s*projectError\s*=\s*"";\s*(?:renderAppState|frontendUnits\.projectWorkspaceShell\.renderAppState)\(event\.workspace\);\s*break;"#,
        )
        .expect("valid regex");

        assert!(
            html.contains("function emptyWorkspace()"),
            "expected workspace rendering fallback helper in embedded html",
        );
        assert!(
            html.contains("function renderAppState(nextState)"),
            "expected app state rendering to live in a named helper",
        );
        assert!(
            html.contains("const tab = activeProjectTab();")
                && html.contains("renderProjectOnboarding(tab);")
                && html.contains("renderWorkspace(tab?.workspace || emptyWorkspace());")
                && html.contains("renderWindowList();"),
            "expected app state rendering to drive onboarding, workspace, and window list updates from the active tab",
        );
        assert!(
            html.contains("function renderWorkspace(workspace)"),
            "expected workspace painting to stay isolated behind a named helper",
        );
        assert!(
            workspace_state_flow.is_match(html),
            "expected workspace_state events to clear project errors and re-render through the app-state workspace helper",
        );
    }

    #[test]
    fn embedded_web_project_bar_includes_app_version_surface() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("id=\"app-version\""),
            "expected embedded html to expose a project bar surface for the app version",
        );
        assert!(
            html.contains("function formatVersionLabel()"),
            "expected version label formatting to live in a named helper",
        );
        assert!(
            html.contains("function renderAppVersion()"),
            "expected project bar version rendering to live in a named helper",
        );
        assert!(
            html.contains("function setVersionState(current, latest = null)"),
            "expected version state updates to be centralized behind a helper",
        );
        assert!(
            html.contains("setVersionState(appState.app_version, versionState.latest);"),
            "expected workspace state rendering to seed the current app version",
        );
        assert!(
            html.contains("setVersionState(event.current, event.latest);"),
            "expected update events to refresh both current and latest version labels",
        );
    }

    #[test]
    fn embedded_web_branches_surface_includes_scope_filter_controls() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("data-branch-filter=\"local\""),
            "expected Local branch filter control in embedded html",
        );
        assert!(
            html.contains("data-branch-filter=\"remote\""),
            "expected Remote branch filter control in embedded html",
        );
        assert!(
            html.contains("data-branch-filter=\"all\""),
            "expected All branch filter control in embedded html",
        );
    }

    #[test]
    fn embedded_web_branches_surface_includes_cleanup_flow_contract() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("branch-cleanup-modal"),
            "expected cleanup modal scaffold in embedded html",
        );
        assert!(
            html.contains("run_branch_cleanup"),
            "expected branch cleanup frontend event in embedded html",
        );
        assert!(
            html.contains("branch_cleanup_result"),
            "expected branch cleanup result handler in embedded html",
        );
        assert!(
            html.contains("target.reference"),
            "expected branch cleanup copy to render the actual merge target ref from the wire payload",
        );
        assert!(
            !html.contains("merged to main") && !html.contains("merged to develop"),
            "expected cleanup copy to stop collapsing merge targets into abstract labels",
        );
    }

    #[test]
    fn embedded_web_branches_surface_keeps_loading_while_cleanup_hydration_is_pending() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("!entry.cleanup_ready"),
            "expected embedded html to branch on cleanup hydration readiness",
        );
        assert!(
            html.contains("const phase = String(event.phase || \"hydrated\").toLowerCase();"),
            "expected branch entries handler to normalize the explicit event phase before using it",
        );
        assert!(
            html.contains("state.loading = phase !== \"hydrated\";"),
            "expected branch entries handler to derive loading state from the normalized phase",
        );
        assert!(
            html.contains("Loading branch details"),
            "expected embedded html to surface loading copy while branch hydration continues",
        );
    }

    #[test]
    fn embedded_web_branches_surface_keeps_inventory_failures_blocking_until_fresh_rows_arrive() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("state.receivedFreshEntries = false;"),
            "expected each branch load request to reset fresh-entry tracking",
        );
        assert!(
            html.contains("state.receivedFreshEntries = true;"),
            "expected branch entries handler to mark when the current request delivered fresh rows",
        );
        assert!(
            html.contains("if (state.receivedFreshEntries) {"),
            "expected branch errors to downgrade to notices only after fresh rows were delivered",
        );
    }

    #[test]
    fn embedded_web_knowledge_bridge_surface_uses_cache_backed_contract() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("knowledge-root"),
            "expected knowledge bridge root scaffold in embedded html",
        );
        assert!(
            html.contains("load_knowledge_bridge"),
            "expected knowledge bridge load event in embedded html",
        );
        assert!(
            html.contains("select_knowledge_bridge_entry"),
            "expected knowledge bridge detail selection event in embedded html",
        );
        assert!(
            html.contains("open_issue_launch_wizard"),
            "expected issue launch wizard event in embedded html",
        );
        assert!(
            html.contains("Refresh cached knowledge"),
            "expected knowledge bridge refresh affordance to describe cache-backed reloads",
        );
        assert!(
            html.contains("Loading cache-backed data"),
            "expected knowledge bridge loading copy to describe cache-backed reads",
        );
        assert!(
            html.contains("No cached items"),
            "expected knowledge bridge empty copy to stay cache-backed",
        );
        assert!(
            !html.contains("gh issue") && !html.contains("gh pr"),
            "expected knowledge bridge guidance to avoid direct gh issue/pr commands",
        );
    }

    #[test]
    fn embedded_web_board_surface_uses_cache_backed_contract() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("board-root"),
            "expected Board root scaffold in embedded html",
        );
        assert!(
            html.contains("load_board"),
            "expected Board load event in embedded html",
        );
        assert!(
            html.contains("post_board_entry"),
            "expected Board post event in embedded html",
        );
        assert!(
            html.contains("runtime_hook_event") && html.contains("coordination_event"),
            "expected Board surface to react to live coordination hook events",
        );
    }

    #[test]
    fn embedded_web_memo_surface_uses_repo_scoped_notes_contract() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("memo-root"),
            "expected Memo root scaffold in embedded html",
        );
        assert!(
            html.contains("load_memo"),
            "expected Memo load event in embedded html",
        );
        assert!(
            html.contains("create_memo_note")
                && html.contains("update_memo_note")
                && html.contains("delete_memo_note"),
            "expected Memo surface to expose create/update/delete note events",
        );
        assert!(
            html.contains("Pinned notes stay at the top of the repo-scoped list."),
            "expected Memo surface to explain the repo-scoped pin ordering contract",
        );
    }

    #[test]
    fn embedded_web_profile_surface_uses_config_backed_contract() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("profile-root"),
            "expected Profile root scaffold in embedded html",
        );
        assert!(
            html.contains("load_profile"),
            "expected Profile load event in embedded html",
        );
        assert!(
            html.contains("select_profile")
                && html.contains("create_profile")
                && html.contains("set_active_profile")
                && html.contains("save_profile")
                && html.contains("delete_profile"),
            "expected Profile surface to expose selection, CRUD, active-switch, and save events",
        );
        assert!(
            html.contains("Merged preview")
                && html
                    .contains("The backend computes this preview from the current OS environment",),
            "expected Profile surface to render the backend-owned merged preview contract",
        );
    }

    #[test]
    fn embedded_web_logs_surface_uses_cache_backed_contract() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("logs-root"),
            "expected Logs root scaffold in embedded html",
        );
        assert!(
            html.contains("load_logs"),
            "expected Logs load event in embedded html",
        );
        assert!(
            html.contains("log_entries") && html.contains("log_entry_appended"),
            "expected Logs surface to accept both snapshot and live append events",
        );
        assert!(
            html.contains("logs-unread-button"),
            "expected Logs surface to expose an unread warning/error badge control",
        );
    }

    #[test]
    fn embedded_web_launch_wizard_actions_flow_through_named_transport() {
        let html = frontend_bundle_source();
        let submit_bounds = regex::Regex::new(
            r#"function sendWizardAction\(action\)\s*\{\s*const payload = \{\s*kind:\s*"launch_wizard_action",\s*action,\s*\};\s*if\s*\(\s*action\.kind === "submit"\s*\)\s*\{\s*payload\.bounds = visibleBounds\(\);\s*\}\s*send\(payload\);\s*\}"#,
        )
        .expect("valid regex");
        let close_controls = regex::Regex::new(
            r#"wizardCloseButton\.addEventListener\("click",\s*\(\)\s*=>\s*\{\s*(?:sendWizardAction|frontendUnits\.launchWizardSurface\.sendAction)\(\{\s*kind:\s*"cancel"\s*\}\);\s*\}\);\s*wizardCancelButton\.addEventListener\("click",\s*\(\)\s*=>\s*\{\s*(?:sendWizardAction|frontendUnits\.launchWizardSurface\.sendAction)\(\{\s*kind:\s*"cancel"\s*\}\);\s*\}\);"#,
        )
        .expect("valid regex");
        let submit_button = regex::Regex::new(
            r#"wizardSubmitButton\.addEventListener\("click",\s*\(\)\s*=>\s*\{\s*(?:flushWizardBranchDraft|frontendUnits\.launchWizardSurface\.flushBranchDraft)\(\);\s*(?:sendWizardAction|frontendUnits\.launchWizardSurface\.sendAction)\(\{\s*kind:\s*"submit"\s*\}\);\s*\}\);"#,
        )
        .expect("valid regex");

        assert!(
            html.contains("function openIssueLaunchWizard(windowId, issueNumber)"),
            "expected issue-launch entrypoint helper in embedded html",
        );
        assert!(
            html.contains("kind: \"open_issue_launch_wizard\"")
                && html.contains("issue_number: issueNumber"),
            "expected issue launch wizard entrypoint to send the canonical frontend event payload",
        );
        assert!(
            submit_bounds.is_match(html),
            "expected wizard actions to be normalized through launch_wizard_action and attach visible bounds on submit",
        );
        assert!(
            close_controls.is_match(html),
            "expected both close controls to route cancel through sendWizardAction",
        );
        assert!(
            submit_button.is_match(html),
            "expected submit control to flush branch draft before dispatching submit",
        );
        let backdrop_cancel = regex::Regex::new(
            r#"if\s*\(\s*event\.target === wizardModal\s*\)\s*\{\s*(?:sendWizardAction|frontendUnits\.launchWizardSurface\.sendAction)\(\{\s*kind:\s*"cancel"\s*\}\);\s*\}"#,
        )
        .expect("valid regex");
        assert!(
            backdrop_cancel.is_match(html),
            "expected backdrop dismissal to share the same wizard cancel transport",
        );
        let wizard_state = regex::Regex::new(
            r#"case\s*"launch_wizard_state":\s*launchWizard\s*=\s*event\.wizard;\s*(?:renderLaunchWizard|frontendUnits\.launchWizardSurface\.render)\(\);\s*break;"#,
        )
        .expect("valid regex");
        assert!(
            wizard_state.is_match(html),
            "expected launch wizard state updates to hydrate the shared wizard renderer",
        );
    }

    #[test]
    fn embedded_web_shared_bundle_keeps_user_facing_copy_english_only() {
        let html = frontend_bundle_source();
        let japanese_scripts = regex::Regex::new(r"[ぁ-んァ-ン一-龯]").expect("valid regex");

        assert!(
            html.contains("Open a project")
                && html.contains("Restore previous workspaces or choose a new folder.")
                && html.contains("Open a standard shell terminal")
                && html.contains("Launch Agent")
                && html.contains("Connected")
                && html.contains("Reconnecting"),
            "expected shared frontend bundle to keep the browser and native user-facing copy on the English contract",
        );
        assert!(
            !japanese_scripts.is_match(html),
            "expected embedded bundle copy to stay English-only for both browser and native modes",
        );
        assert!(
            !html.contains("PoC"),
            "expected user-facing frontend copy to stop referring to the retired PoC surface",
        );
    }

    #[test]
    fn embedded_web_frontend_units_group_stateful_surfaces() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("const frontendUnits = Object.freeze({"),
            "expected embedded html to group frontend responsibilities behind a unit registry",
        );
        assert!(
            html.contains("socketTransport,")
                && html.contains("projectWorkspaceShell,")
                && html.contains("workspaceWindowManager,")
                && html.contains("terminalHost,")
                && html.contains("launchWizardSurface,")
                && html.contains("branchesFileTreeSurface,")
                && html.contains("memoSurface,")
                && html.contains("profileSurface,")
                && html.contains("boardSurface,")
                && html.contains("logsSurface,")
                && html.contains("knowledgeSettingsSurface,"),
            "expected frontend unit registry to expose the extracted transport, workspace, terminal, wizard, tree, Memo, Profile, Board, Logs, and knowledge/settings surfaces",
        );
        assert!(
            !html.contains("window.__POC__"),
            "expected embedded runtime to stop exporting the retired PoC inspection global",
        );
    }

    #[test]
    fn embedded_web_frontend_units_receive_and_bootstrap_through_named_surfaces() {
        let html = frontend_bundle_source();
        let workspace_event = regex::Regex::new(
            r#"case\s*"workspace_state":\s*projectError\s*=\s*"";\s*frontendUnits\.projectWorkspaceShell\.renderAppState\(event\.workspace\);\s*break;"#,
        )
        .expect("valid regex");
        let terminal_event = regex::Regex::new(
            r#"case\s*"terminal_output":\s*frontendUnits\.terminalHost\.writeOutput\(event\.id,\s*event\.data_base64\);\s*break;\s*case\s*"terminal_snapshot":\s*frontendUnits\.terminalHost\.replaceTerminalSnapshot\(event\.id,\s*event\.data_base64\);\s*break;"#,
        )
        .expect("valid regex");
        let profile_event = regex::Regex::new(
            r#"case\s*"profile_snapshot":\s*\{\s*const state = frontendUnits\.profileSurface\.ensureProfileState\(event\.id\);[\s\S]*?frontendUnits\.profileSurface\.renderProfile\(event\.id\);\s*break;\s*\}"#,
        )
        .expect("valid regex");
        let wizard_event = regex::Regex::new(
            r#"case\s*"launch_wizard_state":\s*launchWizard\s*=\s*event\.wizard;\s*frontendUnits\.launchWizardSurface\.render\(\);\s*break;"#,
        )
        .expect("valid regex");

        assert!(
            html.contains("frontendUnits.socketTransport.connect();"),
            "expected frontend bootstrap to connect through the socket transport unit",
        );
        assert!(
            workspace_event.is_match(html),
            "expected workspace_state events to flow through the project workspace shell unit",
        );
        assert!(
            terminal_event.is_match(html),
            "expected terminal output and snapshot events to flow through the terminal host unit",
        );
        assert!(
            profile_event.is_match(html),
            "expected profile snapshot events to flow through the dedicated profile surface unit",
        );
        assert!(
            wizard_event.is_match(html),
            "expected launch wizard state events to render through the wizard surface unit",
        );
    }

    #[test]
    fn embedded_web_inline_module_script_stays_under_phase_1b_budget() {
        let html = index_html();
        let lines: Vec<_> = html.lines().collect();
        let start = lines
            .iter()
            .position(|line| line.contains("<script type=\"module\""))
            .expect("module script tag");
        let end = lines
            .iter()
            .enumerate()
            .skip(start)
            .find_map(|(index, line)| line.contains("</script>").then_some(index))
            .expect("module script end tag");
        let inline_script_lines = end.saturating_sub(start + 1);

        assert!(
            inline_script_lines < 2_000,
            "expected Phase 1B inline module script budget under 2000 lines, got {inline_script_lines}",
        );
    }
}
