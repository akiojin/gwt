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
}
