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

pub(crate) fn xterm_js() -> &'static str {
    include_str!("../web/vendor/xterm/xterm.mjs")
}

pub(crate) fn xterm_fit_js() -> &'static str {
    include_str!("../web/vendor/xterm/addon-fit.mjs")
}

pub(crate) fn xterm_css() -> &'static str {
    include_str!("../web/vendor/xterm/xterm.css")
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

pub(crate) async fn xterm_js_handler() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        xterm_js(),
    )
}

pub(crate) async fn xterm_fit_js_handler() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
        xterm_fit_js(),
    )
}

pub(crate) async fn xterm_css_handler() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        xterm_css(),
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
        let refresh_call = regex::Regex::new(
            r"runtime\.terminal\.refresh\(0,\s*runtime\.terminal\.rows\s*-\s*1\);",
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
                && html.contains("!workspaceWindowById(windowId)?.minimized")
                && refresh_call.is_match(html),
            "expected terminal viewport refresh to skip minimized windows",
        );
        assert!(
            !html.contains("fitTerminal(windowId, false);"),
            "expected terminal output refresh to avoid geometry refits on every PTY chunk",
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
    fn embedded_web_terminal_scrolls_refresh_viewport_after_xterm_scroll() {
        let html = frontend_bundle_source();
        let scroll_listener = regex::Regex::new(
            r"const\s+viewportScrollDisposable\s*=\s*terminal\.onScroll\(\(\)\s*=>\s*\{\s*scheduleTerminalViewportRefresh\(windowId\);\s*\}\s*\);",
        )
        .expect("valid regex");

        assert!(
            html.contains("function installTerminalViewportRefreshHandlers(windowId, terminal)"),
            "expected terminal viewport refresh event wiring to live in a named helper",
        );
        assert!(
            scroll_listener.is_match(html),
            "expected terminal scrollback movement to refresh the visible viewport",
        );
        assert!(
            html.contains("const viewportRefreshCleanup = installTerminalViewportRefreshHandlers(windowId, terminal);")
                && html.contains("viewportRefreshCleanup();"),
            "expected terminal runtime cleanup to dispose viewport refresh listeners",
        );
        assert!(
            html.contains("viewportScrollDisposable.dispose();"),
            "expected xterm scroll listener disposable to be released during cleanup",
        );
    }

    #[test]
    fn embedded_web_terminal_assets_are_local_and_pinned() {
        let html = frontend_bundle_source();

        assert!(
            index_html().contains(r#"href="/assets/xterm/xterm.css""#),
            "expected xterm stylesheet to be served from the embedded local asset route",
        );
        assert!(
            app_js().contains(r#"from "/assets/xterm/xterm.mjs""#)
                && app_js().contains(r#"from "/assets/xterm/addon-fit.mjs""#),
            "expected xterm modules to be served from embedded local asset routes",
        );
        assert!(
            !html.contains("cdn.jsdelivr.net")
                && !html.contains("unpkg.com")
                && !html.contains("cdnjs.cloudflare.com"),
            "expected embedded terminal assets to avoid CDN/runtime network dependencies",
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
    fn embedded_web_project_bar_renders_index_status_badge() {
        let html = index_html();
        let js = app_js();

        assert!(
            html.contains("id=\"index-status\""),
            "expected project bar to expose project index status badge",
        );
        assert!(
            html.contains(".index-status.ready") && html.contains(".index-status.error"),
            "expected embedded css to define index health states",
        );
        assert!(
            js.contains("function setIndexStatus(status)")
                && js.contains("case \"project_index_status\""),
            "expected frontend to consume project_index_status events",
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
            html.contains("data-knowledge-scope=\"open\"")
                && html.contains("data-knowledge-scope=\"closed\""),
            "expected issue knowledge bridge surface to expose open and closed cache tabs",
        );
        assert!(
            html.contains("list_scope"),
            "expected knowledge bridge requests to carry the active issue list scope",
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
    fn embedded_web_knowledge_bridge_surface_uses_semantic_search_contract() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("search_knowledge_bridge"),
            "expected knowledge bridge search input to call the semantic search backend",
        );
        assert!(
            html.contains("knowledge_search_results"),
            "expected frontend to handle semantic search result events",
        );
        assert!(
            html.contains("request_id"),
            "expected semantic search requests to carry request ids for stale-response guards",
        );
        assert!(
            html.contains("Searching semantic index"),
            "expected semantic search to expose an in-progress state",
        );
        assert!(
            html.contains("% match"),
            "expected semantic search result rows to show percentage similarity",
        );
        assert!(
            !html.contains("No matching cached items"),
            "expected semantic search to stop presenting substring-filter empty copy",
        );
    }

    #[test]
    fn embedded_web_knowledge_bridge_cancels_pending_semantic_search_on_window_teardown() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("function clearKnowledgeBridgeState(windowId)"),
            "expected knowledge bridge teardown to clear pending timers before deleting state",
        );
        assert!(
            html.contains("clearKnowledgeBridgeState(windowId);"),
            "expected workspace window removal to use knowledge bridge cleanup",
        );
        assert!(
            html.contains("if (!workspaceWindowById(windowId))"),
            "expected debounced semantic search to verify the window still exists before sending",
        );
    }

    #[test]
    fn embedded_web_knowledge_bridge_waits_for_initial_cache_load_before_semantic_search() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("state.loading && state.baseEntries.length === 0"),
            "expected semantic search scheduling to wait for initial cache load before sending",
        );
        assert!(
            html.contains("const queuedQuery = state.query.trim();")
                && html.contains("if (queuedQuery)")
                && html.contains("frontendUnits.knowledgeSettingsSurface.scheduleKnowledgeSearch("),
            "expected knowledge entries response to resume queued semantic search after cache load",
        );
    }

    #[test]
    fn embedded_web_knowledge_bridge_coalesces_inflight_search_and_preserves_results() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("searchInFlight") && html.contains("inFlightSearchRequestId"),
            "expected semantic search state to track the single in-flight backend request",
        );
        assert!(
            html.contains("queuedSearchQuery")
                && html.contains("const nextQuery = state.queuedSearchQuery;"),
            "expected semantic search state to coalesce additional input to the latest query",
        );
        assert!(
            !html.contains("state.entries = [];\n        state.emptyMessage = \"\";\n        state.pendingSearchTimer"),
            "expected semantic search scheduling to preserve visible entries while searching",
        );
    }

    #[test]
    fn embedded_web_knowledge_bridge_correlates_detail_selection_without_resetting_refresh() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("detailRequestId") && html.contains("knowledgeDetailRequestMatches"),
            "expected entry selection to use a separate detail request correlation id",
        );
        assert!(
            html.contains("request_id: requestId,\n          number,"),
            "expected select_knowledge_bridge_entry requests to carry the detail request id",
        );
        assert!(
            html.contains("state.loadRequestId = requestId;\n        state.detailRequestId = 0;"),
            "expected new cache loads to invalidate older detail response ids",
        );
        assert!(
            html.contains("const matchesLoadRequest =") && html.contains("if (matchesLoadRequest)"),
            "expected detail responses to avoid clearing refresh loading for detail-only replies",
        );
        let search_block = html
            .split("case \"knowledge_search_results\":")
            .nth(1)
            .and_then(|tail| tail.split("case \"knowledge_detail\":").next())
            .expect("knowledge search results block");
        assert!(
            !search_block.contains("state.loading = false;")
                && !search_block.contains("state.refreshing = false;"),
            "expected search results to leave active refresh loading state untouched",
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
    fn embedded_web_board_composer_textarea_keeps_scroll_surface_marker() {
        // SPEC-2008 FR-032 retired the per-class wheel whitelist; the
        // `.board-scroll-surface` marker is now informational only but is
        // still applied to the composer textarea so any future Board-specific
        // styling can hang off it without reintroducing the whitelist.
        let js = app_js();

        assert!(
            js.contains("board-textarea board-scroll-surface"),
            "expected Board composer textarea to retain its scroll surface marker class",
        );
    }

    #[test]
    fn embedded_web_board_surface_uses_chat_first_layout() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("board-chat-shell")
                && html.contains("board-timeline-scroll")
                && html.contains("board-composer-bar"),
            "expected Board scaffold to be a chat timeline with a bottom-fixed composer",
        );
        assert!(
            html.contains("board-message user")
                && html.contains("board-message agent")
                && html.contains("board-message system"),
            "expected Board entries to render through user/agent/system chat message classes",
        );
        assert!(
            !html.contains("board-side-pane"),
            "expected Board v1 GUI to avoid the old dashboard sidebar",
        );
    }

    #[test]
    fn embedded_web_board_messages_put_user_on_right_and_agent_on_left() {
        let html = frontend_bundle_source();
        fn css_block<'a>(html: &'a str, selector: &str) -> &'a str {
            let start = html.find(selector).expect("expected CSS block");
            let rest = &html[start..];
            let end = rest.find('}').expect("expected CSS block end");
            &rest[..=end]
        }

        let user_block = css_block(html, ".board-message.user {");
        assert!(
            user_block.contains("align-self: flex-end")
                && user_block.contains("border-bottom-right-radius"),
            "expected user messages to render on the right, got: {user_block}",
        );

        let agent_block = css_block(html, ".board-message.agent {");
        assert!(
            agent_block.contains("align-self: flex-start")
                && agent_block.contains("border-bottom-left-radius"),
            "expected agent messages to render on the left, got: {agent_block}",
        );
    }

    #[test]
    fn embedded_web_board_composer_is_body_first_and_resets_after_post() {
        let html = frontend_bundle_source();
        let anchor = html
            .find("Share a Board update")
            .expect("expected Board composer anchor copy");
        let composer_start = anchor.saturating_sub(500);
        let composer_end = html.len().min(anchor + 1_500);
        let composer_snippet = &html[composer_start..composer_end];

        assert!(
            composer_snippet.contains("Share a Board update"),
            "expected Board composer to expose body-first posting copy",
        );
        assert!(
            html.contains("pendingSubmit: null")
                && html.contains("existingEntryIds: new Set")
                && html.contains("const completedSubmit = Boolean(state.pendingSubmit")
                && html.contains("!state.pendingSubmit.existingEntryIds.has(entry.id)")
                && html.contains("state.composerBody = \"\";")
                && html.contains("state.pendingSubmit = null;"),
            "expected Board post success to clear drafts only after matching submitted entry appears",
        );
        assert!(
            !composer_snippet.contains("Post update")
                && !composer_snippet.contains("Topics</span>")
                && !composer_snippet.contains("Owners</span>"),
            "expected Board composer to keep kind/topics/owners out of the primary posting path",
        );
    }

    #[test]
    fn embedded_web_board_composer_shift_enter_submits_without_ime_conflict() {
        let html = frontend_bundle_source();
        let keydown_start = html
            .find("bodyInput.addEventListener(\"keydown\"")
            .expect("expected Board composer textarea keydown handler");
        let keydown_block = &html[keydown_start..html.len().min(keydown_start + 700)];

        assert!(
            keydown_block.contains("event.key === \"Enter\"")
                && keydown_block.contains("event.shiftKey")
                && keydown_block.contains("event.isComposing")
                && keydown_block.contains("event.preventDefault()")
                && keydown_block.contains("submitBoardEntry(windowId)"),
            "expected Shift+Enter to submit while guarding IME composition, got: {keydown_block}",
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

    /// SPEC-2008 FR-032: wheel routing must follow the opt-out model.
    ///
    /// Only `.surface-terminal` may consume wheel events for canvas pan or
    /// xterm scrollback. All other surfaces (panels and modals) must let the
    /// browser handle wheel natively. The legacy whitelist helper
    /// `findNativeWheelScrollSurface` must be retired so newly added panel
    /// surfaces never need to remember to register themselves.
    #[test]
    fn embedded_web_wheel_routing_uses_terminal_only_opt_out() {
        let js = app_js();

        assert!(
            !js.contains("function findNativeWheelScrollSurface"),
            "expected the per-class wheel scroll whitelist helper to be removed in favor of an opt-out model",
        );
        assert!(
            !js.contains("findNativeWheelScrollSurface("),
            "expected no remaining call sites for the retired wheel scroll whitelist helper",
        );
        assert!(
            js.contains("function handleCanvasWheelEvent"),
            "expected canvas wheel handler to remain as the single routing entrypoint",
        );
        assert!(
            js.contains("targetElement.closest(\".surface-terminal\")"),
            "expected canvas wheel handler to special-case `.surface-terminal` for the opt-out routing model",
        );
        assert!(
            js.contains("targetElement.closest(\".workspace-window\")"),
            "expected canvas wheel handler to recognize panel windows so native scroll is preserved inside them",
        );
    }

    /// SPEC-2008 FR-033: every panel surface must share the same window
    /// background, titlebar background, and body background. Three CSS rules
    /// participate in this unification:
    ///
    /// - `.workspace-window.surface-*` — the whole-window background (visible
    ///   when minimized or behind the body)
    /// - `.surface-* .titlebar` — the chrome at the top of the window
    /// - `.surface-* .window-body` — the panel content surface
    ///
    /// `.surface-memo`, `.surface-profile`, and `.surface-knowledge` had been
    /// missing from one or more of these rules, which left those panels
    /// partially transparent and visually distinct from the rest.
    #[test]
    fn embedded_web_panel_surfaces_share_opaque_window_chrome_and_body() {
        let html = index_html();

        let panel_surfaces = [
            ".surface-file-tree",
            ".surface-branches",
            ".surface-board",
            ".surface-logs",
            ".surface-knowledge",
            ".surface-mock",
            ".surface-memo",
            ".surface-profile",
        ];

        for (anchor, role) in [
            (
                ".workspace-window.surface-file-tree,",
                "workspace-window background",
            ),
            (".surface-file-tree .titlebar,", "titlebar background"),
            (".surface-file-tree .window-body,", "window-body background"),
        ] {
            let rule_start = html
                .find(anchor)
                .unwrap_or_else(|| panic!("expected unified {role} rule to anchor on `{anchor}`"));
            let rule_block = &html[rule_start..];
            let rule_end = rule_block
                .find('}')
                .unwrap_or_else(|| panic!("expected unified {role} rule to close with `}}`"));
            let rule = &rule_block[..rule_end];

            for surface in panel_surfaces {
                let needle = match role {
                    "workspace-window background" => format!(".workspace-window{surface}"),
                    _ => surface.to_string(),
                };
                assert!(
                    rule.contains(&needle),
                    "expected `{needle}` to participate in the unified {role} rule",
                );
            }
        }
    }

    /// SPEC-2008 FR-034: shared layout primitives must exist in CSS so panel
    /// surfaces stop reinventing their own toolbar/scroll/split/empty-state
    /// blocks. The four primitives below carry only the shared properties;
    /// surface-specific deltas (grid template columns, padding) layer on top
    /// through the surface's own class.
    #[test]
    fn embedded_web_workspace_layout_primitives_define_shared_contracts() {
        let html = index_html();

        let primitives: [(&str, &[&str]); 4] = [
            (
                ".workspace-toolbar {",
                &[
                    "display: flex",
                    "align-items: center",
                    "justify-content: space-between",
                    "gap: 12px",
                    "padding: 10px 12px",
                    "border-bottom: 1px solid",
                ],
            ),
            (
                ".workspace-scroll {",
                &["flex: 1", "min-height: 0", "overflow: auto"],
            ),
            (
                ".workspace-split {",
                &["flex: 1", "min-height: 0", "display: grid"],
            ),
            (
                ".workspace-empty-state {",
                &["padding: 16px 12px", "font-size: 12px", "color: #64748b"],
            ),
        ];

        for (selector, expected_props) in primitives {
            let start = html
                .find(selector)
                .unwrap_or_else(|| panic!("expected layout primitive `{selector}` to be defined"));
            let block = &html[start..];
            let end = block.find('}').unwrap_or_else(|| {
                panic!("expected layout primitive `{selector}` to close with `}}`")
            });
            let body = &block[..end];
            for prop in expected_props {
                assert!(
                    body.contains(prop),
                    "expected layout primitive `{selector}` to declare `{prop}`, got: {body}",
                );
            }
        }
    }

    /// SPEC-2008 FR-034: every panel surface must adopt the shared layout
    /// primitives in its rendered HTML so paddings, scrollbars, and splits
    /// stay in lockstep. The toolbar misnomer `.knowledge-toolbar` (which was
    /// reused by Memo/Profile/Logs/Board as the generic toolbar block) is
    /// retired in favour of `.workspace-toolbar`. Stacked toolbars (multi-row
    /// content with search and filter chips) opt into the
    /// `.workspace-toolbar.is-stacked` modifier rather than carrying a
    /// surface-specific override.
    #[test]
    fn embedded_web_panel_surfaces_compose_with_layout_primitives() {
        let html = index_html();
        let js = app_js();

        assert!(
            !js.contains("knowledge-toolbar"),
            "expected `.knowledge-toolbar` misnomer to be replaced by `.workspace-toolbar` everywhere it was used as a generic toolbar",
        );
        assert!(
            !html.contains(".knowledge-toolbar"),
            "expected `.knowledge-toolbar` CSS rules to be migrated to `.workspace-toolbar`",
        );

        // Each panel surface must reference the `.workspace-toolbar` primitive
        // through its mountWindowBody output. Surface-specific deltas may be
        // layered alongside (e.g. `.branch-toolbar`).
        let toolbar_count = js.matches("class=\"workspace-toolbar").count();
        assert!(
            toolbar_count >= 7,
            "expected at least 7 panel surfaces to mount with the `.workspace-toolbar` primitive, found {toolbar_count}",
        );

        // Stacked modifier replaces the old `.knowledge-toolbar` override.
        assert!(
            html.contains(".workspace-toolbar.is-stacked"),
            "expected the stacked toolbar modifier `.workspace-toolbar.is-stacked` to be defined for multi-row toolbars",
        );

        let split_adopters = [
            "knowledge-split workspace-split",
            "memo-layout workspace-split",
            "profile-layout workspace-split",
            "logs-layout workspace-split",
        ];
        for needle in split_adopters {
            assert!(
                js.contains(needle),
                "expected mountWindowBody output to compose `{needle}` so split layouts share the primitive",
            );
        }

        let scroll_adopters = [
            "knowledge-detail-scroll workspace-scroll",
            "board-timeline-scroll board-scroll-surface workspace-scroll",
            "file-tree-scroll workspace-scroll",
            "branch-scroll workspace-scroll",
        ];
        for needle in scroll_adopters {
            assert!(
                js.contains(needle),
                "expected mountWindowBody output to compose `{needle}` so scroll regions share the primitive",
            );
        }
    }

    /// SPEC-2008 FR-035: shared modal frame primitives must exist so Launch
    /// Wizard, Branch Cleanup, Preset modal, and any future overlay UI render
    /// through a single chrome contract. The primitives are:
    ///
    /// - `.modal-backdrop` — full-window dim layer (single rule, no
    ///   `.wizard-backdrop` parallel implementation)
    /// - `.modal-shell` — the centered modal card surface
    /// - `.modal-header` — title + actions row at the top of the shell
    /// - `.modal-body` — main scrollable content region
    /// - `.modal-footer` — bottom action bar
    #[test]
    fn embedded_web_modal_frame_primitives_define_shared_contracts() {
        let html = index_html();

        let primitives: [(&str, &[&str]); 5] = [
            (
                ".modal-backdrop {",
                &[
                    "position: absolute",
                    "inset: 0",
                    "align-items: center",
                    "justify-content: center",
                    "background: rgba(15, 23, 42",
                ],
            ),
            (
                ".modal-shell {",
                &[
                    "max-height: calc(100vh - 48px)",
                    "overflow: auto",
                    "border-radius: 6px",
                    "background: #ffffff",
                    "border: 1px solid",
                ],
            ),
            (
                ".modal-header {",
                &["display: flex", "align-items: flex-start"],
            ),
            (".modal-body {", &["flex: 1", "min-height: 0"]),
            (
                ".modal-footer {",
                &["display: flex", "justify-content: flex-end"],
            ),
        ];

        for (selector, expected_props) in primitives {
            let start = html.find(selector).unwrap_or_else(|| {
                panic!("expected modal frame primitive `{selector}` to be defined")
            });
            let block = &html[start..];
            let end = block.find('}').unwrap_or_else(|| {
                panic!("expected modal frame primitive `{selector}` to close with `}}`")
            });
            let body = &block[..end];
            for prop in expected_props {
                assert!(
                    body.contains(prop),
                    "expected modal frame primitive `{selector}` to declare `{prop}`, got: {body}",
                );
            }
        }

        assert!(
            !html.contains(".wizard-backdrop"),
            "expected `.wizard-backdrop` to be unified with `.modal-backdrop`",
        );
    }

    /// SPEC-2008 FR-036: Rust `WindowSurface` enum and JS `presetSurface()`
    /// must agree on the panel/terminal taxonomy. The Rust side exposes
    /// `WindowSurface::as_str()` returning the JS-compatible kebab-case
    /// string, and the JS side returns the same set of strings. Whenever a
    /// new panel surface is added, this test forces the backend and frontend
    /// to be updated together.
    #[test]
    fn embedded_web_window_surface_enum_aligns_with_js_preset_surface() {
        use gwt::WindowSurface;

        let js = app_js();

        let pairs: &[(WindowSurface, &str)] = &[
            (WindowSurface::Terminal, "terminal"),
            (WindowSurface::FileTree, "file-tree"),
            (WindowSurface::Branches, "branches"),
            (WindowSurface::Memo, "memo"),
            (WindowSurface::Profile, "profile"),
            (WindowSurface::Board, "board"),
            (WindowSurface::Logs, "logs"),
            (WindowSurface::Knowledge, "knowledge"),
            (WindowSurface::Mock, "mock"),
        ];

        for (variant, expected) in pairs {
            assert_eq!(
                variant.as_str(),
                *expected,
                "expected `{variant:?}.as_str()` to return `{expected}` so the JS contract stays aligned",
            );
            let return_pattern = format!("return \"{expected}\";");
            assert!(
                js.contains(&return_pattern),
                "expected JS `presetSurface()` to return `\"{expected}\"` for the corresponding preset cluster",
            );
        }
    }

    /// SPEC-2008 FR-035: every existing modal must mount through the shared
    /// `.modal-shell` primitive (with optional size modifier such as
    /// `.modal-shell.is-wizard`). The `.modal` and `.wizard-modal` legacy
    /// classes are retired.
    #[test]
    fn embedded_web_existing_modals_compose_with_modal_shell_primitive() {
        let html = index_html();

        assert!(
            !html.contains("class=\"wizard-modal\"") && !html.contains("class=\"wizard-backdrop\""),
            "expected Launch Wizard markup to migrate to `.modal-shell` / `.modal-backdrop`",
        );
        assert!(
            !html.contains("<div class=\"modal\">"),
            "expected legacy `.modal` class to be retired in favor of `.modal-shell`",
        );

        // Each modal entrypoint must declare `.modal-shell` on its card root.
        let shell_count = html.matches("class=\"modal-shell").count();
        assert!(
            shell_count >= 3,
            "expected at least 3 modals (preset / branch-cleanup / launch-wizard) to mount through `.modal-shell`, found {shell_count}",
        );
    }
}
