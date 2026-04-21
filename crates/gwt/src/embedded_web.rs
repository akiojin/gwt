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
    fn embedded_web_profile_surface_uses_config_backed_contract() {
        let html = index_html();

        assert!(
            html.contains("profile-root"),
            "expected Profile window to render a dedicated non-mock root",
        );
        assert!(
            html.contains("list_profiles"),
            "expected Profile window to request a backend profile snapshot",
        );
        assert!(
            html.contains("profile_snapshot"),
            "expected Profile window to handle backend profile snapshots",
        );
        assert!(
            html.contains("profile-add"),
            "expected Profile window to expose inline profile add controls",
        );
        assert!(
            html.contains("profile-merged-env"),
            "expected Profile window to expose an effective environment preview",
        );
    }
}
