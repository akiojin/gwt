//! Embedded web asset manifest (SPEC-3016).
//!
//! Every frontend file shipped inside the gwt binary is declared exactly once
//! in one of two tables:
//!
//! - [`ROOT_JS_MODULE_ASSETS`] — ES modules served at the URL root and
//!   imported by `app.js` (declared via the `root_js_modules!` macro; one
//!   line per asset).
//! - [`STATIC_ASSETS`] — everything else (`/`, `/app.js`, vendor JS/CSS,
//!   stylesheets, fonts) with explicit route, content type, and
//!   cache-control policy.
//!
//! Adding an asset = adding one manifest entry. `embedded_server.rs`
//! registers routes by iterating these tables, so a manifest entry is also
//! the routing source of truth. A missing file fails the build at
//! `include_str!` / `include_bytes!` time instead of 404ing in production.

use axum::{
    http::{header, HeaderValue},
    response::{IntoResponse, Response},
};

const HTML_CONTENT_TYPE: &str = "text/html; charset=utf-8";
const JS_CONTENT_TYPE: &str = "text/javascript; charset=utf-8";
const CSS_CONTENT_TYPE: &str = "text/css; charset=utf-8";
const FONT_CONTENT_TYPE: &str = "font/woff2";

/// First-party sources change on every build; never serve them stale.
const MUTABLE_CACHE_CONTROL: &str = "no-store, max-age=0";
/// Fonts are immutable binaries; let the browser cache them aggressively.
const IMMUTABLE_CACHE_CONTROL: &str = "public, max-age=31536000, immutable";

/// One first-party ES module served at the URL root (`/<file>.js`) and
/// imported by `app.js` (directly or transitively).
#[derive(Clone, Copy)]
pub struct RootJsModuleAsset {
    /// Absolute route the module is served at (`/board-surface.js`).
    pub path: &'static str,
    /// Embedded module source.
    pub source: &'static str,
    /// Export name that must appear in the source; guards against the
    /// manifest pointing at the wrong file.
    pub marker: &'static str,
}

/// Declares [`ROOT_JS_MODULE_ASSETS`]: each `"<file>.js" => "<marker>"` line
/// expands to a [`RootJsModuleAsset`] served at `/<file>.js` with the source
/// embedded from `crates/gwt/web/<file>.js`.
macro_rules! root_js_modules {
    ($($file:literal => $marker:literal,)+) => {
        pub const ROOT_JS_MODULE_ASSETS: &[RootJsModuleAsset] = &[
            $(RootJsModuleAsset {
                path: concat!("/", $file),
                source: include_str!(concat!("../web/", $file)),
                marker: $marker,
            },)+
        ];
    };
}

// Root JS module manifest. Every module imported from "/<name>.js" by any
// shipped module MUST be listed here; otherwise the ES module load 404s at
// runtime and the splash hangs because no boot wiring runs (learned the hard
// way with /release-notes-window.js, see SPEC-2780 and PR #2797 memory).
root_js_modules! {
    "branch-cleanup-modal.js" => "renderBranchCleanupModal",
    // SPEC-2009 Phase 7 (FR-064..FR-067) — Branches detail-check reconnect
    // self-heal / last-known retention / stale-load guard. app.js imports
    // this at module top level, so the asset MUST be registered or the ES
    // module load fails and the splash hangs.
    "branch-list-state.js" => "applyBranchEntriesEvent",
    // SPEC-2013 FR-012: close project tab confirm modal renderer.
    "close-project-tab-confirm-modal.js" => "renderCloseProjectTabConfirmModal",
    // SPEC-2013 2026-06-16 amendment: project switcher popover and
    // Shift+Cmd+Up/Down project tab cycling helpers.
    "project-switcher.js" => "createProjectSwitcherController",
    // SPEC-2013 2026-06-16 amendment: quiet long-running Agent completion
    // notification controller.
    "agent-completion-notifications.js" => "createAgentCompletionNotifier",
    // SPEC-3038 US-3: Close Guard — window close confirm modal renderer.
    "window-close-confirm-modal.js" => "renderWindowCloseConfirmModal",
    "migration-modal.js" => "renderMigrationModal",
    "project-clone-modal.js" => "renderProjectCloneModal",
    "window-docking.js" => "findTitlebarDockTarget",
    "board-surface.js" => "boardEntryMentionsSelf",
    "agent-kanban-surface.js" => "createAgentKanbanSurface",
    "workspace-kanban-surface.js" => "createWorkspaceKanbanSurface",
    "improvement-inbox-surface.js" => "createImprovementInboxSurface",
    "workspace-resume-picker-modal.js" => "createWorkspaceResumePickerController",
    "update-cta.js" => "createUpdateCtaController",
    "terminal-context-menu.js" => "createTerminalContextMenuController",
    "terminal-copy-shortcut.js" => "classifyTerminalCopyKeyEvent",
    "terminal-wheel-scroll.js" => "createTerminalWheelScrollController",
    "canvas-wheel-gesture.js" => "createCanvasWheelGestureClassifier",
    // SPEC-2356 Operator Design System — module assets.
    "theme-manager.js" => "createThemeManager",
    "theme-toggle.js" => "wireThemeToggle",
    "hotkey.js" => "createHotkeyManager",
    "operator-shell.js" => "initOperatorShell",
    "focus-trap.js" => "createFocusTrap",
    // Issue #2698 — stable project tab renderer. Keeps tab DOM keyed by
    // project tab id so status-only workspace refreshes do not rebuild the
    // whole tab strip.
    "project-tabs-renderer.js" => "renderProjectTabs",
    // SPEC-2008 Phase 34 — stable window tab renderer. Keeps grouped-window
    // tab DOM keyed by window id so active-tab switches do not blank/rebuild
    // the tab strip or disturb the terminal body.
    "window-tabs-renderer.js" => "renderWindowTabs",
    // SPEC-1939 Phase 12 / T-IDX-106 — Settings.Index tab renderer.
    "index-settings-panel.js" => "renderIndexSettingsPanel",
    // SPEC-2008 Phase 24 — terminal viewport reflow primitives.
    "terminal-viewport-reflow.js" => "attachHostResizeReflow",
    // SPEC-2008 Phase 25 — revision-aware window geometry sync primitives.
    "window-geometry-sync.js" => "shouldApplyWorkspaceGeometry",
    // Issue #2694 Phase C — kind-coalesced, rAF-flushed WebSocket inbound
    // dispatcher.
    "socket-receive-dispatcher.js" => "createSocketReceiveDispatcher",
    // SPEC-1939 Phase 24 — per-window terminal output batching before xterm
    // write.
    "terminal-output-buffer.js" => "createTerminalOutputBatcher",
    // Issue #2698 PR 1 (B7) — interaction-guard primitive that defers
    // destructive wizard re-renders while the user has a native <select>
    // dropdown open.
    "interaction-guard.js" => "createInteractionGuard",
    // Issue #2704 — terminal-focus guard that lets
    // `scheduleTerminalFocusActivation` skip its xterm `terminal.focus()`
    // step while a modal is open or a text input owns focus.
    "clone-modal-focus-guard.js" => "shouldSkipTerminalFocusActivation",
    // Issue #2698 PR 2 (B1) — viewport-persist throttle that caps the
    // `update_viewport` WebSocket rate during sustained wheel/zoom gestures.
    "viewport-persist-throttle.js" => "createViewportPersistThrottle",
    // Issue #2698 — viewport sync guard. Protects in-flight local pan/zoom
    // from stale workspace_state echoes.
    "viewport-sync.js" => "createViewportSyncState",
    // Issue #2698 follow-up — browser-side metadata trace profiler.
    "ui-trace-profiler.js" => "createUiTraceProfiler",
    "ui-trace-wiring.js" => "createUiTraceWiring",
    // SPEC-1921 T231 — Settings.Custom Agents env editor.
    "custom-agent-env-editor.js" => "renderCustomAgentEnvEditor",
    // SPEC-2780 — release notes window with version selector.
    "release-notes-window.js" => "createReleaseNotesWindow",
    // SPEC-2809 — Console window for external process stdout/stderr live
    // tail (5 fixed kind tabs: gh / git / docker / agent / runner).
    "console-window.js" => "createConsoleWindow",
    // SPEC-2014 2026-05-29 amendment — Launch Agent setting controls.
    "launch-controls.js" => "buildReasoningField",
    // SPEC-2359 W-17 (FR-398) — shared pending state for Resume/Launch
    // requests (double-click guard + deterministic settle on backend ack).
    "launch-pending-controller.js" => "createLaunchPendingController",
    // SPEC-2359 W-17 (FR-399) — full-screen Reconnecting overlay while the
    // WebSocket bridge is down.
    "connection-overlay.js" => "createConnectionOverlay",
    // SPEC-3015 — generated protocol enum contract (wire values serde-derived
    // from the Rust enums; see crates/gwt/src/web_protocol_enums.rs).
    "protocol-enums.js" => "WINDOW_RUNTIME_STATES",
    // SPEC-3015 — window runtime state normalization extracted from app.js.
    "window-runtime-state.js" => "normalizeWindowRuntimeState",
    // SPEC-3064 Phase 3 (E1) — provider usage & rate limits surface
    // (SPEC-2970) extracted from app.js.
    "provider-usage-surface.js" => "createProviderUsageSurface",
    // SPEC-3064 Phase 3 (E2) — terminal attachments & clipboard surface
    // (image paste / file drop / upload progress) extracted from app.js.
    "terminal-attachments.js" => "createTerminalAttachments",
    // SPEC-3064 Phase 3 (E3) — Project Index window surface (index status
    // map + search state/render + Index window mount) extracted from app.js.
    "project-index-search-surface.js" => "createProjectIndexSearchSurface",
    // SPEC-3064 Phase 3 (E4) — Settings windows surface (tabbed Settings
    // body, settings state stores, Teams channel converters, autostart
    // appliers, system-settings interaction guard) extracted from app.js.
    "settings-surface.js" => "createSettingsSurface",
    // SPEC-3064 Phase 3 (E5) — Launch Wizard surface (wizard state +
    // interaction guard, field builders, transitions, renderLaunchWizard,
    // chrome listeners) extracted from app.js.
    "launch-wizard-surface.js" => "createLaunchWizardSurface",
    // SPEC-3064 Phase 3 (E6a) — File Tree window surface (tree state +
    // worktree picker + text/hex viewer + window mount) extracted from
    // app.js.
    "file-tree-surface.js" => "createFileTreeSurface",
    // SPEC-3064 Phase 3 (E6b) — Branches window & branch cleanup surface
    // (branch list state, cleanup modal flow, window mount) extracted from
    // app.js.
    "branches-cleanup-surface.js" => "createBranchesCleanupSurface",
    // SPEC-3064 Phase 3 (E6c) — Board & Logs window surface (board/log
    // state, chat + logs rendering, window mounts) extracted from app.js.
    "board-logs-surface.js" => "createBoardLogsSurface",
    // SPEC-3064 Phase 3 (E6d) — Knowledge Bridge (Kanban) window surface
    // (knowledge state, Kanban rendering + drawer, window mount) extracted
    // from app.js.
    "knowledge-kanban-surface.js" => "createKnowledgeKanbanSurface",
    // SPEC-3064 Phase 3 (E6e) — Profile window surface (profile state +
    // draft editing, window mount) extracted from app.js.
    "profile-window-surface.js" => "createProfileWindowSurface",
    // SPEC-3064 Phase 3 (E7) — Project & workspace shell chrome surface
    // (project tabs + close-tab confirm, recent projects + open-project
    // menu, picker/onboarding, window list dropdown, maximized viewport
    // sync, clone/migration modal glue) extracted from app.js.
    "project-shell-surface.js" => "createProjectShellSurface",
    // SPEC-2008 camera-focus rework — Fleet Minimap overview. Lives in
    // `#fleet-minimap` (canvas-area, outside the stage) and renders the window
    // cell map + camera frame. app.js imports this at module top level, so the
    // asset MUST be registered or the ES module load 404s and the splash hangs.
    "fleet-minimap.js" => "createFleetMinimap",
    // SPEC-3038 (2026-06-20) — Command Rail Windows popover model: groups the
    // cross-tab open-window set by owning project tab so the list matches the
    // badge and supports cross-tab focus.
    "window-list-model.js" => "groupProjectWindowList",
}

/// Embedded payload of a [`StaticAsset`].
#[derive(Clone, Copy)]
pub enum AssetBody {
    /// UTF-8 text embedded via `include_str!`.
    Text(&'static str),
    /// Binary content embedded via `include_bytes!` (fonts).
    Bytes(&'static [u8]),
}

/// One embedded asset served at a fixed route with a fixed content type and
/// cache policy. Routes are registered by iterating [`static_assets`].
#[derive(Clone, Copy)]
pub struct StaticAsset {
    /// Absolute route the asset is served at.
    pub route: &'static str,
    /// Value of the `Content-Type` response header.
    pub content_type: &'static str,
    /// Value of the `Cache-Control` response header; `None` omits the header
    /// (axum default behavior, used for the pinned vendor assets).
    pub cache_control: Option<&'static str>,
    /// Embedded payload.
    pub body: AssetBody,
}

/// Non-root-module asset manifest: entrypoints, vendor JS/CSS, stylesheets,
/// and fonts. One entry per served route.
pub const STATIC_ASSETS: &[StaticAsset] = &[
    StaticAsset {
        route: "/",
        content_type: HTML_CONTENT_TYPE,
        cache_control: Some(MUTABLE_CACHE_CONTROL),
        body: AssetBody::Text(include_str!("../web/index.html")),
    },
    StaticAsset {
        route: "/app.js",
        content_type: JS_CONTENT_TYPE,
        cache_control: Some(MUTABLE_CACHE_CONTROL),
        body: AssetBody::Text(include_str!("../web/app.js")),
    },
    // Vendored xterm.js — pinned versions bundled so the terminal works
    // offline without CDN reach.
    StaticAsset {
        route: "/assets/xterm/xterm.mjs",
        content_type: JS_CONTENT_TYPE,
        cache_control: None,
        body: AssetBody::Text(include_str!("../web/vendor/xterm/xterm.mjs")),
    },
    StaticAsset {
        route: "/assets/xterm/addon-fit.mjs",
        content_type: JS_CONTENT_TYPE,
        cache_control: None,
        body: AssetBody::Text(include_str!("../web/vendor/xterm/addon-fit.mjs")),
    },
    StaticAsset {
        route: "/assets/xterm/xterm.css",
        content_type: CSS_CONTENT_TYPE,
        cache_control: None,
        body: AssetBody::Text(include_str!("../web/vendor/xterm/xterm.css")),
    },
    // SPEC-2009 Phase 2b — highlight.js ES module + a dark GitHub-style
    // theme for the File Tree text viewer. Bundled into the gwt binary so
    // the viewer works offline without CDN reach.
    StaticAsset {
        route: "/assets/highlight/highlight.min.js",
        content_type: JS_CONTENT_TYPE,
        cache_control: None,
        body: AssetBody::Text(include_str!("../web/vendor/highlight/highlight.min.js")),
    },
    StaticAsset {
        route: "/assets/highlight/github-dark.min.css",
        content_type: CSS_CONTENT_TYPE,
        cache_control: None,
        body: AssetBody::Text(include_str!("../web/vendor/highlight/github-dark.min.css")),
    },
    // SPEC-2356 Operator Design System — stylesheets.
    StaticAsset {
        route: "/styles/tokens.css",
        content_type: CSS_CONTENT_TYPE,
        cache_control: Some(MUTABLE_CACHE_CONTROL),
        body: AssetBody::Text(include_str!("../web/styles/tokens.css")),
    },
    StaticAsset {
        route: "/styles/typography.css",
        content_type: CSS_CONTENT_TYPE,
        cache_control: Some(MUTABLE_CACHE_CONTROL),
        body: AssetBody::Text(include_str!("../web/styles/typography.css")),
    },
    StaticAsset {
        route: "/styles/components.css",
        content_type: CSS_CONTENT_TYPE,
        cache_control: Some(MUTABLE_CACHE_CONTROL),
        body: AssetBody::Text(include_str!("../web/styles/components.css")),
    },
    // Issue #2694 Phase D — extracted from the formerly-inline index.html
    // <style> block (~91KB) so initial HTML parse stays fast.
    StaticAsset {
        route: "/styles/app.css",
        content_type: CSS_CONTENT_TYPE,
        cache_control: Some(MUTABLE_CACHE_CONTROL),
        body: AssetBody::Text(include_str!("../web/styles/app.css")),
    },
    // SPEC-2356 Operator Design System — fonts (binary, immutable).
    StaticAsset {
        route: "/assets/fonts/MonaSans.woff2",
        content_type: FONT_CONTENT_TYPE,
        cache_control: Some(IMMUTABLE_CACHE_CONTROL),
        body: AssetBody::Bytes(include_bytes!("../web/fonts/MonaSans.woff2")),
    },
    StaticAsset {
        route: "/assets/fonts/HubotSans-Regular.woff2",
        content_type: FONT_CONTENT_TYPE,
        cache_control: Some(IMMUTABLE_CACHE_CONTROL),
        body: AssetBody::Bytes(include_bytes!("../web/fonts/HubotSans-Regular.woff2")),
    },
    StaticAsset {
        route: "/assets/fonts/HubotSans-Bold.woff2",
        content_type: FONT_CONTENT_TYPE,
        cache_control: Some(IMMUTABLE_CACHE_CONTROL),
        body: AssetBody::Bytes(include_bytes!("../web/fonts/HubotSans-Bold.woff2")),
    },
    StaticAsset {
        route: "/assets/fonts/HubotSansCondensed-Bold.woff2",
        content_type: FONT_CONTENT_TYPE,
        cache_control: Some(IMMUTABLE_CACHE_CONTROL),
        body: AssetBody::Bytes(include_bytes!("../web/fonts/HubotSansCondensed-Bold.woff2")),
    },
    StaticAsset {
        route: "/assets/fonts/JetBrainsMono.woff2",
        content_type: FONT_CONTENT_TYPE,
        cache_control: Some(IMMUTABLE_CACHE_CONTROL),
        body: AssetBody::Bytes(include_bytes!("../web/fonts/JetBrainsMono.woff2")),
    },
];

/// All non-root-module assets; `embedded_server` registers one route per
/// entry.
pub fn static_assets() -> &'static [StaticAsset] {
    STATIC_ASSETS
}

/// All root JS module assets; `embedded_server` registers one route per
/// entry.
pub fn root_js_module_assets() -> &'static [RootJsModuleAsset] {
    ROOT_JS_MODULE_ASSETS
}

#[cfg(test)]
fn find_static_asset(route: &str) -> &'static StaticAsset {
    STATIC_ASSETS
        .iter()
        .find(|asset| asset.route == route)
        .unwrap_or_else(|| panic!("embedded asset manifest has no entry for {route}"))
}

/// Text content of the manifest entry served at `route`. Panics when the
/// route is missing or binary; both indicate a manifest bug caught by tests.
#[cfg(test)]
pub(crate) fn static_asset_text(route: &str) -> &'static str {
    match find_static_asset(route).body {
        AssetBody::Text(text) => text,
        AssetBody::Bytes(_) => panic!("embedded asset {route} is binary, not text"),
    }
}

/// Embedded `index.html` (served at `/`). Test-only accessor; serving goes
/// through [`static_asset_response`].
#[cfg(test)]
pub fn index_html() -> &'static str {
    static_asset_text("/")
}

/// Embedded frontend entrypoint module (served at `/app.js`). Test-only
/// accessor; serving goes through [`static_asset_response`].
#[cfg(test)]
pub fn app_js() -> &'static str {
    static_asset_text("/app.js")
}

/// Builds the response for one [`StaticAsset`] manifest entry.
pub fn static_asset_response(asset: &'static StaticAsset) -> Response {
    let mut response = match asset.body {
        AssetBody::Text(text) => text.into_response(),
        AssetBody::Bytes(bytes) => bytes.into_response(),
    };
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(asset.content_type),
    );
    if let Some(cache_control) = asset.cache_control {
        headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static(cache_control),
        );
    }
    response
}

/// Builds the response for one [`RootJsModuleAsset`] manifest entry
/// (JavaScript content type + mutable cache policy).
pub fn root_js_module_response(asset: RootJsModuleAsset) -> impl IntoResponse {
    debug_assert!(asset.source.contains(asset.marker));
    (
        [
            (header::CONTENT_TYPE, JS_CONTENT_TYPE),
            (header::CACHE_CONTROL, MUTABLE_CACHE_CONTROL),
        ],
        asset.source,
    )
}

#[cfg(test)]
#[path = "embedded_web_tests.rs"]
mod tests;
