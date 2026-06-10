use axum::{
    http::header,
    response::{Html, IntoResponse},
};

const JS_CONTENT_TYPE: &str = "text/javascript; charset=utf-8";
const MUTABLE_CACHE_CONTROL: &str = "no-store, max-age=0";

#[derive(Clone, Copy)]
pub struct RootJsModuleAsset {
    pub path: &'static str,
    pub source: fn() -> &'static str,
    pub marker: &'static str,
}

pub fn index_html() -> &'static str {
    include_str!("../web/index.html")
}

pub fn app_js() -> &'static str {
    include_str!("../web/app.js")
}

pub fn branch_cleanup_modal_js() -> &'static str {
    include_str!("../web/branch-cleanup-modal.js")
}

// SPEC-2013 FR-012: close project tab confirm modal renderer.
pub fn close_project_tab_confirm_modal_js() -> &'static str {
    include_str!("../web/close-project-tab-confirm-modal.js")
}

pub fn migration_modal_js() -> &'static str {
    include_str!("../web/migration-modal.js")
}

pub fn project_clone_modal_js() -> &'static str {
    include_str!("../web/project-clone-modal.js")
}

pub fn window_docking_js() -> &'static str {
    include_str!("../web/window-docking.js")
}

pub fn board_surface_js() -> &'static str {
    include_str!("../web/board-surface.js")
}

pub fn workspace_kanban_surface_js() -> &'static str {
    include_str!("../web/workspace-kanban-surface.js")
}

pub fn workspace_resume_picker_modal_js() -> &'static str {
    include_str!("../web/workspace-resume-picker-modal.js")
}

pub fn update_cta_js() -> &'static str {
    include_str!("../web/update-cta.js")
}

pub fn terminal_context_menu_js() -> &'static str {
    include_str!("../web/terminal-context-menu.js")
}

pub fn terminal_copy_shortcut_js() -> &'static str {
    include_str!("../web/terminal-copy-shortcut.js")
}

pub fn terminal_wheel_scroll_js() -> &'static str {
    include_str!("../web/terminal-wheel-scroll.js")
}

pub fn canvas_wheel_gesture_js() -> &'static str {
    include_str!("../web/canvas-wheel-gesture.js")
}

pub fn xterm_js() -> &'static str {
    include_str!("../web/vendor/xterm/xterm.mjs")
}

pub fn xterm_fit_js() -> &'static str {
    include_str!("../web/vendor/xterm/addon-fit.mjs")
}

pub fn xterm_css() -> &'static str {
    include_str!("../web/vendor/xterm/xterm.css")
}

// SPEC-2009 Phase 2b — syntax highlighting for the File Tree text viewer.
// highlight.js ES module + a dark GitHub-style theme. Bundled into the gwt
// binary so the viewer works offline without CDN reach.
pub fn highlight_js() -> &'static str {
    include_str!("../web/vendor/highlight/highlight.min.js")
}

pub fn highlight_css() -> &'static str {
    include_str!("../web/vendor/highlight/github-dark.min.css")
}

// SPEC-2356 Operator Design System — module assets.
pub fn theme_manager_js() -> &'static str {
    include_str!("../web/theme-manager.js")
}

pub fn theme_toggle_js() -> &'static str {
    include_str!("../web/theme-toggle.js")
}

pub fn hotkey_js() -> &'static str {
    include_str!("../web/hotkey.js")
}

pub fn operator_shell_js() -> &'static str {
    include_str!("../web/operator-shell.js")
}

pub fn focus_trap_js() -> &'static str {
    include_str!("../web/focus-trap.js")
}

// Issue #2698 — stable project tab renderer. Keeps tab DOM keyed by project
// tab id so status-only workspace refreshes do not rebuild the whole tab strip.
pub fn project_tabs_renderer_js() -> &'static str {
    include_str!("../web/project-tabs-renderer.js")
}

// SPEC-1939 Phase 12 / T-IDX-106 — Settings.Index tab renderer.
pub fn index_settings_panel_js() -> &'static str {
    include_str!("../web/index-settings-panel.js")
}

// SPEC-2008 Phase 24 — terminal viewport reflow primitives.
pub fn terminal_viewport_reflow_js() -> &'static str {
    include_str!("../web/terminal-viewport-reflow.js")
}

// SPEC-2008 Phase 25 — revision-aware window geometry sync primitives.
pub fn window_geometry_sync_js() -> &'static str {
    include_str!("../web/window-geometry-sync.js")
}

// Issue #2694 Phase C — kind-coalesced, rAF-flushed WebSocket inbound
// dispatcher.
pub fn socket_receive_dispatcher_js() -> &'static str {
    include_str!("../web/socket-receive-dispatcher.js")
}

// SPEC-1939 Phase 24 — per-window terminal output batching before xterm write.
pub fn terminal_output_buffer_js() -> &'static str {
    include_str!("../web/terminal-output-buffer.js")
}

// Issue #2698 PR 1 (B7) — interaction-guard primitive that defers
// destructive wizard re-renders while the user has a native <select>
// dropdown open.
pub fn interaction_guard_js() -> &'static str {
    include_str!("../web/interaction-guard.js")
}

// Issue #2704 — terminal-focus guard that lets `scheduleTerminalFocusActivation`
// skip its xterm `terminal.focus()` step while a modal is open or a text
// input owns focus. Without this, the Clone Project modal URL/Search input
// loses focus on every backend `workspace_state` event.
pub fn clone_modal_focus_guard_js() -> &'static str {
    include_str!("../web/clone-modal-focus-guard.js")
}

// Issue #2698 PR 2 (B1) — viewport-persist throttle that caps the
// `update_viewport` WebSocket rate during sustained wheel/zoom
// gestures so the backend feedback loop stops driving a frontend
// re-render storm.
pub fn viewport_persist_throttle_js() -> &'static str {
    include_str!("../web/viewport-persist-throttle.js")
}

// Issue #2698 — viewport sync guard. Protects in-flight local pan/zoom from
// stale workspace_state echoes while still allowing project-tab scope changes.
pub fn viewport_sync_js() -> &'static str {
    include_str!("../web/viewport-sync.js")
}

// Issue #2698 follow-up — browser-side metadata trace profiler for diagnosing
// pointer delivery, rAF delay, and render hotspots without terminal contents.
pub fn ui_trace_profiler_js() -> &'static str {
    include_str!("../web/ui-trace-profiler.js")
}

pub fn ui_trace_wiring_js() -> &'static str {
    include_str!("../web/ui-trace-wiring.js")
}

// SPEC-1921 T231 — Settings.Custom Agents env editor.
pub fn custom_agent_env_editor_js() -> &'static str {
    include_str!("../web/custom-agent-env-editor.js")
}

// SPEC-2780 — release notes window with version selector. app.js imports this
// via `import { createReleaseNotesWindow } from "/release-notes-window.js"` at
// module top level. Missing this route causes the ES module load to fail and
// the splash to hang because none of the boot wiring (mission briefing,
// WebSocket connect, operator shell) runs.
pub fn release_notes_window_js() -> &'static str {
    include_str!("../web/release-notes-window.js")
}

// SPEC-2809 — Console window for external process stdout/stderr live tail.
// 5 fixed kind tabs (gh / git / docker / agent / runner). app.js imports
// `createConsoleWindow` at module top level so this asset MUST be registered
// here; otherwise the ES module load fails and splash hangs (PR #2797 memory).
pub fn console_window_js() -> &'static str {
    include_str!("../web/console-window.js")
}

// SPEC-2014 2026-05-29 amendment — Launch Agent setting controls (reasoning
// slider + Auto toggle, count-adaptive segmented/select, boolean toggle).
// app.js imports these builders at module top level, so the asset MUST be
// registered here or the ES module load fails and the splash hangs.
pub fn launch_controls_js() -> &'static str {
    include_str!("../web/launch-controls.js")
}

// SPEC-2009 Phase 7 (FR-064..FR-067) — Branches detail-check reconnect
// self-heal / last-known retention / stale-load guard. app.js imports this at
// module top level, so the asset MUST be registered or the ES module load
// fails and the splash hangs.
pub fn branch_list_state_js() -> &'static str {
    include_str!("../web/branch-list-state.js")
}

pub const ROOT_JS_MODULE_ASSETS: &[RootJsModuleAsset] = &[
    RootJsModuleAsset {
        path: "/branch-cleanup-modal.js",
        source: branch_cleanup_modal_js,
        marker: "renderBranchCleanupModal",
    },
    RootJsModuleAsset {
        path: "/branch-list-state.js",
        source: branch_list_state_js,
        marker: "applyBranchEntriesEvent",
    },
    RootJsModuleAsset {
        path: "/close-project-tab-confirm-modal.js",
        source: close_project_tab_confirm_modal_js,
        marker: "renderCloseProjectTabConfirmModal",
    },
    RootJsModuleAsset {
        path: "/migration-modal.js",
        source: migration_modal_js,
        marker: "renderMigrationModal",
    },
    RootJsModuleAsset {
        path: "/project-clone-modal.js",
        source: project_clone_modal_js,
        marker: "renderProjectCloneModal",
    },
    RootJsModuleAsset {
        path: "/window-docking.js",
        source: window_docking_js,
        marker: "findTitlebarDockTarget",
    },
    RootJsModuleAsset {
        path: "/board-surface.js",
        source: board_surface_js,
        marker: "boardEntryMentionsSelf",
    },
    RootJsModuleAsset {
        path: "/workspace-kanban-surface.js",
        source: workspace_kanban_surface_js,
        marker: "createWorkspaceKanbanSurface",
    },
    RootJsModuleAsset {
        path: "/workspace-resume-picker-modal.js",
        source: workspace_resume_picker_modal_js,
        marker: "createWorkspaceResumePickerController",
    },
    RootJsModuleAsset {
        path: "/update-cta.js",
        source: update_cta_js,
        marker: "createUpdateCtaController",
    },
    RootJsModuleAsset {
        path: "/terminal-context-menu.js",
        source: terminal_context_menu_js,
        marker: "createTerminalContextMenuController",
    },
    RootJsModuleAsset {
        path: "/terminal-copy-shortcut.js",
        source: terminal_copy_shortcut_js,
        marker: "classifyTerminalCopyKeyEvent",
    },
    RootJsModuleAsset {
        path: "/terminal-wheel-scroll.js",
        source: terminal_wheel_scroll_js,
        marker: "createTerminalWheelScrollController",
    },
    RootJsModuleAsset {
        path: "/canvas-wheel-gesture.js",
        source: canvas_wheel_gesture_js,
        marker: "createCanvasWheelGestureClassifier",
    },
    RootJsModuleAsset {
        path: "/theme-manager.js",
        source: theme_manager_js,
        marker: "createThemeManager",
    },
    RootJsModuleAsset {
        path: "/theme-toggle.js",
        source: theme_toggle_js,
        marker: "wireThemeToggle",
    },
    RootJsModuleAsset {
        path: "/hotkey.js",
        source: hotkey_js,
        marker: "createHotkeyManager",
    },
    RootJsModuleAsset {
        path: "/operator-shell.js",
        source: operator_shell_js,
        marker: "initOperatorShell",
    },
    RootJsModuleAsset {
        path: "/focus-trap.js",
        source: focus_trap_js,
        marker: "createFocusTrap",
    },
    RootJsModuleAsset {
        path: "/project-tabs-renderer.js",
        source: project_tabs_renderer_js,
        marker: "renderProjectTabs",
    },
    RootJsModuleAsset {
        path: "/index-settings-panel.js",
        source: index_settings_panel_js,
        marker: "renderIndexSettingsPanel",
    },
    RootJsModuleAsset {
        path: "/terminal-viewport-reflow.js",
        source: terminal_viewport_reflow_js,
        marker: "attachHostResizeReflow",
    },
    RootJsModuleAsset {
        path: "/window-geometry-sync.js",
        source: window_geometry_sync_js,
        marker: "shouldApplyWorkspaceGeometry",
    },
    RootJsModuleAsset {
        path: "/socket-receive-dispatcher.js",
        source: socket_receive_dispatcher_js,
        marker: "createSocketReceiveDispatcher",
    },
    RootJsModuleAsset {
        path: "/terminal-output-buffer.js",
        source: terminal_output_buffer_js,
        marker: "createTerminalOutputBatcher",
    },
    RootJsModuleAsset {
        path: "/interaction-guard.js",
        source: interaction_guard_js,
        marker: "createInteractionGuard",
    },
    RootJsModuleAsset {
        path: "/clone-modal-focus-guard.js",
        source: clone_modal_focus_guard_js,
        marker: "shouldSkipTerminalFocusActivation",
    },
    RootJsModuleAsset {
        path: "/viewport-persist-throttle.js",
        source: viewport_persist_throttle_js,
        marker: "createViewportPersistThrottle",
    },
    RootJsModuleAsset {
        path: "/viewport-sync.js",
        source: viewport_sync_js,
        marker: "createViewportSyncState",
    },
    RootJsModuleAsset {
        path: "/ui-trace-profiler.js",
        source: ui_trace_profiler_js,
        marker: "createUiTraceProfiler",
    },
    RootJsModuleAsset {
        path: "/ui-trace-wiring.js",
        source: ui_trace_wiring_js,
        marker: "createUiTraceWiring",
    },
    RootJsModuleAsset {
        path: "/custom-agent-env-editor.js",
        source: custom_agent_env_editor_js,
        marker: "renderCustomAgentEnvEditor",
    },
    RootJsModuleAsset {
        path: "/release-notes-window.js",
        source: release_notes_window_js,
        marker: "createReleaseNotesWindow",
    },
    RootJsModuleAsset {
        path: "/console-window.js",
        source: console_window_js,
        marker: "createConsoleWindow",
    },
    RootJsModuleAsset {
        path: "/launch-controls.js",
        source: launch_controls_js,
        marker: "buildReasoningField",
    },
];

pub fn root_js_module_assets() -> &'static [RootJsModuleAsset] {
    ROOT_JS_MODULE_ASSETS
}

pub fn styles_tokens_css() -> &'static str {
    include_str!("../web/styles/tokens.css")
}

pub fn styles_typography_css() -> &'static str {
    include_str!("../web/styles/typography.css")
}

pub fn styles_components_css() -> &'static str {
    include_str!("../web/styles/components.css")
}

// Issue #2694 Phase D — extracted from the formerly-inline index.html <style>
// block (~91KB). Served as a separate stylesheet so initial HTML parse is fast
// and modal/dialog repaints do not re-parse the giant inline block.
pub fn styles_app_css() -> &'static str {
    include_str!("../web/styles/app.css")
}

// SPEC-2356 Operator Design System — fonts (binary).
pub fn font_mona_sans() -> &'static [u8] {
    include_bytes!("../web/fonts/MonaSans.woff2")
}

pub fn font_hubot_regular() -> &'static [u8] {
    include_bytes!("../web/fonts/HubotSans-Regular.woff2")
}

pub fn font_hubot_bold() -> &'static [u8] {
    include_bytes!("../web/fonts/HubotSans-Bold.woff2")
}

pub fn font_hubot_condensed_bold() -> &'static [u8] {
    include_bytes!("../web/fonts/HubotSansCondensed-Bold.woff2")
}

pub fn font_jetbrains_mono() -> &'static [u8] {
    include_bytes!("../web/fonts/JetBrainsMono.woff2")
}

pub async fn index_handler() -> impl IntoResponse {
    (
        [(header::CACHE_CONTROL, MUTABLE_CACHE_CONTROL)],
        Html(index_html()),
    )
}

pub async fn app_js_handler() -> impl IntoResponse {
    mutable_js_response(app_js())
}

fn js_response(source: &'static str) -> impl IntoResponse {
    ([(header::CONTENT_TYPE, JS_CONTENT_TYPE)], source)
}

fn mutable_js_response(source: &'static str) -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, JS_CONTENT_TYPE),
            (header::CACHE_CONTROL, MUTABLE_CACHE_CONTROL),
        ],
        source,
    )
}

pub fn root_js_module_response(asset: RootJsModuleAsset) -> impl IntoResponse {
    let source = (asset.source)();
    debug_assert!(source.contains(asset.marker));
    mutable_js_response(source)
}

pub async fn xterm_js_handler() -> impl IntoResponse {
    js_response(xterm_js())
}

pub async fn xterm_fit_js_handler() -> impl IntoResponse {
    js_response(xterm_fit_js())
}

pub async fn xterm_css_handler() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        xterm_css(),
    )
}

pub async fn highlight_js_handler() -> impl IntoResponse {
    js_response(highlight_js())
}

pub async fn highlight_css_handler() -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
        highlight_css(),
    )
}

pub async fn styles_tokens_css_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, MUTABLE_CACHE_CONTROL),
        ],
        styles_tokens_css(),
    )
}

pub async fn styles_typography_css_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, MUTABLE_CACHE_CONTROL),
        ],
        styles_typography_css(),
    )
}

pub async fn styles_components_css_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, MUTABLE_CACHE_CONTROL),
        ],
        styles_components_css(),
    )
}

pub async fn styles_app_css_handler() -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "text/css; charset=utf-8"),
            (header::CACHE_CONTROL, MUTABLE_CACHE_CONTROL),
        ],
        styles_app_css(),
    )
}

fn font_response(bytes: &'static [u8]) -> impl IntoResponse {
    (
        [
            (header::CONTENT_TYPE, "font/woff2"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
        ],
        bytes,
    )
}

pub async fn font_mona_sans_handler() -> impl IntoResponse {
    font_response(font_mona_sans())
}

pub async fn font_hubot_regular_handler() -> impl IntoResponse {
    font_response(font_hubot_regular())
}

pub async fn font_hubot_bold_handler() -> impl IntoResponse {
    font_response(font_hubot_bold())
}

pub async fn font_hubot_condensed_bold_handler() -> impl IntoResponse {
    font_response(font_hubot_condensed_bold())
}

pub async fn font_jetbrains_mono_handler() -> impl IntoResponse {
    font_response(font_jetbrains_mono())
}

#[cfg(test)]
mod tests {
    use super::{
        app_js, index_html, project_tabs_renderer_js, styles_components_css,
        terminal_context_menu_js, xterm_css, xterm_fit_js, xterm_js,
    };
    use super::{
        app_js_handler, index_handler, styles_components_css_handler, styles_tokens_css_handler,
        styles_typography_css_handler, xterm_css_handler, xterm_fit_js_handler, xterm_js_handler,
    };
    use super::{root_js_module_assets, root_js_module_response};

    fn frontend_bundle_source() -> &'static str {
        concat!(
            include_str!("../web/index.html"),
            "\n",
            include_str!("../web/styles/app.css"),
            "\n",
            include_str!("../web/app.js"),
            "\n",
            include_str!("../web/branch-list-state.js"),
            "\n",
            include_str!("../web/board-surface.js"),
            "\n",
            include_str!("../web/workspace-kanban-surface.js"),
            "\n",
            include_str!("../web/update-cta.js"),
            "\n",
            include_str!("../web/terminal-context-menu.js")
        )
    }

    /// Issue #2694 Phase D: surface that combines the embedded HTML with the
    /// stylesheets served via separate routes (`/styles/{tokens,typography,
    /// components,app}.css`). Tests that previously grepped the inline
    /// `<style>` block in `index.html` continue to find the same selectors and
    /// custom properties after that block was moved to `/styles/app.css`.
    fn frontend_styles_bundle() -> &'static str {
        concat!(
            include_str!("../web/index.html"),
            "\n",
            include_str!("../web/styles/tokens.css"),
            "\n",
            include_str!("../web/styles/typography.css"),
            "\n",
            include_str!("../web/styles/components.css"),
            "\n",
            include_str!("../web/styles/app.css")
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
    fn embedded_web_terminal_copy_shortcut_module_is_registered() {
        let paths: Vec<&str> = root_js_module_assets()
            .iter()
            .map(|asset| asset.path)
            .collect();
        assert!(
            paths.contains(&"/terminal-copy-shortcut.js"),
            "expected terminal copy shortcut helper to be served as a root JS module",
        );
    }

    #[test]
    fn embedded_web_terminal_windows_ctrl_c_copy_clears_selection() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("from \"/terminal-copy-shortcut.js\""),
            "expected app.js to import the terminal copy shortcut classifier",
        );
        assert!(
            html.contains("classifyTerminalCopyKeyEvent"),
            "expected app.js to classify Windows Ctrl+C terminal copy before xterm handles input",
        );
        assert!(
            html.contains("clearSelectionAfterCopy"),
            "expected Windows Ctrl+C terminal copy to carry a selection-clear decision",
        );
        assert!(
            html.contains("terminal.clearSelection"),
            "expected Windows Ctrl+C terminal copy to clear the terminal selection after copying",
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
    fn embedded_web_terminal_clipboard_async_path_restores_focus() {
        let html = frontend_bundle_source();
        let async_copy = regex::Regex::new(
            r"await\s+navigator\.clipboard\.writeText\(text\);\s*restoreFocus\?\.\(\);\s*return\s+true;",
        )
        .expect("valid regex");

        assert!(
            async_copy.is_match(html),
            "expected async clipboard success path to restore terminal focus before returning",
        );
    }

    #[test]
    fn embedded_web_terminal_overlay_text_is_copyable() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("className = \"overlay-copy-button\"")
                && html.contains("Copy")
                && html.contains("copyTerminalOverlayMessage"),
            "expected terminal overlay to expose an explicit copy button wired to the overlay message",
        );
        assert!(
            html.contains(".terminal-overlay.visible")
                && html.contains("user-select: text")
                && html.contains("pointer-events: auto"),
            "expected visible terminal overlays to allow normal text selection",
        );
        assert!(
            html.contains("writeClipboardText(messageEl.textContent"),
            "expected overlay copy to reuse the shared clipboard writer",
        );
    }

    #[test]
    fn embedded_web_terminal_image_paste_sends_backend_event_without_text_fallback() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("function installTerminalImagePasteHandlers"),
            "expected terminal image paste handler bootstrap in embedded html",
        );
        assert!(
            html.contains("terminalRoot.addEventListener(\"paste\""),
            "expected paste listener to be installed on the terminal root",
        );
        assert!(
            html.contains("event.clipboardData?.items"),
            "expected paste handler to inspect clipboard items",
        );
        assert!(
            html.contains("SUPPORTED_IMAGE_PASTE_MIME_TYPES"),
            "expected paste handler to constrain supported image MIME types",
        );
        assert!(
            html.contains("event.preventDefault();") && html.contains("event.stopPropagation();"),
            "expected image paste to suppress duplicate text paste injection",
        );
        assert!(
            html.contains("kind: \"paste_image_uploaded\"")
                && html.contains("uploadPastedImage")
                && html.contains("uploadAttachmentFile")
                && html.contains("mime_type")
                && html.contains("filename"),
            "expected image paste backend event with uploaded payload, MIME type, and filename",
        );
    }

    #[test]
    fn embedded_web_terminal_file_drop_sends_attach_files_event() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("function installTerminalFileDropHandlers"),
            "expected terminal file drop handler bootstrap in embedded html",
        );
        assert!(
            html.contains("terminalRoot.addEventListener(\"dragover\"")
                && html.contains("terminalRoot.addEventListener(\"drop\""),
            "expected dragover/drop listeners to be scoped to the terminal root",
        );
        assert!(
            html.contains("event.dataTransfer?.files")
                && html.contains("uploadFilesAsAttachments")
                && html.contains("createAttachmentProgressController"),
            "expected browser file drops to upload files with visible progress",
        );
        assert!(
            html.contains("kind: \"attach_files\"")
                && html.contains("source: \"uploaded\"")
                && html.contains("upload_id")
                && html.contains("operation_id"),
            "expected browser file drops to send uploaded attach_files payloads with operation ids",
        );
        assert!(
            html.contains("attachmentProgressControllers")
                && html.contains("handleAttachmentProgress")
                && html.contains("workspaceWindowElement")
                && !html.contains("attachment-progress__cancel"),
            "expected attachment progress to be scoped to the Agent window without a Cancel action",
        );
        let drop_start = html
            .find("function installTerminalFileDropHandlers")
            .expect("file drop handler source");
        let drop_end = html[drop_start..]
            .find("function installTerminalContextMenuHandlers")
            .expect("next terminal handler source")
            + drop_start;
        let drop_source = &html[drop_start..drop_end];
        assert!(
            !drop_source.contains("SUPPORTED_IMAGE_PASTE_MIME_TYPES"),
            "generic file drops must not inherit the clipboard image MIME allow-list",
        );
        assert!(
            !drop_source.contains("MAX_TOTAL_FILE_DROP_BYTES")
                && !drop_source.contains("droppedFilesWithinTotalSizeLimit"),
            "browser file drops must not enforce the legacy total payload guard",
        );
        assert!(
            !drop_source.contains("Promise.all("),
            "browser file drops must not read every dropped file concurrently",
        );
        assert!(
            html.contains("terminal.focus();"),
            "expected file drop paths to restore terminal focus after sending",
        );
    }

    #[test]
    fn embedded_web_native_file_drop_maps_pointer_to_terminal() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("window.addEventListener(\"gwt:native-file-drop\""),
            "expected native WebView file drops to be bridged through a frontend custom event",
        );
        assert!(
            html.contains("document.elementFromPoint")
                && html.contains(".terminal-root")
                && html.contains(".workspace-window"),
            "expected native drops to map the WebView pointer to the terminal under it",
        );
        assert!(
            html.contains("source: \"native_path\"")
                && html.contains("kind: \"attach_files\"")
                && html.contains("operation_id"),
            "expected native drops to send native_path attach_files payloads with operation ids",
        );
    }

    #[test]
    fn embedded_web_terminal_context_menu_pastes_text_and_images() {
        let html = frontend_bundle_source();
        let context_menu_source = terminal_context_menu_js();

        assert!(
            html.contains("createTerminalContextMenuController"),
            "expected app.js to install the terminal context menu controller",
        );
        assert!(
            html.contains("canvas.addEventListener(\"contextmenu\"")
                && html.contains("event.preventDefault();"),
            "expected non-terminal canvas contextmenu to remain suppressed",
        );
        assert!(
            html.contains("terminal.paste(text)"),
            "expected context menu text paste to flow through xterm paste",
        );
        assert!(
            html.contains("navigator.clipboard?.readText")
                && html.contains("navigator.clipboard?.read"),
            "expected Paste to use async clipboard text and item APIs",
        );
        assert!(
            context_menu_source.contains("textContent = \"Paste\""),
            "terminal context menu user-facing action must be English",
        );
        assert!(
            context_menu_source.contains("readClipboardItems")
                && context_menu_source.contains("pasteImage")
                && html.contains("paste_image_uploaded"),
            "expected context menu Paste to preserve uploaded image paste routing",
        );
    }

    #[test]
    fn embedded_web_terminal_writes_refresh_viewport_after_xterm_parse() {
        let html = frontend_bundle_source();
        let streaming_merge = regex::Regex::new(
            r"(?s)const terminalOutputBatcher = createTerminalOutputBatcher\(\{[\s\S]*?mergeChunks:\s*\(chunks,\s*windowId\)\s*=>\s*\{[\s\S]*?decoderMap\.get\(windowId\)[\s\S]*?chunks\s*\.\s*map\(\(chunk\) => decoder\.decode\(decodeBase64\(chunk\),\s*\{\s*stream:\s*true\s*\}\)\)",
        )
        .expect("valid regex");
        let streaming_enqueue = regex::Regex::new(
            r"(?s)function writeOutput\(windowId, base64\) \{[\s\S]*?terminalOutputBatcher\.enqueue\(\s*windowId,\s*base64\s*\);",
        )
        .expect("valid regex");
        let streaming_write = regex::Regex::new(
            r"(?s)const terminalOutputBatcher = createTerminalOutputBatcher\(\{[\s\S]*?write:\s*\(windowId,\s*text,\s*onWritten\)\s*=>\s*\{[\s\S]*?runtime\.terminal\.write\(text,\s*onWritten\);[\s\S]*?onFlush:\s*\(windowId\)\s*=>\s*\{[\s\S]*?scheduleTerminalViewportRefresh\(windowId\);[\s\S]*?\},[\s\S]*?\}\);",
        )
        .expect("valid regex");
        // SPEC-2008 Phase 26.B / FR-056: snapshot replays must force the
        // activation sequence (refresh -> fit -> sendGeometry) or mark a
        // pending refresh if the terminal is hidden, otherwise the hidden
        // short-circuit on a background tab leaves xterm with stale cell
        // metrics and dead scrollback wheel until the next OS resize.
        let snapshot_write = regex::Regex::new(
            r"(?s)runtime\.terminal\.write\(\s*decoder\.decode\(decodeBase64\(base64\)\),\s*\(\)\s*=>\s*\{[\s\S]*?forceTerminalViewportRefresh\(windowId,\s*\{\s*shouldPersistGeometry:\s*true\s*\}\);[\s\S]*?\}\s*\);",
        )
        .expect("valid regex");
        let snapshot_activation = regex::Regex::new(
            r"(?s)function forceTerminalViewportRefresh\(windowId,[\s\S]*?viewportRefreshPending = true[\s\S]*?runTerminalActivationSequence\(\{[\s\S]*?shouldFocus:\s*false,[\s\S]*?shouldPersistGeometry,[\s\S]*?sendGeometry,[\s\S]*?\}\);",
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
            html.contains("createTerminalViewportRefreshScheduler({")
                && html.contains("terminalViewportRefreshScheduler.enqueue(windowId)"),
            "expected terminal runtime to route viewport refreshes through the shared scheduler",
        );
        assert!(
            html.contains(
                r#"import { createTerminalOutputBatcher } from "/terminal-output-buffer.js";"#
            ),
            "expected app.js to import the terminal output batcher root module",
        );
        assert!(
            streaming_merge.is_match(html),
            "expected terminal output batcher to decode encoded chunks during scheduled flush",
        );
        assert!(
            streaming_enqueue.is_match(html),
            "expected streaming terminal output to enqueue encoded chunks without receive-path decode",
        );
        assert!(
            streaming_write.is_match(html),
            "expected terminal output batch flushes to refresh viewport after xterm parses them",
        );
        assert!(
            snapshot_write.is_match(html),
            "expected terminal snapshots to refresh viewport after xterm parses them",
        );
        assert!(
            snapshot_activation.is_match(html),
            "expected terminal snapshots to force runTerminalActivationSequence when visible and mark pending while hidden (FR-056)",
        );
        assert!(
            html.contains("viewportRefreshPending: false")
                && html.contains("runtime.viewportRefreshPending = true")
                && html.contains("rearmPendingTerminalViewportRefresh")
                && html.contains("rearmRefreshOnVisible({"),
            "expected hidden viewport refreshes to stay pending and re-arm on visible transition",
        );
        assert!(
            html.contains("document.addEventListener(\"visibilitychange\"")
                && html.contains("rearmVisibleTerminalViewportRefreshes();"),
            "expected document visibility restore to re-arm visible terminal refreshes",
        );
        assert!(
            html.contains("terminalOutputBatcher.clear(windowId);"),
            "expected pending terminal output batches to be cleared on snapshot and removed-window cleanup",
        );
        assert!(
            html.contains("terminalViewportRefreshScheduler?.clear(windowId);"),
            "expected terminal cleanup to clear pending shared viewport refreshes",
        );
        assert!(
            html.contains("function canRefreshTerminalViewport(windowId)")
                && html.contains("viewportEligibleForRefresh({")
                && refresh_call.is_match(html),
            "expected terminal viewport refresh predicate to delegate to viewportEligibleForRefresh",
        );
        assert!(
            !html.contains(
                "runtime.terminal.write(decoder.decode(decodeBase64(base64), { stream: true }), () => {\n          fitTerminal(windowId, false);"
            ) && !html.contains(
                "runtime.terminal.write(decoder.decode(decodeBase64(base64)), () => {\n          fitTerminal(windowId, false);"
            ),
            "expected terminal output refresh to avoid geometry refits on every PTY chunk",
        );
        assert!(
            html.contains("const wasMinimized = element.classList.contains(\"minimized\")")
                && html.contains("const previousWidth = parseFloat(element.style.width")
                && html.contains("const previousHeight = parseFloat(element.style.height")
                && html.contains("const dimensionsChanged =")
                && html.contains("(wasMinimized && !windowData.minimized) || dimensionsChanged",)
                && html
                    .contains("scheduleTerminalFit(windowData.id, shouldPersistTerminalGeometry)"),
            "expected terminals to persist fitted geometry to backend on \
             restore-from-minimized OR window resize (Tile/Stack/Align)",
        );
    }

    #[test]
    fn embedded_web_terminal_runtime_buffers_writes_until_initial_fit_handshake() {
        // SPEC-2008 Phase 26.A / FR-057 — writeOutput and
        // replaceTerminalSnapshot must hold incoming bytes until the
        // initial runTerminalActivationSequence has run, otherwise the
        // first Claude Code bytes (generated at the backend's spawn
        // cols/rows) get written into xterm's default 80×24 grid and
        // stay layout-locked there until the next manual resize.
        let html = frontend_bundle_source();
        // The rAF must dispatch to completeInitialFitHandshake — keeping
        // the handshake idempotent and gated on visibility (see helper
        // below). Inlining the activation / replay in the rAF would let
        // `isReady` flip while the window is still hidden, defeating the
        // deferredWrites buffer (CodeRabbit PR #2693 concern).
        let create_runtime_handshake = regex::Regex::new(
            r#"(?s)isReady: false,\s*deferredWrites: \[\],[\s\S]*?handshakeAttempts: 0,\s*\};\s*terminalMap\.set\(windowId, runtime\);\s*decoderMap\.set\(windowId, new TextDecoder\(\)\);[\s\S]*?requestAnimationFrame\(\(\) => completeInitialFitHandshake\(windowId\)\);"#,
        )
        .expect("valid regex");
        // The helper itself must (a) bail when canRefreshTerminalViewport
        // is false so we do not flip isReady while hidden, and (b) only
        // mark the runtime ready after activation succeeds.
        let handshake_helper = regex::Regex::new(
            r#"(?s)function completeInitialFitHandshake\(windowId\) \{[\s\S]*?if \(!runtime \|\| runtime\.isReady\) \{[\s\S]*?return;[\s\S]*?\}[\s\S]*?if \(!canRefreshTerminalViewport\(windowId\)\) \{[\s\S]*?return;[\s\S]*?\}[\s\S]*?const activation = runTerminalActivationSequence\(\{[\s\S]*?\}\);\s*if \(!activation\.ran\) \{[\s\S]*?retryInitialFitHandshake\(windowId, runtime,[\s\S]*?return;[\s\S]*?\}\s*runtime\.handshakeAttempts = 0;\s*runtime\.isReady = true;[\s\S]*?const snapshot = pendingSnapshotMap\.get\(windowId\);[\s\S]*?const pending = pendingOutputMap\.get\(windowId\);[\s\S]*?if \(runtime\.deferredWrites\.length\) \{[\s\S]*?for \(const chunk of flush\) \{[\s\S]*?writeOutput\(windowId, chunk\);[\s\S]*?\}[\s\S]*?\}"#,
        )
        .expect("valid regex");
        // Hidden → visible activation path also needs to drive the
        // handshake — otherwise a window created hidden never drains
        // its deferred buffer until the user manually resizes.
        let reveal_completes_handshake = regex::Regex::new(
            r#"(?s)function scheduleTerminalFocusActivation\(\s*windowId,[\s\S]*?\)\s*\{[\s\S]*?runTerminalActivationSequence\(\{[\s\S]*?\}\);[\s\S]*?if \(activeRuntime\.isReady === false\) \{\s*completeInitialFitHandshake\(windowId\);"#,
        )
        .expect("valid regex");
        let write_gate = regex::Regex::new(
            r#"(?s)function writeOutput\(windowId, base64\) \{[\s\S]*?if \(runtime\.isReady === false\) \{\s*runtime\.deferredWrites\.push\(base64\);\s*return;\s*\}"#,
        )
        .expect("valid regex");
        let snapshot_gate = regex::Regex::new(
            r#"(?s)function replaceTerminalSnapshot\(windowId, base64\) \{[\s\S]*?if \(runtime\.isReady === false\) \{\s*pendingSnapshotMap\.set\(windowId, base64\);\s*return;\s*\}"#,
        )
        .expect("valid regex");

        assert!(
            create_runtime_handshake.is_match(html),
            "expected createTerminalRuntime to dispatch its initial-fit handshake through completeInitialFitHandshake instead of inlining the replay (FR-057, CodeRabbit fix)",
        );
        assert!(
            handshake_helper.is_match(html),
            "expected completeInitialFitHandshake to bail on canRefreshTerminalViewport=false and only set isReady=true after activation succeeds (FR-057, CodeRabbit fix)",
        );
        assert!(
            reveal_completes_handshake.is_match(html),
            "expected scheduleTerminalFocusActivation to invoke completeInitialFitHandshake on hidden -> visible so windows created hidden can still drain deferredWrites (FR-057, CodeRabbit fix)",
        );
        assert!(
            write_gate.is_match(html),
            "expected writeOutput to push to runtime.deferredWrites when runtime.isReady === false (FR-057)",
        );
        assert!(
            snapshot_gate.is_match(html),
            "expected replaceTerminalSnapshot to re-queue into pendingSnapshotMap when runtime.isReady === false (FR-057)",
        );
        // Sanity: legacy "snapshot replay runs synchronously before fit"
        // structure must no longer be present, otherwise the regression
        // can land alongside the new gate and still cause the bug.
        let legacy_sync_replay = regex::Regex::new(
            r#"(?s)requestAnimationFrame\(\(\) => fitTerminal\(windowId,\s*true\)\);\s*const snapshot = pendingSnapshotMap\.get\(windowId\);"#,
        )
        .expect("valid regex");
        assert!(
            !legacy_sync_replay.is_match(html),
            "legacy synchronous snapshot replay before initial fit must be removed (FR-057 regression guard)",
        );

        // Issue #2832 — SPEC-2008 Phase 26.A regression fix: the
        // visibility predicate (canRefreshTerminalViewport) only checks
        // `.hidden` and `.minimized`. It does NOT catch the case where
        // the element is structurally visible but layout has not yet
        // propagated, leaving the parent container at 0×0 at the moment
        // the initial-fit rAF fires. In that state fitAddon.fit() resolves
        // against the 0-sized box and silently leaves xterm at the
        // default 80×24 grid; flushing deferredWrites then renders the
        // post-launch Claude Code output corrupted, with the
        // resize-recovers-on-move signature documented in
        // .gwt/work/memory.md 2026-05-13.
        let layout_box_gate = regex::Regex::new(
            r#"(?s)function completeInitialFitHandshake\(windowId\) \{[\s\S]*?if \(!canRefreshTerminalViewport\(windowId\)\) \{[\s\S]*?return;[\s\S]*?\}[\s\S]*?if \(!terminalContainerHasLayoutBox\(windowId\)\) \{\s*retryInitialFitHandshake\(windowId, runtime,[\s\S]*?\);\s*return;\s*\}"#,
        )
        .expect("valid regex");
        assert!(
            layout_box_gate.is_match(html),
            "expected completeInitialFitHandshake to defer (and rAF-retry) while terminalContainerHasLayoutBox returns false so deferredWrites do not flush into xterm's default 80x24 grid before fit can resolve real cols/rows (Issue #2832)",
        );
        assert!(
            html.contains("elementHasLayoutBox,"),
            "expected app.js to import elementHasLayoutBox from terminal-viewport-reflow.js (Issue #2832)",
        );
        assert!(
            html.contains("function terminalContainerHasLayoutBox(windowId)"),
            "expected app.js to expose terminalContainerHasLayoutBox so the handshake can gate on layout (Issue #2832)",
        );
        assert!(
            html.contains("const terminalHost = runtime?.terminal?.element?.parentElement;")
                && html.contains("return elementHasLayoutBox(terminalHost);"),
            "expected terminalContainerHasLayoutBox to measure the actual xterm host before falling back to the outer workspace window (Issue #2839)",
        );
        assert!(
            html.contains("const HANDSHAKE_RETRY_LIMIT ="),
            "expected app.js to declare HANDSHAKE_RETRY_LIMIT so the retry loop has a ceiling (Issue #2832)",
        );
    }

    #[test]
    fn embedded_web_focus_activation_retries_on_unsettled_layout_box() {
        // Issue #2937 (#2832 parity for the focus trigger): the focus-change
        // reflow path must not be a one-shot silent no-op. When
        // runTerminalActivationSequence returns { ran: false } — e.g. a
        // tab-group member revealed before its container layout box settles —
        // scheduleTerminalFocusActivation must re-arm a bounded rAF retry
        // (activationAttempts capped by HANDSHAKE_RETRY_LIMIT), exactly like
        // completeInitialFitHandshake does for the initial fit. Without this
        // the revealed terminal keeps the stale grid until a manual resize.
        let html = frontend_bundle_source();
        let focus_retry = regex::Regex::new(
            r#"(?s)function scheduleTerminalFocusActivation\([\s\S]*?const activation = runTerminalActivationSequence\(\{[\s\S]*?\}\);\s*if \(!activation\.ran\) \{[\s\S]*?activationAttempts[\s\S]*?HANDSHAKE_RETRY_LIMIT[\s\S]*?scheduleTerminalFocusActivation\(windowId,[\s\S]*?return;\s*\}"#,
        )
        .expect("valid regex");
        assert!(
            focus_retry.is_match(html),
            "expected scheduleTerminalFocusActivation to re-arm a bounded retry (activationAttempts <= HANDSHAKE_RETRY_LIMIT) when runTerminalActivationSequence returns !ran (Issue #2937)",
        );
        let runtime_init =
            regex::Regex::new(r#"(?s)activationFrame: null,[\s\S]*?activationAttempts: 0,"#)
                .expect("valid regex");
        assert!(
            runtime_init.is_match(html),
            "expected createTerminalRuntime to initialize activationAttempts so the focus-path retry has a bounded counter (Issue #2937)",
        );
    }

    #[test]
    fn embedded_web_window_pointer_events_force_reset_on_mismatch() {
        // SPEC-2008 Phase 26.C / FR-059 — Windows WebView2 occasionally
        // emits pointerup / pointercancel with a pointerId that does not
        // match the one captured at pointerdown. The previous handlers
        // gated finishWindowResize behind a strict pointerId equality
        // check, so a mismatched pointerup left resizeState alive until
        // the 30 second staleness guard finally cleaned it up. This
        // contract pins the new fallback: any window-level pointerup or
        // pointercancel that fires while a resize is pending must clean
        // up resizeState immediately via forceResetResizeState.
        let html = frontend_bundle_source();
        let pointerup_fallback = regex::Regex::new(
            r#"(?s)window\.addEventListener\("pointerup", \(event\) => \{[\s\S]*?if \(resizeState\) \{[\s\S]*?if \(resizeState\.pointerId === event\.pointerId\) \{[\s\S]*?finishWindowResize\(event\.pointerId,\s*event\);[\s\S]*?\} else \{[\s\S]*?forceResetResizeState\("window pointerup pointerId mismatch"\);"#,
        )
        .expect("valid regex");
        let pointercancel_fallback = regex::Regex::new(
            r#"(?s)window\.addEventListener\("pointercancel", \(event\) => \{[\s\S]*?if \(resizeState && resizeState\.pointerId !== event\.pointerId\) \{[\s\S]*?forceResetResizeState\("window pointercancel pointerId mismatch"\);[\s\S]*?return;[\s\S]*?\}[\s\S]*?finishWindowResize\(event\.pointerId,\s*event\);"#,
        )
        .expect("valid regex");

        assert!(
            pointerup_fallback.is_match(html),
            "expected window pointerup to fall back to forceResetResizeState when pointerId mismatches (FR-059)",
        );
        assert!(
            pointercancel_fallback.is_match(html),
            "expected window pointercancel to fall back to forceResetResizeState when pointerId mismatches (FR-059)",
        );
    }

    #[test]
    fn embedded_web_terminal_resize_coalesces_fit_and_restores_focus_on_release() {
        let html = frontend_bundle_source();
        let direct_pointermove_fit = regex::Regex::new(
            r"element\.style\.height = `\$\{clamp\((?s:.*?)\)\}px`;\s*fitTerminal\(resizeState\.id,\s*false\);",
        )
        .expect("valid regex");
        let resize_finalizer = regex::Regex::new(
            r"function finishWindowResize\(pointerId,\s*event = null\) \{(?s:.*?)syncResizeStatePointerEvent\(resizeState,\s*event\);(?s:.*?)cancelTerminalResizeFit\(\);(?s:.*?)fitTerminal\(resizeState\.id,\s*false\);(?s:.*?)sendGeometry\((?s:.*?)runtime\?\.terminal\.focus\(\);(?s:.*?)resizeState = null;",
        )
        .expect("valid regex");

        assert!(
            html.contains("function scheduleTerminalResizeFit(windowId)")
                && html.contains("function cancelTerminalResizeFit()"),
            "expected terminal resize fits to be requestAnimationFrame-coalesced",
        );
        assert!(
            !direct_pointermove_fit.is_match(html),
            "expected pointermove resize to avoid direct terminal fit/geometry churn",
        );
        assert!(
            resize_finalizer.is_match(html),
            "expected resize finalizer to cancel pending fit, sync once, and restore terminal focus",
        );
    }

    #[test]
    fn embedded_web_window_resize_cancellation_uses_shared_finalizer() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("function finishWindowResize(pointerId, event = null)"),
            "expected all floating window resize completion paths to share one finalizer",
        );
        assert!(
            html.contains("finishWindowResize(event.pointerId, event);"),
            "expected pointerup resize path to use the shared finalizer with release coordinates",
        );
        assert!(
            html.contains("window.addEventListener(\"pointercancel\", (event) => {")
                && html.contains("finishWindowResize(event.pointerId, event);"),
            "expected pointercancel to finalize resize state with release coordinates",
        );
        assert!(
            html.contains("resizeHandle.addEventListener(\"lostpointercapture\", (event) => {")
                && html.contains("finishWindowResize(event.pointerId, event);"),
            "expected lost pointer capture to finalize resize state with release coordinates",
        );
        assert!(
            html.contains("if (!terminalMap.has(windowId)) {\n          return;\n        }"),
            "expected terminal resize fit scheduling to skip non-terminal panel windows",
        );
    }

    /// SPEC-2014 Phase C4: 高頻度な pointermove を直接 DOM mutation に
    /// 流し込まず、`requestAnimationFrame` で 1 フレーム 1 回に絞り込んで
    /// いることを bundle 上で固定する。Windows WebView2 で layout reflow が
    /// pointermove 速度に追従できず render thread が枯渇する症状を回避するため。
    #[test]
    fn embedded_web_resize_pointermove_is_coalesced_via_request_animation_frame() {
        let html = frontend_bundle_source();
        let direct_pointermove_width = regex::Regex::new(
            r"if\s*\(resizeState && resizeState\.pointerId === event\.pointerId\)\s*\{\s*const element = windowMap\.get\(resizeState\.id\)",
        )
        .expect("valid regex");
        assert!(
            !direct_pointermove_width.is_match(html),
            "expected pointermove resize handler to stop directly mutating element.style; the coalesced applyResizePointermove path must replace it"
        );
        assert!(
            html.contains("function scheduleResizePointermoveApply()"),
            "expected a rAF scheduler that coalesces pointermove-driven DOM writes"
        );
        assert!(
            html.contains("function cancelResizePointermoveApply()"),
            "expected the coalescing scheduler to be cancellable on resize teardown"
        );
        assert!(
            html.contains("function applyResizePointermove(state)"),
            "expected a pure helper that translates resizeState into element.style.width/height"
        );
        assert!(
            html.contains("resizeState.latestClientX = event.clientX;"),
            "expected pointermove to store the latest client coordinates on resizeState"
        );
        assert!(
            html.contains("scheduleResizePointermoveApply();"),
            "expected pointermove to schedule the coalesced apply rather than write to the DOM directly"
        );
        assert!(
            html.contains("applyFrame: null,"),
            "expected resizeState to carry an applyFrame slot so the rAF handle can be cancelled"
        );
    }

    /// SPEC-2014 Phase C1: Windows WebView2 で pointerup / pointercancel /
    /// lostpointercapture のいずれも届かないケースで Wizard / Terminal が永久に
    /// 固まらないよう、resizeState には auto-clear する staleness guard と
    /// 二重 resize 検知時の forceReset が組み込まれていなければならない。
    #[test]
    fn embedded_web_resize_state_guards_against_lost_pointer_end_events() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("function scheduleResizeStalenessGuard(pointerId)"),
            "expected a staleness guard scheduler that triggers when pointerup/cancel/lostpointercapture all miss"
        );
        assert!(
            html.contains("function cancelResizeStalenessGuard()"),
            "expected the staleness guard to be cancellable on the normal resize teardown path"
        );
        assert!(
            html.contains("function forceResetResizeState(reason)"),
            "expected a force-reset helper that clears stale resizeState when a new resize starts"
        );
        assert!(
            html.contains(
                "forceResetResizeState(\"new resize started before previous one finished\");",
            ),
            "expected new pointerdown to force-reset any leaked previous resizeState before opening a new session"
        );
        assert!(
            html.contains("startedAt: performance.now(),"),
            "expected resizeState to carry a startedAt timestamp so the staleness guard can log elapsed time"
        );
        assert!(
            html.contains("stalenessTimer: scheduleResizeStalenessGuard(event.pointerId),"),
            "expected the resize start path to schedule the staleness guard"
        );
        assert!(
            html.contains("try {\n              resizeHandle.setPointerCapture(event.pointerId);")
                && html.contains(
                    "console.warn(\n                \"[resize] setPointerCapture failed, falling back to window-bound pointer events\"",
                ),
            "expected setPointerCapture to be wrapped in try/catch with a warning fallback for WebView2 edge cases"
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
    fn embedded_web_secondary_assets_are_embedded() {
        assert!(!xterm_js().is_empty());
        assert!(!xterm_fit_js().is_empty());
        assert!(xterm_css().contains(".xterm"));
        // Root module registry is the include coverage source: a missing
        // include_str! macro fails CI rather than silently 404ing in production.
        for asset in root_js_module_assets() {
            let source = (asset.source)();
            assert!(!source.is_empty(), "expected {} to be embedded", asset.path);
            assert!(
                source.contains(asset.marker),
                "expected {} to contain marker {}",
                asset.path,
                asset.marker,
            );
        }
    }

    /// SPEC-2780 follow-up — every top-level `import ... from "/x.js"` in any
    /// shipped JS module must be registered in `ROOT_JS_MODULE_ASSETS` or the
    /// embedded server returns 404 at runtime, the ES module load aborts, and
    /// the splash hangs because no boot wiring runs. We learned this the hard
    /// way when `/release-notes-window.js` was added to `app.js` without a
    /// matching route; assert here so the regression cannot recur silently.
    #[test]
    fn every_root_js_module_import_in_app_assets_is_registered() {
        use std::collections::BTreeSet;

        // Crawl every embedded JS source registered with the server plus
        // `app.js` (the entrypoint) for `from "/X.js"` patterns.
        let mut imports: BTreeSet<String> = BTreeSet::new();
        let mut sources: Vec<(&str, &str)> = vec![("/app.js", app_js())];
        for asset in root_js_module_assets() {
            sources.push((asset.path, (asset.source)()));
        }

        for (origin, source) in &sources {
            collect_root_js_imports(source, origin, &mut imports);
        }

        let registered: BTreeSet<String> = root_js_module_assets()
            .iter()
            .map(|asset| asset.path.to_string())
            .collect();

        let missing: Vec<String> = imports
            .difference(&registered)
            .filter(|path| *path != "/app.js")
            .cloned()
            .collect();

        assert!(
            missing.is_empty(),
            "embedded JS modules import these paths but they are not registered \
             in ROOT_JS_MODULE_ASSETS (will 404 at runtime and freeze splash): {:?}",
            missing,
        );
    }

    fn collect_root_js_imports(
        source: &str,
        origin: &str,
        out: &mut std::collections::BTreeSet<String>,
    ) {
        // Tolerant scanner for top-level absolute-path module imports of the
        // shape `from "/something.js"` or `from '/something.js'`. We do not
        // need a full JS parser here; the goal is to catch obvious 404 traps.
        for line in source.lines() {
            let trimmed = line.trim_start();
            if !trimmed.starts_with("import")
                && !trimmed.starts_with("} from")
                && !trimmed.contains(" from ")
            {
                continue;
            }
            for quote in ['"', '\''] {
                let mut rest = line;
                while let Some(idx) = rest.find(quote) {
                    let after = &rest[idx + 1..];
                    if let Some(end) = after.find(quote) {
                        let candidate = &after[..end];
                        if candidate.starts_with('/') && candidate.ends_with(".js") {
                            // Ignore the entrypoint itself; app.js is served by
                            // a dedicated handler, not via root_js_module_assets.
                            if candidate != "/app.js" {
                                out.insert(candidate.to_string());
                            }
                        }
                        rest = &after[end + 1..];
                    } else {
                        break;
                    }
                }
            }
            // Suppress the unused-variable lint when no match path is taken.
            let _ = origin;
        }
    }

    #[tokio::test]
    async fn embedded_web_asset_handlers_set_content_types() {
        use axum::{http::header, response::IntoResponse};

        let js = "text/javascript; charset=utf-8";
        assert_eq!(
            app_js_handler()
                .await
                .into_response()
                .headers()
                .get(header::CONTENT_TYPE)
                .unwrap(),
            js,
        );
        for asset in root_js_module_assets() {
            assert_eq!(
                root_js_module_response(*asset)
                    .into_response()
                    .headers()
                    .get(header::CONTENT_TYPE)
                    .unwrap(),
                js,
                "expected {} to use JavaScript content type",
                asset.path,
            );
        }
        assert_eq!(
            xterm_js_handler()
                .await
                .into_response()
                .headers()
                .get(header::CONTENT_TYPE)
                .unwrap(),
            js,
        );
        assert_eq!(
            xterm_fit_js_handler()
                .await
                .into_response()
                .headers()
                .get(header::CONTENT_TYPE)
                .unwrap(),
            js,
        );
        assert_eq!(
            xterm_css_handler()
                .await
                .into_response()
                .headers()
                .get(header::CONTENT_TYPE)
                .unwrap(),
            "text/css; charset=utf-8",
        );
    }

    #[tokio::test]
    async fn embedded_web_mutable_assets_disable_stale_cache() {
        use axum::{http::header, response::IntoResponse};

        let expected = "no-store, max-age=0";
        assert_eq!(
            index_handler()
                .await
                .into_response()
                .headers()
                .get(header::CACHE_CONTROL)
                .unwrap(),
            expected,
        );
        assert_eq!(
            app_js_handler()
                .await
                .into_response()
                .headers()
                .get(header::CACHE_CONTROL)
                .unwrap(),
            expected,
        );
        for asset in root_js_module_assets() {
            assert_eq!(
                root_js_module_response(*asset)
                    .into_response()
                    .headers()
                    .get(header::CACHE_CONTROL)
                    .unwrap(),
                expected,
                "expected {} to avoid stale WebView cache",
                asset.path,
            );
        }
        for response in [
            styles_tokens_css_handler().await.into_response(),
            styles_typography_css_handler().await.into_response(),
            styles_components_css_handler().await.into_response(),
        ] {
            assert_eq!(
                response.headers().get(header::CACHE_CONTROL).unwrap(),
                expected,
            );
        }
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
    fn embedded_web_canvas_wheel_gesture_classifier_is_served_and_imported() {
        let js = app_js();
        let asset = root_js_module_assets()
            .iter()
            .find(|asset| asset.path == "/canvas-wheel-gesture.js")
            .expect("expected canvas wheel gesture classifier asset");

        assert_eq!(asset.marker, "createCanvasWheelGestureClassifier");
        assert!(
            js.contains("from \"/canvas-wheel-gesture.js\""),
            "expected app.js to import the canvas wheel gesture classifier",
        );
        assert!(
            js.contains("canvasWheelGestureClassifier.classify(event)"),
            "expected canvas wheel routing to classify the whole wheel gesture before routing",
        );
    }

    #[test]
    fn embedded_web_canvas_stage_keeps_transform_layer_hint_opt_in() {
        let html = frontend_styles_bundle();

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
            r#"(?s)function applyViewport\(\)\s*\{\s*stage\.style\.transform = `translate\(\$\{viewport\.x\}px, \$\{viewport\.y\}px\) scale\(\$\{viewport\.zoom\}\)`;\s*applyWorldGridViewport\(\);\s*stage\.style\.willChange = "transform";\s*if \(viewportRasterTimer !== null\) \{\s*clearTimeout\(viewportRasterTimer\);\s*\}\s*viewportRasterTimer = setTimeout\(\(\) => \{\s*stage\.style\.willChange = "auto";\s*viewportRasterTimer = null;\s*\}, 300\);[\s\S]*?\}"#,
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
    fn embedded_web_canvas_grid_tracks_viewport_as_world_space_cue() {
        let html = frontend_styles_bundle();
        let js = app_js();

        assert!(
            html.contains("id=\"canvas-world-grid\""),
            "expected canvas to expose a dedicated world-space grid layer",
        );
        assert!(
            html.contains(".canvas-world-grid"),
            "expected embedded html to define the world-space grid CSS",
        );
        assert!(
            js.contains("const worldGrid = document.getElementById(\"canvas-world-grid\")"),
            "expected frontend to bind the world-space grid element",
        );
        assert!(
            js.contains("function applyWorldGridViewport()"),
            "expected grid viewport sync to live behind a named helper",
        );
        assert!(
            js.contains("applyWorldGridViewport();"),
            "expected applyViewport to update the grid whenever viewport state changes",
        );
    }

    #[test]
    fn embedded_web_window_status_chip_uses_running_idle_stopped_error_variants() {
        let html = frontend_styles_bundle();

        assert!(
            html.contains(".status-chip.idle .status-dot"),
            "expected embedded html to define an idle variant for window status chips",
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
    fn embedded_web_project_bar_omits_index_status_badge() {
        let html = frontend_styles_bundle();
        let js = app_js();
        let project_tabs_js = project_tabs_renderer_js();

        // SPEC-1939 Phase 13: project-bar Index badge withdrawn. The badge
        // surface and its supporting controller / progress-toast wiring must
        // not ship in the embedded assets. The aggregated payload still
        // drives the per-tab dot and the dedicated Index window Health tab.
        assert!(
            !html.contains("id=\"index-status\""),
            "SPEC-1939 Phase 13: project-bar Index badge must be removed",
        );
        assert!(
            !html.contains(".index-status ")
                && !html.contains(".index-status.")
                && !html.contains(".index-status-toast"),
            "SPEC-1939 Phase 13: index-status / toast CSS rules must be removed",
        );
        assert!(
            !html.contains("animation: index-status-spin"),
            "SPEC-1939 Phase 13: badge spinner animation must be removed",
        );
        assert!(
            !js.contains("formatIndexStatusLabel")
                && !js.contains("indexStatusLabel")
                && !js.contains("showRepairingProgressToast")
                && !js.contains("renderIndexStatus("),
            "SPEC-1939 Phase 13: badge formatter / toast helpers must be removed from app.js",
        );
        assert!(
            js.contains("function setIndexStatus(projectRoot, status)")
                && js.contains("case \"project_index_status\""),
            "frontend must still consume project_index_status events for the Index Health tab",
        );
        assert!(
            !js.contains("buildSettingsTab(\"index\"") && js.contains("renderIndexSettingsPanel({"),
            "SPEC-1939 Phase 15: Settings must drop Index while the Index window keeps the health panel",
        );
        assert!(
            html.contains(".project-tab-dot")
                && project_tabs_js.contains("projectTabAgentDotState(tab")
                && !project_tabs_js.contains("aggregateProjectTabDotState"),
            "SPEC-2013 Phase 6: project tab dot must reflect running agent state, not Index health",
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
        let html = frontend_styles_bundle();

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
    fn embedded_web_agent_runtime_maps_idle_to_idle_telemetry() {
        let js = app_js();

        assert!(
            js.contains("idle: \"Idle\""),
            "expected embedded js to expose an Idle runtime label",
        );
        assert!(
            js.contains("case \"idle\":") && js.contains("return \"idle\";"),
            "expected embedded js to count idle runtime states as idle telemetry",
        );
    }

    #[test]
    fn embedded_web_agent_runtime_maps_starting_separately() {
        let js = app_js();
        let html = frontend_styles_bundle();

        assert!(
            js.contains("starting: \"Starting\""),
            "expected embedded js to expose a Starting runtime label (US-69)",
        );
        assert!(
            js.contains("case \"starting\":") && js.contains("return \"not_started\";"),
            "expected the starting runtime state to map onto the separate not_started telemetry rim",
        );
        assert!(
            html.contains(".status-chip.starting .status-dot"),
            "expected embedded html to define a starting status chip variant",
        );
    }

    #[test]
    fn embedded_web_window_role_badges_identify_every_window_surface() {
        let html = frontend_styles_bundle();
        let js = app_js();

        assert!(
            html.contains(".window-role-badge") && html.contains(".window-list-role"),
            "expected role badge styling for titlebars and the window list",
        );
        assert!(
            js.contains("function presetRoleLabel(preset)"),
            "expected a shared presetRoleLabel helper",
        );
        for label in [
            "Shell",
            "Claude",
            "Codex",
            "Agent",
            "File Tree",
            "Branches",
            "Settings",
            "Profile",
            "Logs",
            "Issue",
            "SPEC",
            "Board",
            "PR",
        ] {
            assert!(
                js.contains(label),
                "expected presetRoleLabel to cover {label}",
            );
        }
        assert!(
            js.contains("function shouldShowRuntimeStatus(windowData)")
                && js.contains("runtimeChip.hidden = !shouldShowRuntimeStatus(windowData)"),
            "expected non-terminal panels to hide runtime status chips",
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
    fn embedded_web_programmatic_terminal_focus_reactivates_xterm_after_render() {
        let js = app_js();
        let render_activation = regex::Regex::new(
            r#"(?s)const topmostId = topmostWindowId\(workspace\);.*?focusWindowLocally\(topmostId\);.*?scheduleTerminalFocusActivation\(topmostId,\s*\{[\s\S]*?shouldPersistGeometry:\s*false,?[\s\S]*?\}\);"#,
        )
        .expect("valid regex");
        // SPEC-2008 Phase 26.B / FR-056: activation must delegate to
        // runTerminalActivationSequence so the render-before-fit ordering
        // is enforced. The previous Phase 24 ordering (fitTerminal → then
        // scheduleTerminalViewportRefresh → then focus) silently no-op'd
        // whenever the terminal had been display:none because xterm's
        // proposeDimensions returns undefined while cell.width === 0.
        //
        // Issue #2704: shouldFocus is now computed by the
        // clone-modal-focus-guard so modal/text-input focus is preserved
        // across `workspace_state` events. The regex accepts either the
        // shorthand `shouldFocus,` or an explicit `shouldFocus: <expr>`
        // form, but no longer pins the value to a literal `true`.
        let activation_helper = regex::Regex::new(
            r#"(?s)function scheduleTerminalFocusActivation\(\s*windowId,\s*\{[\s\S]*?shouldPersistGeometry\s*=\s*true[\s\S]*?\}\s*=\s*\{\},\s*\)\s*\{.*?requestAnimationFrame\(\(\) => \{.*?const activeRuntime = terminalMap\.get\(windowId\);.*?runTerminalActivationSequence\(\{[\s\S]*?runtime: activeRuntime,[\s\S]*?shouldFocus(?:\s*[,:])[\s\S]*?shouldPersistGeometry(?:\s*[,:])[\s\S]*?syncGeometryOnGridChange:\s*true,[\s\S]*?sendGeometry,[\s\S]*?\}\);[\s\S]*?scheduleTerminalViewportRefresh\(windowId\);"#,
        )
        .expect("valid regex");

        assert!(
            js.contains("function scheduleTerminalFocusActivation(")
                && js.contains("shouldPersistGeometry = true"),
            "expected a shared xterm activation helper for programmatic window focus with geometry persistence enabled by default",
        );
        assert!(
            activation_helper.is_match(js),
            "expected programmatic terminal activation to refit, refresh, pass computed shouldFocus, and opt into grid-change geometry sync after render",
        );
        assert!(
            render_activation.is_match(js),
            "expected workspace render to reactivate the topmost terminal without echoing geometry into backend resize broadcasts",
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
        // Issue #2694 Phase C: handleSocketOpen now also re-initializes the
        // per-connection dispatcher before the frontend_ready handshake. The
        // regex below is intentionally `[\s\S]*?` (non-greedy any) between
        // setConnectionState and the pendingMessages flush so dispatcher
        // setup is allowed inside the function, but the ordering assertion
        // — frontend_ready strictly precedes the queued-message replay — is
        // preserved.
        let open_flow = regex::Regex::new(
            r#"function handleSocketOpen\(\)\s*\{[\s\S]*?setConnectionState\(true\);\s*send\(\{\s*kind:\s*"frontend_ready"\s*\}\);\s*while\s*\(\s*pendingMessages\.length\s*>\s*0\s*\)\s*\{\s*socket\.send\(JSON\.stringify\(pendingMessages\.shift\(\)\)\);\s*\}\s*\}"#,
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
        // Phase C regression: when the WebSocket cycles, the dispatcher must
        // be recreated and old generations must be guarded so queued events
        // from the previous session do not flush into the new one.
        assert!(
            html.contains("socketReceiveDispatcherGeneration"),
            "expected handleSocketOpen / handleSocketClose to track a generation counter for the per-connection dispatcher",
        );
        assert!(
            html.contains("ownGeneration !== socketReceiveDispatcherGeneration"),
            "expected the dispatcher receive callback to gate on the generation captured at open time",
        );
    }

    #[test]
    fn embedded_web_workspace_state_announces_startup_auto_resume_ready_after_render() {
        let html = frontend_bundle_source();
        let readiness_flow = regex::Regex::new(
            r#"case\s+"workspace_state":\s*\{[\s\S]*?projectWorkspaceShell\.renderAppState\(event\.workspace\);[\s\S]*?sendStartupAutoResumeReady\(\);[\s\S]*?break;"#,
        )
        .expect("valid regex");

        assert!(
            html.contains("function sendStartupAutoResumeReady()"),
            "expected a named one-shot startup auto-resume readiness helper",
        );
        assert!(
            html.contains("kind: \"startup_auto_resume_ready\"")
                && html.contains("bounds: visibleBounds()"),
            "expected readiness payload to carry the current visible canvas bounds",
        );
        assert!(
            readiness_flow.is_match(html),
            "expected workspace_state hydration to render before announcing startup auto-resume readiness",
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
        // SPEC-2359 US-37: workspace_state case wraps in a block to
        // populate the Workspace Overview Completed column from
        // event.workspace.tabs[active].workspace.work_items before
        // breaking. Tolerate the optional `{` and additional code
        // between renderAppState and break.
        let workspace_state_flow = regex::Regex::new(
            r#"case\s*"workspace_state":\s*\{?\s*projectError\s*=\s*"";\s*(?:renderAppState|frontendUnits\.projectWorkspaceShell\.renderAppState)\(event\.workspace\);[\s\S]*?\bbreak;"#,
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
        assert!(
            !html.contains("Branch cleanup timed out"),
            "expected cleanup result handling to be driven by backend events, not a frontend timer"
        );
        assert!(
            !html.contains("BRANCH_CLEANUP_TIMEOUT_MS"),
            "expected branch cleanup to avoid a hard-coded frontend failure timeout"
        );
    }

    #[test]
    fn embedded_web_branches_surface_remains_branch_browser() {
        let html = frontend_bundle_source();
        let branches_block = html
            .split("if (surface === \"branches\")")
            .nth(1)
            .and_then(|tail| tail.split("if (surface === \"profile\")").next())
            .expect("branches render block");

        assert!(
            branches_block.contains("Repository branches")
                && branches_block.contains("branch-list")
                && branches_block.contains("open-branch-cleanup"),
            "expected Branches surface to stay branch-list oriented",
        );
        assert!(
            !branches_block.contains("Planning Session")
                && !branches_block.contains("active_work_projection")
                && !branches_block.contains("workspace-card"),
            "expected Branches surface not to render Workspace or Planning Session cards",
        );
    }

    #[test]
    fn embedded_web_serves_every_root_module_import() {
        let embedded_web_source = include_str!("embedded_web.rs");
        let embedded_server_source = include_str!("embedded_server.rs");
        let mut module_graph_source = String::from(app_js());
        for asset in root_js_module_assets() {
            module_graph_source.push('\n');
            module_graph_source.push_str((asset.source)());
        }

        assert!(
            embedded_server_source.contains("root_js_module_assets()"),
            "expected embedded server root module routes to be registry-driven",
        );

        for asset in root_js_module_assets() {
            let module_path = asset.path;
            let relative_module_path = format!("./{}", module_path.trim_start_matches('/'));
            assert!(
                module_graph_source.contains(module_path)
                    || module_graph_source.contains(&relative_module_path),
                "expected frontend module graph to import {module_path}",
            );

            let source_name = module_path
                .trim_start_matches('/')
                .trim_end_matches(".js")
                .replace('-', "_");
            assert!(
                embedded_web_source.contains(&format!("fn {source_name}_js()")),
                "expected embedded web module source function for {module_path}",
            );
        }
    }

    #[test]
    fn embedded_web_root_js_module_registry_covers_app_imports() {
        let registry_paths: Vec<&str> = root_js_module_assets()
            .iter()
            .map(|asset| asset.path)
            .collect();

        for module_path in [
            "/branch-cleanup-modal.js",
            "/close-project-tab-confirm-modal.js",
            "/migration-modal.js",
            "/window-docking.js",
            "/board-surface.js",
            "/update-cta.js",
            "/theme-manager.js",
            "/theme-toggle.js",
            "/hotkey.js",
            "/operator-shell.js",
            "/focus-trap.js",
            "/terminal-context-menu.js",
            "/terminal-copy-shortcut.js",
            "/terminal-wheel-scroll.js",
            "/custom-agent-env-editor.js",
        ] {
            assert!(
                registry_paths.contains(&module_path),
                "expected root JS registry to include {module_path}",
            );
        }
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
    fn embedded_web_branches_surface_explains_detail_check_state() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("branchLoadStatusSummary"),
            "expected Branches bundle to derive a compact load status summary",
        );
        for expected in [
            "Checking branch details",
            // SPEC-2009 FR-066: the interrupted detail check now self-heals on
            // reconnect, so the copy is reassuring rather than alarming.
            "Reconnecting branch details",
            "Recovering automatically",
            "Safety unknown",
        ] {
            assert!(
                html.contains(expected),
                "expected Branches bundle to include clarity copy: {expected}",
            );
        }
        // FR-066: the manual "Refresh to verify cleanup safety" banner copy is
        // gone — the detail check recovers without a user-driven refresh.
        assert!(
            !html.contains("Refresh to verify cleanup safety"),
            "expected the manual-refresh detail-check banner copy to be removed",
        );
        assert!(
            !html.contains("Cleanup status unavailable"),
            "expected Branches bundle to avoid ambiguous cleanup unavailable copy",
        );
    }

    #[test]
    fn embedded_web_branches_surface_animates_only_checking_detail_state() {
        let html = frontend_bundle_source();

        for expected in [
            "@keyframes branch-detail-check-sweep",
            "@keyframes branch-cleanup-checking-pulse",
            r#".branch-notice[data-branch-status="checking"]::before"#,
            ".branch-cleanup-badge.loading",
            "prefers-reduced-motion: reduce",
        ] {
            assert!(
                html.contains(expected),
                "expected Branches bundle to include checking animation contract: {expected}",
            );
        }
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
            !html.contains("data-knowledge-scope="),
            "expected Kanban bridge to remove legacy open/closed cache tabs",
        );
        assert!(
            !html.contains("list_scope"),
            "expected Kanban bridge requests to omit legacy issue list scope",
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
    fn embedded_web_index_window_exposes_project_index_search_contract() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("data-preset=\"index\""),
            "expected Add Window modal to expose an Index preset",
        );
        assert!(
            html.contains("search_project_index"),
            "expected Index surface search input to call the project index search backend",
        );
        assert!(
            html.contains("project_index_search_results"),
            "expected frontend to handle Index search result events",
        );
        assert!(
            html.contains("index-search-root"),
            "expected a dedicated Index window surface instead of overloading Settings",
        );
        assert!(
            html.contains("data-index-tab=\"health\""),
            "expected Index window to host the existing health/rebuild table",
        );
        assert!(
            html.contains(".index-search-panel[hidden]")
                && html.contains(".index-health-panel[hidden]")
                && html.contains("display: none !important"),
            "Index tabs must hide inactive Search/Health panels even when panel CSS sets display"
        );
        assert!(
            html.contains("index-search-toolbar")
                && html.contains("index-health-toolbar")
                && html.contains("index-health-table"),
            "Index Search and Health controls must be visually separated instead of sharing one toolbar"
        );
        assert!(
            html.contains("index-run-button")
                && html.contains("formatIndexSearchMatch")
                && html.contains("File worktree"),
            "Index Search must expose an explicit search action, friendly match scores, and file-only worktree context"
        );
        assert!(
            html.contains("classList.toggle(\"is-empty\"")
                && html.contains(".index-search-layout.is-empty")
                && html.contains(".index-search-layout.is-empty .index-result-detail"),
            "Index Search must collapse the unused detail pane when there are no results"
        );
        assert!(
            html.contains("INDEX_SEARCH_DEFAULT_SCOPES")
                && html.contains("selectedScopes: new Set(INDEX_SEARCH_DEFAULT_SCOPES)"),
            "Index Search must default to lightweight scopes while leaving files/docs opt-in"
        );
        assert!(
            !html.contains("buildSettingsTab(\"index\""),
            "Settings must no longer expose its own Index tab",
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
    fn embedded_web_board_surface_exposes_audience_reply_and_notification_ui() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("board-for-you-filter")
                && html.contains("board-audience-badge")
                && html.contains("board-reply-button")
                && html.contains("board-reply-banner")
                && html.contains("board-reply-quote")
                && html.contains("showBoardMentionNotification")
                && html.contains("Jump to original"),
            "expected Board UI to expose mention audience, reply, quote, and notification affordances",
        );
    }

    #[test]
    fn embedded_web_board_surface_does_not_render_workspace_or_planning_cards() {
        let html = frontend_bundle_source();
        let board_block = html
            .split("if (surface === \"board\")")
            .nth(1)
            .and_then(|tail| tail.split("if (surface === \"logs\")").next())
            .expect("board render block");

        assert!(
            board_block.contains("board-chat-shell")
                && board_block.contains("board-timeline")
                && board_block.contains("board-composer"),
            "expected Board surface to remain a chat/event log",
        );
        assert!(
            !board_block.contains("Planning Session")
                && !board_block.contains("workspace-card")
                && !board_block.contains("active_work_projection"),
            "expected Board surface not to render Workspace or Planning Session cards",
        );
    }

    /// SPEC-2359 Phase W-12 Slice 3 (FR-351): the sidebar Active Works overview
    /// is removed and the Work surface (Workspace Overview / Kanban) is now the
    /// single home for Work lifecycle. The legacy sidebar render entrypoints and
    /// the `op-active-work` DOM section must no longer ship in the bundle.
    #[test]
    fn embedded_web_active_work_sidebar_overview_is_removed() {
        let html = frontend_bundle_source();

        assert!(
            !html.contains("function renderActiveWorkOverview")
                && !html.contains("function renderActiveWorkAgentCard")
                && !html.contains("id=\"op-active-work\""),
            "sidebar Active Works overview must be removed in favor of the Work surface",
        );
    }

    /// SPEC-2359 Phase W-12 Slice 3 (FR-351): the sidebar-only `op-active-work`
    /// CSS is removed when the section is retired so no orphaned styles linger.
    #[test]
    fn embedded_web_active_work_sidebar_css_is_removed() {
        let css = styles_components_css();

        assert!(
            !css.contains(".op-active-work"),
            "retired Active Works sidebar must not leave orphaned op-active-work CSS",
        );
    }

    /// SPEC-2359 Phase W-12 Slice 3 (FR-351): the Work surface renders each Work
    /// card with its agent-session lifecycle state badge (Active / Paused / Done
    /// / Discarded).
    #[test]
    fn embedded_web_work_surface_renders_lifecycle_state_badge() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("workspace-overview-lifecycle")
                && html.contains("formatLifecycleStateLabel"),
            "Work surface must render a lifecycle_state badge on each Work card",
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
    fn embedded_web_board_message_body_renders_markdown_with_plaintext_fallback() {
        // SPEC-2963: the body is authored in Markdown and rendered from the
        // server-sanitized `body_html`; only the plaintext fallback keeps
        // pre-wrap to preserve author-provided newlines.
        let html = frontend_bundle_source();
        let plaintext_block = {
            let start = html
                .find(".board-message-body.is-plaintext {")
                .expect("expected Board plaintext-fallback CSS block");
            let rest = &html[start..];
            let end = rest.find('}').expect("expected CSS block end");
            &rest[..=end]
        };

        assert!(
            plaintext_block.contains("white-space: pre-wrap"),
            "Board plaintext fallback must preserve newlines, got: {plaintext_block}",
        );
        assert!(
            html.contains("createKnowledgeMarkdownBody(entry, \"board-message-body\")"),
            "Board body must render via the shared Markdown renderer",
        );
        assert!(
            html.contains("createNode(\"div\", \"board-message-title\", entry.title)"),
            "Board card must render the optional post title",
        );
        assert!(
            html.contains(".board-message-title {"),
            "Board title styling must exist",
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
                && html.contains("const pendingSubmit = state.pendingSubmit;")
                && html.contains("const completedSubmit = Boolean(pendingSubmit")
                && html.contains("!pendingSubmit.existingEntryIds.has(entry.id)")
                && html.contains("state.composerBody = \"\";")
                && html.contains("state.pendingSubmit = null;")
                && html.contains("state.pendingSelfPostScroll = true;"),
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
    fn embedded_web_does_not_expose_removed_memo_surface() {
        let html = frontend_bundle_source();

        assert!(
            !html.contains("data-preset=\"memo\""),
            "Memo must not be offered from Add window",
        );
        assert!(
            !html.contains("memo-root"),
            "Memo root scaffold should be removed from embedded html",
        );
        assert!(
            !html.contains("load_memo")
                && !html.contains("create_memo_note")
                && !html.contains("update_memo_note")
                && !html.contains("delete_memo_note"),
            "Memo protocol events should be removed from the frontend bundle",
        );
        assert!(
            !html.contains("memoSurface,"),
            "frontend unit registry should not expose a removed Memo surface",
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
            html.contains("Environment Variables")
                && html.contains("Use OS")
                && html.contains("Override")
                && html.contains("Disabled")
                && html.contains("Result")
                && html.contains("+ Add variable"),
            "expected Profile surface to render a single environment variable grid",
        );
        assert!(
            !html.contains("Save now")
                && !html.contains("Profile variables")
                && !html.contains("Disabled OS variables")
                && !html.contains("Merged preview"),
            "expected Profile Metadata lower content to be unified into the grid",
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
    fn embedded_web_add_window_modal_hides_direct_terminal_presets() {
        let html = frontend_bundle_source();

        assert!(
            !html.contains(r#"data-preset="shell""#)
                && !html.contains(r#"data-preset="claude""#)
                && !html.contains(r#"data-preset="codex""#),
            "expected Add window modal to hide Shell/Claude/Codex direct terminal presets",
        );
        assert!(
            !html.contains(r#"data-preset="branches""#),
            "expected Add window modal to not have a standalone Branches preset — branch browsing is part of the Work surface",
        );
    }

    #[test]
    fn embedded_web_launch_wizard_actions_flow_through_named_transport() {
        let html = frontend_bundle_source();
        let submit_bounds = regex::Regex::new(
            r#"function sendWizardAction\(action\)\s*\{\s*send\(\{\s*kind:\s*"launch_wizard_action",\s*action,\s*bounds:\s*visibleBounds\(\),\s*\}\);\s*\}"#,
        )
        .expect("valid regex");
        let footer_close_control = regex::Regex::new(
            r#"wizardCancelButton\.addEventListener\("click",\s*closeLaunchWizardFromChrome\);"#,
        )
        .expect("valid regex");
        let submit_button = regex::Regex::new(
            r#"function\s+handleLaunchWizardSubmitFromChrome\(\)\s*\{[\s\S]*?releaseWizardInteractionGuardForChromeAction\(\)[\s\S]*?launchWizardOpenError[\s\S]*?(?:flushWizardBranchDraft|frontendUnits\.launchWizardSurface\.flushBranchDraft)\(\);[\s\S]*?(?:sendWizardAction|frontendUnits\.launchWizardSurface\.sendAction)\(\{\s*kind:\s*"submit"\s*\}\);\s*\}"#,
        )
        .expect("valid regex");
        let chrome_guard_release = regex::Regex::new(
            r#"function\s+releaseWizardInteractionGuardForChromeAction\(\)\s*\{[\s\S]*?wizardInteractionGuard\.isActive\(\)[\s\S]*?wizardInteractionGuard\.release\(\)[\s\S]*?return\s+Boolean\(launchWizard\s*\|\|\s*launchWizardOpenError\);"#,
        )
        .expect("valid regex");
        let guarded_submit_button = regex::Regex::new(
            r#"function\s+handleLaunchWizardSubmitFromChrome\(\)[\s\S]*?releaseWizardInteractionGuardForChromeAction\(\)[\s\S]*?(?:flushWizardBranchDraft|frontendUnits\.launchWizardSurface\.flushBranchDraft)\(\);[\s\S]*?(?:sendWizardAction|frontendUnits\.launchWizardSurface\.sendAction)\(\{\s*kind:\s*"submit"\s*\}\);"#,
        )
        .expect("valid regex");
        let guarded_start_method_button = regex::Regex::new(
            r#"const\s+handleStartMethodLaunchAction\s*=\s*\(\)\s*=>\s*\{[\s\S]*?releaseWizardInteractionGuardForChromeAction\(\)[\s\S]*?setLaunchWizardPendingAction\(\{[\s\S]*?kind:\s*"use_start_method"[\s\S]*?(?:sendWizardAction|frontendUnits\.launchWizardSurface\.sendAction)\(\{[\s\S]*?kind:\s*"use_start_method""#,
        )
        .expect("valid regex");
        let submit_pointer_fallback = regex::Regex::new(
            r#"wizardSubmitButton\.addEventListener\("pointerup"[\s\S]*?handleLaunchWizardSubmitFromChrome\(\)"#,
        )
        .expect("valid regex");
        let start_method_pointer_fallback = regex::Regex::new(
            r#"button\.addEventListener\("pointerup"[\s\S]*?handleStartMethodLaunchAction\(\)"#,
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
            "expected wizard actions to be normalized through launch_wizard_action and always attach visible bounds",
        );
        assert!(
            !html.contains(r#"id="wizard-close-button""#) && !html.contains("wizardCloseButton"),
            "expected Launch Wizard to avoid a duplicate header Close control",
        );
        assert!(
            footer_close_control.is_match(html),
            "expected footer dismiss control to own the wizard close helper",
        );
        assert!(
            html.contains(r#"id="wizard-back-button""#)
                && html.contains("show_back_button")
                && html.contains("wizardBackButton.addEventListener(\"click\"")
                && html.contains("kind: \"back\""),
            "expected footer Back control to be backend-gated and dispatch the canonical back action",
        );
        assert!(
            html.contains("function closeLaunchWizardFromChrome()")
                && html.contains("closeLaunchWizardLocal();")
                && html.contains("frontendUnits.launchWizardSurface.sendAction({ kind: \"cancel\" });"),
            "expected close helper to local-close error-only state and send cancel for normal wizard state",
        );
        assert!(
            submit_button.is_match(html),
            "expected submit control to ignore error-only state and flush branch draft before dispatching submit",
        );
        assert!(
            chrome_guard_release.is_match(html)
                && html.contains("if (!releaseWizardInteractionGuardForChromeAction())")
                && guarded_submit_button.is_match(html),
            "expected Launch Wizard chrome actions to release pending interaction guard state before dispatch",
        );
        assert!(
            html.contains("launchWizardPendingAction")
                && html.contains("is-launch-pending")
                && html.contains("Creating agent window...")
                && guarded_start_method_button.is_match(html)
                && submit_pointer_fallback.is_match(html)
                && start_method_pointer_fallback.is_match(html),
            "expected Launch Wizard launch actions to expose local pending feedback and pointer-safe Start method dispatch",
        );
        assert!(
            !html.contains("event.target === wizardModal")
                || !html.contains("closeLaunchWizardFromChrome();"),
            "expected wizard backdrop clicks to stop dismissing the wizard",
        );
        // Issue #2698 PR 1 (B7) — the launch_wizard_state case now
        // also defers via `wizardInteractionGuard.defer(...)` before
        // mutating launchWizard, so the regex permits an optional
        // guard preamble between the case label and the assignment. A
        // null tombstone must not clear an open-error modal during reconnect.
        let wizard_state = regex::Regex::new(
            r#"case\s*"launch_wizard_state":[\s\S]*?clearLaunchWizardPendingAction\(\);\s*clearLaunchWizardOpening\(\);\s*if\s*\(event\.wizard\)\s*\{[\s\S]*?launchWizardOpenError\s*=\s*null;[\s\S]*?\}\s*launchWizard\s*=\s*event\.wizard;\s*(?:renderLaunchWizard|frontendUnits\.launchWizardSurface\.render)\(\);\s*break;"#,
        )
        .expect("valid regex");
        assert!(
            wizard_state.is_match(html),
            "expected launch wizard state updates to hydrate the shared wizard renderer",
        );
    }

    #[test]
    fn embedded_web_launch_wizard_runtime_target_payload_uses_backend_enum_values() {
        let html = frontend_bundle_source();
        let payload_mapper = regex::Regex::new(
            r#"function runtimeTargetPayload\(value\)\s*\{\s*return value === "docker" \? "Docker" : "Host";\s*\}"#,
        )
        .expect("valid regex");
        let runtime_target_action = regex::Regex::new(
            r#"kind:\s*"set_runtime_target",\s*target:\s*runtimeTargetPayload\(value\),"#,
        )
        .expect("valid regex");

        assert!(
            payload_mapper.is_match(html),
            "expected Runtime target UI values to map to LaunchRuntimeTarget serde variants",
        );
        assert!(
            runtime_target_action.is_match(html),
            "expected Runtime target changes to send backend enum values instead of raw UI values",
        );
    }

    #[test]
    fn embedded_web_start_work_mode_hides_branch_controls_in_shared_wizard_renderer() {
        let html = frontend_bundle_source();

        assert!(
            html.contains(r#"case "start-work":"#) && html.contains(r#"kind: "open_start_work""#),
            "expected Start Work to use a global command instead of a Branches window action",
        );
        assert!(
            html.contains("launchWizard.show_branch_controls !== false")
                && html.contains("Work launch"),
            "expected Start Work wizard mode to suppress branch controls and branch-oriented meta copy",
        );
        assert!(
            !html.contains("isStartWorkMode")
                && !html.contains(r#"wizardModal.classList.toggle("is-drawer""#),
            "expected Start Work wizard mode to share the centered Launch Wizard modal",
        );
        assert!(
            html.contains("launchWizard.show_agent_settings") && html.contains("\"Agent\""),
            "expected Start Work to keep the existing Agent settings renderer available",
        );
    }

    #[test]
    fn embedded_web_launch_wizard_start_methods_contract() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("wizard-progress-rail")
                && html.contains("wizard-main")
                && html.contains("wizard-content-pane"),
            "expected Launch Wizard to render a split progress/content layout",
        );
        assert!(
            html.contains("progress_steps")
                && html.contains("start_methods")
                && html.contains("show_start_methods")
                && html.contains("primary_action_label"),
            "expected Launch Wizard renderer to follow backend progress, start methods, and footer label",
        );
        assert!(
            html.contains("\"Start methods\"")
                && html.contains("start_methods")
                && html.contains(r#"kind: "use_start_method""#)
                && !html.contains("\"Quick start\"")
                && !html.contains(r#"kind: "select_quick_start""#)
                && !html.contains("quick-start-actions"),
            "expected Start methods to render direct actions instead of the old Quick Start selection model",
        );
    }

    #[test]
    fn embedded_web_launch_wizard_runtime_confirmation_summary_contract() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("const isRuntimeConfirmation = Boolean(")
                && html.contains("launchWizard.runtime_context_resolved")
                && html.contains("launchWizard.show_runtime_confirmation"),
            "expected Launch Wizard to derive a dedicated Runtime confirmation state",
        );
        assert!(
            html.contains("const showSetupForms = showManualSetup && !isRuntimeConfirmation;"),
            "expected setup form rendering to be disabled during Runtime confirmation",
        );
        assert!(
            html.contains("renderWizardSummary();")
                && html.contains("const showStartMethods = Boolean(")
                && html.contains("if (showStartMethods)"),
            "expected Runtime confirmation to keep the read-only summary while hiding selection/setup rows",
        );
        assert!(
            html.contains("showSetupForms && launchWizard.show_branch_controls !== false")
                && html.contains("showSetupForms && launchWizard.show_linked_issue"),
            "expected branch and linked issue controls to be gated to setup forms \
             (linked issue uses dedicated show_linked_issue flag per SPEC-2014 FR-057)",
        );
    }

    // SPEC-2014 Amendment 2026-05-20 (US-25 / FR-057-059 / SC-032)
    // The Launch Wizard "Linked issue" section must render through the
    // `show_linked_issue` gate and must not contain an editable input that
    // dispatches `set_linked_issue` / `clear_linked_issue` from the UI. The
    // value is rendered as static read-only text.
    #[test]
    fn embedded_web_launch_wizard_linked_issue_is_readonly_for_issue_bridge() {
        let html = frontend_bundle_source();

        assert!(
            html.contains("showSetupForms && launchWizard.show_linked_issue"),
            "expected Linked issue section to be gated by show_linked_issue (FR-057)",
        );
        assert!(
            !html.contains(r#"kind: "set_linked_issue""#),
            "expected the frontend to stop dispatching set_linked_issue from the wizard UI (FR-058)",
        );
        assert!(
            !html.contains(r#"kind: "clear_linked_issue""#),
            "expected the frontend to stop dispatching clear_linked_issue from the wizard UI (FR-058)",
        );
        assert!(
            html.contains("launchWizard.linked_issue_number") && html.contains("Issue number"),
            "expected the Linked issue section to render the issue number as static read-only text",
        );
    }

    #[test]
    fn embedded_web_shared_bundle_keeps_user_facing_copy_english_only() {
        let html = frontend_bundle_source();
        let japanese_scripts = regex::Regex::new(r"[ぁ-んァ-ン一-龯]").expect("valid regex");

        assert!(
            html.contains("Open a project")
                && html.contains("Restore previous work or choose a new folder.")
                && html.contains("Launch Agent")
                && html.contains("Connected")
                && html.contains("Reconnecting"),
            "expected shared frontend bundle to keep the browser and native user-facing copy on the English contract",
        );
        // SPEC-1933 NFR-005 exception: the System Settings > Language select
        // option label "日本語" is the only approved Japanese string in the
        // embedded bundle. Strip the option text before scanning so the
        // English-only contract still catches every other unintended copy.
        let stripped = html.replace(r#"text: "日本語""#, r#"text: ""#);
        assert!(
            !japanese_scripts.is_match(&stripped),
            "expected embedded bundle copy to stay English-only for both browser and native modes \
             (SPEC-1933 NFR-005 only allows the Language select '日本語' option label)",
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
                && html.contains("profileSurface,")
                && html.contains("boardSurface,")
                && html.contains("logsSurface,")
                && html.contains("knowledgeSettingsSurface,"),
            "expected frontend unit registry to expose the extracted transport, workspace, terminal, wizard, tree, Profile, Board, Logs, and knowledge/settings surfaces",
        );
        assert!(
            !html.contains("window.__POC__"),
            "expected embedded runtime to stop exporting the retired PoC inspection global",
        );
    }

    #[test]
    fn embedded_web_frontend_units_receive_and_bootstrap_through_named_surfaces() {
        let html = frontend_bundle_source();
        // SPEC-2359 US-37: workspace_state case wraps in a block to
        // populate the Workspace Overview Completed column from
        // event.workspace.tabs[active].workspace.work_items. Tolerate
        // the optional `{` and additional code between renderAppState
        // and break.
        let workspace_event = regex::Regex::new(
            r#"case\s*"workspace_state":\s*\{?\s*projectError\s*=\s*"";\s*frontendUnits\.projectWorkspaceShell\.renderAppState\(event\.workspace\);[\s\S]*?\bbreak;"#,
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
        // Issue #2698 PR 1 (B7) — wizard_state / wizard_open_error
        // now defer through `wizardInteractionGuard.defer(...)` before
        // mutating module state, so the regex tolerates an optional
        // guard preamble between the case label and the mutation. A
        // null tombstone must not clear an open-error modal during reconnect.
        let wizard_event = regex::Regex::new(
            r#"case\s*"launch_wizard_state":[\s\S]*?clearLaunchWizardPendingAction\(\);\s*clearLaunchWizardOpening\(\);\s*if\s*\(event\.wizard\)\s*\{[\s\S]*?launchWizardOpenError\s*=\s*null;[\s\S]*?\}\s*launchWizard\s*=\s*event\.wizard;\s*frontendUnits\.launchWizardSurface\.render\(\);\s*break;"#,
        )
        .expect("valid regex");
        let wizard_open_error_event = regex::Regex::new(
            r#"case\s*"launch_wizard_open_error":[\s\S]*?launchWizard\s*=\s*null;[\s\S]*?launchWizardOpenError\s*=\s*\{[\s\S]*?frontendUnits\.launchWizardSurface\.render\(\);\s*break;"#,
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
        assert!(
            wizard_open_error_event.is_match(html),
            "expected launch wizard open errors to render through the wizard surface unit",
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
    /// `.surface-profile`, and `.surface-knowledge` had been
    /// missing from one or more of these rules, which left those panels
    /// partially transparent and visually distinct from the rest.
    #[test]
    fn embedded_web_panel_surfaces_share_opaque_window_chrome_and_body() {
        let html = frontend_styles_bundle();

        let panel_surfaces = [
            ".surface-file-tree",
            ".surface-branches",
            ".surface-board",
            ".surface-logs",
            ".surface-knowledge",
            ".surface-mock",
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
        let html = frontend_styles_bundle();

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
                // SPEC-2356 — empty state typography flows through tokens so
                // the surface respects dual-theme text colour and the body
                // font scale. The numeric pixel value moved into `--type-sm`.
                &[
                    "padding: 16px 12px",
                    "font-size: var(--type-sm)",
                    "color: var(--color-text-muted)",
                ],
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

    #[test]
    fn embedded_web_profile_root_is_constrained_to_window_body() {
        let html = frontend_styles_bundle();

        let start = html
            .find(".profile-root")
            .expect("expected Profile root CSS to be defined");
        let block = &html[start..];
        let end = block
            .find('}')
            .expect("expected Profile root CSS rule to close");
        let body = &block[..end];

        for prop in [
            "position: absolute",
            "inset: 0",
            "display: flex",
            "flex-direction: column",
        ] {
            assert!(
                body.contains(prop),
                "expected `.profile-root` to declare `{prop}` so Profile panes can own vertical scroll, got: {body}",
            );
        }
    }

    /// SPEC-2008 FR-034: every panel surface must adopt the shared layout
    /// primitives in its rendered HTML so paddings, scrollbars, and splits
    /// stay in lockstep. The toolbar misnomer `.knowledge-toolbar` (which was
    /// reused by Profile/Logs/Board as the generic toolbar block) is
    /// retired in favour of `.workspace-toolbar`. Stacked toolbars (multi-row
    /// content with search and filter chips) opt into the
    /// `.workspace-toolbar.is-stacked` modifier rather than carrying a
    /// surface-specific override.
    #[test]
    fn embedded_web_panel_surfaces_compose_with_layout_primitives() {
        let html = frontend_styles_bundle();
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
            toolbar_count >= 6,
            "expected at least 6 panel surfaces to mount with the `.workspace-toolbar` primitive, found {toolbar_count}",
        );

        // Stacked modifier replaces the old `.knowledge-toolbar` override.
        assert!(
            html.contains(".workspace-toolbar.is-stacked"),
            "expected the stacked toolbar modifier `.workspace-toolbar.is-stacked` to be defined for multi-row toolbars",
        );

        let split_adopters = [
            "knowledge-split workspace-split",
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
        let html = frontend_styles_bundle();

        // SPEC-2356 FR-001: modal frame primitives must reference design
        // tokens from `tokens.css` instead of raw colour literals so the
        // Operator dark/light dual flagship can theme them via CSS variables.
        let primitives: [(&str, &[&str]); 5] = [
            (
                ".modal-backdrop {",
                &[
                    "position: absolute",
                    "inset: 0",
                    "align-items: center",
                    "justify-content: center",
                    "background: var(--color-overlay)",
                ],
            ),
            (
                ".modal-shell {",
                &[
                    "max-height: calc(100vh - 48px)",
                    "overflow: auto",
                    "border-radius: var(--radius-lg)",
                    "background: var(--color-surface-elevated)",
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
            // Branches now redirects to the workspace surface in JS;
            // the enum variant is kept for backend compatibility but
            // no longer needs its own JS return path.
            (WindowSurface::Profile, "profile"),
            (WindowSurface::Board, "board"),
            (WindowSurface::Logs, "logs"),
            (WindowSurface::Knowledge, "knowledge"),
            (WindowSurface::Index, "index"),
            (WindowSurface::Work, "work"),
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

        assert!(
            js.contains("preset === \"branches\"") && js.contains("return \"work\";"),
            "expected JS `presetSurface()` to route branches preset to work surface",
        );
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

    #[test]
    fn embedded_web_project_picker_exposes_github_clone_action_and_modal() {
        let html = index_html();

        assert!(
            html.contains("id=\"picker-clone-project\"") && html.contains("Clone from GitHub..."),
            "Project Picker must expose the GitHub clone action next to Open Project"
        );
        assert!(
            html.contains("id=\"clone-project-modal\"")
                && html.contains("aria-labelledby=\"clone-project-modal-title\"")
                && html.contains("id=\"clone-project-modal-title\""),
            "Clone Project modal must be present in embedded HTML with dialog labels"
        );
        assert!(
            html.contains("data-clone-mode=\"url\"") && html.contains("data-clone-mode=\"search\""),
            "Clone Project modal must expose URL and Search modes"
        );
    }

    // Issue #2684 — top-toolbar Open Project must surface the GitHub clone path
    // even while an active project tab hides the project picker overlay.
    #[test]
    fn embedded_web_top_toolbar_open_project_is_split_button() {
        let html = index_html();

        assert!(
            html.contains("id=\"open-project-group\"")
                && html.contains("class=\"split-button-group\""),
            "top toolbar must mount an Open Project split-button group"
        );
        assert!(
            html.contains("id=\"open-project-menu-button\"")
                && html.contains("aria-haspopup=\"menu\"")
                && html.contains("aria-controls=\"open-project-menu\""),
            "caret button must declare popup/controls semantics for the menu"
        );
        assert!(
            html.contains("id=\"open-project-menu\"") && html.contains("role=\"menu\""),
            "dropdown #open-project-menu must mount with role=menu"
        );
        assert!(
            html.contains("id=\"open-project-menu-open\"")
                && html.contains("id=\"open-project-menu-clone\""),
            "dropdown must expose Open Project and Clone from GitHub menu items"
        );
        assert!(
            html.contains("id=\"open-project-menu-recent\""),
            "dropdown must expose a Recent projects container"
        );
    }

    /// Launch Wizard hydration can add QuickStart, Docker, and Advanced form
    /// content after the initial loading state. The footer must stay outside
    /// that scrollable form body so the Launch button never overlaps hydrated
    /// content.
    #[test]
    fn embedded_web_launch_wizard_scrolls_body_without_footer_overlap() {
        let html = frontend_styles_bundle();

        let css_body = |selector: &str| {
            let start = html
                .find(selector)
                .unwrap_or_else(|| panic!("expected `{selector}` CSS rule to exist"));
            let block = &html[start..];
            let end = block
                .find('}')
                .unwrap_or_else(|| panic!("expected `{selector}` CSS rule to close"));
            &block[..end]
        };

        let wizard_shell = css_body(".modal-shell.is-wizard {");
        assert!(
            wizard_shell.contains("overflow: hidden"),
            "expected Launch Wizard shell to hide shell-level overflow so only the form body scrolls, got: {wizard_shell}",
        );

        let wizard_body = css_body("#wizard-body {");
        for prop in ["overflow: auto", "min-height: 0"] {
            assert!(
                wizard_body.contains(prop),
                "expected #wizard-body to declare `{prop}` as the scroll container, got: {wizard_body}",
            );
        }

        let wizard_footer = css_body(".wizard-footer {");
        assert!(
            wizard_footer.contains("flex: 0 0 auto"),
            "expected wizard footer to keep a stable non-overlapping height, got: {wizard_footer}",
        );
    }

    /// SPEC-2008 FR-035 (JS-side guard): the legacy `.modal` shell class is
    /// retired in HTML, so any `querySelector(".modal")` left in `app.js`
    /// resolves to `null` and silently breaks the modal it backs (this
    /// regressed the Branch Cleanup body in v9.11.0). Lock the JS side to the
    /// `.modal-shell` primitive.
    #[test]
    fn embedded_web_app_js_uses_modal_shell_selector() {
        let js = app_js();
        assert!(
            !js.contains("querySelector(\".modal\")"),
            "expected app.js to query `.modal-shell` instead of the retired `.modal` class (SPEC-2008 FR-035)",
        );
        assert!(
            js.contains("querySelector(\".modal-shell\")"),
            "expected app.js to resolve at least one modal dialog through the `.modal-shell` primitive",
        );
    }

    /// SPEC-2008 FR-050 Phase 24: an OS host window (WebView) `resize` event
    /// must fan out per-terminal `fitTerminal()` so xterm.js cols/rows stay
    /// aligned with the new viewport, and `UpdateWindowGeometry` must be sent
    /// for each visible terminal so the backend PTY cols/rows match. Without
    /// this fan-out the wrap stays stuck until the user resizes a single
    /// terminal manually.
    #[test]
    fn embedded_web_host_window_resize_fans_out_terminal_fit() {
        let js = app_js();
        // After SPEC-2008 Phase 24 follow-up (PR #2590), the host resize
        // fan-out is dispatched through `attachHostResizeReflow` from
        // `terminal-viewport-reflow.js`. The behaviour assertion lives in
        // `__tests__/terminal-viewport-reflow.test.mjs`; this Rust contract
        // only confirms the wiring stays in the bundle.
        let attach_call = regex::Regex::new(r#"(?s)attachHostResizeReflow\(\{(?P<body>.*?)\}\);"#)
            .expect("valid regex");
        let captures = attach_call
            .captures(js)
            .expect("expected attachHostResizeReflow wiring for host window.resize");
        let body = captures.name("body").map(|m| m.as_str()).unwrap_or("");
        assert!(
            body.contains("terminalIds: () => terminalMap.keys()")
                || body.contains("terminalMap.keys()"),
            "expected attachHostResizeReflow to iterate terminalMap so per-terminal fit \
             can fan out to every visible terminal window (SPEC-2008 FR-050), body: {body}",
        );
        assert!(
            js.contains("createTerminalFitScheduler({ fitTerminal })")
                && body.contains("fitTerminal: scheduleTerminalFit"),
            "expected attachHostResizeReflow to route fit requests through the shared \
             terminal fit scheduler while preserving fitTerminal semantics; body: {body}",
        );
        assert!(
            body.contains("syncMaximizedWindowsToViewport()"),
            "expected attachHostResizeReflow.beforeFan to keep maximized window sync, body: {body}",
        );
    }

    /// SPEC-2008 FR-051 Phase 24: `canRefreshTerminalViewport` must reject
    /// hidden terminals (display:none-equivalent `.hidden` attribute set when
    /// a window-tab is non-active) so a fit during the hidden phase cannot
    /// pollute xterm.js with cols/rows = 0. The check must consider both
    /// minimized and the DOM `element.hidden` flag.
    #[test]
    fn embedded_web_can_refresh_terminal_viewport_skips_hidden_tabs() {
        let js = app_js();
        let signature = "function canRefreshTerminalViewport(windowId) {";
        let start = js
            .find(signature)
            .expect("expected canRefreshTerminalViewport predicate definition");
        let after = &js[start..];
        let end_offset = after[signature.len()..]
            .find("\n      function ")
            .map(|i| signature.len() + i)
            .unwrap_or(after.len());
        let body = &after[..end_offset];
        // After SPEC-2008 Phase 24 follow-up, the predicate delegates to
        // `viewportEligibleForRefresh` in `terminal-viewport-reflow.js`.
        // Behaviour (.hidden short-circuit + minimised short-circuit) is
        // pinned by `__tests__/terminal-viewport-reflow.test.mjs`; this
        // Rust contract only confirms the wiring stays in the bundle.
        assert!(
            body.contains("viewportEligibleForRefresh({"),
            "expected canRefreshTerminalViewport to delegate to viewportEligibleForRefresh \
             (SPEC-2008 FR-051 + Phase 24 follow-up); body: {body}",
        );
        assert!(
            body.contains("windowMap.get(windowId)") && body.contains("workspaceWindowById"),
            "expected predicate to forward both the DOM element and the workspace window \
             so the helper can short-circuit on .hidden / minimized; body: {body}",
        );
    }

    /// SPEC-2008 FR-051 Phase 24: when a window tab transitions from
    /// `.hidden = true` to `.hidden = false` (tab switch / focus cycle /
    /// window list / Command Palette), the terminal must run fit + viewport
    /// refresh + focus on the next animation frame so scrollback wheel input
    /// works without a user-driven resize.
    #[test]
    fn embedded_web_tab_visibility_transition_triggers_terminal_focus_activation() {
        let js = app_js();
        let visibility_block = regex::Regex::new(
            r"(?s)for \(const windowData of workspace\.windows\) \{(?P<body>.*?)\}\s*\n\s*scheduleMaximizedWindowsToViewportSync\(\);",
        )
        .expect("valid regex");
        let captures = visibility_block
            .captures(js)
            .expect("expected the workspace.windows visibility loop that sets element.hidden");
        let body = captures.name("body").map(|m| m.as_str()).unwrap_or("");
        // After SPEC-2008 Phase 24 follow-up, the loop delegates to
        // `applyVisibilityTransition` from `terminal-viewport-reflow.js`.
        // Hidden->visible behaviour is pinned by the linkedom behaviour
        // test; here we only assert the wiring is intact.
        assert!(
            body.contains("applyVisibilityTransition({"),
            "expected the visibility loop to delegate to applyVisibilityTransition \
             so element.hidden mutation + reveal hook share one helper; body: {body}",
        );
        assert!(
            body.contains("visibleWindowData(windowData)"),
            "expected the loop to compute shouldHide from visibleWindowData; body: {body}",
        );
        assert!(
            body.contains("scheduleTerminalFocusActivation(") && body.contains("terminalMap"),
            "expected hidden->visible transition to schedule terminal focus activation \
             (fit + viewport refresh + focus) so scrollback responds without a manual \
             resize (SPEC-2008 FR-051); body: {body}",
        );
        assert!(
            js.contains("function scheduleMaximizedWindowsToViewportSync()")
                && js.contains("let maximizedViewportSyncFrame = null;"),
            "expected maximized viewport sync to be routed through the coalesced \
             frame scheduler (SPEC-1939 Phase 52)",
        );
        assert!(
            !js.contains("requestAnimationFrame(syncMaximizedWindowsToViewport)"),
            "expected renderWorkspace to avoid raw maximized viewport sync rAF fan-out",
        );
    }
}
