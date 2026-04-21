use axum::response::Html;

pub(crate) fn index_html() -> &'static str {
    include_str!("../web/index.html")
}

pub(crate) async fn index_handler() -> Html<&'static str> {
    Html(index_html())
}

#[cfg(test)]
mod tests {
    use super::index_html;

    #[test]
    fn embedded_web_terminal_copy_shortcut_uses_ctrl_shift_c() {
        let html = index_html();

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
        let html = index_html();

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
        let html = index_html();

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
        let html = index_html();
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
        let html = index_html();
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
        let html = index_html();

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
    fn embedded_web_socket_protocol_wiring_uses_named_handlers() {
        let html = index_html();

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
        let html = index_html();
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
    fn embedded_web_workspace_state_renders_active_workspace_through_app_state_helper() {
        let html = index_html();
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
        let html = index_html();

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
        let html = index_html();

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
        let html = index_html();

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
    }

    #[test]
    fn embedded_web_branches_surface_keeps_loading_while_cleanup_hydration_is_pending() {
        let html = index_html();

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
        let html = index_html();

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
        let html = index_html();

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
    }

    #[test]
    fn embedded_web_launch_wizard_actions_flow_through_named_transport() {
        let html = index_html();
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
    fn embedded_web_frontend_units_group_stateful_surfaces() {
        let html = index_html();

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
                && html.contains("knowledgeSettingsSurface,"),
            "expected frontend unit registry to expose the extracted transport, workspace, terminal, wizard, tree, and knowledge/settings surfaces",
        );
        assert!(
            html.contains("window.__POC__ = { receive, frontendStateOwners, frontendUnits };"),
            "expected embedded runtime to expose the frontend unit registry for inspection",
        );
    }

    #[test]
    fn embedded_web_frontend_units_receive_and_bootstrap_through_named_surfaces() {
        let html = index_html();
        let workspace_event = regex::Regex::new(
            r#"case\s*"workspace_state":\s*projectError\s*=\s*"";\s*frontendUnits\.projectWorkspaceShell\.renderAppState\(event\.workspace\);\s*break;"#,
        )
        .expect("valid regex");
        let terminal_event = regex::Regex::new(
            r#"case\s*"terminal_output":\s*frontendUnits\.terminalHost\.writeOutput\(event\.id,\s*event\.data_base64\);\s*break;\s*case\s*"terminal_snapshot":\s*frontendUnits\.terminalHost\.replaceTerminalSnapshot\(event\.id,\s*event\.data_base64\);\s*break;"#,
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
            wizard_event.is_match(html),
            "expected launch wizard state events to render through the wizard surface unit",
        );
    }
}
