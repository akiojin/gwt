import { test } from "node:test";
import assert from "node:assert/strict";
import { existsSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";

const here = dirname(fileURLToPath(import.meta.url));
const indexPath = resolve(here, "../index.html");
const html = readFileSync(indexPath, "utf8");
const { document } = parseHTML(html);
const operatorShellSource = readFileSync(resolve(here, "../operator-shell.js"), "utf8");
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
const projectTabsRendererSource = readFileSync(
  resolve(here, "../project-tabs-renderer.js"),
  "utf8",
);
const branchCleanupSource = readFileSync(resolve(here, "../branch-cleanup-modal.js"), "utf8");
const windowDockingSource = readFileSync(resolve(here, "../window-docking.js"), "utf8");
const workspaceKanbanPath = resolve(here, "../workspace-kanban-surface.js");
const workspaceKanbanSource = existsSync(workspaceKanbanPath)
  ? readFileSync(workspaceKanbanPath, "utf8")
  : "";
const workspaceKanbanCombinedSource = `${appSource}\n${workspaceKanbanSource}`;
const appAndProjectTabsSource = `${appSource}\n${projectTabsRendererSource}`;
const typographySource = readFileSync(resolve(here, "../styles/typography.css"), "utf8");
// Issue #2694 Phase D: the formerly-inline <style> block now lives at
// /styles/app.css and is loaded via `<link rel="stylesheet">`. The grep
// surface used by the CSS contract tests below remains stable.
const inlineStyle = readFileSync(resolve(here, "../styles/app.css"), "utf8");

function cssRemVar(source, name) {
  const match = source.match(new RegExp(`${name}:\\s*([0-9.]+)rem\\s*;`));
  assert.ok(match, `missing typography token: ${name}`);
  return Number(match[1]);
}

function terminalOptionNumber(name) {
  const runtime = appSource.match(/new\s+Terminal\(\{\s*([\s\S]*?)\s*\}\);/);
  assert.ok(runtime, "expected xterm Terminal constructor options");
  const match = runtime[1].match(new RegExp(`${name}:\\s*([0-9.]+)`));
  assert.ok(match, `missing terminal option: ${name}`);
  return Number(match[1]);
}

// SPEC-2356 — extract every @media block matching the supplied condition,
// using brace-depth tracking so nested rules don't truncate the body.
function extractMediaBlocks(css, condition) {
  const out = [];
  const marker = `@media`;
  let cursor = 0;
  while (true) {
    const at = css.indexOf(marker, cursor);
    if (at < 0) break;
    const headerEnd = css.indexOf("{", at);
    if (headerEnd < 0) break;
    const header = css.slice(at, headerEnd);
    if (!header.includes(condition)) {
      cursor = headerEnd + 1;
      continue;
    }
    // Walk forward from headerEnd, tracking brace depth.
    let depth = 1;
    let i = headerEnd + 1;
    while (i < css.length && depth > 0) {
      const ch = css[i];
      if (ch === "{") depth += 1;
      else if (ch === "}") depth -= 1;
      i += 1;
    }
    out.push(css.slice(headerEnd + 1, i - 1));
    cursor = i;
  }
  return out.join("\n");
}

test("index.html declares Operator chrome scaffold", () => {
  for (const sel of [
    "#op-theme-toggle",
    ".op-sidebar",
    ".op-status-strip",
    "#op-strip-clock",
    "#op-strip-active",
    "#op-strip-idle",
    "#op-strip-blocked",
    "#op-briefing",
    "#op-palette-backdrop",
    "#op-palette-input",
    "#op-palette-list",
    "#op-hotkey-overlay",
    "#op-palette-button",
  ]) {
    assert.ok(document.querySelector(sel), `missing chrome element: ${sel}`);
  }
});

test("project bar exposes a Layers column with three layers", () => {
  const layers = document.querySelectorAll(".op-sidebar__section .op-layer[data-layer]");
  assert.equal(layers.length, 3, "expected three sidebar layers");
  const labels = Array.from(layers).map((el) => el.dataset.layer);
  assert.deepEqual(labels.sort(), ["agents", "git", "hooks"]);
});

test("project bar and command palette expose Start Work outside the Branches surface", () => {
  const projectBarAction = document.querySelector('.op-sidebar .op-layer[data-cmd="start-work"]');
  assert.ok(projectBarAction, "expected Project Bar to expose a Start Work action");
  assert.match(projectBarAction.textContent, /Start Work/);
  assert.match(
    operatorShellSource,
    /id:\s*"start-work"[\s\S]+label:\s*"Start Work"/,
    "expected Command Palette registry to include Start Work",
  );
  assert.match(
    appSource,
    /case\s+"start-work":[\s\S]+kind:\s*"open_start_work"/,
    "expected Start Work command to send the global open_start_work event",
  );
});

test("frontend handles active work projection as status-strip telemetry", () => {
  assert.match(
    appSource,
    /case\s+"active_work_projection":[\s\S]+activeWorkProjection\s*=\s*event\.projection/,
    "expected frontend to store active_work_projection separately from canvas workspace_state",
  );
  assert.match(
    appSource,
    /activeWorkProjection[\s\S]+applyTelemetryCounts/,
    "expected active work projection to feed Operator telemetry",
  );
});

test("Sidebar Layers Agents counter filters non-agent preset windows", () => {
  // SPEC-2356 follow-up: recomputeOperatorTelemetry walked windowMap.values()
  // without checking preset, so every workspace window with data-agent-state
  // (Board / Workspace / Logs / Branches / etc.) inflated counts.agents and
  // the Sidebar Layers "Agents" row showed e.g. 4 when only 2 agent panes
  // were live. The DOM walk must scope to presets that represent agent panes.
  const fnMatch = appSource.match(
    /function\s+recomputeOperatorTelemetry[\s\S]*?(?=\n\s+function\s+\w)/,
  );
  assert.ok(
    fnMatch,
    "expected recomputeOperatorTelemetry definition in app.js",
  );
  const body = fnMatch[0];
  assert.match(
    body,
    /windowMap\.entries\(\)/,
    "recomputeOperatorTelemetry must iterate windowMap.entries() so it can resolve each window's preset before counting it as an agent",
  );
  assert.match(
    body,
    /presetSupportsWaitingStatus/,
    "recomputeOperatorTelemetry must filter via presetSupportsWaitingStatus(preset) so non-agent windows do not inflate the Sidebar Agents counter",
  );
});

test("Workspace sidebar exposes active work and per-agent overview", () => {
  assert.ok(
    document.querySelector("#op-active-work"),
    "expected Workspace shell to expose an active work overview region",
  );
  assert.ok(
    document.querySelector("#op-active-work-agents"),
    "expected Workspace shell to expose a per-agent list region",
  );
  assert.match(
    appSource,
    /function\s+renderActiveWorkOverview\(\)[\s\S]+activeWorkFocusableAgents\(activeWorkProjection\)/,
    "expected frontend to render focusable per-agent projection data, not only aggregate counters",
  );
  assert.match(
    appSource,
    /op-agent-card[\s\S]+last_board_entry_id/,
    "expected agent cards to preserve board linkage for handoff/debugging",
  );
});

test("Workspace sidebar keeps Quick before the expanding active work list", () => {
  const sections = Array.from(document.querySelectorAll(".op-sidebar > .op-sidebar__section"));
  const headings = sections.map((section) =>
    section.querySelector(".op-sidebar__heading span")?.textContent?.trim(),
  );
  assert.deepEqual(
    headings.slice(0, 3),
    ["Layers", "Quick", "Active Work"],
    "Quick must stay above Active Work so agent cards do not push it off-screen",
  );
});

test("workspace windows expose role badges and hide panel runtime chips", () => {
  assert.match(
    appSource,
    /function\s+presetRoleLabel\(preset\)/,
    "expected a shared preset role label helper",
  );
  assert.match(
    appSource,
    /class="window-role-badge"/,
    "expected titlebar template to include a role badge surface",
  );
  assert.match(
    appSource,
    /function\s+shouldShowRuntimeStatus\(windowData\)/,
    "expected runtime chip visibility to be centralized",
  );
  assert.match(
    appSource,
    /runtimeChip\.hidden\s*=\s*!shouldShowRuntimeStatus\(windowData\)/,
    "expected non-terminal panels to hide the runtime status chip",
  );
  assert.match(
    inlineStyle,
    /\.status-chip\[hidden\]\s*\{[\s\S]*display:\s*none/,
    "hidden runtime chips must stay hidden even though .status-chip uses inline-flex",
  );
  assert.match(
    appSource,
    /window-list-role/,
    "expected window list rows to include a role badge",
  );
});

test("Memo is not exposed as a workspace window feature", () => {
  assert.equal(
    document.querySelector('.preset-button[data-preset="memo"]'),
    null,
    "Add window must not expose the unusable Memo surface",
  );
  assert.doesNotMatch(
    appSource,
    /memoSurface|memo-root|load_memo|create_memo_note|update_memo_note|delete_memo_note/,
    "Memo frontend state, renderer, and protocol events should be removed",
  );
  assert.doesNotMatch(
    inlineStyle,
    /surface-memo|memo-layout|memo-note|memo-editor|memo-status/,
    "Memo-specific CSS should be removed with the surface",
  );
});

test("Agent window titles are truncated with long focus detail kept as tooltip", () => {
  assert.match(
    appSource,
    /function\s+windowTitleTooltip\(windowData\)/,
    "expected a shared helper for long title detail tooltip",
  );
  assert.match(
    appSource,
    /titleText\.title\s*=\s*windowTitleTooltip\(windowData\)/,
    "expected titlebar to keep long focus detail in a tooltip",
  );
  assert.match(
    inlineStyle,
    /\.title\s*\{[\s\S]*flex:\s*1[\s\S]*min-width:\s*0[\s\S]*overflow:\s*hidden/,
    "title row must shrink instead of pushing window controls",
  );
  assert.match(
    inlineStyle,
    /\.title-text\s*\{[\s\S]*overflow:\s*hidden[\s\S]*text-overflow:\s*ellipsis[\s\S]*white-space:\s*nowrap/,
    "title text must use one-line ellipsis",
  );
  assert.match(
    inlineStyle,
    /\.window-actions\s*\{[\s\S]*flex-shrink:\s*0/,
    "window controls must not shrink or overlap with long titles",
  );
});

test("canvas world grid is synchronized from viewport state", () => {
  assert.ok(
    document.querySelector("#canvas-world-grid.canvas-world-grid"),
    "expected canvas to expose a dedicated world-space grid layer",
  );
  assert.match(
    appSource,
    /const\s+worldGrid\s*=\s*document\.getElementById\("canvas-world-grid"\)/,
    "expected app.js to bind the canvas world grid element",
  );
  assert.match(
    appSource,
    /function\s+applyWorldGridViewport\(\)[\s\S]+viewport\.x[\s\S]+viewport\.y[\s\S]+viewport\.zoom/,
    "expected world grid sync to derive position and size from the viewport",
  );
  assert.match(
    appSource,
    /function\s+applyViewport\(\)[\s\S]+applyWorldGridViewport\(\)/,
    "expected applyViewport to update the grid together with the stage transform",
  );
});

test("Workspace active work overview behaves like a command center", () => {
  assert.match(
    appSource,
    /Add Agent to This Work/,
    "expected Workspace-origin launch copy to make same-work agent addition explicit",
  );
  assert.match(
    appSource,
    /last_board_entry_kind/,
    "expected agent cards to expose the latest coordination milestone kind",
  );
  assert.match(
    appSource,
    /coordination_scope/,
    "expected agent cards to expose owner/topic scope for each agent",
  );
  assert.match(
    appSource,
    /function\s+focusBoardEntry\(/,
    "expected Board actions to deep-link to the referenced coordination entry",
  );
  assert.match(
    appSource,
    /data-board-entry-id/,
    "expected Board timeline entries to be addressable from Workspace links",
  );
});

test("Active Work title prefers concrete work context over Start Work workflow label", () => {
  assert.match(
    appSource,
    /function\s+activeWorkDisplayTitle\(projection,\s*agents\)[\s\S]+agent\.title_summary[\s\S]+projection\?\.summary[\s\S]+projection\?\.owner[\s\S]+projection\?\.title[\s\S]+Start Work[\s\S]+Active Work/,
    "expected Active Work title resolution to prefer Agent/Workspace work context and treat Start Work as a fallback-only workflow label",
  );
  assert.match(
    appSource,
    /createNode\("div",\s*"op-work-title",\s*activeWorkDisplayTitle\(activeWorkProjection,\s*agents\)\)/,
    "expected Active Work summary title to use the display-title helper",
  );
  assert.doesNotMatch(
    appSource,
    /createNode\("div",\s*"op-work-title",\s*activeWorkProjection\.title\s*\|\|\s*"Active Work"\)/,
    "Active Work must not render the saved Start Work workflow title directly",
  );
});

test("Active Work sidebar only renders while live Agent windows are focusable", () => {
  assert.match(
    appSource,
    /function\s+activeWorkFocusableAgents\(projection\)[\s\S]+workspaceWindowById\(agent\.window_id\)/,
    "expected Active Work cards to be filtered against live workspace windows",
  );
  assert.match(
    appSource,
    /const\s+activeWorkSection\s*=\s*document\.getElementById\("op-active-work"\)/,
    "expected Active Work visibility to be controlled at the section level",
  );
  assert.match(
    appSource,
    /function\s+setActiveWorkSectionVisible\(visible\)[\s\S]+activeWorkSection\.hidden\s*=\s*!visible/,
    "expected no-Agent Active Work state to hide the entire section instead of leaving stale work UI",
  );
  assert.match(
    appSource,
    /if\s*\(agentCount\s*===\s*0\)\s*\{[\s\S]+setActiveWorkSectionVisible\(false\)[\s\S]+return;/,
    "expected no focusable Agent windows to remove the Active Work sidebar section",
  );
  assert.match(
    appSource,
    /function\s+focusActiveWorkAgentWindow\(agent\)[\s\S]+restore_window[\s\S]+focusWindowRemotely\(agent\.window_id,\s*\{\s*center:\s*true\s*\}\)/,
    "expected Focus to restore minimized Agent windows before focusing them",
  );
  assert.match(
    appSource,
    /function\s+agentStatusLabel\(state\)[\s\S]+Running[\s\S]+Blocked[\s\S]+Idle[\s\S]+Done/,
    "expected raw active/blocked/idle/done status values to be mapped to user-facing labels",
  );
  assert.match(
    appSource,
    /function\s+agentRuntimeStatusLabel\(agent\)[\s\S]+runtimeStateForWindow\(windowData\)[\s\S]+windowRuntimeLabel\(runtimeState\)[\s\S]+agentStatusLabel\(agent\.status_category\)/,
    "expected Active Work agent cards to derive their visible runtime label from the live window state before falling back to workspace category",
  );
  assert.match(
    appSource,
    /createNode\("div",\s*"op-agent-state",\s*agentRuntimeStatusLabel\(agent\)\)/,
    "Active Work must render Waiting from WindowState even when workspace status_category remains active",
  );
  assert.doesNotMatch(
    appSource,
    /createNode\("div",\s*"op-agent-state",\s*state\)/,
    "Active Work must not render raw status wire values such as ACTIVE",
  );
  assert.match(
    appSource,
    /function\s+renderAppState\(nextState\)[\s\S]+renderActiveWorkOverview\(\)/,
    "expected workspace changes to re-evaluate whether Active Work agents are still focusable",
  );
});

test("Launch Wizard live sessions render window runtime status", () => {
  assert.match(
    appSource,
    /function\s+liveSessionStatusLabel\(session\)[\s\S]+session\.runtime_status[\s\S]+windowRuntimeLabel\(runtimeState\)[\s\S]+window/,
    "expected live-session rows to label Running/Waiting/Error from runtime_status",
  );
  assert.match(
    appSource,
    /createNode\(\s*"div",\s*"live-session-status",\s*liveSessionStatusLabel\(session\),?\s*\)/,
    "expected Launch Wizard live-session status copy to use runtime_status instead of active boolean copy",
  );
});

test("Workspace Overview is separate from live-only Active Work", () => {
  assert.ok(
    document.querySelector("#op-workspace-overview-entry"),
    "expected Sidebar to expose a Workspace Overview entry even when Active Work is hidden",
  );
  assert.ok(
    !document.querySelector("#project-workspace-overview-button"),
    "Workspace Overview is per-project content and must live in the Sidebar, not the global Project Bar",
  );
  assert.match(
    appSource,
    /function\s+openWorkspaceOverview\(/,
    "expected a shared opener for the Sidebar entry",
  );
  assert.match(
    appSource,
    /function\s+openWorkspaceOverview\(\)\s*\{[\s\S]{0,300}?focusOrSpawnPreset\("workspace"\)/,
    "expected Workspace Overview to open the Workspace Kanban window instead of a drawer",
  );
  assert.match(
    appSource,
    /op-workspace-overview-entry[\s\S]+openWorkspaceOverview/,
    "expected Sidebar Workspace Overview entry to open the overview",
  );
});

test("Workspace Overview uses the shared full-window Kanban + detail layout", () => {
  assert.ok(
    workspaceKanbanSource.length > 0,
    "expected Workspace Kanban renderer to live in workspace-kanban-surface.js",
  );
  assert.match(
    appSource,
    /from\s+"\/workspace-kanban-surface\.js"/,
    "expected app.js to import the Workspace Kanban surface module",
  );
  assert.match(
    appSource,
    /presetSurface\(preset\)[\s\S]+preset\s*===\s*"workspace"[\s\S]+return\s+"workspace"/,
    "expected Workspace to be a first-class window surface",
  );
  assert.match(
    workspaceKanbanCombinedSource,
    /workspace-kanban-root[\s\S]+workspace-split[\s\S]+kanban-shell/,
    "expected Workspace Overview to share the split Kanban shell",
  );
  for (const column of ["Active", "Inactive", "Completed"]) {
    assert.match(
      workspaceKanbanCombinedSource,
      new RegExp(`workspace-column-name">${column}`),
      `expected Workspace Kanban to include ${column} column`,
    );
  }
  assert.match(
    workspaceKanbanCombinedSource,
    /workspace-kanban-detail-pane/,
    "expected Workspace Kanban to keep selected Workspace detail on the right",
  );
  assert.match(
    workspaceKanbanSource,
    /function\s+workspaceCardsFromProjection\([^)]*\)[\s\S]+journal_entries/,
    "expected Workspace Kanban to render Workspace journal entries from active_work_projection",
  );
  // SPEC-2359 US-42 — Resume action now opens the Workspace Resume
  // Picker via `list_resumable_agents` instead of the legacy
  // `resume_workspace` event. The picker drives the actual restart with
  // `resume_workspace_agent` after the user selects a candidate agent.
  assert.match(
    workspaceKanbanSource,
    /function\s+resumeWorkspaceCard\([^)]*\)[\s\S]+list_resumable_agents/,
    "expected Workspace card Resume action to ask the backend to list resumable agents",
  );
  assert.match(
    workspaceKanbanSource,
    /kind:\s*"journal"[\s\S]+branch:\s*""[\s\S]+worktree_path:\s*""/,
    "expected suspended journal cards not to borrow the current Workspace branch/worktree",
  );
  assert.doesNotMatch(
    appSource,
    /function\s+(workspaceCardsFromProjection|renderWorkspaceKanbanCard|renderWorkspaceKanbanDetail)\(/,
    "Workspace Kanban rendering internals should not remain in app.js",
  );
});

test("Workspace Overview Kanban root participates in full-window layout", () => {
  const fullWindowRootRule = inlineStyle.match(
    /\.file-tree-root,[\s\S]*?\.mock-root\s*\{[^}]+\}/,
  );
  assert.ok(fullWindowRootRule, "expected shared full-window root layout rule");
  assert.match(
    fullWindowRootRule[0],
    /\.workspace-kanban-root/,
    "Workspace Kanban root must fill the window body so column bodies can scroll",
  );
  assert.match(fullWindowRootRule[0], /position:\s*absolute/);
  assert.match(fullWindowRootRule[0], /display:\s*flex/);
  assert.match(fullWindowRootRule[0], /flex-direction:\s*column/);
});

test("Workspace Overview legacy drawer scaffold is retired", () => {
  assert.doesNotMatch(
    html,
    /workspace-overview-drawer/,
    "Workspace Overview should no longer ship an unused drawer scaffold",
  );
  assert.doesNotMatch(
    appSource,
    /closeWorkspaceOverview|renderWorkspaceOverview|workspaceOverviewFocusTrapRelease/,
    "Workspace Overview drawer lifecycle code should not remain in app.js",
  );
});

test("non-agent surface presets open maximized from command focus paths", () => {
  assert.match(
    appSource,
    /function\s+isAutoMaximizedSurfacePreset\([^)]*\)[\s\S]+file_tree[\s\S]+branches[\s\S]+settings[\s\S]+profile[\s\S]+logs[\s\S]+issue[\s\S]+spec[\s\S]+workspace[\s\S]+board[\s\S]+pr/,
    "expected frontend focus/spawn path to identify every non-agent surface preset as auto-maximized",
  );
  assert.match(
    appSource,
    /focusOrSpawnPreset\(preset\)[\s\S]+isAutoMaximizedSurfacePreset\(preset\)[\s\S]+bounds:\s*visibleBounds\(\)/,
    "expected focusOrSpawnPreset to send viewport bounds when focusing existing non-agent surfaces",
  );
});

test("Active Work and Workspace Overview render PR metadata as links", () => {
  assert.match(
    appSource,
    /function\s+createWorkspacePrMeta\(/,
    "expected a shared PR metadata renderer instead of duplicating string-only PR labels",
  );
  assert.match(
    workspaceKanbanSource,
    /createWorkspacePrMeta\?\.\(cardData\)/,
    "expected Workspace Overview to render the saved PR link/state from the projection",
  );
  assert.match(
    appSource,
    /createWorkspacePrMeta\(activeWorkProjection\)/,
    "expected Active Work sidebar to render the live PR link/state from the projection",
  );
  assert.match(
    appSource,
    /pr_url[\s\S]+href[\s\S]+PR #/,
    "expected PR metadata to use the backend-provided URL for the link target",
  );
  assert.match(
    appSource,
    /pr_state[\s\S]+appendMeta/,
    "expected PR state to be displayed next to the PR link",
  );
});

test("Workspace Overview exposes user-confirmed cleanup for completed workspaces", () => {
  assert.match(
    appSource,
    /cleanup_candidate/,
    "expected active work projection cleanup_candidate to drive Workspace cleanup",
  );
  assert.match(
    appSource,
    /function\s+openWorkspaceCleanup\(/,
    "expected Workspace Overview to open a cleanup confirmation instead of deleting automatically",
  );
  assert.match(
    appSource,
    /default_delete_remote[\s\S]+deleteRemote\s*:\s*false|deleteRemote\s*:\s*false[\s\S]+default_delete_remote/,
    "expected Workspace cleanup to default remote deletion off",
  );
  assert.match(
    `${appSource}\n${branchCleanupSource}`,
    /Also delete matching remote branches/,
    "expected remote deletion to remain an explicit opt-in in the confirmation UI",
  );
});

test("Agent window chrome resolves dynamic purpose titles before legacy titles", () => {
  assert.match(
    appSource,
    /function\s+windowDisplayTitle\(windowData\)[\s\S]+dynamic_title[\s\S]+purpose_title[\s\S]+title/,
    "expected a shared display-title helper with dynamic > purpose > legacy title precedence",
  );
  assert.match(
    appSource,
    /window-list-title">\$\{escapeHtml\(windowDisplayTitle\(entry\)\)\}/,
    "expected the Windows dropdown to escape the shared display-title helper output",
  );
  assert.match(
    appSource,
    /title-text"\)\.textContent\s*=\s*windowDisplayTitle\(windowData\)/,
    "expected window titlebars to use the shared display-title helper",
  );
});

test("Branches remains a branch browser, not a planning workspace", () => {
  const branchPreset = document.querySelector('.preset-button[data-preset="branches"]');
  assert.ok(branchPreset, "expected Branches preset to remain available");
  assert.match(branchPreset.textContent, /Browse repository branches and launch agents/);
  assert.doesNotMatch(
    `${html}\n${appSource}`,
    /Planning Session|Workspace card/i,
    "Branches should not render Planning Session or Workspace-card concepts",
  );
});

test("Branches loading state becomes recoverable when the WebSocket disconnects", () => {
  assert.match(
    appSource,
    /function\s+failLoadingBranchesOnConnectionLoss\(windowId,\s*state\)/,
    "expected a dedicated Branches loading connection-loss helper",
  );
  assert.match(
    appSource,
    /failLoadingBranchesOnConnectionLoss[\s\S]+state\.loading\s*=\s*false[\s\S]+state\.receivedFreshEntries\s*=\s*false/,
    "expected connection loss to clear stale Branches loading flags",
  );
  assert.match(
    appSource,
    /Connection lost while loading branches/,
    "expected initial branch inventory loss to surface a retryable error",
  );
  assert.match(
    appSource,
    /function\s+setConnectionState\(connected\)[\s\S]+failLoadingBranchesOnConnectionLoss\(windowId,\s*state\)[\s\S]+renderBranches\(windowId\)/,
    "expected socket disconnect to re-render Branches after clearing stale loading",
  );
});

test("hotkey overlay lists ⌘P/⌘B/⌘G/⌘L/⌘?/Esc (sidebar toggle hotkey is removed in Phase 9)", () => {
  const overlay = document.getElementById("op-hotkey-overlay");
  assert.ok(overlay, "hotkey overlay missing");
  const text = overlay.textContent.replace(/\s+/g, " ");
  for (const phrase of ["⌘ P", "⌘ B", "⌘ G", "⌘ L", "⌘ K", "⌘ ?", "Esc"]) {
    assert.ok(text.includes(phrase), `expected ${phrase} in hotkey overlay`);
  }
  // SPEC-2356 Phase 9 (FR-021/FR-032): the Layout group + "Toggle sidebar / ⌘\\"
  // row are removed in favor of the hover-reveal peek 帯.
  assert.ok(!text.includes("⌘ \\"), "Cmd+\\\\ must not appear in the hotkey overlay");
  assert.ok(
    !text.toLowerCase().includes("toggle sidebar"),
    "Toggle sidebar row must not appear in the hotkey overlay",
  );
  const groups = Array.from(overlay.querySelectorAll(".op-hotkey-card__group-title")).map((el) =>
    el.textContent?.trim().toLowerCase(),
  );
  for (const expected of ["navigation", "help"]) {
    assert.ok(groups.includes(expected), `expected hotkey overlay group "${expected}", got ${groups.join("/")}`);
  }
  assert.ok(!groups.includes("layout"), "Layout group is removed in Phase 9");
});

test("head loads tokens, typography, components, and Operator modules", () => {
  const css = Array.from(document.querySelectorAll("link[rel=stylesheet]")).map((l) => l.href);
  for (const required of [
    "/styles/tokens.css",
    "/styles/typography.css",
    "/styles/components.css",
    "/assets/xterm/xterm.css",
  ]) {
    assert.ok(css.some((href) => href.endsWith(required)), `expected stylesheet: ${required}`);
  }
  const inlineScripts = Array.from(document.querySelectorAll("head script:not([src])")).map((s) => s.textContent);
  assert.ok(
    inlineScripts.some((src) => src.includes("data-theme") && src.includes("prefers-color-scheme")),
    "expected FOUC-prevention bootstrap script in <head>",
  );
});

test("body markup wires Mission Briefing reveal lines (US-1 AS-1)", () => {
  const lines = document.querySelectorAll(".op-briefing__line");
  assert.ok(lines.length >= 4, `expected >=4 briefing lines, got ${lines.length}`);
  const online = document.querySelector(".op-briefing__online");
  assert.ok(online, "expected OPERATOR ONLINE marker");
  assert.match(online.textContent, /OPERATOR ONLINE/);
});

test("font preload hints exist for Mona/Hubot/JetBrains", () => {
  const preloads = Array.from(document.querySelectorAll("link[rel=preload][as=font]")).map((l) => l.href);
  for (const expected of ["MonaSans.woff2", "HubotSans-Bold.woff2", "JetBrainsMono.woff2"]) {
    assert.ok(preloads.some((h) => h.endsWith(`/assets/fonts/${expected}`)), `missing preload: ${expected}`);
  }
});

test("developer readability typography keeps working text above minimum sizes", () => {
  assert.ok(cssRemVar(typographySource, "--type-xs") >= 0.75, "--type-xs must be at least 12px");
  assert.ok(cssRemVar(typographySource, "--type-sm") >= 0.875, "--type-sm must be at least 14px");
  assert.match(
    typographySource,
    /\.t-mono\s*\{[\s\S]*?line-height:\s*1\.(?:4|[5-9])[\s\S]*?\}/,
    "expected mono utility line-height to stay readable",
  );
  assert.doesNotMatch(
    typographySource,
    /\.(?:t-body|t-mono)\s*\{[\s\S]*?font-stretch:\s*75%/,
    "body and mono working text must not use condensed display typography",
  );
});

test("Mission Briefing has accessible role and live region", () => {
  const briefing = document.getElementById("op-briefing");
  assert.ok(briefing);
  assert.equal(briefing.getAttribute("role"), "status");
  assert.equal(briefing.getAttribute("aria-live"), "polite");
  assert.match(briefing.getAttribute("aria-label") ?? "", /boot|operator/i);
});

test("Status Strip is exposed as a live region with semantic value labels", () => {
  const strip = document.getElementById("op-status-strip");
  assert.ok(strip);
  assert.equal(strip.getAttribute("role"), "status");
  assert.equal(strip.getAttribute("aria-live"), "polite");
  for (const id of ["op-strip-active", "op-strip-idle", "op-strip-blocked", "op-strip-branches"]) {
    const el = document.getElementById(id);
    assert.ok(el, `expected element ${id}`);
    assert.ok(el.getAttribute("aria-label"), `${id} must have an aria-label`);
  }
  // clock cell is intentionally hidden from screen readers (per-second updates)
  const clockCell = document.getElementById("op-strip-clock")?.parentElement;
  assert.ok(clockCell, "clock cell exists");
  assert.equal(clockCell.getAttribute("aria-hidden"), "true");
});

test("Sidebar Quick rows expose aria-keyshortcuts and kbd badges", () => {
  for (const [cmd, key] of [
    ["open-board", "B"],
    ["open-git", "G"],
    ["open-logs", "L"],
  ]) {
    const button = document.querySelector(`.op-layer[data-cmd="${cmd}"]`);
    assert.ok(button, `expected Quick row for ${cmd}`);
    const shortcut = button.getAttribute("aria-keyshortcuts");
    assert.ok(shortcut, `${cmd} must declare aria-keyshortcuts`);
    assert.match(shortcut, new RegExp(`Meta\\+${key}`));
    const kbd = button.querySelector("kbd.op-layer__kbd");
    assert.ok(kbd, `${cmd} must show a kbd badge`);
  }
});

test("Command Palette trigger button declares aria-keyshortcuts", () => {
  const trigger = document.getElementById("op-palette-button");
  assert.ok(trigger, "palette trigger exists");
  const shortcut = trigger.getAttribute("aria-keyshortcuts") ?? "";
  assert.ok(shortcut.includes("Meta+K"), "trigger must declare Meta+K");
  assert.ok(shortcut.includes("Meta+P"), "trigger must declare Meta+P");
});

test("chrome visibility uses peek 帯 hover-reveal instead of click chips", () => {
  // SPEC-2356 Phase 9 (FR-021/FR-022): the chip-style toggles and Project Bar
  // text toggles are removed; auto-hide chrome is summoned via the peek 帯
  // (`.op-sidebar-peek` / `.op-window-controls-peek`).
  assert.equal(
    document.getElementById("op-sidebar-toggle"),
    null,
    "Project Bar must not expose a SIDEBAR text toggle",
  );
  assert.equal(
    document.getElementById("op-window-controls-toggle"),
    null,
    "Project Bar must not expose a WINDOWS text toggle",
  );
  assert.equal(
    document.getElementById("op-sidebar-edge-toggle"),
    null,
    "<< chip toggle is removed in Phase 9",
  );
  assert.equal(
    document.getElementById("op-window-controls-edge-toggle"),
    null,
    "vv chip toggle is removed in Phase 9",
  );

  const sidebarPeek = document.querySelector(".op-sidebar-peek");
  const windowControlsPeek = document.querySelector(".op-window-controls-peek");
  assert.ok(sidebarPeek, "expected .op-sidebar-peek hover trigger");
  assert.ok(windowControlsPeek, "expected .op-window-controls-peek hover trigger");
  assert.equal(sidebarPeek.getAttribute("aria-controls"), "op-sidebar");
  assert.equal(
    windowControlsPeek.getAttribute("aria-controls"),
    "floating-window-controls-actions",
  );
  assert.match(sidebarPeek.getAttribute("aria-label") ?? "", /show sidebar/i);
  assert.match(windowControlsPeek.getAttribute("aria-label") ?? "", /show window controls/i);
  assert.equal(sidebarPeek.getAttribute("tabindex"), "0", "sidebar peek 帯 must be keyboard-focusable");
  assert.equal(
    windowControlsPeek.getAttribute("tabindex"),
    "0",
    "window controls peek 帯 must be keyboard-focusable",
  );
  assert.equal(sidebarPeek.getAttribute("role"), "button");
  assert.equal(windowControlsPeek.getAttribute("role"), "button");
});

test("window controls peek 帯 targets only collapsible control groups", () => {
  const windowControlsPeek = document.querySelector(".op-window-controls-peek");
  assert.ok(windowControlsPeek, "expected window controls peek 帯");

  const controlledIds = (windowControlsPeek.getAttribute("aria-controls") ?? "")
    .split(/\s+/)
    .filter(Boolean);
  assert.deepEqual(controlledIds, ["floating-window-controls-actions"]);
  assert.ok(!controlledIds.includes("floating-window-controls"));

  const actionsGroup = document.getElementById("floating-window-controls-actions");
  const primaryGroup = document.getElementById("floating-window-controls-primary");
  const addGroup = document.getElementById("floating-window-controls-add");
  assert.ok(actionsGroup, "expected continuous window controls actions group");
  assert.ok(primaryGroup, "expected primary window controls group");
  assert.ok(addGroup, "expected add-window control group");

  const floatingControls = document.getElementById("floating-window-controls");
  assert.ok(floatingControls, "expected floating window controls root");
  const toolbarChildren = Array.from(floatingControls.children);
  assert.ok(
    toolbarChildren.indexOf(windowControlsPeek) < toolbarChildren.indexOf(actionsGroup),
    "peek must precede actions in DOM order so forward Tab enters the revealed controls",
  );

  assert.ok(actionsGroup.contains(primaryGroup), "primary controls must stay inside the continuous actions group");
  assert.ok(actionsGroup.contains(addGroup), "add controls must stay inside the continuous actions group");

  for (const id of ["tile-button", "stack-button", "align-button", "window-list-button"]) {
    const control = document.getElementById(id);
    assert.ok(control, `expected ${id}`);
    assert.ok(actionsGroup.contains(control), `${id} must be inside the continuous actions group`);
  }

  const addButton = document.getElementById("add-button");
  assert.ok(addButton, "expected add-button");
  assert.ok(addGroup.contains(addButton), "add-button must be inside the add controlled group");
  assert.ok(actionsGroup.contains(addButton), "add-button must be reachable through the continuous actions group");

  for (const id of ["op-palette-button", "zoom-out-button", "zoom-reset-button", "zoom-in-button"]) {
    const control = document.getElementById(id);
    assert.ok(control, `expected ${id}`);
    assert.equal(
      actionsGroup.contains(control),
      false,
      `${id} must remain outside collapsible window controls groups`,
    );
  }
});

test("floating window controls mark only window operations as hideable", () => {
  for (const id of ["tile-button", "stack-button", "align-button", "window-list-button", "add-button"]) {
    const button = document.getElementById(id);
    assert.ok(button, `expected ${id}`);
    assert.equal(button.dataset.windowControl, "true", `${id} should be hidden by the window controls toggle`);
  }
  for (const id of ["op-palette-button", "zoom-out-button", "zoom-reset-button", "zoom-in-button"]) {
    const button = document.getElementById(id);
    assert.ok(button, `expected ${id}`);
    assert.notEqual(button.dataset.windowControl, "true", `${id} should remain visible when window controls are hidden`);
  }
});

test("workspace windows expose draggable tab docking affordances", () => {
  assert.match(
    appSource,
    /class="window-tab-strip"|className\s*=\s*"window-tab-strip"/,
    "expected grouped windows to render a tab strip",
  );
  assert.match(
    appSource,
    /function\s+titlebarDockTargetAt\(/,
    "expected ungrouped windows to find dock targets from titlebar drag",
  );
  assert.match(
    windowDockingSource,
    /export\s+const\s+TITLEBAR_DOCK_HIT_HEIGHT\s*=\s*38/,
    "expected dock hit-testing to be constrained to the titlebar height",
  );
  assert.match(
    windowDockingSource,
    /point\.y\s*<=\s*geometry\.y\s*\+\s*titlebarHeight/,
    "expected body/canvas drops to avoid the dock path",
  );
  assert.match(
    appSource,
    /function\s+updateTitlebarDockPreview\(/,
    "expected titlebar drag to update dock target hover preview",
  );
  assert.match(
    appSource,
    /function\s+clearTitlebarDockPreview\(/,
    "expected dock target preview to be cleared after drop or cancel",
  );
  assert.match(
    appSource,
    /dragState\.dockTargetId[\s\S]+kind:\s*"dock_window_tab"[\s\S]+target_id:\s*dragState\.dockTargetId/,
    "expected titlebar drag pointerup to dock only when a titlebar target was hit",
  );
  assert.match(
    appSource,
    /kind:\s*"dock_window_tab"/,
    "expected tab drop to send dock_window_tab",
  );
  assert.match(
    appSource,
    /kind:\s*"detach_window_tab"/,
    "expected tab drag outside a group to send detach_window_tab",
  );
  assert.match(
    appSource,
    /kind:\s*"activate_window_tab"/,
    "expected tab click to activate a grouped window tab",
  );
  assert.match(
    inlineStyle,
    /\.workspace-window\.dock-target\s+\.titlebar/,
    "expected dockable titlebar targets to have a visible preview state",
  );
  assert.match(
    inlineStyle,
    /\.workspace-window\.dock-target\s*\{/,
    "expected dockable targets to outline the whole window, not just the titlebar",
  );
  assert.match(
    inlineStyle,
    /\.workspace-window\.dock-target\s+\.window-tab-strip::before/,
    "expected dockable targets to expose a tab insertion indicator",
  );
});

test("Project Bar brand prefix wraps GWT OPERATOR with bracket flank", () => {
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  // The pseudo-element content lives only in CSS, not in the DOM, so we
  // assert the rule itself contains the bracket-flanked brand string.
  const projectBarBefore = css.match(/\.project-bar::before\s*\{[\s\S]*?\}/);
  assert.ok(projectBarBefore, "expected .project-bar::before rule");
  assert.match(projectBarBefore[0], /content:\s*"⌜ GWT · OPERATOR ⌟"/);
});

test("Mission Briefing splash has dismissible affordance (pointer-events + cursor)", () => {
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  const briefing = css.match(/\.op-briefing\s*\{[\s\S]*?\}/);
  assert.ok(briefing, "expected .op-briefing rule");
  assert.match(briefing[0], /pointer-events:\s*auto/);
  assert.match(briefing[0], /cursor:\s*pointer/);
});

test("theme-toggle wires aria-label updates on every render", () => {
  // SPEC-2356 FR-024 — segmented toggle exposes the live preference + effective
  // theme via the radiogroup container's aria-label so screen readers always
  // announce the current state.
  const themeToggle = readFileSync(resolve(here, "../theme-toggle.js"), "utf8");
  assert.match(
    themeToggle,
    /root\.setAttribute\(\s*"aria-label"/,
    "expected segmented theme toggle to update aria-label on every render",
  );
  assert.match(
    themeToggle,
    /Theme: \$\{pref === "auto" \? `auto/,
    "expected aria-label format to disclose preference + effective theme",
  );
});

test("xterm content stays on the dark Operator palette across app theme changes", () => {
  assert.match(
    appSource,
    /theme:\s*XTERM_THEME_DARK/,
    "expected Terminal initialization to use the dark xterm palette directly",
  );
  assert.doesNotMatch(
    appSource,
    /XTERM_THEME_LIGHT/,
    "xterm content must not define a light palette; only the terminal chrome follows app theme",
  );
  assert.doesNotMatch(
    appSource,
    /registerXtermThemeAdapter/,
    "theme toggles must not swap xterm content away from the dark palette",
  );
});

test("xterm developer readability defaults use larger font metrics", () => {
  assert.ok(terminalOptionNumber("fontSize") >= 14, "xterm fontSize must be at least 14px");
  assert.ok(
    terminalOptionNumber("lineHeight") >= 1.3,
    "xterm lineHeight must be at least 1.30 to avoid cramped SS.mov output",
  );
});

test("operator-shell wires hover-reveal chrome and Mission Briefing early dismiss", () => {
  const operatorShell = readFileSync(resolve(here, "../operator-shell.js"), "utf8");
  // SPEC-2356 Phase 9 (FR-021/FR-022/FR-031): hover-reveal state machine sets
  // root.dataset.opSidebar = "revealed" / opWindowControls = "revealed" rather
  // than "collapsed", and Cmd+\\ hotkey is removed.
  assert.ok(
    !operatorShell.includes('hotkey.register("cmd+\\\\"'),
    "Cmd+backslash hotkey must be removed in Phase 9",
  );
  assert.doesNotMatch(
    operatorShell,
    /toggleSidebar\s*:/,
    "Phase 9 hover-reveal controller must not expose a toggleSidebar entrypoint",
  );
  assert.match(
    operatorShell,
    /root\.dataset\[datasetKey\]\s*=\s*"revealed"/,
    "expected hover-reveal state machine to write data-op-* = \"revealed\"",
  );
  assert.match(
    operatorShell,
    /earlyDismiss/,
    "expected Mission Briefing earlyDismiss helper",
  );
});

test("app state rendering dismisses Mission Briefing so startup cannot stay on splash", () => {
  assert.match(
    appSource,
    /function\s+dismissOperatorBriefing\(\)[\s\S]+op-briefing[\s\S]+hidden\s*=\s*true/,
    "expected app.js to expose a fail-open Mission Briefing dismiss helper",
  );
  assert.match(
    appSource,
    /function\s+renderAppState\(nextState\)\s*\{\s*dismissOperatorBriefing\(\)/,
    "expected first app state render to unblock Welcome/Workspace surfaces",
  );
});

test("components.css hover-reveals only the marked floating window control groups", () => {
  // SPEC-2356 Phase 9 (FR-022): the continuous actions group auto-hides by default and
  // are revealed only when [data-op-window-controls="revealed"] is set.
  // Palette and Zoom controls remain in the toolbar regardless.
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  assert.match(css, /#floating-window-controls-actions[\s\S]*?display:\s*none/);
  assert.match(css, /#floating-window-controls-actions\s*\{[^}]*order:\s*1/);
  assert.match(css, /\.op-window-controls-peek\s*\{[^}]*order:\s*2/);
  assert.match(
    css,
    /\[data-op-window-controls="revealed"\][\s\S]+?#floating-window-controls-actions[\s\S]*?display:\s*flex/,
  );
  assert.doesNotMatch(css, /\[data-op-window-controls="revealed"\][\s\S]+#op-palette-button/);
  assert.doesNotMatch(css, /\[data-op-window-controls="revealed"\][\s\S]+#zoom-reset-button/);
  assert.doesNotMatch(css, /\[data-op-window-controls="hidden"\]/);
  assert.match(css, /\.op-window-controls-peek\s*\{/);
  assert.match(css, /\.op-sidebar-peek\s*\{/);
});

test("floating actions expose Align without resizing windows", () => {
  const button = document.getElementById("align-button");
  assert.ok(button, "expected Align button");
  assert.equal(button.textContent.trim(), "Align");
  assert.match(
    appSource,
    /alignButton\.addEventListener\("click",\s*\(\)\s*=>\s*arrangeWindows\("align"\)\)/,
    "expected Align to reuse arrange_windows with the align mode",
  );
});

test("operator-shell migrates legacy chrome keys and wires hover-reveal independently", () => {
  // SPEC-2356 Phase 9 (FR-032): legacy localStorage keys are removed on boot
  // and the chip-style toggles (and their Project Bar text counterparts) must
  // not be referenced anywhere in operator-shell.
  const operatorShell = readFileSync(resolve(here, "../operator-shell.js"), "utf8");
  assert.doesNotMatch(operatorShell, /SIDEBAR_COLLAPSED_KEY\s*=\s*"gwt:ui:sidebar-collapsed"/);
  assert.doesNotMatch(operatorShell, /WINDOW_CONTROLS_KEY\s*=\s*"gwt:ui:window-controls"/);
  assert.doesNotMatch(operatorShell, /op-sidebar-edge-toggle/);
  assert.doesNotMatch(operatorShell, /op-window-controls-edge-toggle/);
  assert.doesNotMatch(operatorShell, /op-sidebar-toggle/);
  assert.doesNotMatch(operatorShell, /op-window-controls-toggle/);
  assert.match(operatorShell, /removeItem\("gwt:ui:sidebar-collapsed"\)/);
  assert.match(operatorShell, /removeItem\("gwt:ui:window-controls"\)/);
  assert.match(operatorShell, /op:chrome-visibility-changed/);
  assert.match(operatorShell, /op:window-controls-changed/);
  assert.match(operatorShell, /\.op-sidebar-peek/);
  assert.match(operatorShell, /\.op-window-controls-peek/);
});

test("components.css declares Status Strip BLOCKED pulse + live indicator", () => {
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  // PR #2414 introduced the pulse animation; PR #2404 the layer live dot.
  assert.match(css, /op-status-strip-blocked-pulse/);
  assert.match(css, /\.op-layer\[data-live="true"\] \.op-layer__label::before/);
});

test("components.css declares Operator scrollbar + tinted text selection", () => {
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  assert.match(css, /::-webkit-scrollbar-thumb/);
  assert.match(css, /scrollbar-color:\s*var\(--color-border-strong\)/);
  assert.match(css, /::selection/);
});

test("components.css uses op-divider utility class", () => {
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  assert.match(css, /\.op-divider\s*\{/);
  assert.match(css, /\.op-divider--vertical/);
  assert.match(css, /\.op-divider--strong/);
});

test("Drawer + preset modals have role/aria-modal/aria-hidden wiring", () => {
  // Every modal-backdrop must declare aria-hidden initially and contain a
  // modal-shell with role="dialog" + aria-modal="true". Without this, screen
  // readers don't recognize the modal as a dialog and the inert state isn't
  // signaled when closed.
  for (const id of ["branch-cleanup-modal", "migration-modal", "preset-modal", "wizard-modal"]) {
    const backdrop = document.getElementById(id);
    assert.ok(backdrop, `expected modal backdrop #${id}`);
    assert.equal(
      backdrop.getAttribute("aria-hidden"),
      "true",
      `${id} backdrop must start aria-hidden="true"`,
    );
    const shell = backdrop.querySelector(".modal-shell");
    assert.ok(shell, `${id} must contain a .modal-shell`);
    assert.equal(shell.getAttribute("role"), "dialog", `${id} shell needs role="dialog"`);
    assert.equal(shell.getAttribute("aria-modal"), "true", `${id} shell needs aria-modal="true"`);
    assert.equal(shell.getAttribute("tabindex"), "-1", `${id} shell needs tabindex="-1" for focus management`);
    // Either aria-label or aria-labelledby must point at a real label
    const label = shell.getAttribute("aria-label");
    const labelledby = shell.getAttribute("aria-labelledby");
    assert.ok(label || labelledby, `${id} shell needs aria-label or aria-labelledby`);
    if (labelledby) {
      assert.ok(
        document.getElementById(labelledby),
        `${id}: aria-labelledby="${labelledby}" must point at an existing element`,
      );
    }
  }
});

test("WebView modal text uses native selection and terminal overlays use explicit copy", () => {
  const modalShellRule = inlineStyle.match(/\.modal-shell\s*\{[\s\S]*?\}/);
  assert.ok(modalShellRule, "expected shared modal shell CSS rule");
  assert.doesNotMatch(
    modalShellRule[0],
    /user-select\s*:\s*none/,
    "modal shells must not disable browser-native text selection",
  );

  const visibleOverlayRule = inlineStyle.match(
    /\.terminal-overlay\.visible\s*\{[\s\S]*?\}/,
  );
  assert.ok(visibleOverlayRule, "expected visible terminal overlay CSS rule");
  assert.match(visibleOverlayRule[0], /pointer-events:\s*auto/);
  assert.match(visibleOverlayRule[0], /user-select:\s*text/);

  const overlayMessageRule = inlineStyle.match(/\.overlay-message\s*\{[\s\S]*?\}/);
  assert.ok(overlayMessageRule, "expected terminal overlay message CSS rule");
  assert.match(overlayMessageRule[0], /user-select:\s*text/);
  assert.match(appSource, /copyButton\.className\s*=\s*"overlay-copy-button"/);
  assert.match(
    appSource,
    /copyButton\.addEventListener\("click"[\s\S]*copyTerminalOverlayMessage\(windowData\.id\)/,
    "terminal overlays must use an explicit copy button instead of modal-shell auto-copy",
  );
});

test("Every keyframes-driven animation has a prefers-reduced-motion override", () => {
  // Catch the gap where someone adds a new @keyframes + animation without
  // pairing it with a reduced-motion override. Approach: for each
  // keyframes name, find its `animation: <name>` use site, walk back to
  // the enclosing selector, and verify that selector (or a parent
  // matching it) appears in any prefers-reduced-motion block.
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  const keyframes = [...new Set([...css.matchAll(/@keyframes\s+([\w-]+)\s*\{/g)].map((m) => m[1]))];
  assert.ok(keyframes.length >= 6, `expected >= 6 keyframes, got ${keyframes.length}`);

  // CSS @media blocks have nested rules with their own closing braces, so
  // a naive regex with `[^}]*` or `[\s\S]*?\n\}` undercaptures. Walk the
  // string and use depth tracking to extract every prefers-reduced-motion
  // block in full.
  const reducedMotionUnion = extractMediaBlocks(css, "prefers-reduced-motion: reduce");
  assert.ok(reducedMotionUnion.length > 0, "expected at least one prefers-reduced-motion block");

  for (const name of keyframes) {
    // Locate `animation: <name>` (skip the @keyframes definition itself).
    const useIndex = css.search(new RegExp(`animation:[^;]*\\b${name}\\b[^;]*;`));
    assert.ok(useIndex > 0, `@keyframes ${name} has no animation: use site`);
    // Walk back from the use site to the most recent `{` to find the
    // enclosing selector — that's the line that opens the rule.
    const before = css.slice(0, useIndex);
    const lastOpenBrace = before.lastIndexOf("{");
    assert.ok(lastOpenBrace > 0, `cannot locate opening brace for ${name}`);
    // Selector text is the line(s) immediately before the `{`. Walk
    // backward to the previous `}` (or BOF) for the rule start.
    const ruleStart = before.lastIndexOf("}", lastOpenBrace - 1);
    const selectorText = before.slice(ruleStart + 1, lastOpenBrace).trim();
    // Grab every class, attribute selector, or id used in the selector.
    const tokens = selectorText.match(/\.[\w-]+|\[[\w-]+(="[^"]*")?\]|#[\w-]+/g) || [];
    const hasOverride = tokens.some((tok) => reducedMotionUnion.includes(tok));
    assert.ok(
      hasOverride,
      `@keyframes ${name} (selector "${selectorText}") has no prefers-reduced-motion override`,
    );
  }
});

test("Branch rows are keyboard-navigable with click + dblclick parity", () => {
  // Branch rows are <div>s with both click (select) and dblclick (open
  // launch wizard) handlers. Keyboard parity:
  //   Enter / Space → select (same as click)
  //   Cmd/Ctrl + Enter → activate (same as dblclick — open wizard)
  // Without keyboard support, keyboard-only users could Tab to a row
  // but couldn't pick branches or launch agents from the Branches
  // surface at all.
  assert.match(
    appSource,
    /row\.tabIndex\s*=\s*0[\s\S]*row\.setAttribute\("role",\s*"button"\)[\s\S]*createBranchRow|createBranchRow[\s\S]*row\.tabIndex\s*=\s*0[\s\S]*row\.setAttribute\("role",\s*"button"\)/,
    "expected createBranchRow to set tabindex and role=button",
  );
  // The keydown handler must distinguish plain Enter/Space (select)
  // from modified Enter (activate / open wizard).
  assert.match(
    appSource,
    /event\.metaKey\s*\|\|\s*event\.ctrlKey[\s\S]*activate\(\)/,
    "expected modified Enter to invoke activate (open launch wizard)",
  );
});

test("Branches toolbar exposes explicit Resume and Launch Agent actions for selected branch", () => {
  assert.match(
    appSource,
    /data-action="open-branch-resume"/,
    "expected Branches toolbar to include an explicit Resume action",
  );
  assert.match(
    appSource,
    /data-action="open-branch-launch"/,
    "expected Branches toolbar to include an explicit Launch Agent action",
  );
  assert.match(
    appSource,
    /selectedBranchName[\s\S]{0,360}?kind:\s*"resume_branch_latest_agent"|kind:\s*"resume_branch_latest_agent"[\s\S]{0,360}?selectedBranchName/,
    "expected Branches toolbar Resume action to resume the selected branch",
  );
  assert.match(
    appSource,
    /selectedBranchName[\s\S]{0,300}?kind:\s*"open_launch_wizard"|kind:\s*"open_launch_wizard"[\s\S]{0,300}?selectedBranchName/,
    "expected Branches toolbar Launch Agent action to send selectedBranchName",
  );
  assert.match(
    appSource,
    /row\.addEventListener\("dblclick",\s*activate\)/,
    "expected branch row double-click to keep Launch Agent activation",
  );
});

test("Keyboard-navigable rows have :focus-visible outlines", () => {
  // After PRs #2464/#2465 made file-tree-row and branch-row keyboard-
  // navigable, the rows could receive focus but had no visible outline
  // — keyboard users couldn't see where focus was. Project tabs use
  // the same div+role+tabindex pattern and need the same focus-visible
  // treatment. Source-of-truth audit: every selector in app.js that
  // sets `tabIndex = 0` and `role="button"` must be in the CSS group.
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  for (const selector of [".project-tab", ".file-tree-row", ".branch-row"]) {
    const re = new RegExp(`:root\\[data-theme\\] \\${selector}:focus-visible`);
    assert.match(
      css,
      re,
      `expected ${selector} to be in the unified focus-visible group`,
    );
  }
});

test("Launch wizard open errors render in wizard modal and close locally", () => {
  assert.match(
    appSource,
    /let\s+launchWizardOpenError\s*=\s*null/,
    "expected frontend to track launch wizard open errors separately from project_open_error",
  );
  // Issue #2698 PR 1 (B7) — the case body now defers via
  // `wizardInteractionGuard.defer(...)` before mutating the error
  // state, so the regex window between the case label and the
  // assignment must permit the guard preamble. The intent is
  // unchanged: this case populates `launchWizardOpenError`.
  assert.match(
    appSource,
    /case\s+"launch_wizard_open_error":[\s\S]{0,600}?launchWizardOpenError\s*=/,
    "expected launch_wizard_open_error events to populate wizard error state",
  );
  assert.match(
    appSource,
    /function\s+closeLaunchWizardLocal\(\)[\s\S]{0,300}?launchWizardOpenError\s*=\s*null/,
    "expected a local close path for error-only wizard state",
  );
  assert.match(
    appSource,
    /wizardModal\.classList\.contains\("open"\)[\s\S]{0,500}?closeLaunchWizardLocal\(\)[\s\S]{0,500}?sendAction\(\{\s*kind:\s*"cancel"/,
    "expected Esc/close to locally dismiss error-only wizard state before sending backend cancel",
  );
});

test("Launch wizard removes duplicate header close button", () => {
  assert.equal(
    document.getElementById("wizard-close-button"),
    null,
    "Launch wizard header must not render a second Close button",
  );
  assert.ok(
    document.getElementById("wizard-cancel-button"),
    "Launch wizard footer dismiss button must remain available",
  );
  assert.equal(
    appSource.includes("wizardCloseButton"),
    false,
    "Launch wizard should not keep dead wiring for a removed header close button",
  );
  assert.match(
    appSource,
    /wizardCancelButton\.addEventListener\("click",\s*closeLaunchWizardFromChrome\)/,
    "expected the footer dismiss button to own the close helper",
  );
});

test("Launch wizard uses one visible dismiss control in the footer", () => {
  assert.equal(
    document.getElementById("wizard-close-button"),
    null,
    "Launch wizard header must not render a second Close button",
  );
  assert.ok(
    document.getElementById("wizard-cancel-button"),
    "Launch wizard footer dismiss button must remain available",
  );
  assert.equal(
    appSource.includes("wizardCloseButton"),
    false,
    "Launch wizard should not keep dead wiring for a removed header close button",
  );
  assert.match(
    appSource,
    /wizardCancelButton\.addEventListener\("click",\s*closeLaunchWizardFromChrome\)/,
    "expected the footer dismiss button to own the close helper",
  );
});

test("File tree rows are keyboard-navigable (tabindex + role + keydown)", () => {
  // <div>-based rows can't be Tab'd to or activated with the keyboard
  // unless explicitly opted in. Add tabindex, role="button", and a
  // keydown handler that mirrors the click handler for Enter/Space.
  // Without all three, keyboard-only users couldn't browse the file
  // tree — only mouse users could.
  assert.match(appSource, /row\.tabIndex\s*=\s*0/, "expected file tree row tabindex=0");
  assert.match(
    appSource,
    /row\.setAttribute\("role",\s*"button"\)/,
    "expected file tree row role='button'",
  );
  assert.match(
    appSource,
    /row\.addEventListener\("keydown",[\s\S]*event\.key === "Enter" \|\| event\.key === " "[\s\S]*activate\(\)/,
    "expected file tree row keydown handler invoking the click activate path on Enter/Space",
  );
});

test("File tree directory rows expose collapse state via aria-expanded", () => {
  // The file tree shows directories with ▾/▸ caret glyphs to indicate
  // expand/collapse state visually. Without aria-expanded, screen
  // readers can't tell whether a directory is open or closed, so the
  // user has to guess via context. File (non-directory) rows must
  // explicitly removeAttribute so a row that was a directory and
  // became a file (via state change) doesn't keep the marker.
  assert.match(
    appSource,
    /isDirectory[\s\S]*row\.setAttribute\("aria-expanded",\s*expanded\s*\?\s*"true"\s*:\s*"false"\)/,
    "expected directory rows to set aria-expanded based on expanded state",
  );
  assert.match(
    appSource,
    /row\.removeAttribute\("aria-expanded"\)/,
    "expected non-directory rows to remove aria-expanded",
  );
});

test("Launch wizard choice buttons expose toggle state via aria-pressed", () => {
  // The wizard's agent / preset picker uses createChoiceButton for
  // mutually-exclusive options. Without aria-pressed, screen readers
  // can't announce which option is currently chosen — they just hear
  // a button list with no selection state.
  assert.match(
    appSource,
    /button\.setAttribute\("aria-pressed",\s*selected\s*\?\s*"true"\s*:\s*"false"\)/,
    "expected createChoiceButton to set aria-pressed based on selected",
  );
});

test("Launch wizard separates launch settings from runtime controls", () => {
  assert.equal(
    appSource.includes("wizardAdvancedOpen"),
    false,
    "Launch wizard should not keep an Advanced disclosure state",
  );
  for (const retiredCopy of ["Advanced", "Show advanced", "Hide advanced"]) {
    assert.equal(
      appSource.includes(`"${retiredCopy}"`),
      false,
      `Launch wizard should not render ${retiredCopy}`,
    );
  }

  const launchSettingsStart = appSource.indexOf(
    'createLaunchSection(\n            "Launch settings"',
  );
  const linkedIssueStart = appSource.indexOf(
    'createLaunchSection(\n            "Linked issue"',
  );
  const runtimeStart = appSource.indexOf(
    'createLaunchSection(\n            "Runtime"',
  );

  assert.notEqual(launchSettingsStart, -1, "expected Launch settings section");
  assert.notEqual(linkedIssueStart, -1, "expected Linked issue section");
  assert.notEqual(runtimeStart, -1, "expected Runtime section");
  assert.ok(
    launchSettingsStart < linkedIssueStart && linkedIssueStart < runtimeStart,
    "expected Launch settings before Linked issue and Runtime after Linked issue",
  );

  const launchSettingsBlock = appSource.slice(
    launchSettingsStart,
    linkedIssueStart,
  );
  for (const copy of ["Version", "Skip permission prompts", "Codex fast mode"]) {
    assert.ok(
      launchSettingsBlock.includes(`"${copy}"`),
      `expected Launch settings to include ${copy}`,
    );
  }
  assert.equal(
    launchSettingsBlock.includes('"Runtime target"'),
    false,
    "Launch settings should not contain runtime target controls",
  );

  const runtimeBlock = appSource.slice(
    runtimeStart,
    appSource.indexOf("wizardBody.appendChild(panel);"),
  );
  for (const copy of ["Runtime target", "Docker service", "Docker lifecycle"]) {
    assert.ok(
      runtimeBlock.includes(`"${copy}"`),
      `expected Runtime to include ${copy}`,
    );
  }
  for (const copy of ["Version", "Skip permission prompts", "Codex fast mode"]) {
    assert.equal(
      runtimeBlock.includes(`"${copy}"`),
      false,
      `Runtime should not contain ${copy}`,
    );
  }
});

test("Launch wizard runtime confirmation shows summary without setup forms", () => {
  assert.match(
    appSource,
    /const showManualSetup = launchWizard\.show_manual_setup !== false;[\s\S]*?const isRuntimeConfirmation = Boolean\([\s\S]*?const showSetupForms = showManualSetup && !isRuntimeConfirmation;/,
    "expected showManualSetup to be initialized before Runtime confirmation setup gating",
  );
  assert.match(
    appSource,
    /const isRuntimeConfirmation = Boolean\(\s*launchWizard\.runtime_context_resolved\s*&&\s*launchWizard\.show_runtime_confirmation\s*\);/,
    "expected renderer to derive a dedicated Runtime confirmation state",
  );
  assert.match(
    appSource,
    /const showSetupForms = showManualSetup && !isRuntimeConfirmation;/,
    "expected manual setup forms to be suppressed during Runtime confirmation",
  );
  assert.match(
    appSource,
    /renderWizardSummary\(\);[\s\S]*?const isRuntimeConfirmation = Boolean/,
    "expected read-only launch summary to remain visible before Runtime-only body rendering",
  );
  assert.match(
    appSource,
    /if\s*\(\s*!isRuntimeConfirmation\s*&&\s*\(\s*\(launchWizard\.quick_start_entries \|\| \[\]\)\.length > 0/,
    "expected Quick Start selection rows to be hidden during Runtime confirmation",
  );
  assert.match(
    appSource,
    /if\s*\(\s*showSetupForms\s*&&\s*launchWizard\.show_branch_controls !== false\s*\)/,
    "expected Branch controls to be part of setup forms only",
  );
  assert.match(
    appSource,
    /if\s*\(\s*showSetupForms\s*\)\s*\{[\s\S]*?createLaunchSection\(\s*"Launch"/,
    "expected Launch controls to be part of setup forms only",
  );
  assert.match(
    appSource,
    /if\s*\(\s*showSetupForms\s*&&[\s\S]*?launchWizard\.show_codex_fast_mode[\s\S]*?\)\s*\{[\s\S]*?createLaunchSection\(\s*"Launch settings"/,
    "expected Launch settings controls to be part of setup forms only",
  );
  // SPEC-2014 Amendment 2026-05-20 (FR-057): Linked issue is gated by its
  // dedicated `show_linked_issue` flag so it only appears when the wizard
  // was opened through the Knowledge Issue Bridge.
  assert.match(
    appSource,
    /if\s*\(\s*showSetupForms\s*&&\s*launchWizard\.show_linked_issue\s*\)/,
    "expected Linked issue controls to be gated by show_linked_issue and setup forms",
  );
});

test("Launch wizard submit button uses Continue before runtime context is resolved", () => {
  assert.match(
    appSource,
    /launchWizard\.primary_action_label\s*\|\|[\s\S]{0,260}?launchWizard\.runtime_context_resolved === false\s*\?\s*"Continue"\s*:/,
    "expected unresolved Launch Agent runtime context to use Continue instead of Launch",
  );
});

test("Launch wizard keeps cancel available during runtime resolution", () => {
  const closeHelper = appSource.match(
    /function closeLaunchWizardFromChrome\(\) \{([\s\S]*?)\n      \}/,
  );
  assert.ok(closeHelper, "expected launch wizard close helper");
  assert.equal(
    closeHelper[1].includes("runtime_resolution_pending"),
    false,
    "runtime resolution pending must not block the footer Cancel button",
  );
  assert.match(
    appSource,
    /wizardCancelButton\.disabled\s*=\s*false/,
    "Cancel button must stay enabled while runtime resolution is pending",
  );
  const escapeHandler = appSource.match(
    /if \(wizardModal\.classList\.contains\("open"\)\) \{([\s\S]*?)event\.preventDefault\(\);\n          return;/,
  );
  assert.ok(escapeHandler, "expected launch wizard Escape handler");
  assert.equal(
    escapeHandler[1].includes("runtime_resolution_pending"),
    false,
    "Escape must keep using the cancel path while runtime resolution is pending",
  );
});

test("Launch wizard disables panel controls while runtime resolution is pending", () => {
  assert.match(
    appSource,
    /const selector =\n\s+"input, textarea, select, button, \[role='button'\], \[contenteditable='true'\]";[\s\S]*?querySelectorAll\(selector\)/,
    "expected a pending-state helper to disable wizard panel controls",
  );
  assert.match(
    appSource,
    /panel\.classList\.toggle\("wizard-disabled",\s*isRuntimeResolutionPending\)/,
    "expected the launch panel to expose a disabled visual state while pending",
  );
  assert.match(
    inlineStyle,
    /\.launch-panel\.wizard-disabled[\s\S]*?pointer-events:\s*none;/,
    "expected pending wizard controls to ignore pointer events",
  );
});

test("Launch wizard renders centered split flow without backdrop dismissal", () => {
  assert.equal(
    appSource.includes("isStartWorkMode"),
    false,
    "Start Work should share the centered wizard modal instead of toggling drawer mode",
  );
  assert.equal(
    appSource.includes('wizardModal.classList.toggle("is-drawer"'),
    false,
    "Launch Wizard should not toggle drawer placement",
  );
  assert.ok(
    appSource.includes("wizard-progress-rail")
      && appSource.includes("wizard-main")
      && appSource.includes("wizard-content-pane"),
    "expected the wizard body to be split into progress rail and content pane",
  );
  assert.equal(
    /if\s*\(\s*event\.target === wizardModal\s*\)\s*\{\s*closeLaunchWizardFromChrome\(\);\s*\}/.test(appSource),
    false,
    "wizard backdrop clicks must not dismiss the wizard",
  );
});

test("Launch wizard selected quick start hover preserves selected styling", () => {
  assert.match(
    inlineStyle,
    /\.quick-start-card:hover:not\(\.selected\),\r?\n\.live-session-button:hover:not\(\.selected\)/,
    "selected quick start and live session rows must not use the unselected hover background",
  );
});

test("Launch wizard quick start is selected before footer submit", () => {
  assert.ok(
    appSource.includes('kind: "select_quick_start"'),
    "expected quick start rows to update wizard selection instead of launching inline",
  );
  assert.ok(
    appSource.includes("selected_launch_path")
      && appSource.includes("selected_quick_start_index")
      && appSource.includes("primary_action_label"),
    "expected frontend to render the backend-selected launch path and footer primary label",
  );
  assert.equal(
    appSource.includes("quick-start-actions"),
    false,
    "quick start rows should not render multiple inline action buttons",
  );
});

test("Selected list rows mark active item with aria-current", () => {
  // Same pattern as project tabs (PR #2455): list-style buttons with a
  // selected state need aria-current to announce which row is active.
  // Coverage: knowledge-row, profile-row, logs-entry.
  // The set/remove pair is required so a previously-selected row
  // doesn't retain the marker after the user picks a different row.
  for (const desc of [
    "knowledge",
    "profile",
    "logs entry",
  ]) {
    // Each list iterates over its rows and conditionally sets
    // aria-current="true" on the selected one. We assert both the
    // setAttribute and removeAttribute calls exist somewhere in app.js.
    // The descriptive label is just for the failure message.
  }
  // Count occurrences: knowledge, profile, logs, file-tree, plus the
  // project-tabs case from PR #2455.
  const setMatches =
    appAndProjectTabsSource.match(/setAttribute\("aria-current",\s*"(true|page)"\)/g) || [];
  const removeMatches =
    appAndProjectTabsSource.match(/removeAttribute\("aria-current"\)/g) || [];
  assert.ok(
    setMatches.length >= 5,
    `expected >= 5 aria-current set calls (project tab + 4 row types), got ${setMatches.length}`,
  );
  assert.ok(
    removeMatches.length >= 5,
    `expected >= 5 aria-current remove calls (one per set call), got ${removeMatches.length}`,
  );
});

test("Project tabs mark the active project with aria-current=\"page\"", () => {
  // Project tabs use role="button" but represent a navigation choice. The
  // appropriate ARIA pattern is aria-current="page" on the active tab so
  // screen readers announce it as the current location. Inactive tabs
  // must explicitly clear the attribute so the previously-active tab
  // doesn't retain the marker after a switch.
  assert.match(
    projectTabsRendererSource,
    /button\.setAttribute\("aria-current",\s*"page"\)/,
    "expected the active project tab to set aria-current=\"page\"",
  );
  assert.match(
    projectTabsRendererSource,
    /button\.removeAttribute\("aria-current"\)/,
    "expected inactive project tabs to remove aria-current",
  );
});

test("Error regions declare role=\"alert\" so screen readers announce them", () => {
  // Without role="alert" (or aria-live="assertive"), errors that appear
  // in wizard-error and project-picker-error are silent for screen reader
  // users — they only see them after manually navigating to the error
  // element. role="alert" implies aria-live="assertive" + aria-atomic so
  // the full message is announced immediately when the element becomes
  // non-hidden.
  for (const id of ["wizard-error", "project-picker-error"]) {
    const el = document.getElementById(id);
    assert.ok(el, `expected error region #${id}`);
    assert.equal(el.getAttribute("role"), "alert", `${id} must have role="alert"`);
  }
});

test("Wizard and preset modals activate focus-trap on open and release on close (app.js inline)", () => {
  // The wizard and preset modals are rendered inline in app.js (not in
  // separate renderer modules), so they need closure-scoped trap-release
  // variables. Pin both pairs.
  assert.match(appSource, /import\s*\{\s*createFocusTrap\s*\}\s*from\s*"\/focus-trap\.js"/);
  // Wizard
  assert.match(appSource, /let\s+wizardFocusTrapRelease\s*=\s*null/);
  assert.match(
    appSource,
    /wizardFocusTrapRelease\s*=\s*createFocusTrap\(wizardDialog,\s*\{\s*document\s*\}\)/,
    "wizard must activate focus trap on the dialog",
  );
  assert.match(
    appSource,
    /typeof\s+wizardFocusTrapRelease\s*===\s*"function"\s*\)\s*\{\s*wizardFocusTrapRelease\(\)/,
    "wizard must release focus trap on close",
  );
  // Preset
  assert.match(appSource, /let\s+presetModalFocusTrapRelease\s*=\s*null/);
  assert.match(
    appSource,
    /presetModalFocusTrapRelease\s*=\s*createFocusTrap\(presetShell,\s*\{\s*document\s*\}\)/,
    "preset modal must activate focus trap on the shell",
  );
  assert.match(
    appSource,
    /typeof\s+presetModalFocusTrapRelease\s*===\s*"function"\s*\)\s*\{\s*presetModalFocusTrapRelease\(\)/,
    "preset modal must release focus trap on close",
  );
});

test("Drawer modal renderers activate focus-trap on open and release on close", () => {
  // Focus trap prevents Tab from escaping the modal into background
  // content. Both renderers must:
  // - Import createFocusTrap
  // - Maintain a focusTrapMap WeakMap to track release functions
  // - Call createFocusTrap on fresh open and store the release
  // - Invoke the release before restoring focus on close
  const branchCleanupSrc = readFileSync(
    resolve(here, "../branch-cleanup-modal.js"),
    "utf8",
  );
  const migrationSrc = readFileSync(
    resolve(here, "../migration-modal.js"),
    "utf8",
  );
  for (const [src, name] of [[branchCleanupSrc, "branch-cleanup-modal"], [migrationSrc, "migration-modal"]]) {
    assert.match(
      src,
      /import\s*\{\s*createFocusTrap\s*\}\s*from\s*"\.\/focus-trap\.js"/,
      `${name} must import createFocusTrap from ./focus-trap.js (relative path so Node tests resolve)`,
    );
    assert.match(src, /const\s+focusTrapMap\s*=\s*new\s+WeakMap\(\)/, `${name} must define focusTrapMap`);
    assert.match(
      src,
      /const\s+release\s*=\s*createFocusTrap\(dialogEl,\s*\{\s*document:\s*ownerDoc\s*\}\)/,
      `${name} must activate focus trap on open with the dialog as container`,
    );
    assert.match(
      src,
      /focusTrapMap\.set\(modalEl,\s*release\)/,
      `${name} must store the trap release in focusTrapMap`,
    );
    assert.match(
      src,
      /releaseTrap\s*=\s*focusTrapMap\.get\(modalEl\)[\s\S]*if\s*\(typeof\s+releaseTrap\s*===\s*"function"\)\s*releaseTrap\(\)/,
      `${name} must release the trap before restoring focus on close`,
    );
  }
});

test("Dynamically-created form fields without surrounding <label> have aria-label", () => {
  // Form fields that aren't wrapped in a <label> element need aria-label
  // so screen readers announce their purpose instead of just "edit text"
  // (input/textarea) or the first option (select). This audit covers
  // the launch wizard and profile editor surfaces.
  // SPEC-2014 Amendment 2026-05-20 (FR-058): the wizard "Issue number"
  // field is now rendered as static read-only text (no <input>), so its
  // accessibility name is conveyed by the surrounding launch-field label
  // instead of a per-input aria-label. The audit entry is removed
  // accordingly.
  const expected = [
    { selector: 'input\\.setAttribute\\("aria-label", "Branch name"\\)', desc: "wizard branch name" },
    { selector: 'keyInput\\.setAttribute\\("aria-label", `Env var key, row ', desc: "env var key" },
    { selector: 'valueInput\\.setAttribute\\("aria-label", `Env var value, row ', desc: "env var value" },
    { selector: 'keyInput\\.setAttribute\\("aria-label", `Disabled env key, row ', desc: "disabled env key" },
    { selector: 'select\\.setAttribute\\("aria-label", label\\)', desc: "launch-field select reuses label" },
  ];
  for (const { selector, desc } of expected) {
    assert.match(
      appSource,
      new RegExp(selector),
      `expected ${desc} field to have aria-label set`,
    );
  }
});

test("Every role=\"dialog\" element has a programmatic accessible name", () => {
  // Catch the case where a new dialog is added without aria-label or
  // aria-labelledby — without an accessible name, screen readers
  // announce just "dialog" with no context. Iterates the live DOM
  // tree so future additions are covered automatically.
  const dialogs = document.querySelectorAll('[role="dialog"]');
  assert.ok(dialogs.length >= 5, `expected >= 5 role="dialog" elements, got ${dialogs.length}`);
  for (const dialog of dialogs) {
    const label = dialog.getAttribute("aria-label");
    const labelledby = dialog.getAttribute("aria-labelledby");
    const id = dialog.getAttribute("id") || dialog.parentElement?.getAttribute("id") || "(no id)";
    assert.ok(
      (label && label.length > 0) || (labelledby && labelledby.length > 0),
      `role="dialog" at #${id} must have aria-label or aria-labelledby`,
    );
    if (labelledby) {
      const target = document.getElementById(labelledby);
      assert.ok(
        target,
        `role="dialog" at #${id} aria-labelledby="${labelledby}" must point at an existing element`,
      );
      assert.ok(
        target.textContent && target.textContent.trim().length > 0,
        `role="dialog" at #${id} aria-labelledby target must have non-empty text`,
      );
    }
  }
});

test("Migration progress bar references the phase label via aria-labelledby", () => {
  // The native <progress> element gets an implicit role="progressbar"
  // but no programmatic name unless aria-label or aria-labelledby is
  // provided. Without it, screen readers announce just "progressbar 50%"
  // and the user doesn't know which phase is running.
  const migrationSrc = readFileSync(
    resolve(here, "../migration-modal.js"),
    "utf8",
  );
  assert.match(
    migrationSrc,
    /phaseLabel\.id\s*=\s*"migration-modal-phase-label"/,
    "expected migration phase label to have a stable id",
  );
  assert.match(
    migrationSrc,
    /progress\.setAttribute\("aria-labelledby",\s*"migration-modal-phase-label"\)/,
    "expected migration <progress> to reference the phase label via aria-labelledby",
  );
});

test("Drawer modal renderers signal busy state via aria-busy during async stages", () => {
  // The branch-cleanup running stage and migration running stage are
  // synchronous async operations. Without aria-busy, screen readers don't
  // signal that the dialog is in a loading state — users may try to
  // interact with controls that are about to disappear.
  const branchCleanupSrc = readFileSync(
    resolve(here, "../branch-cleanup-modal.js"),
    "utf8",
  );
  const migrationSrc = readFileSync(
    resolve(here, "../migration-modal.js"),
    "utf8",
  );
  for (const [src, name] of [[branchCleanupSrc, "branch-cleanup-modal"], [migrationSrc, "migration-modal"]]) {
    assert.match(
      src,
      /dialogEl\.setAttribute\("aria-busy",\s*"true"\)/,
      `${name} must set aria-busy="true" during the running stage`,
    );
    assert.match(
      src,
      /dialogEl\.setAttribute\("aria-busy",\s*"false"\)/,
      `${name} must clear aria-busy in non-running stages`,
    );
  }
});

test("Drawer modal renderers restore focus on close (WeakMap pattern)", () => {
  // Companion to fresh-open focus management: when the modal closes the
  // trigger element captured at open time must regain focus, otherwise
  // keyboard users land on document.body. Both renderers must implement
  // the WeakMap-keyed return-focus pattern.
  const branchCleanupSrc = readFileSync(
    resolve(here, "../branch-cleanup-modal.js"),
    "utf8",
  );
  const migrationSrc = readFileSync(
    resolve(here, "../migration-modal.js"),
    "utf8",
  );
  for (const [src, name] of [[branchCleanupSrc, "branch-cleanup-modal"], [migrationSrc, "migration-modal"]]) {
    assert.match(src, /const\s+focusReturnMap\s*=\s*new\s+WeakMap\(\)/, `${name} must define focusReturnMap WeakMap`);
    assert.match(src, /focusReturnMap\.set\(modalEl,\s*ownerDoc\.activeElement\)/, `${name} must save activeElement on open`);
    assert.match(src, /focusReturnMap\.get\(modalEl\)/, `${name} must read trigger on close`);
    assert.match(src, /focusReturnMap\.delete\(modalEl\)/, `${name} must clear the map after close`);
    assert.match(src, /returnTo\.focus\(\{\s*preventScroll:\s*true\s*\}\)/, `${name} must restore focus with preventScroll`);
  }
});

test("Drawer modal renderers move focus into dialog on fresh open", () => {
  // The Hotkey overlay (PR #2440) already does this; the drawer modals
  // need parity so screen readers announce them and keyboard users land
  // inside. Re-renders during running/result stages must NOT re-move focus
  // (users keep their place navigating buttons).
  const branchCleanupSrc = readFileSync(
    resolve(here, "../branch-cleanup-modal.js"),
    "utf8",
  );
  const migrationSrc = readFileSync(
    resolve(here, "../migration-modal.js"),
    "utf8",
  );
  for (const [src, name] of [[branchCleanupSrc, "branch-cleanup-modal"], [migrationSrc, "migration-modal"]]) {
    // Each renderer must capture wasOpen BEFORE adding the .open class
    // and gate the focus move on !wasOpen.
    assert.match(
      src,
      /const\s+wasOpen\s*=\s*modalEl\.classList\.contains\("open"\)/,
      `${name} must capture wasOpen before adding the .open class`,
    );
    assert.match(
      src,
      /!wasOpen[\s\S]*dialogEl\.focus\(\{\s*preventScroll:\s*true\s*\}\)/,
      `${name} must focus dialogEl with preventScroll only on fresh open`,
    );
  }
});

test("Preset (Add Window) modal manages focus on open and restores on close", () => {
  // The "+" Add Window modal opens via openModal()/closeModal() in app.js.
  // Verify the same focus-management pattern is wired: capture trigger,
  // move focus to modal-shell on open, restore on close.
  assert.match(appSource, /let\s+presetModalFocusReturn\s*=\s*null/);
  assert.match(appSource, /presetModalFocusReturn\s*=\s*document\.activeElement/);
  assert.match(
    appSource,
    /presetShell\.focus\(\{\s*preventScroll:\s*true\s*\}\)/,
    "expected preset modal shell focus on open with preventScroll",
  );
  assert.match(
    appSource,
    /presetModalFocusReturn\.focus\(\{\s*preventScroll:\s*true\s*\}\)/,
    "expected preset modal restore focus on close with preventScroll",
  );
});

test("Wizard modal manages focus on open and restores on close", () => {
  // The wizard modal's renderer lives in app.js (not a separate module),
  // so focus management is wired inline. Verify the same pattern as the
  // drawer modals: wizardFocusReturn captures activeElement on open,
  // wizardDialog.focus({ preventScroll: true }) moves focus into the
  // dialog, and wizardFocusReturn.focus() restores on close.
  assert.match(appSource, /let\s+wizardFocusReturn\s*=\s*null/);
  assert.match(appSource, /wizardFocusReturn\s*=\s*document\.activeElement/);
  assert.match(
    appSource,
    /wizardDialog\.focus\(\{\s*preventScroll:\s*true\s*\}\)/,
    "expected wizardDialog focus on open with preventScroll",
  );
  assert.match(
    appSource,
    /wizardFocusReturn\.focus\(\{\s*preventScroll:\s*true\s*\}\)/,
    "expected wizardFocusReturn focus on close with preventScroll",
  );
});

test("Drawer modals close on Escape — keyboard parity with backdrop click", () => {
  // The Hotkey overlay and Command Palette already had Esc-close (PR #2440).
  // branch-cleanup and migration modals previously closed only on backdrop
  // click — keyboard users were trapped. app.js must wire a global keydown
  // that maps Escape to the same close path for both modals.
  assert.match(
    appSource,
    /event\.key\s*!==?\s*"Escape"/,
    "expected app.js to gate keydown on Escape",
  );
  assert.match(
    appSource,
    /branchCleanupModal\.classList\.contains\("open"\)[\s\S]*closeBranchCleanupModal/,
    "expected Esc to call closeBranchCleanupModal when that modal is open",
  );
  // SPEC-1934 US-7 / FR-032: the confirmation modal is Accept-only. Esc no
  // longer routes to skip_migration; at the confirm stage it is swallowed
  // (no backend send), at the error stage it dismisses the UI without
  // flipping `migration_pending`.
  assert.doesNotMatch(
    appSource,
    /migrationModal\s*&&\s*migrationModal\.classList\.contains\("open"\)[\s\S]{0,400}skip_migration/,
    "Esc must not send skip_migration when the Accept-only migration modal is open",
  );
  assert.match(
    appSource,
    /migrationModal\s*&&\s*migrationModal\.classList\.contains\("open"\)[\s\S]*migrationModalState\.stage\s*===\s*"error"[\s\S]*migrationModalState\.open\s*=\s*false/,
    "expected Esc on the migration modal error stage to dismiss the dialog without backend skip",
  );
  assert.match(
    appSource,
    /wizardModal\.classList\.contains\("open"\)[\s\S]*launchWizardSurface\.sendAction\(\{\s*kind:\s*"cancel"/,
    "expected Esc to send cancel when wizard modal is open",
  );
  assert.match(
    appSource,
    /if\s*\(windowListOpen\)\s*\{[\s\S]*windowListOpen\s*=\s*false[\s\S]*windowListButton\.focus/,
    "expected Esc to close window list dropdown and restore focus to trigger",
  );
  // SPEC-2356 — preset modal Esc-close: closes via closeModal() which
  // handles both the .open class flip and focus restore.
  assert.match(
    appSource,
    /if\s*\(modal\.classList\.contains\("open"\)\)\s*\{[\s\S]*closeModal\(\)/,
    "expected Esc to call closeModal when preset modal is open",
  );
});

test("Drawer modal renderers toggle aria-hidden alongside .open class", () => {
  const branchCleanupSrc = readFileSync(
    resolve(here, "../branch-cleanup-modal.js"),
    "utf8",
  );
  const migrationSrc = readFileSync(
    resolve(here, "../migration-modal.js"),
    "utf8",
  );
  // Both renderers must flip aria-hidden in lockstep with the .open class.
  for (const [src, name] of [[branchCleanupSrc, "branch-cleanup-modal"], [migrationSrc, "migration-modal"]]) {
    assert.match(
      src,
      /modalEl\.setAttribute\("aria-hidden",\s*"true"\)/,
      `${name} must set aria-hidden="true" on close`,
    );
    assert.match(
      src,
      /modalEl\.removeAttribute\("aria-hidden"\)/,
      `${name} must remove aria-hidden on open`,
    );
  }
});

test("Hotkey overlay manages focus on open / close (modal dialog a11y)", () => {
  // The dialog must be focusable so we can move focus inside on open and
  // return it to the trigger on close — without this, screen readers don't
  // announce the dialog and keyboard users get stranded after Esc.
  const card = document.querySelector("#op-hotkey-overlay .op-hotkey-card");
  assert.ok(card, "expected hotkey overlay card");
  assert.equal(card.getAttribute("tabindex"), "-1");
  assert.equal(card.getAttribute("role"), "dialog");
  assert.equal(card.getAttribute("aria-modal"), "true");
  // operator-shell.js wires the dynamic focus pieces.
  assert.match(operatorShellSource, /returnFocusTo\s*=\s*doc\.activeElement/);
  assert.match(operatorShellSource, /card\.focus\(\{\s*preventScroll:\s*true\s*\}\)/);
  assert.match(operatorShellSource, /returnFocusTo\.focus/);
});

test("Command Palette implements the WAI-ARIA combobox/listbox pattern", () => {
  // The input must be a combobox that controls the list and announces the
  // active option via aria-activedescendant. Without these the listbox role
  // already on the <ul> is meaningless to screen readers.
  const paletteInput = document.querySelector("#op-palette-input");
  assert.ok(paletteInput, "expected palette input");
  assert.equal(paletteInput.getAttribute("role"), "combobox");
  assert.equal(paletteInput.getAttribute("aria-controls"), "op-palette-list");
  assert.equal(paletteInput.getAttribute("aria-autocomplete"), "list");
  // aria-expanded starts false; operator-shell flips it on open/close.
  assert.equal(paletteInput.getAttribute("aria-expanded"), "false");
  assert.match(paletteInput.getAttribute("aria-label") ?? "", /command/i);
  // operator-shell.js must wire the dynamic pieces.
  assert.match(operatorShellSource, /input\.setAttribute\("aria-expanded",\s*"true"\)/);
  assert.match(operatorShellSource, /input\.setAttribute\("aria-activedescendant"/);
  assert.match(operatorShellSource, /input\.removeAttribute\("aria-activedescendant"\)/);
  assert.match(operatorShellSource, /li\.setAttribute\("role",\s*"option"\)/);
  assert.match(operatorShellSource, /li\.setAttribute\("aria-selected"/);
  assert.match(operatorShellSource, /li\.id\s*=\s*`op-palette-row-\$\{idx\}`/);
});

test("op-drawer scaffold honors prefers-reduced-motion (parity with legacy is-drawer modal)", () => {
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  // The legacy `.modal-backdrop.is-drawer .modal-shell.is-drawer-shell` already
  // suppresses its slide transform under reduced-motion. The forward-looking
  // op-drawer scaffold needed parity — without this guard, future direct
  // adoption would slide in even for users who opted out of motion.
  const reducedMotionBlocks = css.match(
    /@media\s*\(\s*prefers-reduced-motion:\s*reduce\s*\)\s*\{[\s\S]*?\n\}/g,
  );
  assert.ok(reducedMotionBlocks, "expected at least one prefers-reduced-motion block");
  const opDrawerCovered = reducedMotionBlocks.some(
    (block) => /\.op-drawer\b/.test(block) && /transition:\s*none/.test(block),
  );
  assert.ok(opDrawerCovered, ".op-drawer scaffold must drop its transition under reduced-motion");
});

test("mapAgentTelemetryState emits only the four Living Telemetry states CSS handles", () => {
  // app.js has a closure-scoped runtime→Living Telemetry mapper. CSS only
  // styles `[data-agent-state]` for active/idle/blocked/done — any drift
  // (e.g. emitting "warn" or "exited") would silently render no rim. Pin
  // the contract so refactors can't introduce undeclared states.
  const mapperBlock = appSource.match(
    /function\s+mapAgentTelemetryState\s*\([^)]*\)\s*\{[\s\S]*?\n\s{6,8}\}/,
  );
  assert.ok(mapperBlock, "expected mapAgentTelemetryState to be defined in app.js");
  const returnedStates = new Set();
  for (const m of mapperBlock[0].matchAll(/return\s+"([^"]+)"/g)) {
    returnedStates.add(m[1]);
  }
  const allowed = new Set(["active", "idle", "blocked", "done"]);
  for (const state of returnedStates) {
    assert.ok(allowed.has(state), `mapAgentTelemetryState returned undeclared state: ${state}`);
  }
  // And the four design states must all be reachable, not just allowed.
  for (const required of allowed) {
    assert.ok(returnedStates.has(required), `Living Telemetry state never emitted: ${required}`);
  }
});

test("Status Strip ACTIVE / IDLE / BLOCKED cells all tint with their state color", () => {
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  // The ACTIVE / IDLE cells previously had no tonal hint — only BLOCKED did.
  // Add parallel symmetry so the three count cells render with matching state
  // colors (cyan / gray / red) for at-a-glance scanning.
  assert.match(css, /\.op-status-strip__cell--active\s+\.op-status-strip__value\s*\{[^}]*--color-state-active/);
  assert.match(css, /\.op-status-strip__cell--idle\s+\.op-status-strip__value\s*\{[^}]*--color-state-idle/);
  assert.match(css, /\.op-status-strip__cell--blocked\s+\.op-status-strip__value\s*\{[^}]*--color-state-blocked/);
  // Markup also needs the modifiers wired so the CSS selectors actually match.
  const indexHtml = readFileSync(resolve(here, "../index.html"), "utf8");
  assert.match(indexHtml, /op-status-strip__cell\s+op-status-strip__cell--active/);
  assert.match(indexHtml, /op-status-strip__cell\s+op-status-strip__cell--idle/);
});

test("agent cards style all four Living Telemetry states (active / blocked / idle / done)", () => {
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  // Each of the four states must have a distinct visual treatment so operators
  // can scan card boundaries at a glance — not just the high-priority pair.
  assert.match(css, /\.op-agent-card\[data-state="active"\]\s*\{[^}]*--color-state-active/);
  assert.match(css, /\.op-agent-card\[data-state="blocked"\]\s*\{[^}]*--color-state-blocked/);
  assert.match(css, /\.op-agent-card\[data-state="idle"\]\s*\{[^}]*--color-state-idle/);
  assert.match(css, /\.op-agent-card\[data-state="done"\]\s*\{[^}]*--color-state-done/);
  // The chip labels also need the matching foreground tint per state.
  assert.match(css, /\.op-agent-card\[data-state="idle"\]\s*\.op-agent-state\s*\{[^}]*--color-state-idle/);
  assert.match(css, /\.op-agent-card\[data-state="done"\]\s*\.op-agent-state\s*\{[^}]*--color-state-done/);
});

test("terminal surface body stays on the dark Operator canvas across themes (FR-013)", () => {
  // SPEC-2356 FR-013 改定: terminal window の body 領域 (.window-body と
  // terminal-root padding) は xterm の Dark Operator background と連続させ
  // るため、Light テーマでも Dark Operator background に固定する。Light で
  // --color-canvas (#e9edf0) が見えると xterm 周囲に「外枠」が現れて二重
  // 枠になるので、ここで結合してしまうのが正解。
  const surfaceTerminalRule =
    /\.surface-terminal\s+\.window-body\s*\{[^}]*background:\s*([^;]+);/;
  const match = inlineStyle.match(surfaceTerminalRule);
  assert.ok(match, "expected .surface-terminal .window-body rule with explicit background");
  const value = match[1].trim();
  assert.doesNotMatch(
    value,
    /var\(\s*--color-canvas\s*\)/,
    `.surface-terminal .window-body must not follow --color-canvas (got "${value}")`,
  );
  assert.doesNotMatch(
    value,
    /var\(\s*--color-surface(?:-elevated)?\s*\)/,
    `.surface-terminal .window-body must not follow surface tokens (got "${value}")`,
  );
  // Accept either an explicit dark hex (Dark Operator canvas) or a dedicated
  // dark canvas token.  We codify the value contract so the regression cannot
  // silently flip back to a light token.
  const usesDarkOperatorBackground =
    /^#0a0d12$/i.test(value) || /var\(\s*--color-canvas-dark\s*\)/.test(value);
  assert.ok(
    usesDarkOperatorBackground,
    `.surface-terminal .window-body must use Dark Operator canvas (#0a0d12 or --color-canvas-dark); got "${value}"`,
  );
});

test("terminal root spacing stays inside the window body (SPEC-2008 FR-060)", () => {
  const terminalRootRule = inlineStyle.match(/\.terminal-root\s*\{([^}]*)\}/);
  assert.ok(terminalRootRule, "expected .terminal-root CSS rule");
  const body = terminalRootRule[1];

  assert.match(
    body,
    /position:\s*absolute/,
    ".terminal-root must remain absolutely positioned inside .window-body",
  );
  assert.match(
    body,
    /inset:\s*8px\s+10px\s+10px\s*;/,
    ".terminal-root must express terminal chrome spacing as inset so its outer box stays inside .window-body",
  );
  assert.match(
    body,
    /overflow:\s*hidden/,
    ".terminal-root must clip xterm internals within the in-bounds terminal host box",
  );
});

test("terminal root does not combine full inset with padding overflow (SPEC-2008 SC-039)", () => {
  const terminalRootRule = inlineStyle.match(/\.terminal-root\s*\{([^}]*)\}/);
  assert.ok(terminalRootRule, "expected .terminal-root CSS rule");
  const body = terminalRootRule[1];

  assert.doesNotMatch(
    body,
    /inset:\s*0\s*;[\s\S]*padding:\s*(?!0\b)[^;]+;/,
    ".terminal-root must not use inset:0 plus nonzero padding; content-box padding extends past .window-body and clips the bottom prompt",
  );
  assert.match(
    body,
    /padding:\s*0\s*;/,
    ".terminal-root padding must stay zero so FitAddon measures the same box xterm can paint into",
  );
});

test("non-terminal surface bodies still follow the overall theme (FR-013 boundary)", () => {
  // The Dark fix is scoped to .surface-terminal.  Other surfaces (Board /
  // Logs / File Tree / Branches / Knowledge / Mock / Profile) must
  // keep tracking the active theme via --color-surface so tabbed windows
  // still flip body color when a non-terminal tab is selected.
  const otherSurfaceRule =
    /(?:\.surface-(?:file-tree|branches|board|logs|knowledge|mock|profile)\s+\.window-body,?\s*)+\{[^}]*background:\s*var\(\s*--color-surface\s*\)/;
  assert.match(
    inlineStyle,
    otherSurfaceRule,
    "non-terminal surface bodies must continue to use var(--color-surface)",
  );
});

test("Sidebar Layer buttons reset UA chrome so Windows WebView2 stops drawing default border (FR-030)", () => {
  // SPEC-2356 FR-030 / US-4 AS-11: WebView2 / Chromium の `<button>` UA
  // default は border + grey background を出す。`.op-layer` は indicator
  // dot + label color + token-driven focus ring のみで状態を表現するため、
  // base rule で UA chrome を解除する必要がある。
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  const layerRule = css.match(/\.op-layer\s*\{([^}]*)\}/);
  assert.ok(layerRule, "expected base .op-layer rule in components.css");
  const body = layerRule[1];
  assert.match(
    body,
    /appearance:\s*none/,
    ".op-layer must declare appearance:none to disable UA <button> chrome",
  );
  assert.match(
    body,
    /border:\s*0/,
    ".op-layer must zero out border so WebView2 stops drawing the default frame",
  );
  assert.match(
    body,
    /background:\s*transparent/,
    ".op-layer must clear background so the sidebar surface shows through",
  );
});
