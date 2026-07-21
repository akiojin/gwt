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
// SPEC-3064 Phase 3 (E5): the Launch Wizard surface (state, interaction
// guard, builders, renderLaunchWizard, chrome listeners) moved from app.js
// to launch-wizard-surface.js; wizard render/source patterns are pinned
// against the extracted module while app.js keeps thin delegates.
const launchWizardSource = readFileSync(
  resolve(here, "../launch-wizard-surface.js"),
  "utf8",
);
// SPEC-3064 Phase 3 (E6a): the File Tree window surface moved from app.js
// to file-tree-surface.js.
const fileTreeSurfaceSource = readFileSync(
  resolve(here, "../file-tree-surface.js"),
  "utf8",
);
// SPEC-3064 Phase 3 (E6b): the Branches window & cleanup surface moved from
// app.js to branches-cleanup-surface.js.
const branchesCleanupSource = readFileSync(
  resolve(here, "../branches-cleanup-surface.js"),
  "utf8",
);
// SPEC-3064 Phase 3 (E6c): the Board & Logs window surface moved from
// app.js to board-logs-surface.js.
const boardLogsSurfaceSource = readFileSync(
  resolve(here, "../board-logs-surface.js"),
  "utf8",
);
// SPEC-3064 Phase 3 (E6d): the Knowledge Bridge (Kanban) window surface
// moved from app.js to knowledge-kanban-surface.js.
const knowledgeKanbanSurfaceSource = readFileSync(
  resolve(here, "../knowledge-kanban-surface.js"),
  "utf8",
);
// SPEC-3064 Phase 3 (E6e): the Profile window surface moved from app.js
// to profile-window-surface.js.
const profileWindowSurfaceSource = readFileSync(
  resolve(here, "../profile-window-surface.js"),
  "utf8",
);
// SPEC-3064 Phase 3 (E7): the project & workspace shell chrome (project
// tabs, recent projects + open-project menu, picker/onboarding, window list
// dropdown, maximized viewport sync, clone/migration modal glue) moved from
// app.js to project-shell-surface.js.
const projectShellSurfaceSource = readFileSync(
  resolve(here, "../project-shell-surface.js"),
  "utf8",
);
const projectTabsRendererSource = readFileSync(
  resolve(here, "../project-tabs-renderer.js"),
  "utf8",
);
const windowTabsRendererSource = readFileSync(
  resolve(here, "../window-tabs-renderer.js"),
  "utf8",
);
const branchCleanupSource = readFileSync(resolve(here, "../branch-cleanup-modal.js"), "utf8");
const branchListStateSource = readFileSync(resolve(here, "../branch-list-state.js"), "utf8");
const windowDockingSource = readFileSync(resolve(here, "../window-docking.js"), "utf8");
const workspaceOverviewPath = resolve(here, "../workspace-kanban-surface.js");
const workspaceOverviewSource = existsSync(workspaceOverviewPath)
  ? readFileSync(workspaceOverviewPath, "utf8")
  : "";
const appAndProjectTabsSource = `${appSource}\n${projectTabsRendererSource}`;
const typographySource = readFileSync(resolve(here, "../styles/typography.css"), "utf8");
// Issue #2694 Phase D: the formerly-inline <style> block now lives at
// /styles/app.css and is loaded via `<link rel="stylesheet">`. The grep
// surface used by the CSS contract tests below remains stable.
const inlineStyle = readFileSync(resolve(here, "../styles/app.css"), "utf8");
const componentsStyle = readFileSync(resolve(here, "../styles/components.css"), "utf8");
const frontendStyle = `${inlineStyle}\n${componentsStyle}`;

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

// merged from origin/develop (SPEC-1939): extract a named function's body by
// brace-depth tracking so the perf hot-path assertions can scan its source.
function extractFunctionBody(source, name) {
  const start = source.indexOf(`function ${name}(`);
  assert.notEqual(start, -1, `expected function ${name} in source`);
  const paramsOpen = source.indexOf("(", start);
  assert.notEqual(paramsOpen, -1, `expected function ${name} parameters`);
  let parenDepth = 0;
  let paramsClose = -1;
  for (let i = paramsOpen; i < source.length; i += 1) {
    const char = source[i];
    if (char === "(") parenDepth += 1;
    if (char === ")") {
      parenDepth -= 1;
      if (parenDepth === 0) {
        paramsClose = i;
        break;
      }
    }
  }
  assert.notEqual(paramsClose, -1, `expected function ${name} parameter close`);
  const open = source.indexOf("{", paramsClose);
  assert.notEqual(open, -1, `expected function ${name} body`);
  let depth = 0;
  for (let i = open; i < source.length; i += 1) {
    const char = source[i];
    if (char === "{") depth += 1;
    if (char === "}") {
      depth -= 1;
      if (depth === 0) {
        return source.slice(open + 1, i);
      }
    }
  }
  assert.fail(`unterminated function ${name}`);
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

function extractContainerBlocks(css, condition) {
  const out = [];
  const marker = `@container`;
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
    ".op-rail",
    ".op-status-strip",
    "#op-strip-clock",
    "#op-strip-running",
    "#op-strip-idle",
    "#op-strip-waiting",
    "#op-strip-error",
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

test("SPEC-3038 Command Rail retires the legacy sidebar entirely", () => {
  // SPEC-3038 US-1: the 240px auto-hide overlay sidebar (`.op-sidebar`) and
  // its checkbox-style `.op-layer` rows are replaced by the always-visible
  // 56px Command Rail. No legacy sidebar scaffolding may remain.
  assert.equal(document.querySelector(".op-sidebar"), null, ".op-sidebar must be removed");
  assert.equal(
    document.querySelectorAll(".op-layer").length,
    0,
    ".op-layer rows must be replaced by .op-rail__item buttons",
  );
  assert.equal(
    document.querySelector(".op-sidebar__heading"),
    null,
    "sidebar section headings must be removed — rail groups carry aria-labels instead",
  );
  for (const id of [
    "op-layer-count-agents",
    "op-layer-count-git",
    "op-layer-count-hooks",
    "op-sidebar-count",
  ]) {
    assert.equal(document.getElementById(id), null, `${id} counter must be removed`);
  }
});

// SPEC-3245 Phase 3: Start Work is removed from the command rail and palette;
// the 2-lane entries (Intake / Open Workspace) replace it.
test("command rail and palette drop Start Work in favor of the 2-lane entries", () => {
  assert.equal(
    document.querySelector('.op-rail .op-rail__item[data-cmd="start-work"]'),
    null,
    "the Command Rail must not expose a Start Work action",
  );
  assert.doesNotMatch(
    operatorShellSource,
    /id:\s*"start-work"/,
    "Command Palette registry must not include start-work",
  );
  assert.doesNotMatch(
    operatorShellSource,
    /id:\s*"spawn-agent"/,
    "Command Palette registry must not include the spawn-agent alias of Start Work",
  );
  assert.doesNotMatch(
    appSource,
    /case\s+"start-work":/,
    "app.js must not route a start-work command",
  );
  // The replacements exist.
  assert.match(operatorShellSource, /id:\s*"intake-session"/, "Intake entry present");
  assert.match(operatorShellSource, /id:\s*"open-branches"/, "Open Workspace entry present");
});

// SPEC-3214 Phase 3 / SPEC-3245 Phase 4: the command palette exposes an
// "Intake" entry (the "session" suffix is dropped to avoid colliding with the
// Session domain term) that sends open_intake_session (the ephemeral,
// branchless new-work entry).
test("command palette exposes Intake wired to open_intake_session", () => {
  assert.match(
    operatorShellSource,
    /id:\s*"intake-session"[\s\S]+label:\s*"Intake"/,
    "expected Command Palette registry to include the Intake entry",
  );
  assert.match(
    appSource,
    /case\s+"intake-session":[\s\S]+kind:\s*"open_intake_session"/,
    "expected Intake session command to send open_intake_session",
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
  assert.match(
    appSource,
    /counts\.branches[\s\S]+activeWorks\.length/,
    "expected Status Strip Work telemetry to count active Works, not only branch-list rows",
  );
});

test("Status Strip RUNNING counter filters non-agent preset windows", () => {
  // SPEC-2356 follow-up: recomputeOperatorTelemetry walked windowMap.values()
  // without checking preset, so every workspace window with data-agent-state
  // (Board / Workspace / Logs / Branches / etc.) inflated counts.agents and
  // the Status Strip RUNNING cell showed e.g. 4 when only 2 agent panes were
  // live. The DOM walk must scope to presets that represent agent panes.
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
    "recomputeOperatorTelemetry must filter via presetSupportsWaitingStatus(preset) so non-agent windows do not inflate the Status Strip ACTIVE counter",
  );
});

test("Status Strip does not count empty active Work projection as an idle agent", () => {
  const body = extractFunctionBody(appSource, "recomputeOperatorTelemetry");
  assert.doesNotMatch(
    body,
    /category\s*===\s*"idle"[\s\S]{0,120}counts\.idle\s*=\s*Math\.max\(counts\.idle,\s*1\)/,
    "an active_work_projection with zero agents must not make the Status Strip show IDLE 1",
  );
  assert.match(
    body,
    /category\s*===\s*"idle"[\s\S]{0,180}activeAgents\s*>\s*0[\s\S]{0,180}counts\.idle/,
    "the idle fallback must be guarded by a real active agent count",
  );
});

test("Sidebar retires the Active Works overview in favor of the Work surface (SPEC-2359 W-12 FR-351)", () => {
  // Slice 3 aggregates Work lifecycle into the Work surface (Workspace
  // Overview / Kanban). The sidebar no longer renders an Active Works region
  // or its per-agent cards.
  assert.equal(
    document.querySelector("#op-active-work"),
    null,
    "sidebar Active Works overview must be removed",
  );
  assert.equal(
    document.querySelector("#op-active-work-agents"),
    null,
    "sidebar per-Work agent list must be removed",
  );
  assert.doesNotMatch(
    appSource,
    /function\s+renderActiveWorkOverview\b/,
    "sidebar renderActiveWorkOverview must be removed",
  );
  assert.doesNotMatch(
    appSource,
    /function\s+renderActiveWorkAgentCard\b/,
    "sidebar renderActiveWorkAgentCard must be removed",
  );
  // The Work projection still feeds Operator telemetry through the plural
  // active_works data even though the sidebar overview is gone.
  assert.match(
    appSource,
    /function\s+activeWorkItemsFromProjection\(\)[\s\S]+active_works/,
    "telemetry must still derive from plural active_works data",
  );
});

test("Command Rail groups items into Navigate / Windows / Agents / System (SPEC-3038 US-1)", () => {
  // SPEC-3038 US-1: the rail is the single always-visible home for navigation
  // (Start Work / Work / Board / Logs), window operations (Tile / Stack /
  // Align / Windows / Add), and system actions (Palette / Update) — in that
  // order. Groups carry aria-labels instead of visual headings. SPEC-2356
  // Anshin (FR-042) inserts an Agents group (STOP ALL) before System.
  const groups = Array.from(document.querySelectorAll(".op-rail > .op-rail__group")).map(
    (group) => group.getAttribute("aria-label"),
  );
  // User verification 2026-06-12: the Update CTA returned to its fixed
  // bottom-right home, so the sidebar no longer carries an Update section.
  assert.deepEqual(
    groups,
    ["Navigate", "Windows", "Agents", "System"],
    "Rail order must be Navigate → Windows → Agents → System",
  );
});

test("Command Rail Windows group carries the window-operation controls (SPEC-3038 US-1)", () => {
  const windowsGroup = document.querySelector('.op-rail > .op-rail__group[aria-label="Windows"]');
  assert.ok(windowsGroup, "expected a Windows group in the rail");
  for (const id of ["tile-button", "stack-button", "align-button", "window-list-button", "add-button"]) {
    const control = document.getElementById(id);
    assert.ok(control, `expected ${id}`);
    assert.ok(
      windowsGroup.contains(control),
      `${id} must live inside the rail Windows group`,
    );
    assert.ok(
      control.classList.contains("op-rail__item"),
      `${id} must adopt the rail item vocabulary`,
    );
  }
  // The window-list popover rides along with its trigger.
  assert.ok(
    windowsGroup.querySelector("#window-list-panel"),
    "window-list-panel must anchor to its rail trigger",
  );
  // AS-1.4: the Windows item carries an open-window count badge.
  const badge = windowsGroup.querySelector("#op-rail-window-count.op-rail__badge");
  assert.ok(badge, "expected the Windows rail item to expose a window-count badge");
});

test("Command Rail System group hosts the Command Palette trigger (SPEC-3038 US-1)", () => {
  const systemGroup = document.querySelector('.op-rail > .op-rail__group[aria-label="System"]');
  assert.ok(systemGroup, "expected a System group in the rail");
  const trigger = document.getElementById("op-palette-button");
  assert.ok(trigger, "expected op-palette-button");
  assert.ok(systemGroup.contains(trigger), "Command Palette trigger must live in the rail");
});

test("Right-bottom floating controls are removed; the canvas corner is empty (SPEC-2356)", () => {
  // The whole `.floating-actions` / `#floating-window-controls` toolbar and the
  // window-controls peek 帯 are gone. Nothing floats over the bottom-right of
  // the canvas anymore.
  assert.equal(
    document.querySelector(".floating-actions"),
    null,
    ".floating-actions toolbar must be removed",
  );
  assert.equal(
    document.getElementById("floating-window-controls"),
    null,
    "#floating-window-controls root must be removed",
  );
  assert.equal(
    document.getElementById("floating-window-controls-actions"),
    null,
    "floating-window-controls-actions group must be removed",
  );
  assert.equal(
    document.querySelector(".op-window-controls-peek"),
    null,
    "window controls peek 帯 must be removed — the sidebar peek reveals the controls now",
  );
});

test("Status Strip hosts the zoom controls so canvas zoom is always reachable (SPEC-2356)", () => {
  const strip = document.getElementById("op-status-strip");
  assert.ok(strip, "expected op-status-strip");
  for (const id of ["zoom-out-button", "zoom-reset-button", "zoom-in-button"]) {
    const control = document.getElementById(id);
    assert.ok(control, `expected ${id}`);
    assert.ok(strip.contains(control), `${id} must live in the Status Strip`);
  }
  // Labels are preserved so the existing app.js zoom handlers stay wired by id.
  assert.equal(document.getElementById("zoom-reset-button").textContent.trim(), "100%");
});

test("Update CTA and alerts share one fixed bottom-right layout host", () => {
  // SPEC-2356 moved the Update CTA into the sidebar and SPEC-3038 briefly
  // anchored it to the rail, but the user found chrome-docked placements hard
  // to notice. SPEC-2041 Phase 23 retains the bottom-right home while making
  // one roleless layout host own the corner for both alerts and the CTA.
  assert.equal(
    document.getElementById("update-cta-anchor"),
    null,
    "no chrome-docked update anchor may remain",
  );
  const noticeHost = document.getElementById("operator-notice-stack");
  assert.ok(noticeHost, "expected the shared operator notice stack");
  assert.equal(noticeHost.parentElement, document.body, "notice host is a body child");
  assert.equal(noticeHost.hasAttribute("role"), false, "layout host has no ARIA role");
  assert.equal(
    noticeHost.hasAttribute("aria-live"),
    false,
    "layout host is not a live region",
  );

  const updateCtaSource = readFileSync(resolve(here, "../update-cta.js"), "utf8");
  assert.doesNotMatch(
    updateCtaSource,
    /update-cta-anchor/,
    "update-cta.js does not return to a chrome anchor",
  );
  assert.match(updateCtaSource, /getElementById\(["']operator-notice-stack["']\)/);
  assert.match(appSource, /getElementById\(["']operator-notice-stack["']\)/);
  assert.match(appSource, /alertsToasts\.mount\(/);

  const hostBlock = inlineStyle.match(/\.operator-notice-stack\s*\{[^}]*\}/)?.[0];
  assert.ok(hostBlock, "expected operator notice stack CSS");
  assert.match(hostBlock, /position:\s*fixed/);
  assert.match(hostBlock, /\bright\s*:/);
  assert.match(hostBlock, /\bbottom\s*:/);
  assert.match(hostBlock, /z-index\s*:/);

  const alertsBlock = inlineStyle.match(/\.toast-alerts\s*\{[^}]*\}/)?.[0];
  assert.ok(alertsBlock, "expected alerts lane CSS");
  assert.doesNotMatch(alertsBlock, /position:\s*fixed/);
  assert.doesNotMatch(alertsBlock, /\b(?:right|bottom|z-index)\s*:/);

  const componentsCss = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  const ctaShellBlock = componentsCss.match(/\.update-cta-shell\s*\{[^}]*\}/)?.[0];
  assert.ok(ctaShellBlock, "expected update CTA lane CSS");
  assert.doesNotMatch(ctaShellBlock, /position:\s*fixed/);
  assert.doesNotMatch(ctaShellBlock, /\b(?:right|bottom|z-index)\s*:/);

  const tokensCss = readFileSync(resolve(here, "../styles/tokens.css"), "utf8");
  const definedTokens = new Set();
  for (const source of [tokensCss, inlineStyle]) {
    for (const match of source.matchAll(/(--[a-z0-9-]+)\s*:/g)) {
      definedTokens.add(match[1]);
    }
  }
  for (const match of hostBlock.matchAll(/var\(\s*(--[a-z0-9-]+)/g)) {
    assert.ok(definedTokens.has(match[1]), `undefined notice host token ${match[1]}`);
  }
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
    projectShellSurfaceSource,
    /window-list-role/,
    "expected window list rows to include a role badge",
  );
});

test("workspace windows expose lane badges as a separate contract from agent color", () => {
  assert.match(
    appSource,
    /from "\/window-lane-identity\.js"/,
    "app.js must import lane identity helpers",
  );
  assert.match(
    appSource,
    /class="window-lane-badge"/,
    "titlebar template must include a lane badge separate from the role badge",
  );
  assert.match(
    appSource,
    /applyWindowLaneData\(element,\s*windowData\)/,
    "window root must carry data-lane-kind",
  );
  assert.match(
    appSource,
    /appendRenderKeyPart\(parts,\s*windowLaneKind\(windowData\)\)/,
    "workspace window render keys must use the same lane normalization as the badge",
  );
  assert.match(
    projectShellSurfaceSource,
    /window-list-lane/,
    "window list rows must include the same lane badge contract",
  );
  assert.match(
    projectShellSurfaceSource,
    /appendRenderKeyPart\(parts,\s*windowLaneKind\(entry\)\)/,
    "window list render keys must use the same lane normalization as the badge",
  );
  assert.match(
    inlineStyle,
    /\.window-lane-badge\s*\{[\s\S]*border:\s*1px solid var\(--color-border/,
    "lane badges must use Operator tokens, not raw colors",
  );
  assert.match(
    frontendStyle,
    /\.fleet-minimap__cell\[data-lane-symbol\]::before/,
    "minimap cells must render a compact lane marker",
  );
});

test("Agent title role badges resolve runtime identity instead of generic presets", () => {
  assert.match(
    appSource,
    /const\s+AGENT_ROLE_LABELS\s*=\s*Object\.freeze\(\{[\s\S]*claude:\s*"Claude Code"[\s\S]*codex:\s*"Codex"/,
    "expected Agent role badges to map runtime ids to display names",
  );
  assert.match(
    appSource,
    /function\s+agentRoleLabel\(windowData\)[\s\S]+windowData\?\.agent_id[\s\S]+AGENT_ROLE_LABELS/,
    "expected Agent role badge labels to resolve from window.agent_id",
  );
  assert.match(
    appSource,
    /function\s+windowRoleBadgeLabel\(windowData\)[\s\S]+agentRoleLabel\(windowData\)/,
    "expected titlebar and list role badges to share an Agent-aware label helper",
  );
  assert.doesNotMatch(
    appSource,
    /window-role-badge"\)\.textContent\s*=\s*presetRoleLabel\(windowData\.preset\)/,
    "titlebar role badge must not show the generic Agent preset label",
  );
  assert.doesNotMatch(
    appSource,
    /window-list-role">\$\{presetRoleLabel\(entry\.preset\)\}/,
    "window list role badge must not show the generic Agent preset label",
  );
});

test("Non-Agent duplicate title role badges are omitted", () => {
  assert.match(
    appSource,
    /function\s+windowRoleBadgeLabel\(windowData\)[\s\S]+windowDisplayTitle\(windowData\)[\s\S]+return\s+""/,
    "expected duplicate role badges such as Branches / Branches to be suppressed",
  );
  assert.match(
    appSource,
    /function\s+setWindowRoleBadge\(badgeElement,\s*windowData\)[\s\S]+badgeElement\.hidden\s*=\s*!label/,
    "expected titlebar role badge updates to hide empty labels",
  );
  assert.match(
    inlineStyle,
    /\.window-role-badge\[hidden\]\s*\{[\s\S]*display:\s*none/,
    "hidden role badges must stay hidden even though .window-role-badge uses inline-flex",
  );
});

test("Agent fallback runtime titles still show Agent role badges", () => {
  assert.match(
    appSource,
    /function\s+windowRoleBadgeLabel\(windowData\)[\s\S]+const\s+isAgentWindow\s*=\s*isAgentWindowPreset\(windowData\?\.preset\)/,
    "expected duplicate suppression to distinguish Agent windows from static panels",
  );
  assert.match(
    appSource,
    /if\s*\(!isAgentWindow\s*&&\s*label\s*===\s*displayTitle\)\s*return\s+""/,
    "Agent role badges must remain visible when the fallback title is the same runtime name",
  );
  assert.doesNotMatch(
    appSource,
    /if\s*\(!label\s*\|\|\s*label\s*===\s*displayTitle\)\s*return\s+""/,
    "generic duplicate suppression must not hide Agent role badges",
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

test("Board deep-linking survives the Active Works sidebar retirement (SPEC-2359 W-12 FR-351)", () => {
  // The sidebar Active Works command-center is removed; Board deep-linking and
  // the addressable Board timeline entries it relied on remain so the Work
  // surface and Board can still cross-reference coordination entries.
  // SPEC-3064 Phase 3 (E6c): the Board renderers moved into the board &
  // logs surface module.
  assert.match(
    boardLogsSurfaceSource,
    /function\s+focusBoardEntry\(/,
    "expected Board actions to deep-link to the referenced coordination entry",
  );
  assert.match(
    boardLogsSurfaceSource,
    /data-board-entry-id/,
    "expected Board timeline entries to be addressable from Workspace links",
  );
  // The retired sidebar copy and per-agent card helpers must be gone.
  assert.doesNotMatch(
    appSource,
    /Add Agent to This Work/,
    "sidebar Active Works launch copy must be removed",
  );
  assert.doesNotMatch(
    appSource,
    /function\s+renderActiveWorkAgentCard\b/,
    "sidebar per-agent card renderer must be removed",
  );
});

test("Active Work sidebar title + visibility helpers are retired (SPEC-2359 W-12 FR-351)", () => {
  // Slice 3 removes the sidebar-only Active Works render path. The display
  // title, section-visibility, and focus helpers it owned must be gone; the
  // Work surface (Kanban) is now the single home for Work cards.
  for (const removed of [
    /function\s+activeWorkDisplayTitle\b/,
    /function\s+setActiveWorkSectionVisible\b/,
    /function\s+focusActiveWorkAgentWindow\b/,
    /function\s+agentRuntimeStatusLabel\b/,
    /function\s+renderActiveWorkOverview\b/,
    /getElementById\("op-active-work"\)/,
  ]) {
    assert.doesNotMatch(appSource, removed, `sidebar Active Works code must be removed: ${removed}`);
  }
  // The focusable-agent filter survives because telemetry still derives from
  // live agent windows.
  assert.match(
    appSource,
    /function\s+activeWorkFocusableAgents\(work\)[\s\S]+workspaceWindowById\(agent\.window_id\)/,
    "telemetry must still filter Works against live workspace windows",
  );
  // agentStatusLabel is exported to the Work surface, so it must remain.
  assert.match(
    appSource,
    /function\s+agentStatusLabel\(state\)[\s\S]+Running[\s\S]+Blocked[\s\S]+Idle[\s\S]+Done/,
    "shared agentStatusLabel must remain for the Work surface",
  );
});

test("Launch Wizard focus start method renders backend-provided running session detail", () => {
  assert.match(
    launchWizardSource,
    /for\s*\(\s*const method of launchWizard\.start_methods \|\| \[\]\s*\)/,
    "expected Launch Wizard to render backend-provided start methods",
  );
  assert.match(
    launchWizardSource,
    /const detail = method\.enabled === false[\s\S]*?method\.disabled_reason[\s\S]*?: method\.detail;[\s\S]*?createNode\("div", "start-method-detail", detail\)/,
    "expected running-session Focus details to come from the backend start method payload",
  );
});

test("Launch Wizard keeps session mode choices in Start methods, not settings", () => {
  assert.match(
    launchWizardSource,
    /for\s*\(\s*const method of launchWizard\.start_methods \|\| \[\]\s*\)/,
    "expected session-oriented actions to render through Start methods",
  );
  assert.doesNotMatch(
    launchWizardSource,
    /appendChoiceOrSelectField\(\s*grid,\s*"Execution mode"/,
    "Execution mode must not be rendered as a Manual setup setting",
  );
});

test("Launch Wizard groups Start methods by backend UX priority", () => {
  assert.match(
    launchWizardSource,
    /const\s+startMethodGroups\s*=\s*\[[\s\S]*?recommended[\s\S]*?available[\s\S]*?unavailable/,
    "expected Start methods to render in Recommended / Available / Unavailable groups",
  );
  assert.match(
    launchWizardSource,
    /method\.group\s*\|\|[\s\S]*?method\.enabled === false[\s\S]*?unavailable/,
    "expected backend group metadata with enabled-state fallback",
  );
  assert.match(
    launchWizardSource,
    /method\.recommended[\s\S]*?start-method-button--recommended/,
    "expected recommended Start method to get a stronger visual treatment",
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
    /function\s+openWorkspaceOverview\(\)\s*\{[\s\S]{0,300}?focusOrSpawnPreset\("work"\)/,
    "expected Workspace Overview to open the Work window instead of a drawer",
  );
  assert.match(
    appSource,
    /op-workspace-overview-entry[\s\S]+openWorkspaceOverview/,
    "expected Sidebar Workspace Overview entry to open the overview",
  );
});

test("Workspace Overview uses the Quiet Work list filter + Detail layout", () => {
  assert.ok(
    workspaceOverviewSource.length > 0,
    "expected Workspace Overview renderer to live in workspace-kanban-surface.js",
  );
  assert.match(
    appSource,
    /from\s+"\/workspace-kanban-surface\.js"/,
    "expected app.js to import the Workspace Overview surface module",
  );
  assert.match(
    appSource,
    /presetSurface\(preset\)[\s\S]+preset\s*===\s*"work"[\s\S]+return\s+"work"/,
    "expected Work to be a first-class window surface",
  );
  for (const token of [
    "workspace-overview-root",
    "workspace-overview-list-pane",
    "workspace-overview-filter-bar",
    "workspace-overview-list",
    "workspace-overview-detail-pane",
  ]) {
    assert.match(
      workspaceOverviewSource,
      new RegExp(token),
      `expected Workspace Overview source to include ${token}`,
    );
  }
  assert.doesNotMatch(
    workspaceOverviewSource,
    /ATTENTION_LANES|workspace-attention-lane/,
    "Workspace Overview must not classify Work into visible Kanban lanes",
  );
  assert.match(
    workspaceOverviewSource,
    /workspace-agent-queue/,
    "expected unassigned agents to stay in a dedicated queue outside Workspace rows",
  );
  assert.match(
    workspaceOverviewSource,
    /function\s+workspacesFromProjection\([^)]*\)[\s\S]+projection\.works[\s\S]+projection\.workspaces[\s\S]+projection\.work_items/,
    "expected Work Overview to render Work entries from active_work_projection",
  );
  // SPEC-2359 US-42 — Resume action now opens the Workspace Resume
  // Picker via `list_resumable_agents` instead of the legacy
  // `resume_workspace` event. The picker drives the actual restart with
  // `resume_workspace_agent` after the user selects a candidate agent.
  assert.match(
    workspaceOverviewSource,
    /function\s+resumeWorkspace\([^)]*\)[\s\S]+list_resumable_agents/,
    "expected Workspace Resume action to ask the backend to list resumable agents",
  );
  assert.doesNotMatch(
    workspaceOverviewSource,
    /workspace-kanban-board|data-workspace-column|workspace-kanban-column/,
    "Workspace Overview must not reintroduce retired Workspace-specific Kanban columns",
  );
  assert.doesNotMatch(
    appSource,
    /function\s+(workspaceCardsFromProjection|renderWorkspaceKanbanCard|renderWorkspaceKanbanDetail)\(/,
    "Workspace Overview rendering internals should not remain in app.js",
  );
});

test("Workspace Overview root participates in full-window layout", () => {
  const fullWindowRootRule = inlineStyle.match(
    /\.file-tree-root,[\s\S]*?\.mock-root\s*\{[^}]+\}/,
  );
  assert.ok(fullWindowRootRule, "expected shared full-window root layout rule");
  assert.match(
    fullWindowRootRule[0],
    /\.workspace-overview-root/,
    "Workspace Overview root must fill the window body so list/detail panes can scroll",
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

test("no surface presets auto-maximize — uniform 720×420 floating windows", () => {
  assert.match(
    appSource,
    /function\s+isAutoMaximizedSurfacePreset\([^)]*\)\s*\{[^}]*return\s+false/,
    "expected isAutoMaximizedSurfacePreset to always return false (auto-maximize abolished)",
  );
});

test("Work surface renders PR metadata as links via the shared renderer (SPEC-2359 W-12 FR-351)", () => {
  assert.match(
    appSource,
    /function\s+createWorkspacePrMeta\(/,
    "expected a shared PR metadata renderer instead of duplicating string-only PR labels",
  );
  assert.match(
    workspaceOverviewSource,
    /createWorkspacePrMeta\?\.\(item\)/,
    "expected the Work surface to render the saved PR link/state from the projection",
  );
  assert.match(
    appSource,
    /PR #\$\{projection\.pr_number\}[\s\S]{0,200}?projection\.pr_url[\s\S]{0,160}?link\.href = projection\.pr_url/,
    "expected PR metadata to use the backend-provided URL for the link target",
  );
  assert.match(
    appSource,
    /appendMeta\(item,\s*projection\.pr_state\)/,
    "expected PR state to be displayed next to the PR link",
  );
});

test("Workspace Overview exposes user-confirmed cleanup for completed workspaces", () => {
  // SPEC-3064 Phase 3 (E6b): the cleanup entry moved into the branches
  // cleanup surface module.
  assert.match(
    branchesCleanupSource,
    /cleanup_candidate/,
    "expected active work projection cleanup_candidate to drive Workspace cleanup",
  );
  assert.match(
    branchesCleanupSource,
    /function\s+openWorkspaceCleanup\(/,
    "expected Workspace Overview to open a cleanup confirmation instead of deleting automatically",
  );
  assert.match(
    branchesCleanupSource,
    /default_delete_remote[\s\S]+deleteRemote\s*:\s*false|deleteRemote\s*:\s*false[\s\S]+default_delete_remote/,
    "expected Workspace cleanup to default remote deletion off",
  );
  assert.match(
    `${branchesCleanupSource}\n${branchCleanupSource}`,
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
  // FR-045 (anshin): the row now derives `displayTitle` from the shared helper
  // (so the activity line can compare against it) and escapes that value into
  // the title node. Assert both the helper binding and the escaped render.
  assert.match(
    projectShellSurfaceSource,
    /const\s+displayTitle\s*=\s*windowDisplayTitle\(entry\)/,
    "expected the Windows dropdown to bind the shared display-title helper",
  );
  assert.match(
    projectShellSurfaceSource,
    /window-list-title">\$\{escapeHtml\(displayTitle\)\}/,
    "expected the Windows dropdown to escape the shared display-title helper output",
  );
  assert.match(
    appSource,
    /title-text"\)\.textContent\s*=\s*windowDisplayTitle\(windowData\)/,
    "expected window titlebars to use the shared display-title helper",
  );
});

test("Branches preset is removed — branch browsing is part of the Work surface", () => {
  const branchPreset = document.querySelector('.preset-button[data-preset="branches"]');
  assert.equal(branchPreset, null, "Branches preset button must not exist — use Work surface");
  assert.doesNotMatch(
    `${html}\n${appSource}`,
    /Planning Session|Workspace[- ]card/i,
    "Work surface should not render Planning Session or Workspace-card concepts",
  );
});

test("Branches loading state becomes recoverable when the WebSocket disconnects", () => {
  // SPEC-3064 Phase 3 (E6b): the helper moved into the branches cleanup
  // surface; app.js setConnectionState keeps the call sites.
  assert.match(
    branchesCleanupSource,
    /function\s+failLoadingBranchesOnConnectionLoss\(windowId,\s*state\)/,
    "expected a dedicated Branches loading connection-loss helper",
  );
  // SPEC-2009 FR-064/FR-065: the connection-loss transition lives in the
  // extracted branch-list-state module (markBranchDetailInterrupted).
  assert.match(
    branchListStateSource,
    /function\s+markBranchDetailInterrupted\(state\)[\s\S]+state\.loading\s*=\s*false[\s\S]+state\.receivedFreshEntries\s*=\s*false/,
    "expected connection loss to clear stale Branches loading flags",
  );
  assert.match(
    branchListStateSource,
    /Connection lost while loading branches/,
    "expected initial branch inventory loss to surface a retryable error",
  );
  assert.match(
    appSource,
    /function\s+setConnectionState\(connected\)[\s\S]+failLoadingBranchesOnConnectionLoss\(windowId,\s*state\)[\s\S]+renderBranches\(windowId\)/,
    "expected socket disconnect to re-render Branches after clearing stale loading",
  );
  // SPEC-2009 FR-064: reconnect self-heal re-requests interrupted Branches
  // windows automatically — no manual Refresh needed.
  assert.match(
    appSource,
    /branchWindowNeedsResync\(state\)[\s\S]+requestBranches\(windowId\)/,
    "expected reconnect to auto re-hydrate interrupted Branches windows",
  );
});

test("Branches detail-check state self-heals and retains last-known cleanup safety", () => {
  assert.match(
    branchListStateSource,
    /export\s+function\s+branchLoadStatusSummary\(state\)/,
    "expected Branches to derive a status summary from branch load state",
  );
  // SPEC-2009 FR-066: the interrupted detail check self-heals on reconnect, so
  // the copy is reassuring rather than an alarming manual-refresh banner.
  for (const copy of [
    "Checking branch details",
    "Reconnecting branch details",
    "Recovering automatically",
  ]) {
    assert.ok(
      branchListStateSource.includes(copy),
      `expected Branches clarity copy: ${copy}`,
    );
  }
  // First-load fallback only; carried-over rows keep their real badge.
  assert.ok(appSource.includes("Safety unknown"), "expected first-load fallback copy");
  // FR-066: the manual "Refresh to verify cleanup safety" copy is gone for good.
  assert.ok(
    !appSource.includes("Refresh to verify cleanup safety") &&
      !branchListStateSource.includes("Refresh to verify cleanup safety"),
    "expected the manual-refresh detail-check banner copy to be removed",
  );
  // FR-065: carried last-known badges are flagged stale so destructive
  // selection stays gated on fresh verification.
  assert.match(
    branchListStateSource,
    /cleanup_stale/,
    "expected last-known retention to flag carried cleanup badges",
  );
  assert.doesNotMatch(
    appSource,
    /Cleanup status unavailable/,
    "Branches rows should explain unknown safety instead of showing an unavailable cleanup placeholder",
  );
  assert.doesNotMatch(
    appSource,
    /state\.loading\s*\?\s*"loading"\s*:\s*"unknown"/,
    "Branches cleanup badges should not collapse interrupted hydration into an ambiguous unknown label",
  );
  for (const selector of [
    ".branch-notice-title",
    ".branch-notice-detail",
    ".branch-notice-hint",
  ]) {
    assert.ok(inlineStyle.includes(selector), `missing Branches status summary CSS: ${selector}`);
  }
  assert.match(
    inlineStyle,
    /\.branch-notice\[hidden\]\s*\{[\s\S]*display:\s*none/,
    "hidden Branches notices must not leave an empty status band behind",
  );
});

test("Branches detail-check checking state uses motion without animating static states", () => {
  assert.match(
    inlineStyle,
    /@keyframes\s+branch-detail-check-sweep/,
    "expected Branches checking summary to define a named sweep animation",
  );
  assert.match(
    inlineStyle,
    /@keyframes\s+branch-cleanup-checking-pulse/,
    "expected Branches checking badge to define a named pulse animation",
  );
  assert.match(
    inlineStyle,
    /\.branch-notice\[data-branch-status="checking"\]::before[\s\S]+animation:\s*branch-detail-check-sweep/,
    "expected only the checking summary notice to run the progress sweep",
  );
  assert.match(
    inlineStyle,
    /\.branch-cleanup-badge\.loading[\s\S]+animation:\s*branch-cleanup-checking-pulse/,
    "expected only checking cleanup badges to pulse",
  );

  const reducedMotion = extractMediaBlocks(inlineStyle, "prefers-reduced-motion: reduce");
  assert.match(
    reducedMotion,
    /branch-notice\[data-branch-status="checking"\]::before[\s\S]+animation:\s*none/,
    "reduced-motion users must not get the checking summary sweep",
  );
  assert.match(
    reducedMotion,
    /branch-cleanup-badge\.loading[\s\S]+animation:\s*none/,
    "reduced-motion users must not get the checking badge pulse",
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
  for (const id of [
    "op-strip-running",
    "op-strip-idle",
    "op-strip-waiting",
    "op-strip-error",
    "op-strip-branches",
    "op-strip-runtime-health-value",
  ]) {
    const el = document.getElementById(id);
    assert.ok(el, `expected element ${id}`);
    assert.ok(el.getAttribute("aria-label"), `${id} must have an aria-label`);
  }
  // clock cell is intentionally hidden from screen readers (per-second updates)
  const clockCell = document.getElementById("op-strip-clock")?.parentElement;
  assert.ok(clockCell, "clock cell exists");
  assert.equal(clockCell.getAttribute("aria-hidden"), "true");
});

test("Status Strip exposes a compact PERF cell for runtime health", () => {
  const cell = document.getElementById("op-strip-runtime-health");
  assert.ok(cell, "expected runtime health PERF cell");
  assert.match(cell.textContent ?? "", /PERF/);
  assert.equal(cell.getAttribute("aria-label"), "Runtime performance");
  assert.match(operatorShellSource, /applyRuntimeHealth/);
});

test("Runtime health PERF detail uses structured diagnostic classes", () => {
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  for (const token of [
    "runtimeHealthStateLabel",
    "op-runtime-health-detail__summary",
    "op-runtime-health-detail__chip",
    "op-runtime-health-detail__queue",
    "op-runtime-health-detail__process-list",
    "op-runtime-health-detail__process--focusable",
    "op-runtime-health-detail__process-role",
    "op-runtime-health-detail__process-name",
    "op-runtime-health-detail__process-metric",
  ]) {
    assert.match(operatorShellSource, new RegExp(token), `expected renderer token: ${token}`);
  }
  assert.match(
    operatorShellSource,
    /value\.textContent\s*=\s*`\$\{runtimeHealthStateLabel\(state\)\}\s+\$\{formatRuntimeCpu/,
    "compact PERF value must be severity-first",
  );
  assert.match(
    css,
    /\.op-status-strip__cell--runtime-health\s+\.op-status-strip__value\s*\{[\s\S]*min-width:/,
    "compact PERF value must reserve stable width",
  );
  assert.match(
    css,
    /\.op-runtime-health-detail__process\s*\{[\s\S]*display:\s*grid;[\s\S]*grid-template-columns:/,
    "process rows must use columns instead of a raw text line",
  );
  assert.match(
    operatorShellSource,
    /focusWindow/,
    "focusable runtime rows must call the injected focus callback",
  );
  assert.doesNotMatch(
    operatorShellSource,
    /op-runtime-health-detail[\s\S]{0,8000}(kill|terminate|stop_process|close_window)/i,
    "runtime health detail must not expose destructive process controls",
  );
});

test("Status Strip omits the retired server URL cell", () => {
  assert.equal(document.getElementById("op-strip-server-url"), null);
  assert.equal(document.getElementById("op-strip-server-url-copy"), null);
  assert.equal(document.querySelector(".op-status-strip__cell--server-url"), null);
  assert.doesNotMatch(appSource, /op-strip-server-url/);
  assert.doesNotMatch(appSource, /kind:\s*"open_server_url"/);
});

test("Status Strip labels the WebSocket connection state as ONLINE/OFFLINE", () => {
  const strip = document.getElementById("op-status-strip");
  const connectionLabel = strip?.querySelector("[data-role='connection-label']");
  assert.ok(connectionLabel, "expected a connection label in the status strip");
  assert.equal(connectionLabel.textContent?.trim(), "ONLINE");
  assert.doesNotMatch(html, />\s*LIVE\s*</);
  assert.match(appSource, /connectionStatusLabel\.textContent\s*=\s*connected\s*\?\s*"ONLINE"\s*:\s*"OFFLINE"/);
});

test("Command Rail keeps real shortcuts on its keyshortcut items (SPEC-3038 AS-1.2)", () => {
  // After the 2026-06-20 Update the Navigate group is Start Work + Workspace
  // only, so the Workspace rail item is the keyshortcut-bearing Navigate entry.
  const workspace = document.getElementById("op-workspace-overview-entry");
  assert.ok(workspace, "expected the Workspace rail item");
  assert.ok(
    workspace.classList.contains("op-rail__item"),
    "Workspace entry must be a rail item",
  );
  const shortcut = workspace.getAttribute("aria-keyshortcuts") ?? "";
  assert.match(shortcut, /Meta\+G/, "Workspace must declare its Meta+G shortcut");
  const kbd = workspace.querySelector(".op-rail__flyout kbd.op-rail__kbd");
  assert.ok(kbd, "Workspace must show a kbd hint inside its flyout");
});

test("Command Rail Navigate group drops Board / Logs but keeps their access paths (SPEC-3038 2026-06-20 Update)", () => {
  // User feedback (2026-06-20): Board / Logs are noise in the rail. They are
  // demoted out of the rail but stay reachable via the Add Window presets, the
  // command palette, and the ⌘B / ⌘L hotkeys — no capability is removed.
  assert.equal(
    document.querySelector('.op-rail .op-rail__item[data-cmd="open-board"]'),
    null,
    "Board must not appear as a Command Rail item",
  );
  assert.equal(
    document.querySelector('.op-rail .op-rail__item[data-cmd="open-logs"]'),
    null,
    "Logs must not appear as a Command Rail item",
  );
  // Access path 1: the Add Window preset deck still offers both surfaces.
  assert.ok(
    document.querySelector('[data-preset="board"]'),
    "Add Window must keep the Board preset",
  );
  assert.ok(
    document.querySelector('[data-preset="logs"]'),
    "Add Window must keep the Logs preset",
  );
  // Access paths 2 + 3: command palette seeds and ⌘B / ⌘L hotkeys stay wired.
  assert.match(operatorShellSource, /id:\s*"open-board"/, "palette must keep the Board entry");
  assert.match(operatorShellSource, /id:\s*"open-logs"/, "palette must keep the Logs entry");
  assert.match(operatorShellSource, /hotkey\.register\("cmd\+b"/, "⌘B hotkey must stay wired");
  assert.match(operatorShellSource, /hotkey\.register\("cmd\+l"/, "⌘L hotkey must stay wired");
});

test("Command Rail retires the pseudo kbd badges (SPEC-3038 FR-012)", () => {
  // GO / INFO looked like keyboard shortcuts but were not. kbd chips are
  // reserved for real shortcuts only.
  const kbdTexts = Array.from(document.querySelectorAll(".op-rail kbd")).map((el) =>
    el.textContent?.trim(),
  );
  for (const fake of ["GO", "INFO"]) {
    assert.ok(!kbdTexts.includes(fake), `pseudo kbd badge ${fake} must be removed`);
  }
});

test("Command Rail items are icon buttons with accessible names and flyout labels (SPEC-3038 AS-1.2/AS-1.3)", () => {
  const items = Array.from(document.querySelectorAll(".op-rail .op-rail__item"));
  // Navigate 2 (Start Work + Workspace) + Windows 5 + System 1 = 8 after the
  // 2026-06-20 Update removed Board / Logs from the rail.
  assert.ok(items.length >= 8, `expected >=8 rail items, got ${items.length}`);
  for (const item of items) {
    assert.ok(
      item.getAttribute("aria-label"),
      "every rail item must carry an aria-label (icon-only button)",
    );
    const icon = item.querySelector(".op-rail__icon");
    assert.ok(icon, "every rail item must render a Unicode symbol icon");
    assert.equal(
      icon.getAttribute("aria-hidden"),
      "true",
      "rail icons are decorative — aria-hidden",
    );
    const flyout = item.querySelector(".op-rail__flyout");
    assert.ok(flyout, "every rail item must provide a hover flyout label");
    assert.equal(
      flyout.getAttribute("aria-hidden"),
      "true",
      "flyout labels duplicate the aria-label — keep them aria-hidden",
    );
    assert.ok(
      flyout.querySelector(".op-rail__flyout-label")?.textContent?.trim(),
      "flyout must carry a readable label",
    );
  }
});

test("Command Palette trigger button declares aria-keyshortcuts", () => {
  const trigger = document.getElementById("op-palette-button");
  assert.ok(trigger, "palette trigger exists");
  const shortcut = trigger.getAttribute("aria-keyshortcuts") ?? "";
  assert.ok(shortcut.includes("Meta+K"), "trigger must declare Meta+K");
  assert.ok(shortcut.includes("Meta+P"), "trigger must declare Meta+P");
});

test("Command Rail is always visible — hover-reveal chrome is fully retired (SPEC-3038 AS-1.1/AS-1.6)", () => {
  // SPEC-3038 US-1: the rail is docked into the layout grid, so the peek 帯,
  // the chip toggles, and the data-op-sidebar reveal machinery are all gone.
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
    "<< chip toggle stays removed",
  );
  assert.equal(
    document.getElementById("op-window-controls-edge-toggle"),
    null,
    "vv chip toggle stays removed",
  );
  assert.equal(
    document.querySelector(".op-sidebar-peek"),
    null,
    "the 6px peek 帯 is retired — the rail is always visible",
  );
  assert.equal(
    document.querySelector(".op-window-controls-peek"),
    null,
    "window controls peek 帯 stays retired",
  );

  const rail = document.getElementById("op-rail");
  assert.ok(rail, "expected #op-rail");
  assert.ok(rail.classList.contains("op-rail"), "rail root carries .op-rail");
  assert.match(rail.getAttribute("aria-label") ?? "", /command rail/i);
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
    `${appSource}\n${windowTabsRendererSource}`,
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

test("modal buttons follow the ghost / primary / destructive hierarchy (SPEC-3038 FR-011 / US-5)", () => {
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  // Base wizard buttons become ghost — the solid dark-block treatment is
  // reserved for the single primary action per view.
  assert.match(
    css,
    /:root\[data-theme\] \.wizard-button\s*\{[^}]*background:\s*transparent/,
    "base .wizard-button must be ghost (transparent background)",
  );
  // Primary stays the single solid accent.
  assert.match(
    css,
    /:root\[data-theme\] \.wizard-button\.primary\s*\{[^}]*var\(--color-state-active\)/,
    ".wizard-button.primary keeps the solid accent treatment",
  );
  // Destructive becomes a red-tinted ghost on the WCAG-checked blocked token.
  assert.match(
    css,
    /\.wizard-button\.destructive[\s\S]{0,400}?color-mix\(in oklab, var\(--color-state-blocked\)/,
    ".wizard-button.destructive must tint from --color-state-blocked",
  );
  assert.doesNotMatch(
    css,
    /--color-state-danger, #c83a3a/,
    "hardcoded danger fallbacks are replaced by the checked blocked token",
  );
  // Modal titles speak the Mission Control display voice.
  assert.match(
    css,
    /\.modal-shell h2[\s\S]{0,400}?var\(--font-display\)/,
    "modal titles must use the display face",
  );
});

test("the permanent canvas hint bar is retired; guidance lives in the hotkey overlay (SPEC-3038 US-4)", () => {
  assert.equal(document.querySelector(".hint"), null, ".hint must be removed");
  assert.equal(document.getElementById("connection-dot"), null);
  assert.equal(document.getElementById("connection-label"), null);
  // Connection state has exactly one home: the Status Strip ONLINE cell.
  assert.doesNotMatch(appSource, /\bconnectionDot\b/);
  assert.doesNotMatch(appSource, /\bconnectionLabel\b/);
  const overlayText = document
    .getElementById("op-hotkey-overlay")
    .textContent.replace(/\s+/g, " ");
  for (const phrase of ["Pan canvas", "Zoom canvas", "Move window"]) {
    assert.ok(
      overlayText.includes(phrase),
      `hotkey overlay must carry canvas guidance: ${phrase}`,
    );
  }
  assert.doesNotMatch(inlineStyle, /\.hint\s*\{/);
  assert.doesNotMatch(inlineStyle, /\.hint__copy/);
  assert.doesNotMatch(inlineStyle, /\.connection-dot/);
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  assert.doesNotMatch(css, /\.hint\b/);
  assert.doesNotMatch(css, /\.connection-dot/);
});

test("Status Strip spells out WORK and drops the cryptic T clock label (SPEC-3038 US-4)", () => {
  const branchesCell = document.getElementById("op-strip-branches")?.parentElement;
  assert.ok(branchesCell, "expected the Work cell");
  assert.equal(
    branchesCell.querySelector(".op-status-strip__label")?.textContent?.trim(),
    "WORK",
    "the WK label must spell out WORK",
  );
  const clockCell = document.getElementById("op-strip-clock")?.parentElement;
  assert.ok(clockCell, "expected the clock cell");
  assert.equal(
    clockCell.querySelector(".op-status-strip__label"),
    null,
    "the clock needs no cryptic T label",
  );
});

test("empty canvas shows a first-window call to action (SPEC-3038 AS-4.5)", () => {
  const empty = document.getElementById("canvas-empty-state");
  assert.ok(empty, "expected #canvas-empty-state");
  assert.ok(
    empty.hasAttribute("hidden"),
    "empty state ships hidden until the workspace reports zero windows",
  );
  // SPEC-3245 Phase 3: the empty state offers the 2-lane entries (Curate =
  // Intake, Execute = Open Workspace) + Add window — Start Work is removed.
  assert.equal(
    empty.querySelector("#canvas-empty-start-work"),
    null,
    "Start Work action must be removed (SPEC-3245)",
  );
  assert.ok(
    empty.querySelector("#canvas-empty-intake"),
    "expected an Intake (Curate) action",
  );
  assert.ok(
    empty.querySelector("#canvas-empty-open-workspace"),
    "expected an Open Workspace (Execute) action",
  );
  assert.ok(
    empty.querySelector("#canvas-empty-add-window"),
    "expected an Add window action",
  );
  assert.match(
    appSource,
    /function\s+updateCanvasEmptyState\(\)[\s\S]{0,300}?windowMap\.size/,
    "app.js must toggle the empty state from the live window count",
  );
  assert.doesNotMatch(appSource, /canvas-empty-start-work/, "Start Work wiring must be removed");
  assert.match(appSource, /canvas-empty-intake/, "Intake action must be wired");
  assert.match(appSource, /canvas-empty-open-workspace/, "Open Workspace action must be wired");
  assert.match(appSource, /canvas-empty-add-window/, "Add window action must be wired");
});

test("renderWorkspace refreshes operator telemetry when windows mount/unmount (SPEC-3038)", () => {
  const body = extractFunctionBody(appSource, "renderWorkspace");
  assert.match(
    body,
    /recomputeOperatorTelemetry\(\)/,
    "window-count badge + empty state must update when windows mount/unmount",
  );
});

test("Improvement candidates refresh already-mounted inbox windows without workspace_state", () => {
  const refreshBody = extractFunctionBody(appSource, "refreshMountedImprovementInboxWindows");
  assert.match(
    refreshBody,
    /querySelectorAll\(\s*["']\.workspace-window\[data-preset="improvement"\]["']\s*,?\s*\)/,
    "refresh helper must target already-mounted Improvement Inbox windows",
  );
  assert.match(
    refreshBody,
    /querySelector\(\s*["']\.window-body["']\s*\)/,
    "refresh helper must remount the existing window body",
  );
  assert.match(
    refreshBody,
    /improvementInboxSurface\.mount\(\s*body\s*,\s*\{\s*improvement_candidates:\s*improvementCandidates\s*,?\s*\}/,
    "refresh helper must pass the latest candidate snapshot into the mounted surface",
  );

  const receiveBody = extractFunctionBody(appSource, "receive");
  const caseIndex = receiveBody.indexOf('case "improvement_candidates":');
  const revisionIndex = receiveBody.indexOf("improvementCandidatesRevision += 1;", caseIndex);
  const refreshIndex = receiveBody.indexOf("refreshMountedImprovementInboxWindows();", caseIndex);
  assert.ok(
    caseIndex >= 0 && revisionIndex > caseIndex && refreshIndex > revisionIndex,
    "improvement_candidates receive path must refresh mounted inbox windows after recording the new revision",
  );
});

test("Improvement async replies are scoped to the active project", () => {
  const scopeBody = extractFunctionBody(appSource, "improvementEventMatchesActiveProject");
  assert.match(
    scopeBody,
    /event\?\.project_root/,
    "improvement replies must carry their source project root",
  );
  assert.match(
    scopeBody,
    /activeProjectTab\(\)\?\.project_root/,
    "improvement replies must compare against the active project",
  );

  const receiveBody = extractFunctionBody(appSource, "receive");
  for (const kind of [
    "improvement_candidates",
    "improvement_action_result",
    "improvement_action_error",
  ]) {
    const start = receiveBody.indexOf(`case "${kind}":`);
    const end = receiveBody.indexOf("case ", start + 5);
    const arm = receiveBody.slice(start, end);
    assert.match(
      arm,
      /improvementEventMatchesActiveProject\(event\)/,
      `${kind} must ignore a delayed reply from another project`,
    );
  }

  const renderBody = extractFunctionBody(appSource, "renderAppState");
  assert.match(
    renderBody,
    /improvementCandidatesProjectRoot\s*!==\s*activeProjectRoot/,
    "project switching must clear the prior project's candidate snapshot",
  );
  assert.match(
    renderBody,
    /improvementCandidates\s*=\s*\[\]/,
    "stale candidate rows must not remain visible while the new project refresh loads",
  );
});

test("SPEC-3038 (2026-06-20): Windows badge counts windows across all project tabs", () => {
  const body = extractFunctionBody(appSource, "recomputeOperatorTelemetry");
  assert.match(
    body,
    /windows:\s*allProjectWindowIds\(\)\.length/,
    "the rail Windows badge must count the cross-tab open-window total",
  );
  assert.doesNotMatch(
    body,
    /windows:\s*windowMap\.size/,
    "windowMap.size only counts windows mounted in visited tabs and undercounts the badge",
  );
});

test("SPEC-3038 (2026-06-20): Windows popover renders the cross-tab window-list model", () => {
  assert.match(
    projectShellSurfaceSource,
    /import\s*\{\s*groupProjectWindowList\s*\}\s*from\s*"\/window-list-model\.js"/,
    "project-shell-surface must import the cross-tab window-list model",
  );
  const body = extractFunctionBody(projectShellSurfaceSource, "renderWindowList");
  assert.match(
    body,
    /groupProjectWindowList\(getAppState\(\),\s*windowListEntries\)/,
    "renderWindowList must source rows from the cross-tab model, not activeWorkspace() alone",
  );
  assert.match(
    body,
    /window-list-group/,
    "renderWindowList must render per-project group headers for multi-project shells",
  );
});

test("window close always routes through the Close Guard confirm modal (SPEC-3038 US-3)", () => {
  // index.html ships the modal scaffold.
  const modal = document.getElementById("window-close-confirm-modal");
  assert.ok(modal, "expected #window-close-confirm-modal backdrop");
  assert.ok(
    modal.classList.contains("modal-backdrop"),
    "close guard reuses the shared .modal-backdrop primitive",
  );
  assert.ok(
    modal.querySelector(".modal-shell.window-close-confirm-shell"),
    "close guard reuses the shared .modal-shell primitive",
  );
  // app.js wires both close paths through requestCloseWindow.
  assert.match(
    appSource,
    /from\s+"\/window-close-confirm-modal\.js"/,
    "app.js must import the close-confirm renderer module",
  );
  assert.match(
    appSource,
    /function\s+requestCloseWindow\(windowId\)/,
    "expected a requestCloseWindow entrypoint",
  );
  assert.match(
    appSource,
    /closeButton\.addEventListener\("click",\s*\(event\)\s*=>\s*\{\s*event\.stopPropagation\(\);\s*requestCloseWindow\(windowData\.id\);/,
    "titlebar × must route through requestCloseWindow",
  );
  assert.doesNotMatch(
    appSource,
    /closeButton\.addEventListener\("click",[\s\S]{0,120}?send\(\{\s*kind:\s*"close_window"/,
    "titlebar × must not send close_window directly",
  );
  // The tab strip passes the same entrypoint into the renderer.
  const renderTabsBody = extractFunctionBody(appSource, "renderWindowTabs");
  assert.match(
    renderTabsBody,
    /requestClose:\s*requestCloseWindow/,
    "tab × must route through requestCloseWindow",
  );
  // The renderer itself must not own a direct close_window send anymore.
  assert.doesNotMatch(
    windowTabsRendererSource,
    /close_window/,
    "window-tabs-renderer must not send close_window directly",
  );
});

test("window tabs receive agent runtime state from runtime status (SPEC-3038 US-2)", () => {
  // SPEC-3038 AS-2.1: tabs carry compact runtime-state cues while the full
  // window chrome keeps the semantic telemetry mapping.
  assert.match(
    appSource,
    /function\s+windowTabTelemetryState\(tab\)[\s\S]{0,400}?shouldShowRuntimeStatus\(tab\)[\s\S]{0,400}?normalizeWindowRuntimeState\(tab\.status,\s*tab\.preset\)[\s\S]{0,120}?return\s+runtimeState/,
    "expected a tab telemetry helper that gates on agent windows and returns raw runtime state for the tab cue",
  );
  const renderTabsBody = extractFunctionBody(appSource, "renderWindowTabs");
  assert.match(
    renderTabsBody,
    /agent_state:\s*windowTabTelemetryState\(tab\)/,
    "renderWindowTabs must decorate tab data with telemetry state",
  );
  // AS-2.2: runtime state changes refresh the visible tab strip of the group.
  const applyStatusBody = extractFunctionBody(appSource, "applyStatus");
  assert.match(
    applyStatusBody,
    /refreshWindowTabTelemetry\(windowData\)/,
    "status changes must refresh tab telemetry across the tab group",
  );
  assert.match(
    appSource,
    /function\s+refreshWindowTabTelemetry\(windowData\)[\s\S]{0,400}?windowTabsFor\(windowData\)/,
    "expected a group-wide tab telemetry refresh helper",
  );
});

test("Window tab activation updates tab chrome in place without remounting terminal body", () => {
  assert.match(
    appSource,
    /from\s+"\/window-tabs-renderer\.js"/,
    "app.js must use the extracted stable window tab renderer",
  );
  const renderTabsBody = extractFunctionBody(appSource, "renderWindowTabs");
  assert.match(
    renderTabsBody,
    /renderWindowTabsView\(\{/,
    "window tab chrome updates must be delegated to the stable renderer",
  );
  assert.doesNotMatch(
    renderTabsBody,
    /innerHTML\s*=/,
    "window tab activation must not clear and rebuild the tab strip",
  );
  assert.doesNotMatch(
    windowTabsRendererSource,
    /innerHTML\s*=/,
    "stable window tab renderer must update keyed tab nodes in place",
  );
  assert.match(
    windowTabsRendererSource,
    /dataset\.windowTabId/,
    "stable window tab renderer must key DOM nodes by window id",
  );

  const ensureWindowBody = extractFunctionBody(appSource, "ensureWindow");
  const mountCalls =
    ensureWindowBody.match(/mountWindowBody\(windowData,\s*element\)/g) || [];
  assert.equal(
    mountCalls.length,
    2,
    "body remounting must stay limited to preset changes plus the Agent Kanban dynamic body",
  );
  assert.match(
    ensureWindowBody,
    /surface\s*===\s*"agent-kanban"[\s\S]*mountWindowBody\(windowData,\s*element\)/,
    "Agent Kanban is the only dynamic body remount path",
  );
  const mountIndex = ensureWindowBody.indexOf("mountWindowBody(windowData, element);");
  const renderKeyIndex = ensureWindowBody.indexOf(
    "const nextWindowElementKey = windowElementRenderKey(windowData);",
  );
  assert.ok(
    mountIndex !== -1 && renderKeyIndex !== -1 && mountIndex < renderKeyIndex,
    "window render-key updates, including tab_group_active changes, must run after the body mount guard",
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
  assert.match(
    appSource,
    /lineHeight:\s*isBlinkBrowser\(\)\s*\?\s*1\.3[0-9]*\s*:\s*1\.3/,
    "xterm lineHeight must be at least 1.30 for both Blink and WebKit paths (Issue #2903)",
  );
});

test("xterm fontFamily is resolved without CSS var() so OffscreenCanvas char measurement matches the rendered font", () => {
  // xterm 6.0.0 measures the char cell via an OffscreenCanvas strategy
  // (ctx.font = `${fontSize}px ${fontFamily}`). Canvas 2D `ctx.font` CANNOT
  // resolve CSS custom properties, so a `var(--font-mono)` family token is
  // dropped and the SHORTER system fallback (SF Mono / Menlo) is measured,
  // while the rendered rows resolve var() to the TALLER JetBrains Mono — the
  // permanent measure-vs-render mismatch that clips glyphs vertically.
  const runtime = appSource.match(/new\s+Terminal\(\{\s*([\s\S]*?)\s*\}\);/);
  assert.ok(runtime, "expected xterm Terminal constructor options");
  assert.doesNotMatch(
    runtime[1],
    /var\(/,
    "xterm Terminal options must not pass a CSS var() font family; OffscreenCanvas ctx.font cannot resolve it",
  );
  assert.match(
    appSource,
    /function\s+resolveTerminalFontFamily\s*\(/,
    "expected a resolveTerminalFontFamily() helper that resolves --font-mono to a var()-free stack",
  );
  // The resolver's hardcoded fallback must still prefer JetBrains Mono.
  assert.match(
    appSource,
    /resolveTerminalFontFamily[\s\S]{0,400}?JetBrains Mono/,
    "resolveTerminalFontFamily fallback must reference JetBrains Mono",
  );
});

test("operator-shell keeps Mission Briefing early dismiss without any reveal machinery (SPEC-3038)", () => {
  const operatorShell = readFileSync(resolve(here, "../operator-shell.js"), "utf8");
  // SPEC-2356 Phase 9 removed Cmd+\\ and toggle entrypoints; SPEC-3038 removes
  // the hover-reveal state machine entirely — the rail is always visible.
  assert.ok(
    !operatorShell.includes('hotkey.register("cmd+\\\\"'),
    "Cmd+backslash hotkey stays removed",
  );
  assert.doesNotMatch(
    operatorShell,
    /toggleSidebar\s*:/,
    "no sidebar toggle entrypoint may exist",
  );
  assert.doesNotMatch(
    operatorShell,
    /dataset\[datasetKey\]\s*=\s*"revealed"/,
    "no hover-reveal state machine may write reveal attributes",
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

test("components.css retires the floating window controls toolbar CSS (SPEC-2356)", () => {
  // SPEC-2356 chrome cleanup: window operations live in the sidebar, so the
  // separate floating-window-controls toolbar, its hover-reveal state, and the
  // window-controls peek 帯 styling are all removed. The sidebar peek 帯 is the
  // only auto-hide affordance left.
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  assert.doesNotMatch(css, /#floating-window-controls-actions/);
  assert.doesNotMatch(css, /\.op-window-controls-peek/);
  assert.doesNotMatch(css, /\[data-op-window-controls/);
  assert.doesNotMatch(css, /\.floating-actions/);
  // SPEC-3038: the sidebar peek 帯 and the data-op-sidebar reveal state are
  // retired together with the overlay sidebar — the rail is grid-docked.
  assert.doesNotMatch(css, /op-sidebar/);
  assert.doesNotMatch(css, /\.op-layer\b/);
  assert.match(css, /\.op-rail\s*\{/);
});

test("Windows group exposes Align without resizing windows", () => {
  const button = document.getElementById("align-button");
  assert.ok(button, "expected Align button");
  assert.equal(
    button.querySelector(".op-rail__flyout-label")?.textContent?.trim(),
    "Align",
    "Align rail item must label its flyout",
  );
  assert.match(
    appSource,
    /alignButton\.addEventListener\("click",\s*\(\)\s*=>\s*arrangeWindows\("align"\)\)/,
    "expected Align to reuse arrange_windows with the align mode",
  );
});

test("operator-shell migrates legacy chrome keys and drops the hover-reveal machinery (SPEC-3038)", () => {
  // SPEC-2356 Phase 9 (FR-032): legacy localStorage keys are removed on boot
  // and the chip-style toggles must not be referenced anywhere in
  // operator-shell. SPEC-3038 retires the hover-reveal state machine entirely:
  // the rail is always visible, so no peek 帯 wiring and no reveal dataset
  // attribute may remain.
  const operatorShell = readFileSync(resolve(here, "../operator-shell.js"), "utf8");
  assert.doesNotMatch(operatorShell, /SIDEBAR_COLLAPSED_KEY\s*=\s*"gwt:ui:sidebar-collapsed"/);
  assert.doesNotMatch(operatorShell, /WINDOW_CONTROLS_KEY\s*=\s*"gwt:ui:window-controls"/);
  assert.doesNotMatch(operatorShell, /op-sidebar-edge-toggle/);
  assert.doesNotMatch(operatorShell, /op-window-controls-edge-toggle/);
  assert.doesNotMatch(operatorShell, /op-sidebar-toggle/);
  assert.doesNotMatch(operatorShell, /op-window-controls-toggle/);
  assert.doesNotMatch(operatorShell, /op-window-controls-peek/);
  assert.doesNotMatch(operatorShell, /op:window-controls-changed/);
  assert.match(operatorShell, /removeItem\("gwt:ui:sidebar-collapsed"\)/);
  assert.match(operatorShell, /removeItem\("gwt:ui:window-controls"\)/);
  assert.doesNotMatch(operatorShell, /createHoverRevealController/);
  assert.doesNotMatch(operatorShell, /op-sidebar-peek/);
  assert.doesNotMatch(operatorShell, /opSidebar\b/);
  assert.doesNotMatch(operatorShell, /op:chrome-visibility-changed/);
});

test("components.css declares Status Strip alert pulse", () => {
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  // PR #2414 introduced the pulse animation. SPEC-2356 chrome cleanup retires
  // the Layers `data-live` row indicator together with the Layers section.
  assert.match(css, /op-status-strip-alert-pulse/);
  assert.doesNotMatch(css, /\.op-layer\[data-live="true"\]/);
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

test("terminal overlay never covers raw terminal status details", () => {
  const visibleToggle = appSource.match(
    /overlay\.classList\.toggle\(\s*"visible",\s*([\s\S]*?)\s*\);/,
  );
  assert.ok(visibleToggle, "expected terminal overlay visibility toggle");
  assert.match(
    visibleToggle[1],
    /shouldShowOverlay/,
    "expected terminal overlay visibility to be driven by the named guard",
  );
  const overlayVisibilitySource = appSource.match(
    /const\s+shouldShowOverlay\s*=\s*([\s\S]*?);\s*const\s+shouldSpin/,
  );
  assert.ok(overlayVisibilitySource, "expected shouldShowOverlay guard");
  assert.equal(
    overlayVisibilitySource[1].trim(),
    "false",
    "terminal status details must never add a foreground overlay over the raw TTY",
  );
  assert.doesNotMatch(
    overlayVisibilitySource[1],
    /runtimeState\s*===\s*"error"/,
    "error status details must stay in terminal status surfaces instead of an overlay",
  );
  assert.doesNotMatch(
    overlayVisibilitySource[1],
    /runtimeState\s*===\s*"running"/,
    "running launch details must not cover the raw terminal TTY with an overlay",
  );
  const spinnerSource = appSource.match(
    /const\s+shouldSpin\s*=\s*([\s\S]*?);\s*const\s+spinner/,
  );
  assert.ok(spinnerSource, "expected shouldSpin guard");
  assert.equal(
    spinnerSource[1].trim(),
    "false",
    "hidden terminal overlays must never run a foreground startup spinner",
  );
  assert.doesNotMatch(
    spinnerSource[1],
    /runtimeState\s*===\s*"error"/,
    "error status details must not start a foreground terminal overlay spinner",
  );
  assert.doesNotMatch(
    spinnerSource[1],
    /runtimeState\s*===\s*"running"/,
    "running launch details must not start a foreground terminal overlay spinner",
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
    branchesCleanupSource,
    /row\.tabIndex\s*=\s*0[\s\S]*row\.setAttribute\("role",\s*"button"\)[\s\S]*createBranchRow|createBranchRow[\s\S]*row\.tabIndex\s*=\s*0[\s\S]*row\.setAttribute\("role",\s*"button"\)/,
    "expected createBranchRow to set tabindex and role=button",
  );
  // The keydown handler must distinguish plain Enter/Space (select)
  // from modified Enter (activate / open wizard).
  assert.match(
    branchesCleanupSource,
    /event\.metaKey\s*\|\|\s*event\.ctrlKey[\s\S]*activate\(\)/,
    "expected modified Enter to invoke activate (open launch wizard)",
  );
});

test("Branches separates row actions from cleanup toolbar action", () => {
  // SPEC-3064 Phase 3 (E6b): the Branches mount + row renderers moved into
  // the branches cleanup surface (mountBranchesWindow / createBranchRow).
  const branchesBlock = branchesCleanupSource
    .split("function mountBranchesWindow(")
    .at(1)
    ?.split("function clearBranchCleanupForWindow(")
    .at(0);

  assert.ok(branchesBlock, "expected Branches surface render block");
  assert.match(branchesBlock, /<div class="branch-heading">Repository branches<\/div>/);
  assert.doesNotMatch(
    branchesBlock,
    /double-click to launch/i,
    "Branches heading should not explain the double-click shortcut",
  );
  assert.match(
    branchesBlock,
    /data-action="open-branch-cleanup"/,
    "cleanup remains a toolbar-level selection action",
  );
  assert.doesNotMatch(
    branchesBlock,
    /data-action="open-branch-resume"|data-action="open-branch-launch"/,
    "Resume and Launch must not remain toolbar-level selected-branch actions",
  );

  assert.match(branchesCleanupSource, /branch-row-actions/, "branch rows must render action buttons");
  assert.match(
    branchesCleanupSource,
    /setAttribute\("data-branch-row-action",\s*"resume"\)/,
    "branch rows must expose a row-level Resume action",
  );
  assert.match(
    branchesCleanupSource,
    /setAttribute\("data-branch-row-action",\s*"launch"\)/,
    "branch rows must expose a row-level Launch action",
  );
  assert.match(
    branchesCleanupSource,
    /entry\.resume\.available[\s\S]{0,500}?resumeButton\.disabled/,
    "Resume row action must be disabled when the branch is not resumable",
  );
  assert.match(
    branchesCleanupSource,
    /branchName[\s\S]{0,260}?kind:\s*"resume_branch_latest_agent"|kind:\s*"resume_branch_latest_agent"[\s\S]{0,260}?branchName/,
    "row Resume action must send its own branch name",
  );
  assert.match(
    branchesCleanupSource,
    /branchName[\s\S]{0,220}?kind:\s*"open_launch_wizard"|kind:\s*"open_launch_wizard"[\s\S]{0,220}?branchName/,
    "row Launch action must send its own branch name",
  );
  assert.match(
    branchesCleanupSource,
    /row\.addEventListener\("dblclick",\s*activate\)/,
    "branch row double-click remains a Launch Agent shortcut",
  );
});

test("Branches row layout responds to minimized window width", () => {
  assert.match(
    inlineStyle,
    /\.branch-list-root\s*{[\s\S]*container-type:\s*inline-size/,
    "Branches list must own an inline-size container so minimized app windows drive row layout",
  );

  const minimizedBranchBlock = extractContainerBlocks(inlineStyle, "max-width: 900px");
  assert.match(
    minimizedBranchBlock,
    /\.branch-row\s*{[\s\S]*grid-template-columns:\s*auto\s+minmax\(0,\s*1fr\)\s+auto/,
    "minimized Branches rows should keep branch text wide and reserve one actions column",
  );
  assert.match(
    minimizedBranchBlock,
    /\.branch-meta\s*{[\s\S]*grid-column:\s*2\s*\/\s*4[\s\S]*grid-row:\s*2/,
    "minimized Branches rows should move metadata below the branch text instead of squeezing it",
  );
  assert.match(
    minimizedBranchBlock,
    /\.branch-row-actions\s*{[\s\S]*grid-column:\s*3[\s\S]*grid-row:\s*1/,
    "minimized Branches rows should keep Resume/Launch aligned on the top row",
  );
  assert.match(
    inlineStyle,
    /\.branch-upstream,\s*\n\.branch-date\s*{[\s\S]*white-space:\s*nowrap[\s\S]*text-overflow:\s*ellipsis/,
    "branch upstream/date text should truncate instead of wrapping into vertical columns",
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
    launchWizardSource,
    /let\s+launchWizardOpenError\s*=\s*null/,
    "expected frontend to track launch wizard open errors separately from project_open_error",
  );
  // Issue #2698 PR 1 (B7) — the applier defers via
  // `wizardInteractionGuard.defer(...)` before mutating the error
  // state. SPEC-3064 Phase 3 (E5): the app.js case arm delegates into
  // the surface applier, which populates `launchWizardOpenError`.
  assert.match(
    appSource,
    /case\s+"launch_wizard_open_error":[\s\S]{0,300}?applyLaunchWizardOpenErrorEvent\(event\);\s*break;/,
    "expected launch_wizard_open_error events to delegate into the wizard surface",
  );
  assert.match(
    launchWizardSource,
    /function\s+applyLaunchWizardOpenErrorEvent\(event\)[\s\S]{0,700}?launchWizardOpenError\s*=/,
    "expected launch_wizard_open_error events to populate wizard error state",
  );
  assert.match(
    launchWizardSource,
    /function\s+closeLaunchWizardLocal\(\)[\s\S]{0,300}?launchWizardOpenError\s*=\s*null/,
    "expected a local close path for error-only wizard state",
  );
  assert.match(
    launchWizardSource,
    /wizardModal\.classList\.contains\("open"\)[\s\S]{0,700}?closeLaunchWizardLocal\(\)[\s\S]{0,500}?sendWizardAction\(\{\s*kind:\s*"cancel"/,
    "expected Esc/close to locally dismiss error-only wizard state before sending backend cancel",
  );
  assert.match(
    launchWizardSource,
    /launchWizardOpenError\.title\s*===\s*"Intake"[\s\S]{0,80}?\?\s*"Curate session"/,
    "expected Intake open errors to keep Intake/Curate copy instead of Plan Agent copy",
  );
});

test("Launch wizard tombstone does not dismiss open-error modal state", () => {
  assert.match(
    launchWizardSource,
    /if\s*\(deferred\.kind\s*===\s*"launch_wizard_state"\)[\s\S]{0,500}?if\s*\(deferred\.wizard\)\s*\{[\s\S]{0,160}?launchWizardOpenError\s*=\s*null[\s\S]{0,160}?\}[\s\S]{0,240}?launchWizard\s*=\s*deferred\.wizard/,
    "expected deferred launch_wizard_state tombstones to preserve launchWizardOpenError",
  );
  assert.match(
    launchWizardSource,
    /function\s+applyLaunchWizardStateEvent\(event\)[\s\S]{0,900}?if\s*\(event\.wizard\)\s*\{[\s\S]{0,160}?launchWizardOpenError\s*=\s*null[\s\S]{0,160}?\}[\s\S]{0,240}?launchWizard\s*=\s*event\.wizard/,
    "expected launch_wizard_state tombstones to clear stale wizard state without clearing launchWizardOpenError",
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
    appSource.includes("wizardCloseButton") ||
      launchWizardSource.includes("wizardCloseButton"),
    false,
    "Launch wizard should not keep dead wiring for a removed header close button",
  );
  assert.match(
    launchWizardSource,
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
    appSource.includes("wizardCloseButton") ||
      launchWizardSource.includes("wizardCloseButton"),
    false,
    "Launch wizard should not keep dead wiring for a removed header close button",
  );
  assert.match(
    launchWizardSource,
    /wizardCancelButton\.addEventListener\("click",\s*closeLaunchWizardFromChrome\)/,
    "expected the footer dismiss button to own the close helper",
  );
});

test("Launch wizard renders a backend-gated Back control in the footer", () => {
  assert.ok(
    document.getElementById("wizard-back-button"),
    "Launch wizard footer must expose a Back button for returning to Start methods",
  );
  assert.match(
    launchWizardSource,
    /const wizardBackButton = document\.getElementById\("wizard-back-button"\)/,
    "expected the wizard surface to bind the footer Back button",
  );
  assert.match(
    launchWizardSource,
    /wizardBackButton\.hidden\s*=\s*!launchWizard\.show_back_button/,
    "expected Back visibility to be controlled by backend view state",
  );
  assert.match(
    launchWizardSource,
    /wizardBackButton\.addEventListener\("click",\s*\(\)\s*=>\s*\{[\s\S]*?kind:\s*"back"/,
    "expected Back to dispatch the canonical backend action",
  );
});

test("File tree rows are keyboard-navigable (tabindex + role + keydown)", () => {
  // <div>-based rows can't be Tab'd to or activated with the keyboard
  // unless explicitly opted in. Add tabindex, role="button", and a
  // keydown handler that mirrors the click handler for Enter/Space.
  // Without all three, keyboard-only users couldn't browse the file
  // tree — only mouse users could.
  assert.match(fileTreeSurfaceSource, /row\.tabIndex\s*=\s*0/, "expected file tree row tabindex=0");
  assert.match(
    fileTreeSurfaceSource,
    /row\.setAttribute\("role",\s*"button"\)/,
    "expected file tree row role='button'",
  );
  assert.match(
    fileTreeSurfaceSource,
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
    fileTreeSurfaceSource,
    /isDirectory[\s\S]*row\.setAttribute\("aria-expanded",\s*expanded\s*\?\s*"true"\s*:\s*"false"\)/,
    "expected directory rows to set aria-expanded based on expanded state",
  );
  assert.match(
    fileTreeSurfaceSource,
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
    launchWizardSource,
    /button\.setAttribute\("aria-pressed",\s*selected\s*\?\s*"true"\s*:\s*"false"\)/,
    "expected createChoiceButton to set aria-pressed based on selected",
  );
});

test("Launch wizard separates launch settings from runtime controls", () => {
  assert.equal(
    launchWizardSource.includes("wizardAdvancedOpen"),
    false,
    "Launch wizard should not keep an Advanced disclosure state",
  );
  for (const retiredCopy of ["Advanced", "Show advanced", "Hide advanced"]) {
    assert.equal(
      launchWizardSource.includes(`"${retiredCopy}"`),
      false,
      `Launch wizard should not render ${retiredCopy}`,
    );
  }

  const launchSettingsStart = launchWizardSource.indexOf(
    'createLaunchSection(\n            "Launch settings"',
  );
  const linkedIssueStart = launchWizardSource.indexOf(
    'createLaunchSection(\n            "Linked issue"',
  );
  const runtimeStart = launchWizardSource.indexOf(
    'createLaunchSection(\n            "Runtime"',
  );

  assert.notEqual(launchSettingsStart, -1, "expected Launch settings section");
  assert.notEqual(linkedIssueStart, -1, "expected Linked issue section");
  assert.notEqual(runtimeStart, -1, "expected Runtime section");
  assert.ok(
    launchSettingsStart < linkedIssueStart && linkedIssueStart < runtimeStart,
    "expected Launch settings before Linked issue and Runtime after Linked issue",
  );

  const launchSettingsBlock = launchWizardSource.slice(
    launchSettingsStart,
    linkedIssueStart,
  );
  for (const copy of ["Version", "Skip permission prompts", "Fast mode"]) {
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

  const runtimeBlock = launchWizardSource.slice(
    runtimeStart,
    launchWizardSource.indexOf("wizardBody.appendChild(panel);"),
  );
  for (const copy of ["Runtime target", "Docker service", "Docker lifecycle"]) {
    assert.ok(
      runtimeBlock.includes(`"${copy}"`),
      `expected Runtime to include ${copy}`,
    );
  }
  for (const copy of ["Version", "Skip permission prompts", "Fast mode"]) {
    assert.equal(
      runtimeBlock.includes(`"${copy}"`),
      false,
      `Runtime should not contain ${copy}`,
    );
  }
});

test("Launch wizard runtime confirmation shows summary without setup forms", () => {
  assert.match(
    launchWizardSource,
    /const showConfirm = Boolean\(launchWizard\.show_confirm\);[\s\S]*?const isRuntimeConfirmation = Boolean\(\s*launchWizard\.runtime_context_resolved\s*&&\s*launchWizard\.show_runtime_confirmation\s*&&\s*!showConfirm\s*\);[\s\S]*?const showManualSetup =\s*launchWizard\.show_manual_setup !== false\s*&&\s*!isRuntimeConfirmation\s*&&\s*!showConfirm;[\s\S]*?const showStartMethods = Boolean\(\s*launchWizard\.show_start_methods\s*&&\s*!isRuntimeConfirmation\s*&&\s*!showConfirm[\s\S]*?const showSetupForms = showManualSetup && !isRuntimeConfirmation;/,
    "expected renderer to derive mutually exclusive Runtime/Confirm/setup gating",
  );
  assert.match(
    launchWizardSource,
    /const isRuntimeConfirmation = Boolean\(\s*launchWizard\.runtime_context_resolved\s*&&\s*launchWizard\.show_runtime_confirmation\s*&&\s*!showConfirm\s*\);/,
    "expected renderer to derive a dedicated Runtime confirmation state outside Confirm",
  );
  assert.match(
    launchWizardSource,
    /const showSetupForms = showManualSetup && !isRuntimeConfirmation;/,
    "expected manual setup forms to be suppressed during Runtime confirmation",
  );
  assert.match(
    launchWizardSource,
    /renderWizardSummary\(\);[\s\S]*?const isRuntimeConfirmation = Boolean/,
    "expected read-only launch summary to remain visible before Runtime-only body rendering",
  );
  assert.match(
    launchWizardSource,
    /if\s*\(\s*showStartMethods\s*\)/,
    "expected Start methods rows to be gated by backend start-method state",
  );
  assert.match(
    launchWizardSource,
    /if\s*\(\s*showSetupForms\s*&&\s*launchWizard\.show_branch_controls !== false\s*\)/,
    "expected Branch controls to be part of setup forms only",
  );
  assert.match(
    launchWizardSource,
    /if\s*\(\s*showSetupForms\s*\)\s*\{[\s\S]*?createLaunchSection\(\s*"Launch"/,
    "expected Launch controls to be part of setup forms only",
  );
  assert.match(
    launchWizardSource,
    /if\s*\(\s*showSetupForms\s*&&[\s\S]*?launchWizard\.show_fast_mode[\s\S]*?\)\s*\{[\s\S]*?createLaunchSection\(\s*"Launch settings"/,
    "expected Launch settings controls to be part of setup forms only",
  );
  // SPEC-2014 2026-05-29 amendment (FR-109): Fast mode is now a toggle switch
  // (appendToggleField) instead of a checkbox, but stays provider-neutral —
  // wired to launchWizard.fast_mode + set_fast_mode, not Codex-only state.
  assert.match(
    launchWizardSource,
    /appendToggleField\(\s*grid,\s*"Fast mode"[\s\S]*?launchWizard\.fast_mode[\s\S]*?kind:\s*"set_fast_mode"/,
    "expected Launch settings to wire provider-neutral Fast mode controls",
  );
  assert.doesNotMatch(
    launchWizardSource,
    /Codex fast mode/,
    "expected Launch settings copy to avoid Codex-only Fast mode wording",
  );
  // SPEC-2014 Amendment 2026-05-20 (FR-057): Linked issue is gated by its
  // dedicated `show_linked_issue` flag so it only appears when the wizard
  // was opened through the Knowledge Issue Bridge.
  assert.match(
    launchWizardSource,
    /if\s*\(\s*showSetupForms\s*&&\s*launchWizard\.show_linked_issue\s*\)/,
    "expected Linked issue controls to be gated by show_linked_issue and setup forms",
  );
});

test("Launch wizard submit button uses Continue before runtime context is resolved", () => {
  assert.match(
    launchWizardSource,
    /launchWizard\.primary_action_label\s*\|\|[\s\S]{0,260}?launchWizard\.runtime_context_resolved === false\s*\?\s*"Continue"\s*:/,
    "expected unresolved Launch Agent runtime context to use Continue instead of Launch",
  );
});

test("Launch wizard keeps cancel available during runtime resolution", () => {
  const closeHelper = launchWizardSource.match(
    /function closeLaunchWizardFromChrome\(\) \{([\s\S]*?)\n      \}/,
  );
  assert.ok(closeHelper, "expected launch wizard close helper");
  assert.equal(
    closeHelper[1].includes("runtime_resolution_pending"),
    false,
    "runtime resolution pending must not block the footer Cancel button",
  );
  assert.match(
    launchWizardSource,
    /wizardCancelButton\.disabled\s*=\s*false/,
    "Cancel button must stay enabled while runtime resolution is pending",
  );
  const escapeHandler = launchWizardSource.match(
    /if \(wizardModal\.classList\.contains\("open"\)\) \{([\s\S]*?)event\.preventDefault\(\);\s*return true;/,
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
    launchWizardSource,
    /const selector =\n\s+"input, textarea, select, button, \[role='button'\], \[contenteditable='true'\]";[\s\S]*?querySelectorAll\(selector\)/,
    "expected a pending-state helper to disable wizard panel controls",
  );
  assert.match(
    launchWizardSource,
    /panel\.classList\.toggle\(\s*"wizard-disabled"[\s\S]{0,180}?isRuntimeResolutionPending\s*\|\|\s*isLaunchActionPending/,
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
    launchWizardSource.includes("isStartWorkMode"),
    false,
    "Start Work should share the centered wizard modal instead of toggling drawer mode",
  );
  assert.equal(
    launchWizardSource.includes('wizardModal.classList.toggle("is-drawer"'),
    false,
    "Launch Wizard should not toggle drawer placement",
  );
  assert.ok(
    launchWizardSource.includes("wizard-progress-rail")
      && launchWizardSource.includes("wizard-main")
      && launchWizardSource.includes("wizard-content-pane"),
    "expected the wizard body to be split into progress rail and content pane",
  );
  assert.equal(
    /if\s*\(\s*event\.target === wizardModal\s*\)\s*\{\s*closeLaunchWizardFromChrome\(\);\s*\}/.test(launchWizardSource),
    false,
    "wizard backdrop clicks must not dismiss the wizard",
  );
});

test("Launch wizard start methods keep disabled and hover states distinct", () => {
  assert.match(
    inlineStyle,
    /\.start-method-button:hover:not\(:disabled\)/,
    "enabled start method rows should expose a hover state",
  );
  assert.match(
    inlineStyle,
    /\.start-method-button:disabled/,
    "disabled start method rows should expose a disabled state",
  );
});

test("Launch wizard pending launch feedback has stable styling hooks", () => {
  assert.match(
    inlineStyle,
    /\.modal-shell\.is-wizard\.is-launch-pending[\s\S]*?cursor:\s*progress;/,
    "expected Launch Wizard modal to expose progress cursor while a launch action is pending",
  );
  assert.match(
    inlineStyle,
    /\.start-method-button\.is-pending[\s\S]*?cursor:\s*progress;/,
    "expected pending Start method row to expose a progress cursor",
  );
  assert.match(
    inlineStyle,
    /\.launch-pending-note[\s\S]*?color:\s*var\(--color-text-strong\)/,
    "expected pending launch copy to be styled as visible modal feedback",
  );
});

test("Launch wizard renders start methods as direct actions", () => {
  assert.ok(
    launchWizardSource.includes('"Start methods"'),
    "expected Launch Wizard to label the first section as Start methods",
  );
  assert.ok(
    launchWizardSource.includes("start_methods")
      && launchWizardSource.includes("show_start_methods")
      && launchWizardSource.includes('kind: "use_start_method"'),
    "expected start method rows to dispatch direct backend actions",
  );
  assert.equal(
    launchWizardSource.includes('"Quick start"'),
    false,
    "Launch Wizard must not expose the old Quick start heading",
  );
  assert.equal(
    launchWizardSource.includes('kind: "select_quick_start"'),
    false,
    "start methods should not use the old selection-before-footer-submit model",
  );
  assert.equal(
    launchWizardSource.includes("quick-start-actions"),
    false,
    "start methods should not render multiple inline action buttons",
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
  // project-tabs case from PR #2455. SPEC-3064 Phase 3 (E6): per-window
  // surfaces extracted from app.js count too.
  const rowSurfacesSource = [
    appAndProjectTabsSource,
    fileTreeSurfaceSource,
    boardLogsSurfaceSource,
    knowledgeKanbanSurfaceSource,
    profileWindowSurfaceSource,
  ].join("\n");
  const setMatches =
    rowSurfacesSource.match(/setAttribute\("aria-current",\s*"(true|page)"\)/g) || [];
  const removeMatches =
    rowSurfacesSource.match(/removeAttribute\("aria-current"\)/g) || [];
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
  // The preset modal is rendered inline in app.js; the wizard renderer
  // moved to launch-wizard-surface.js (SPEC-3064 E5). Both need
  // closure-scoped trap-release variables. Pin both pairs.
  assert.match(appSource, /import\s*\{\s*createFocusTrap\s*\}\s*from\s*"\/focus-trap\.js"/);
  assert.match(
    launchWizardSource,
    /import\s*\{\s*createFocusTrap\s*\}\s*from\s*"\/focus-trap\.js"/,
  );
  // Wizard
  assert.match(launchWizardSource, /let\s+wizardFocusTrapRelease\s*=\s*null/);
  assert.match(
    launchWizardSource,
    /wizardFocusTrapRelease\s*=\s*createFocusTrap\(wizardDialog,\s*\{\s*document\s*\}\)/,
    "wizard must activate focus trap on the dialog",
  );
  assert.match(
    launchWizardSource,
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
  // SPEC-3064 Phase 3 (E5/E6e): wizard fields render in the extracted
  // launch wizard surface; profile editor fields render in the extracted
  // profile window surface.
  const expected = [
    { source: launchWizardSource, selector: 'input\\.setAttribute\\("aria-label", "Branch name"\\)', desc: "wizard branch name" },
    { source: profileWindowSurfaceSource, selector: 'keyInput\\.setAttribute\\("aria-label", `Environment variable key, row ', desc: "env var key" },
    { source: profileWindowSurfaceSource, selector: 'modeSelect\\.setAttribute\\("aria-label", `Environment variable mode, row ', desc: "env var mode" },
    { source: profileWindowSurfaceSource, selector: 'valueInput\\.setAttribute\\("aria-label", `Profile value, row ', desc: "profile env value" },
    { source: launchWizardSource, selector: 'select\\.setAttribute\\("aria-label", label\\)', desc: "launch-field select reuses label" },
  ];
  for (const { source, selector, desc } of expected) {
    assert.match(
      source,
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
  // The wizard modal's renderer lives in launch-wizard-surface.js
  // (SPEC-3064 E5), so focus management is wired there. Verify the same
  // pattern as the drawer modals: wizardFocusReturn captures
  // activeElement on open, wizardDialog.focus({ preventScroll: true })
  // moves focus into the dialog, and wizardFocusReturn.focus() restores
  // on close.
  assert.match(launchWizardSource, /let\s+wizardFocusReturn\s*=\s*null/);
  assert.match(launchWizardSource, /wizardFocusReturn\s*=\s*document\.activeElement/);
  assert.match(
    launchWizardSource,
    /wizardDialog\.focus\(\{\s*preventScroll:\s*true\s*\}\)/,
    "expected wizardDialog focus on open with preventScroll",
  );
  assert.match(
    launchWizardSource,
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
  // SPEC-3064 Phase 3 (E7): the migration Esc branch delegates into the
  // project shell surface, which owns the migration modal state.
  assert.doesNotMatch(
    projectShellSurfaceSource,
    /migrationModal\s*&&\s*migrationModal\.classList\.contains\("open"\)[\s\S]{0,400}skip_migration/,
    "Esc must not send skip_migration when the Accept-only migration modal is open",
  );
  assert.match(
    projectShellSurfaceSource,
    /migrationModal\s*&&\s*migrationModal\.classList\.contains\("open"\)[\s\S]*migrationModalState\.stage\s*===\s*"error"[\s\S]*migrationModalState\.open\s*=\s*false/,
    "expected Esc on the migration modal error stage to dismiss the dialog without backend skip",
  );
  assert.match(
    appSource,
    /if\s*\(handleMigrationModalEscape\(event\)\)\s*\{\s*return;\s*\}/,
    "expected the global Esc handler to delegate the migration branch into the surface",
  );
  // SPEC-3064 Phase 3 (E5): the wizard Esc branch delegates into the
  // launch wizard surface, which owns the cancel dispatch.
  assert.match(
    appSource,
    /if\s*\(handleWizardEscapeKeydown\(event\)\)\s*\{\s*return;\s*\}/,
    "expected the global Esc handler to delegate the wizard branch into the surface",
  );
  assert.match(
    launchWizardSource,
    /wizardModal\.classList\.contains\("open"\)[\s\S]*sendWizardAction\(\{\s*kind:\s*"cancel"/,
    "expected Esc to send cancel when wizard modal is open",
  );
  // SPEC-3064 Phase 3 (E7): the Windows dropdown Esc branch delegates into
  // the project shell surface, which owns the dropdown state.
  assert.match(
    projectShellSurfaceSource,
    /if\s*\(windowListOpen\)\s*\{[\s\S]*windowListOpen\s*=\s*false[\s\S]*windowListButton\.focus/,
    "expected Esc to close window list dropdown and restore focus to trigger",
  );
  // SPEC-2008 camera-focus: the Windows dropdown Esc branch is now guarded so
  // a bare Esc that the dropdown did NOT consume can fall through to the
  // camera overview (enterOverview) below it.
  assert.match(
    appSource,
    /if\s*\(handleWindowListEscape\(event\)\)\s*\{\s*return;\s*\}/,
    "expected the global Esc handler to delegate the Windows dropdown branch into the surface",
  );
  // SPEC-2356 — preset modal Esc-close: closes via closeModal() which
  // handles both the .open class flip and focus restore.
  assert.match(
    appSource,
    /if\s*\(modal\.classList\.contains\("open"\)\)\s*\{[\s\S]*closeModal\(\)/,
    "expected Esc to call closeModal when preset modal is open",
  );
  // SPEC-2008 camera-focus: a bare Esc that nothing else consumed zooms the
  // camera out to frame all windows (overview), but only when no text entry /
  // focused terminal owns the keystroke (vim, TUI apps rely on Esc).
  assert.match(
    appSource,
    /if\s*\(event\.defaultPrevented\s*\|\|\s*isTextEntryFocused\(\)\)\s*\{\s*return;\s*\}\s*enterOverview\(\)/,
    "expected an unconsumed Esc to enter camera overview, guarded by isTextEntryFocused",
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

test("mapAgentTelemetryState emits only Living Telemetry states CSS handles", () => {
  // SPEC-3015 moved the runtime→Living Telemetry mapper from app.js to
  // window-runtime-state.js. CSS only styles `[data-agent-state]` for
  // declared telemetry states — any drift (e.g. emitting "warn" or
  // "exited") would silently render no rim. Pin the contract so refactors
  // can't introduce undeclared states.
  const windowRuntimeStateSource = readFileSync(
    resolve(here, "../window-runtime-state.js"),
    "utf8",
  );
  const mapperBlock = windowRuntimeStateSource.match(
    /function\s+mapAgentTelemetryState\s*\([^)]*\)\s*\{[\s\S]*?\n\}/,
  );
  assert.ok(
    mapperBlock,
    "expected mapAgentTelemetryState to be defined in window-runtime-state.js",
  );
  const returnedStates = new Set();
  for (const m of mapperBlock[0].matchAll(/return\s+"([^"]+)"/g)) {
    returnedStates.add(m[1]);
  }
  // FR-039 / SPEC-2356 follow-up: the telemetry vocabulary mirrors runtime
  // states for user-facing consistency. STARTING aggregates into RUNNING.
  const allowed = new Set(["running", "idle", "waiting", "error", "done"]);
  for (const state of returnedStates) {
    assert.ok(allowed.has(state), `mapAgentTelemetryState returned undeclared state: ${state}`);
  }
  // And every design state must be reachable, not just allowed.
  for (const required of allowed) {
    assert.ok(returnedStates.has(required), `Living Telemetry state never emitted: ${required}`);
  }
});

test("Status Strip RUNNING / IDLE / WAITING / ERROR cells all tint with their state color", () => {
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  // The RUNNING / IDLE cells previously had no tonal hint — only ERROR did.
  // Add parallel symmetry so the count cells render with matching state colors
  // (cyan / gray / amber / red) for at-a-glance scanning. FR-039 (安心) adds the
  // WAITING cell tinted with the needs-input amber.
  assert.match(css, /\.op-status-strip__cell--running\s+\.op-status-strip__value\s*\{[^}]*--color-state-active/);
  assert.match(css, /\.op-status-strip__cell--idle\s+\.op-status-strip__value\s*\{[^}]*--color-state-idle/);
  assert.match(css, /\.op-status-strip__cell--waiting\s+\.op-status-strip__value\s*\{[^}]*--color-state-needs-input/);
  assert.match(css, /\.op-status-strip__cell--error\s+\.op-status-strip__value\s*\{[^}]*--color-state-blocked/);
  // Markup also needs the modifiers wired so the CSS selectors actually match.
  const indexHtml = readFileSync(resolve(here, "../index.html"), "utf8");
  assert.match(indexHtml, /op-status-strip__cell\s+op-status-strip__cell--running/);
  assert.match(indexHtml, /op-status-strip__cell\s+op-status-strip__cell--idle/);
  assert.match(indexHtml, /op-status-strip__cell\s+op-status-strip__cell--waiting/);
  assert.match(indexHtml, /op-status-strip__cell\s+op-status-strip__cell--error/);
  assert.doesNotMatch(indexHtml, /op-status-strip__cell\s+op-status-strip__cell--active/);
  assert.doesNotMatch(indexHtml, /op-status-strip__cell\s+op-status-strip__cell--blocked/);
  assert.match(indexHtml, /<span class="op-status-strip__label">RUNNING<\/span>/);
  assert.match(indexHtml, /<span class="op-status-strip__label">ERROR<\/span>/);
  assert.match(indexHtml, /op-status-strip__cell\s+op-status-strip__cell--runtime-health/);
  assert.match(css, /\.op-status-strip__cell--runtime-health\[data-state="warn"\]/);
});

test("FR-039 (安心): WAITING cell drives a LOUD waiting alert pulse like ERROR", () => {
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  // The WAITING cell must pulse via the same op-status-strip alert mechanism the
  // ERROR cell uses, so "agents waiting for input" reads just as loud.
  assert.match(
    css,
    /\.op-status-strip__cell--waiting\.op-status-strip__cell--alert\s+\.op-status-strip__value/,
    "WAITING cell needs an --alert pulse rule mirroring ERROR",
  );
  // The window rim + minimap dot must render the waiting telemetry state.
  assert.match(css, /\[data-agent-state="waiting"\]\s*\{/);
  assert.match(css, /\.fleet-minimap__cell\[data-telemetry="waiting"\]::after/);
  // applyTelemetryCounts must route the waiting count into the WAITING cell.
  assert.match(operatorShellSource, /op-strip-waiting/);
  assert.match(operatorShellSource, /counts\.waiting/);
});

test("active work projection only upgrades live work to RUNNING telemetry", () => {
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
    /counts\.running\s*=\s*Math\.max\(counts\.running,\s*activeAgents\s*\|\|\s*activeWorks\.length\)/,
    "active work projection should lift live work into RUNNING telemetry",
  );
  assert.doesNotMatch(
    body,
    /counts\.error\s*=\s*Math\.max\(counts\.error,\s*blockedAgents/,
    "workspace-level blocked_agents must not become Status Strip ERROR",
  );
  assert.doesNotMatch(
    body,
    /category\s*===\s*"blocked"[\s\S]{0,120}counts\.error/,
    "workspace status_category=blocked must not become Status Strip ERROR",
  );
});

test("FR-041/044 (安心): window chrome carries STOP + RESTART kill-switch controls", () => {
  // The window titlebar actions must expose STOP and RESTART alongside close,
  // both starting hidden (visibility is driven per render from runtime state).
  assert.match(appSource, /data-action="stop"[^>]*aria-label="Stop agent"/);
  assert.match(appSource, /data-action="restart"[^>]*aria-label="Restart agent"/);
  // STOP click sends stop_window (PTY halts, window stays); RESTART sends
  // restart_window (relaunch in place).
  assert.match(appSource, /kind:\s*"stop_window",\s*id:\s*windowData\.id/);
  assert.match(appSource, /kind:\s*"restart_window",\s*id:\s*windowData\.id/);
  // The render path toggles the controls based on runtime state.
  assert.match(appSource, /updateWindowKillSwitchControls/);
});

test("FR-042 (安心): STOP ALL is reachable from the rail and the palette with a confirm", () => {
  const railItem = document.querySelector('.op-rail__item[data-cmd="stop-all-windows"]');
  assert.ok(railItem, "expected a Stop all agents rail item");
  assert.equal(railItem.getAttribute("aria-label"), "Stop all agents");
  // op:command + palette both route to requestStopAllWindows, which confirms
  // and emits stop_all_windows.
  assert.match(appSource, /case "stop-all-windows":/);
  assert.match(appSource, /id:\s*"stop-all-agents"/);
  assert.match(appSource, /kind:\s*"stop_all_windows"/);
  assert.match(appSource, /function requestStopAllWindows\(\)/);
});

test("FR-043 (安心): send-input routes to the focused agent pane via session_id", () => {
  // The palette entry + helper inject one line into the focused agent pane
  // using pane_send_input scoped to the window's session_id.
  assert.match(appSource, /id:\s*"send-input-focused-agent"/);
  assert.match(appSource, /kind:\s*"pane_send_input",\s*session_id:/);
  assert.match(appSource, /function sendFocusedPaneInput\(/);
});

test("FR-040 (安心): in-app attention toasts are wired with click-to-jump", () => {
  // The attention toaster fires in-app toasts (no away gate); the renderer
  // frames the window on click and respects reduced-motion via the CSS layer.
  assert.match(appSource, /createAgentAttentionToaster/);
  assert.match(appSource, /agentAttentionToaster\.handleRuntimeState/);
  assert.match(appSource, /function showAttentionToast/);
  assert.match(appSource, /frameWindow\(notice\.windowId\)/);
  // SPEC #3206: attention now renders through the shared bottom-right alerts
  // stack (.toast-alerts*), not its own .attention-toast block.
  assert.match(inlineStyle, /\.toast-alerts__item\s*\{/);
  assert.match(
    inlineStyle,
    /@media \(prefers-reduced-motion: reduce\)[\s\S]*?\.toast-alerts__item\s*\{[\s\S]*?animation:\s*none/,
    "alerts toast must drop its entrance animation under reduced motion",
  );
});

test("FR-040 refinement: error toasts persist, stack newest-on-top, share a fixed width", () => {
  // SPEC #3206: the FR-040 contract is preserved through the shared alerts
  // stack. Error stays until dismissed (timeoutMs 0 = sticky); quieter flavors
  // auto-hide via timeoutMs.
  assert.match(
    appSource,
    /flavor\s*===\s*"error"\s*\?\s*0\s*:/,
    "error flavor must be sticky (timeoutMs 0)",
  );
  // Newest toast on top + collapse-on-close are owned by the shared region.
  assert.match(appSource, /newestOnTop:\s*true/, "alerts stack puts the freshest on top");
  assert.match(appSource, /animateDismiss:\s*true/, "alerts stack collapses on close");
  // Fixed width (not content-sized) so toasts line up.
  assert.match(
    inlineStyle,
    /\.toast-alerts\s*\{[^}]*width:\s*min\(\s*360px/,
    "alerts toasts must use a fixed width",
  );
  // The leaving state collapses height to let the stack settle.
  assert.match(
    inlineStyle,
    /\.toast-alerts__item\[data-leaving="true"\]\s*\{[^}]*max-height:\s*0/,
    "leaving toasts must collapse their height",
  );
});

test("SPEC #3206 P2: alerts toast CSS only references defined Operator tokens", () => {
  // An undefined token in var() silently resolves to nothing (the legacy
  // attention block shipped `var(--shadow-3)`, which is defined nowhere, so
  // the alerts cards rendered with no elevation shadow at all). Every token
  // the .toast-alerts family references must exist in tokens.css /
  // typography.css (or be a custom property app.css itself defines).
  const tokensCss = readFileSync(resolve(here, "../styles/tokens.css"), "utf8");
  const typographyCss = readFileSync(resolve(here, "../styles/typography.css"), "utf8");
  const defined = new Set();
  for (const source of [tokensCss, typographyCss, inlineStyle]) {
    for (const m of source.matchAll(/(--[a-z0-9-]+)\s*:/g)) {
      defined.add(m[1]);
    }
  }
  const blocks = inlineStyle.match(/\.toast-alerts[^{}]*\{[^}]*\}/g) ?? [];
  assert.ok(blocks.length >= 5, "expected the .toast-alerts rule family in app.css");
  for (const block of blocks) {
    for (const m of block.matchAll(/var\(\s*(--[a-z0-9-]+)/g)) {
      assert.ok(
        defined.has(m[1]),
        `.toast-alerts references undefined token ${m[1]}: ${block.trim().split("\n")[0]}`,
      );
    }
  }
});

test("Work surface lifecycle badge styles every agent-session state (SPEC-2359 W-12 FR-351)", () => {
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  // Slice 3 retires the sidebar `op-agent-card` states; the Work surface now
  // carries the agent-session lifecycle badge with a distinct treatment per
  // state so operators can scan Work lifecycle at a glance.
  assert.doesNotMatch(
    css,
    /\.op-agent-card/,
    "retired sidebar agent-card CSS must be removed",
  );
  assert.match(css, /\.workspace-overview-lifecycle\s*\{/);
  assert.match(
    css,
    /\.workspace-overview-lifecycle\[data-lifecycle="active"\]\s*\{[^}]*--color-state-active/,
  );
  assert.match(
    css,
    /\.workspace-overview-lifecycle\[data-lifecycle="paused"\]\s*\{[^}]*--color-state-idle/,
  );
  assert.match(
    css,
    /\.workspace-overview-lifecycle\[data-lifecycle="done"\]\s*\{[^}]*--color-state-done/,
  );
  assert.match(
    css,
    /\.workspace-overview-lifecycle\[data-lifecycle="discarded"\]\s*\{/,
  );
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
    /inset:\s*8px\s+4px\s+4px\s*;/,
    ".terminal-root must express terminal chrome spacing as inset so its outer box stays inside .window-body (Issue #2923 follow-up: side / bottom insets reduced to 4px so the cell grid recovers ~1 column at the gwt-default 720×420 window)",
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

test("terminal transcript host paints every xterm layer as an opaque dark body (SPEC-2356 FR-035)", () => {
  const expectedSelectors = [
    ".surface-terminal .terminal-root",
    ".surface-terminal .terminal-root .xterm",
    ".surface-terminal .terminal-root .xterm-viewport",
    ".surface-terminal .terminal-root .xterm-screen",
    ".surface-terminal .terminal-root .xterm-helper-textarea",
  ];

  for (const selector of expectedSelectors) {
    const block = cssBlockContaining(inlineStyle, selector);
    assert.match(
      block,
      /background:\s*(?:#0a0d12|var\(\s*--color-canvas-dark\s*\))/i,
      `${selector} must explicitly paint the Dark Operator terminal background`,
    );
    assert.doesNotMatch(
      block,
      /transparent|backdrop-filter|opacity:\s*0\.[0-9]/i,
      `${selector} must not let the Canvas or sibling windows show through text`,
    );
  }
});

test("terminal overlay surfaces are opaque and copyable, not translucent glass (SPEC-2356 FR-033)", () => {
  for (const selector of [".terminal-overlay", ".terminal-overlay.visible"]) {
    const block = cssBlockContaining(inlineStyle, selector);
    assert.doesNotMatch(
      block,
      /transparent|backdrop-filter|opacity:\s*0\.[0-9]/i,
      `${selector} must not use translucent readable backgrounds`,
    );
    assert.match(
      block,
      /background:\s*(?:#0a0d12|var\(\s*--color-canvas-dark\s*\)|var\(\s*--color-surface(?:-elevated)?\s*\)|color-mix\([^;]+var\(\s*--color-canvas-dark\s*\)[^;]+\))/i,
      `${selector} must use an opaque terminal/text surface background`,
    );
  }
});

test("agent-state telemetry never makes readable workspace windows translucent (SPEC-2356 FR-033)", () => {
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  for (const state of ["idle", "not_started"]) {
    const selector = `.workspace-window[data-agent-state="${state}"]`;
    const block = cssBlockContaining(css, selector);
    assert.match(
      block,
      /opacity:\s*1\s*;/,
      `${selector} must keep the entire window fully opaque; state dimming belongs on non-window telemetry affordances only`,
    );
  }

  for (const state of ["idle", "not_started"]) {
    const block = cssBlockContaining(css, `[data-agent-state="${state}"]`);
    assert.doesNotMatch(
      block,
      /opacity:\s*0\.[0-9]/,
      `[data-agent-state="${state}"] must not dim every descendant surface by applying opacity to the parent window`,
    );
  }
});

test("non-terminal surface bodies still follow the overall theme (FR-013 boundary)", () => {
  // The Dark fix is scoped to .surface-terminal.  Other surfaces (Board /
  // Logs / File Tree / Branches / Knowledge / Workspace / Agent Kanban /
  // Console / Mock / Profile / Improvement) must keep tracking the active theme via --color-surface so tabbed windows
  // still flip body color when a non-terminal tab is selected.
  const otherSurfaceRule =
    /(?:\.surface-(?:file-tree|agent-kanban|branches|board|logs|knowledge|index|work|console|mock|profile|improvement)\s+\.window-body,?\s*)+\{[^}]*background:\s*var\(\s*--color-surface\s*\)/;
  assert.match(
    inlineStyle,
    otherSurfaceRule,
    "non-terminal surface bodies must continue to use var(--color-surface)",
  );
});

test("mountWindowBody clears every known surface class before applying the active surface", () => {
  const mountBody = appSource.match(
    /function\s+mountWindowBody\(windowData,\s*element\)\s*\{[\s\S]*?if\s*\(surface\s*===\s*"terminal"\)/,
  );
  assert.ok(mountBody, "expected mountWindowBody implementation");
  for (const surfaceClass of [
    "surface-terminal",
    "surface-agent-kanban",
    "surface-file-tree",
    "surface-branches",
    "surface-board",
    "surface-logs",
    "surface-knowledge",
    "surface-index",
    "surface-work",
    "surface-profile",
    "surface-improvement",
    "surface-console",
    "surface-mock",
  ]) {
    assert.match(
      mountBody[0],
      new RegExp(`"${surfaceClass}"`),
      `mountWindowBody must remove stale ${surfaceClass} classes before adding the active surface`,
    );
  }
});

test("every readable non-terminal surface participates in the opaque window chrome grammar", () => {
  for (const surface of [
    "file-tree",
    "branches",
    "board",
    "logs",
    "knowledge",
    "index",
    "work",
    "profile",
    "improvement",
    "console",
    "mock",
  ]) {
    assert.match(
      inlineStyle,
      new RegExp(`\\.workspace-window\\.surface-${surface}\\b`),
      `${surface} window shell must opt into the shared opaque surface background`,
    );
    assert.match(
      inlineStyle,
      new RegExp(`\\.surface-${surface}\\s+\\.titlebar\\b`),
      `${surface} titlebar must use the shared opaque titlebar rule`,
    );
    assert.match(
      inlineStyle,
      new RegExp(`\\.surface-${surface}\\s+\\.window-body\\b`),
      `${surface} body must use the shared opaque body rule`,
    );
    assert.match(
      inlineStyle,
      new RegExp(`\\.surface-${surface}\\s+\\.status-chip\\b`),
      `${surface} status chip text must use the shared readable chrome rule`,
    );
    assert.match(
      inlineStyle,
      new RegExp(`\\.surface-${surface}\\s+\\.icon-button\\b`),
      `${surface} icon button text must use the shared readable chrome rule`,
    );
  }
});

test("Rail item buttons reset UA chrome so Windows WebView2 stops drawing default border (FR-030)", () => {
  // SPEC-2356 FR-030 / SPEC-3038: WebView2 / Chromium の `<button>` UA
  // default は border + grey background を出す。`.op-rail__item` は ghost
  // ボタンとして icon + flyout + token focus ring のみで状態を表現するため、
  // base rule で UA chrome を解除する必要がある。
  const css = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  const itemRule = css.match(/\.op-rail__item\s*\{([^}]*)\}/);
  assert.ok(itemRule, "expected base .op-rail__item rule in components.css");
  const body = itemRule[1];
  assert.match(
    body,
    /appearance:\s*none/,
    ".op-rail__item must declare appearance:none to disable UA <button> chrome",
  );
  assert.match(
    body,
    /border:\s*0|border:\s*none/,
    ".op-rail__item must zero out border so WebView2 stops drawing the default frame",
  );
  assert.match(
    body,
    /background:\s*transparent/,
    ".op-rail__item must clear background so the rail surface shows through",
  );
});

// --- SPEC-2359 Work Unification (US-49): Workspace → Work/Works labels ---

// SPEC-2359 W-15 (FR-392) supersedes the former SC-207 blanket "no Workspace
// labels" rule (US-49): the W-13 three-layer model names the place "Workspace"
// (Workspace = place / Work = launch / Session = conversation), so surface
// entry points say "Workspace" while launch-level rows keep "Work".
test("FR-392: surface entry points are labelled 'Workspace' (3-layer model)", () => {
  const railLabel = document.querySelector(
    "#op-workspace-overview-entry .op-rail__flyout-label",
  );
  assert.ok(railLabel, "expected rail Workspace entry to exist");
  assert.equal(railLabel.textContent.trim(), "Workspace");

  const sidebarAria = document.querySelector("#op-workspace-overview-entry");
  assert.equal(sidebarAria.getAttribute("aria-label"), "Workspace");

  const paletteEntry = Array.from(document.querySelectorAll(".preset-button strong"))
    .find((btn) => /^Work(space)?$/.test(btn.textContent.trim()));
  if (paletteEntry) {
    assert.equal(paletteEntry.textContent.trim(), "Workspace",
      "palette surface entry must say 'Workspace'");
  }

  const hotkeyRows = Array.from(document.querySelectorAll(".op-hotkey-card__row span"))
    .map((el) => el.textContent.trim());
  assert.ok(!hotkeyRows.includes("Work surface"),
    "hotkey card must name the surface 'Workspace surface', not 'Work surface'");
});

function cssBlockContaining(css, selector) {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const regex = new RegExp(`(?:^|\\n)([^{}]*${escaped}[^{}]*)\\{([^}]*)\\}`, "m");
  const match = css.match(regex);
  assert.ok(match, `missing CSS rule containing selector: ${selector}`);
  return match[2];
}


// === merged from origin/develop: SPEC-1939/2014 perf + Launch Wizard coverage ===

// SPEC-3245 Phase 3: the Intake session command reuses the pending-wizard
// mechanism (formerly Start Work) to keep the modal open before backend state.
test("Intake session command opens a pending wizard before backend state arrives", () => {
  const commandCase = appSource.match(
    /case\s+"intake-session":[\s\S]*?case\s+"theme-cycle"/,
  );
  assert.ok(commandCase, "expected Intake session command case");
  assert.match(
    commandCase[0],
    /openIntakePendingWizard\(\)[\s\S]*?kind:\s*"open_intake_session"/,
    "expected Intake session to render a local pending wizard before sending open_intake_session",
  );
  assert.match(
    launchWizardSource,
    /function\s+openIntakePendingWizard\(\)[\s\S]*?title:\s*"Intake"[\s\S]*?message:\s*"Preparing Intake session\.\.\."/,
    "expected Intake pending wizard copy to avoid Start Work wording",
  );
  assert.match(
    launchWizardSource,
    /let\s+launchWizardOpening\s*=\s*null/,
    "expected local pending wizard state",
  );
  assert.match(
    launchWizardSource,
    /if\s*\(!launchWizard\s*&&\s*!launchWizardOpenError\s*&&\s*!launchWizardOpening\)/,
    "renderLaunchWizard must keep the modal open for local pending Start Work state",
  );
});

test("Hydrated Intake wizard uses Curate copy instead of direct Launch copy", () => {
  assert.match(
    launchWizardSource,
    /const\s+isIntakeWizard\s*=\s*launchWizard\.mode\s*===\s*"intake"/,
    "hydrated Intake wizard must derive copy from the backend mode",
  );
  assert.match(
    launchWizardSource,
    /isIntakeWizard[\s\S]{0,120}?"Curate session"/,
    "hydrated Intake wizard meta must identify the Curate lane",
  );
  assert.match(
    launchWizardSource,
    /isIntakeWizard\s*\?\s*"Intake setup"\s*:\s*"Start methods"/,
    "hydrated Intake wizard must not reuse the direct Launch start-method heading",
  );
  assert.match(
    launchWizardSource,
    /"Choose how to prepare this intake session\."/,
    "hydrated Intake wizard must use Curate-oriented start-method copy",
  );
  assert.match(
    launchWizardSource,
    /"Other ways to prepare or resume this intake session\."/,
    "hydrated Intake wizard must avoid direct Launch available-group copy",
  );
  assert.match(
    launchWizardSource,
    /isIntakeWizard\s*\?\s*"Optional — describe the work to turn into an Issue or SPEC\. You can skip this\."/,
    "hydrated Intake wizard must not mention the Plan Agent in the Register an Issue prompt",
  );
});

test("Agent Kanban Launch Agent action opens pending Launch Agent wizard with lane target", () => {
  const factoryStart = appSource.indexOf(
    "const agentKanbanSurface = createAgentKanbanSurface({",
  );
  assert.notEqual(factoryStart, -1, "expected Agent Kanban surface factory wiring");
  const factoryEnd = appSource.indexOf("\n      // SPEC-3064", factoryStart);
  assert.notEqual(factoryEnd, -1, "expected end marker after Agent Kanban factory wiring");
  const factoryCall = appSource.slice(factoryStart, factoryEnd);
  assert.match(
    factoryCall,
    /onLaunchAgent:\s*\(\{\s*boardId,\s*laneId\s*\}\)\s*=>/,
    "lane action must be named as Launch Agent, not an add-existing-window path",
  );
  assert.doesNotMatch(
    factoryCall,
    /onAddAgent:/,
    "Agent Kanban must not expose the old Add Agent callback name",
  );
  assert.match(
    factoryCall,
    /agentKanbanPendingPlacement\.begin\(\{\s*boardId,\s*laneId,\s*knownAgentWindowIds,/,
    "Launch Agent must record the originating board and lane as pending placement target",
  );
  assert.match(
    factoryCall,
    /openLaunchAgentPendingWizard\(\)[\s\S]*send\(\{[\s\S]{0,180}?kind:\s*"open_agent_kanban_launch_wizard"[\s\S]{0,180}?board_id:\s*boardId[\s\S]{0,180}?lane_id:\s*laneId/,
    "Launch Agent must open the pending wizard before requesting backend Launch Agent state with a lane target",
  );
  assert.match(
    appSource,
    /case\s+"launch_wizard_open_error":[\s\S]{0,220}?agentKanbanPendingPlacement\.clear\(\);[\s\S]{0,120}?applyLaunchWizardOpenErrorEvent\(event\);/,
    "backend wizard open errors must clear stale Kanban placement targets",
  );
  assert.match(
    appSource,
    /case\s+"launch_wizard_state":[\s\S]{0,220}?agentKanbanPendingPlacement\.clear\(\);[\s\S]{0,120}?applyLaunchWizardStateEvent\(event\);/,
    "backend wizard state must clear Kanban placement targets now owned by the launch session",
  );
});

test("Launch pending wizard clears when backend state, error, or local close wins", () => {
  assert.match(
    launchWizardSource,
    /function\s+clearLaunchWizardOpening\(\)\s*\{[\s\S]{0,120}?launchWizardOpening\s*=\s*null/,
    "expected a helper that clears pending open state",
  );
  assert.match(
    launchWizardSource,
    /function\s+closeLaunchWizardLocal\(\)[\s\S]{0,260}?clearLaunchWizardOpening\(\)/,
    "local wizard close must clear pending open state",
  );
  assert.match(
    launchWizardSource,
    /function\s+applyLaunchWizardOpenErrorEvent\(event\)[\s\S]{0,900}?clearLaunchWizardOpening\(\)[\s\S]{0,240}?launchWizardOpenError\s*=/,
    "backend open errors must replace pending open state",
  );
  assert.match(
    launchWizardSource,
    /function\s+applyLaunchWizardStateEvent\(event\)[\s\S]{0,900}?clearLaunchWizardOpening\(\)[\s\S]{0,260}?launchWizard\s*=\s*event\.wizard/,
    "backend wizard state must replace pending open state",
  );
});

test("Board workspace id sync avoids stringify and reuses active Work id cache", () => {
  // SPEC-3064 Phase 3 (E6c): the Work-id tracking moved into the board &
  // logs surface; app.js keeps the renderAppState / receive() call sites.
  const renderAppStateBody = extractFunctionBody(appSource, "renderAppState");
  assert.match(
    boardLogsSurfaceSource,
    /let\s+currentProjectWorkspaceKey\s*=/,
    "the board surface must track a stable key for the current Board workspace id set",
  );
  assert.match(
    boardLogsSurfaceSource,
    /let\s+activeWorkProjectionWorkspaceIds\s*=/,
    "the board surface must cache active Work projection workspace ids",
  );
  assert.match(
    boardLogsSurfaceSource,
    /function\s+syncCurrentProjectWorkspaceIds\s*\(/,
    "the board surface must centralize Board workspace id synchronization",
  );
  assert.doesNotMatch(
    renderAppStateBody,
    /JSON\.stringify\s*\(/,
    "workspace_state renderAppState must not serialize workspace id arrays for Board sync",
  );
  assert.match(
    renderAppStateBody,
    /syncCurrentProjectWorkspaceIds\s*\(\s*deriveCurrentProjectWorkspaceIds\s*\(\s*tab\?\.workspace\s*\|\|\s*\{\}\s*\)\s*,?\s*\)/,
    "renderAppState must route Board workspace id updates through the shared sync helper",
  );

  const deriveBody = extractFunctionBody(
    boardLogsSurfaceSource,
    "deriveCurrentProjectWorkspaceIds",
  );
  const cachedProjectionIndex = deriveBody.indexOf("activeWorkProjectionWorkspaceIds");
  const workspaceAgentsIndex = deriveBody.indexOf("workspaceState?.workspace?.agents");
  assert.ok(
    cachedProjectionIndex >= 0,
    "deriveCurrentProjectWorkspaceIds must read cached active Work projection ids",
  );
  assert.ok(
    workspaceAgentsIndex >= 0,
    "deriveCurrentProjectWorkspaceIds must keep the workspace-agent fallback",
  );
  assert.ok(
    cachedProjectionIndex < workspaceAgentsIndex,
    "cached active Work projection ids must be used before walking workspace agents",
  );
  assert.doesNotMatch(
    deriveBody,
    /activeWorkProjection\.active_works\.map/,
    "workspace_state id derivation must not remap active_work_projection on every render",
  );

  const receiveBody = extractFunctionBody(appSource, "receive");
  const activeProjectionCase = receiveBody.match(
    /case\s+"active_work_projection":[\s\S]*?break;/,
  );
  assert.ok(activeProjectionCase, "expected active_work_projection receive case");
  const cacheIndex = activeProjectionCase[0].indexOf(
    "cacheActiveWorkProjectionWorkspaceIds(activeWorkProjection)",
  );
  const syncIndex = activeProjectionCase[0].indexOf("syncCurrentProjectWorkspaceIds(");
  assert.ok(
    cacheIndex >= 0,
    "active_work_projection must update the cached active Work id set",
  );
  assert.ok(
    syncIndex > cacheIndex,
    "active_work_projection must sync Board ids after refreshing the cache",
  );
  assert.doesNotMatch(
    activeProjectionCase[0],
    /currentProjectWorkspaceId\s*=\s*deriveCurrentProjectWorkspaceIds/,
    "active_work_projection must not bypass the shared sync helper",
  );

  const syncBody = extractFunctionBody(
    boardLogsSurfaceSource,
    "syncCurrentProjectWorkspaceIds",
  );
  assert.match(
    syncBody,
    /workIdsKey\s*\(/,
    "sync helper must compare stable Board workspace id keys",
  );
  assert.match(
    syncBody,
    /refreshBoardCurrentWorkspaceId\s*\(\s*\)/,
    "sync helper must still refresh Board states when the id set changes",
  );
});

test("workspace_state hot path gates Project Tabs redraw by tab shell key", () => {
  const renderAppStateBody = extractFunctionBody(appSource, "renderAppState");
  assert.match(
    appSource,
    /let\s+renderedProjectTabsKey\s*=/,
    "app.js must track the last rendered Project Tabs shell key",
  );
  assert.match(
    appSource,
    /function\s+projectTabsRenderKey\s*\(/,
    "app.js must define a Project Tabs shell key helper",
  );
  assert.match(
    renderAppStateBody,
    /projectTabsRenderKey\s*\(\s*appState\s*\)/,
    "renderAppState must derive the next Project Tabs shell key from appState",
  );
  assert.match(
    renderAppStateBody,
    /renderedProjectTabsKey[\s\S]*!==[\s\S]*nextProjectTabsKey[\s\S]*renderProjectTabs\(\)/,
    "renderAppState must redraw Project Tabs only when the shell key changes",
  );
});

test("Project Tabs shell key ignores workspace geometry but includes tab identity", () => {
  const keyBody = extractFunctionBody(appSource, "projectTabsRenderKey");
  assert.match(
    keyBody,
    /active_tab_id/,
    "Project Tabs shell key must include active_tab_id",
  );
  for (const field of ["id", "title", "project_root"]) {
    assert.match(
      keyBody,
      new RegExp(`\\b${field}\\b`),
      `Project Tabs shell key must include tab ${field}`,
    );
  }
  for (const workspaceField of ["workspace", "windows", "geometry", "viewport"]) {
    assert.doesNotMatch(
      keyBody,
      new RegExp(`\\b${workspaceField}\\b`),
      `Project Tabs shell key must ignore ${workspaceField}`,
    );
  }
});

test("Project Tabs shell key avoids JSON stringify allocation", () => {
  const keyBody = extractFunctionBody(appSource, "projectTabsRenderKey");
  assert.match(
    appSource,
    /function\s+appendRenderKeyPart\s*\(/,
    "app.js must expose the primitive render-key append helper",
  );
  assert.match(
    keyBody,
    /appendRenderKeyPart\s*\(/,
    "Project Tabs shell key must append primitive fields directly",
  );
  assert.doesNotMatch(
    keyBody,
    /JSON\.stringify\s*\(/,
    "Project Tabs shell key must not serialize an object graph on every workspace_state",
  );
  assert.doesNotMatch(
    keyBody,
    /\.map\s*\(/,
    "Project Tabs shell key must not allocate a mapped tab array",
  );
  assert.match(
    keyBody,
    /for\s*\(\s*const\s+tab\s+of\s+tabs\s*\)/,
    "Project Tabs shell key must iterate tabs directly",
  );
});

test("Project Tabs renderer avoids mapped selector snapshots on tab switches", () => {
  const renderBody = extractFunctionBody(projectTabsRendererSource, "renderProjectTabs");
  assert.doesNotMatch(
    renderBody,
    /nextTabs\.map\s*\(/,
    "Project Tabs renderer must not allocate a mapped tab id source",
  );
  assert.doesNotMatch(
    renderBody,
    /querySelectorAll\s*\(/,
    "Project Tabs renderer hot path must walk existing child buttons directly",
  );
  assert.doesNotMatch(
    renderBody,
    /Array\.from\s*\(/,
    "Project Tabs renderer must not snapshot child buttons into an array",
  );
  assert.doesNotMatch(
    renderBody,
    /nextTabs\.forEach\s*\(/,
    "Project Tabs renderer must not allocate a per-render callback for tab ordering",
  );
  assert.match(
    renderBody,
    /for\s*\(\s*let\s+index\s*=\s*0;\s*index\s*<\s*nextTabs\.length;\s*index\s*\+=\s*1\s*\)/,
    "Project Tabs renderer must update tabs with an indexed direct loop",
  );
  assert.match(
    renderBody,
    /projectTabs\.children/,
    "Project Tabs renderer must reuse the live child collection for stale cleanup and ordering",
  );
});

test("hidden project picker does not rebuild Recent Projects on workspace_state", () => {
  // SPEC-3064 Phase 3 (E7): the picker / recent-projects renderers and
  // their render keys live in the project shell surface.
  const renderProjectPickerBody = extractFunctionBody(
    projectShellSurfaceSource,
    "renderProjectPicker",
  );
  assert.match(
    projectShellSurfaceSource,
    /let\s+renderedRecentProjectsKey\s*=/,
    "the project shell surface must track the visible picker Recent Projects render key",
  );
  assert.match(
    projectShellSurfaceSource,
    /function\s+recentProjectsRenderKey\s*\(/,
    "the project shell surface must define a Recent Projects render key helper",
  );
  assert.match(
    renderProjectPickerBody,
    /if\s*\(\s*shouldShow\s*\)[\s\S]*renderRecentProjects\(\)/,
    "renderProjectPicker must render Recent Projects only when the picker is visible",
  );
  assert.doesNotMatch(
    renderProjectPickerBody.replace(/if\s*\(\s*shouldShow\s*\)[\s\S]*?renderRecentProjects\(\)\s*;?/, ""),
    /renderRecentProjects\(\)/,
    "renderProjectPicker must not also call renderRecentProjects unconditionally",
  );
});

test("Recent Projects render key ignores workspace state", () => {
  const keyBody = extractFunctionBody(
    projectShellSurfaceSource,
    "recentProjectsRenderKey",
  );
  for (const field of ["title", "kind", "path"]) {
    assert.match(
      keyBody,
      new RegExp(`\\b${field}\\b`),
      `Recent Projects key must include ${field}`,
    );
  }
  for (const workspaceField of ["workspace", "windows", "geometry", "viewport"]) {
    assert.doesNotMatch(
      keyBody,
      new RegExp(`\\b${workspaceField}\\b`),
      `Recent Projects key must ignore ${workspaceField}`,
    );
  }
});

test("viewport-only workspace_state skips unchanged window reconciliation", () => {
  const renderWorkspaceBody = extractFunctionBody(appSource, "renderWorkspace");
  assert.match(
    appSource,
    /let\s+renderedWorkspaceWindowsKey\s*=/,
    "app.js must track the last reconciled Workspace Windows shell key",
  );
  assert.match(
    appSource,
    /function\s+workspaceWindowsRenderKey\s*\(/,
    "app.js must define a Workspace Windows render key helper",
  );
  assert.match(
    appSource,
    /let\s+viewportDomApplied\s*=\s*false\s*;/,
    "app.js must track whether the viewport DOM has been initialized",
  );

  const nextViewportIndex = renderWorkspaceBody.indexOf(
    "const nextViewport = viewportSyncState.applyServerViewport",
  );
  const viewportChangedIndex = renderWorkspaceBody.indexOf(
    "const viewportChanged = !sameViewportValues(viewport, nextViewport);",
  );
  const assignViewportIndex = renderWorkspaceBody.indexOf("viewport = nextViewport;");
  const applyViewportIndex = renderWorkspaceBody.indexOf("applyViewport();");
  const keyIndex = renderWorkspaceBody.indexOf(
    "const nextWorkspaceWindowsKey = workspaceWindowsRenderKey(workspace);",
  );
  const guardIndex = renderWorkspaceBody.indexOf(
    "if (renderedWorkspaceWindowsKey === nextWorkspaceWindowsKey)",
  );
  const classifyIndex = renderWorkspaceBody.indexOf(
    "classifyProjectWindowVisibility",
  );
  const ensureIndex = renderWorkspaceBody.indexOf("ensureWindow(windowData)");
  const focusIndex = renderWorkspaceBody.indexOf("focusWindowLocally(topmostId)");
  const applyCalls = [...renderWorkspaceBody.matchAll(/applyViewport\(\);/g)];

  assert.notEqual(
    nextViewportIndex,
    -1,
    "renderWorkspace must store the applied server viewport before DOM writes",
  );
  assert.ok(
    viewportChangedIndex > nextViewportIndex,
    "renderWorkspace must compare the current viewport with the applied server viewport",
  );
  assert.ok(
    assignViewportIndex > viewportChangedIndex,
    "renderWorkspace must compare before replacing the current viewport reference",
  );
  assert.equal(
    applyCalls.length,
    1,
    "renderWorkspace must keep server viewport DOM writes behind one guarded apply call",
  );
  assert.ok(
    applyViewportIndex > assignViewportIndex,
    "changed server viewport must assign the new viewport before applying DOM writes",
  );
  assert.match(
    renderWorkspaceBody.slice(assignViewportIndex, keyIndex),
    /if\s*\(\s*!viewportDomApplied\s*\|\|\s*viewportChanged\s*\)\s*\{\s*applyViewport\(\);\s*\}/,
    "unchanged server viewport must skip applyViewport only after the first DOM apply",
  );
  assert.ok(keyIndex > applyViewportIndex, "window key guard must run after viewport");
  assert.ok(guardIndex > keyIndex, "renderWorkspace must guard on the window key");
  assert.ok(
    guardIndex < classifyIndex && guardIndex < ensureIndex && guardIndex < focusIndex,
    "unchanged window key must return before reconciliation and focus activation",
  );
  assert.match(
    renderWorkspaceBody.slice(guardIndex, classifyIndex),
    /return\s*;/,
    "unchanged window key guard must return before reconciliation",
  );
});

// SPEC-2008 2026-06-20 Camera Focus Rework: manual maximize-to-fill was
// replaced by the per-viewer camera (frameWindow / enterOverview). The whole
// shared-maximized-geometry sync machinery — the coalesced scheduler, its
// pending-frame slot, and the sync body itself — was removed, so
// renderWorkspace no longer schedules any maximized viewport sync.
test("maximized viewport sync machinery is removed in favor of the per-viewer camera", () => {
  const renderWorkspaceBody = extractFunctionBody(appSource, "renderWorkspace");

  // The removed scheduler / sync functions must not be referenced as code in
  // either surface (only the removal-comment lines may mention the names).
  for (const symbol of [
    "scheduleMaximizedWindowsToViewportSync",
    "syncMaximizedWindowsToViewport",
    "maximizedViewportSyncFrame",
    "workspaceHasVisibleMaximizedWindow",
  ]) {
    // Allow the name to appear only on comment lines documenting the removal.
    const codeLines = (source) =>
      source
        .split("\n")
        .filter((line) => line.includes(symbol) && !line.trimStart().startsWith("//"));
    assert.deepEqual(
      codeLines(appSource),
      [],
      `app.js must not reference removed maximized-sync symbol ${symbol} in code`,
    );
    assert.deepEqual(
      codeLines(projectShellSurfaceSource),
      [],
      `project-shell-surface.js must not reference removed maximized-sync symbol ${symbol} in code`,
    );
  }

  assert.doesNotMatch(
    renderWorkspaceBody,
    /MaximizedWindowsToViewport/,
    "renderWorkspace must not schedule any maximized viewport sync",
  );
  assert.doesNotMatch(
    renderWorkspaceBody,
    /requestAnimationFrame\s*\(\s*syncMaximizedWindowsToViewport\s*\)/,
    "renderWorkspace must not schedule raw maximized sync frames per workspace_state",
  );
});

test("clamped no-op zoom returns before local viewport apply and persist work", () => {
  const zoomBody = extractFunctionBody(appSource, "zoomCanvasAt");
  assert.match(
    appSource,
    /function\s+sameViewportValues\s*\(/,
    "app.js must define a semantic viewport equality helper",
  );
  const nextViewportIndex = zoomBody.indexOf("const nextViewport = {");
  const guardIndex = zoomBody.indexOf("if (sameViewportValues(viewport, nextViewport))");
  const assignIndex = zoomBody.indexOf("viewport = nextViewport;");
  const recordIndex = zoomBody.indexOf("recordLocalViewportEdit();");
  const applyIndex = zoomBody.indexOf("applyViewport();");
  const persistIndex = zoomBody.indexOf("persistViewport();");

  assert.notEqual(nextViewportIndex, -1, "zoomCanvasAt must compute next viewport first");
  assert.ok(guardIndex > nextViewportIndex, "no-op guard must inspect the computed viewport");
  assert.ok(
    guardIndex < assignIndex
      && guardIndex < recordIndex
      && guardIndex < applyIndex
      && guardIndex < persistIndex,
    "clamped no-op zoom must return before assignment, local edit tracking, viewport writes, and persist scheduling",
  );
  assert.match(
    zoomBody.slice(guardIndex, assignIndex),
    /return\s*;/,
    "same viewport guard must return before changed-zoom apply/persist sequence",
  );
  assert.ok(
    assignIndex < recordIndex && recordIndex < applyIndex && applyIndex < persistIndex,
    "changed zoom must still assign next viewport before the existing apply/persist sequence",
  );
});

test("Workspace Windows render key ignores viewport and includes window shell fields", () => {
  const keyBody = extractFunctionBody(appSource, "workspaceWindowsRenderKey");
  assert.match(
    keyBody,
    /active_tab_id/,
    "Workspace Windows key must include active tab identity",
  );
  assert.match(
    keyBody,
    /allProjectWindowIds\s*\(\s*\)/,
    "Workspace Windows key must include all project window ids for stale mounted window cleanup",
  );
  for (const field of [
    "id",
    "preset",
    "title",
    "dynamic_title",
    "dynamic_title_detail",
    "purpose_title",
    "agent_id",
    "agent_color",
    "status",
    "geometry",
    "x",
    "y",
    "width",
    "height",
    "minimized",
    "maximized",
    "z_index",
    "tab_group_id",
    "tab_group_active",
  ]) {
    assert.match(
      keyBody,
      new RegExp(`\\b${field}\\b`),
      `Workspace Windows key must include ${field}`,
    );
  }
  assert.doesNotMatch(
    keyBody,
    /\bviewport\b/,
    "Workspace Windows key must ignore viewport-only state",
  );
});

test("Workspace Windows render key avoids JSON stringify allocation", () => {
  const keyBody = extractFunctionBody(appSource, "workspaceWindowsRenderKey");
  assert.match(
    appSource,
    /function\s+appendRenderKeyPart\s*\(/,
    "app.js must provide a primitive render-key append helper",
  );
  assert.match(
    keyBody,
    /appendRenderKeyPart\s*\(/,
    "Workspace Windows key must append primitive key parts directly",
  );
  assert.doesNotMatch(
    keyBody,
    /JSON\.stringify\s*\(/,
    "Workspace Windows key must not serialize a nested object graph",
  );
  assert.doesNotMatch(
    keyBody,
    /\bwindows\.map\s*\(/,
    "Workspace Windows key must not allocate per-window map results",
  );
  assert.match(
    keyBody,
    /for\s*\(\s*const\s+windowData\s+of\s+windows\s*\)/,
    "Workspace Windows key must walk windows with a direct loop",
  );
  assert.match(
    keyBody,
    /for\s*\(\s*const\s+windowId\s+of\s+allProjectWindowIds\s*\(\s*\)\s*\)/,
    "Workspace Windows key must include mounted project window ids with a direct loop",
  );
});

test("Window List skips unchanged row rebuilds after updating open state", () => {
  // SPEC-3064 Phase 3 (E7): the Window List dropdown renderer, its render
  // key, and its rendered-key slot live in the project shell surface.
  const renderWindowListBody = extractFunctionBody(
    projectShellSurfaceSource,
    "renderWindowList",
  );
  assert.match(
    projectShellSurfaceSource,
    /let\s+renderedWindowListKey\s*=/,
    "the project shell surface must track the last rendered Window List row key",
  );
  assert.match(
    projectShellSurfaceSource,
    /function\s+windowListRenderKey\s*\(/,
    "the project shell surface must define a Window List render key helper",
  );

  const hiddenIndex = renderWindowListBody.indexOf("windowListPanel.hidden");
  const ariaIndex = renderWindowListBody.indexOf("aria-expanded");
  const keyIndex = renderWindowListBody.indexOf(
    "const nextWindowListKey = windowListRenderKey();",
  );
  const closedGuardIndex = renderWindowListBody.indexOf("if (!windowListOpen)");
  const closedSentinelIndex = renderWindowListBody.indexOf(
    'renderedWindowListKey = "__closed__";',
  );
  const guardIndex = renderWindowListBody.indexOf(
    "if (renderedWindowListKey === nextWindowListKey)",
  );
  const clearIndex = renderWindowListBody.indexOf("windowListPanel.innerHTML = \"\";");

  assert.notEqual(hiddenIndex, -1, "renderWindowList must update panel hidden state");
  assert.notEqual(ariaIndex, -1, "renderWindowList must update trigger aria-expanded");
  assert.notEqual(closedGuardIndex, -1, "renderWindowList must have a closed fast path");
  assert.notEqual(
    closedSentinelIndex,
    -1,
    "closed Window List fast path must store a stable sentinel key",
  );
  assert.ok(
    closedGuardIndex > hiddenIndex && closedGuardIndex > ariaIndex,
    "closed Window List fast path must run after chrome hidden/expanded updates",
  );
  assert.ok(
    closedGuardIndex < keyIndex,
    "closed Window List fast path must return before key generation",
  );
  assert.ok(
    closedSentinelIndex > closedGuardIndex && closedSentinelIndex < keyIndex,
    "closed Window List fast path must store the sentinel before any key generation",
  );
  assert.match(
    renderWindowListBody.slice(closedGuardIndex, keyIndex),
    /return\s*;/,
    "closed Window List fast path must return before windowListRenderKey()",
  );
  assert.ok(
    keyIndex > hiddenIndex && keyIndex > ariaIndex,
    "open Window List key must run after open state updates",
  );
  assert.ok(guardIndex > keyIndex, "renderWindowList must guard on the Window List key");
  assert.ok(
    guardIndex < clearIndex,
    "unchanged Window List key must return before row clearing/rebuild",
  );
  assert.match(
    renderWindowListBody.slice(guardIndex, clearIndex),
    /return\s*;/,
    "unchanged Window List key guard must return before clearing rows",
  );

  const toggleWindowListBody = extractFunctionBody(
    projectShellSurfaceSource,
    "toggleWindowList",
  );
  const invalidateIndex = toggleWindowListBody.indexOf("renderedWindowListKey = \"\";");
  const renderIndex = toggleWindowListBody.indexOf("renderWindowList();");
  const requestIndex = toggleWindowListBody.indexOf("requestWindowList();");
  assert.notEqual(invalidateIndex, -1, "toggleWindowList must invalidate the Window List key");
  assert.ok(
    invalidateIndex < renderIndex && renderIndex < requestIndex,
    "opening Window List must render current rows before requesting backend entries",
  );
});

test("Window List row source rebuild avoids mapped intermediate arrays", () => {
  const renderWindowListBody = extractFunctionBody(
    projectShellSurfaceSource,
    "renderWindowList",
  );
  assert.doesNotMatch(
    renderWindowListBody,
    /workspaceWindows\.map\s*\(/,
    "Window List row rebuild must not allocate a mapped workspace-window lookup source",
  );
  assert.doesNotMatch(
    renderWindowListBody,
    /windowListEntries[\s\S]*?\.map\s*\(/,
    "Window List row rebuild must not map backend entries before filtering",
  );
  assert.doesNotMatch(
    renderWindowListBody,
    /\.filter\s*\(/,
    "Window List row rebuild must avoid chained filter allocation",
  );
  // SPEC-3038 (2026-06-20): rows now come from the cross-tab window-list
  // model grouped by project tab, rendered with direct loops (no inline alloc).
  assert.match(
    renderWindowListBody,
    /for\s*\(\s*const\s+group\s+of\s+model\.groups\s*\)/,
    "Window List row rebuild must iterate the cross-tab model groups with a direct loop",
  );
  assert.match(
    renderWindowListBody,
    /for\s*\(\s*const\s+entry\s+of\s+group\.entries\s*\)/,
    "Window List row rebuild must derive per-group entries with a direct loop",
  );
});

test("Window List render key ignores viewport and includes row shell fields", () => {
  const keyBody = extractFunctionBody(
    projectShellSurfaceSource,
    "windowListRenderKey",
  );
  // appendRenderKeyPart stays in app.js (shared with the render keys that
  // remained there) and reaches the surface through the deps bag.
  assert.match(
    appSource,
    /function\s+appendRenderKeyPart\s*\(/,
    "app.js must expose the primitive render-key append helper",
  );
  assert.match(
    keyBody,
    /appendRenderKeyPart\s*\(/,
    "Window List key must append primitive fields directly",
  );
  assert.doesNotMatch(
    keyBody,
    /JSON\.stringify\s*\(/,
    "Window List key must not serialize an object graph while open",
  );
  assert.doesNotMatch(
    keyBody,
    /\.map\s*\(/,
    "Window List key must not allocate mapped arrays while open",
  );
  assert.match(
    keyBody,
    /active_tab_id/,
    "Window List key must include active tab identity",
  );
  assert.match(
    keyBody,
    /windowListEntries/,
    "Window List key must include server-provided window_list entries",
  );
  assert.match(
    keyBody,
    /groupProjectWindowList\s*\(/,
    "Window List key must include the cross-tab window-list model identity/order",
  );
  assert.match(
    keyBody,
    /runtimeStateForWindow\s*\(/,
    "Window List key must include runtime state used by status chips",
  );
  for (const field of [
    "id",
    "preset",
    "title",
    "dynamic_title",
    "dynamic_title_detail",
    "purpose_title",
    "agent_id",
    "agent_color",
    "status",
    "geometry",
    "x",
    "y",
    "width",
    "height",
    "minimized",
    "maximized",
    "z_index",
    "tab_group_id",
    "tab_group_active",
  ]) {
    assert.match(
      keyBody,
      new RegExp(`\\b${field}\\b`),
      `Window List key must include ${field}`,
    );
  }
  assert.doesNotMatch(
    keyBody,
    /\bviewport\b/,
    "Window List key must ignore viewport-only state",
  );
});

test("Static project chrome renderers guard unchanged DOM writes", () => {
  assert.match(
    appSource,
    /let\s+renderedAppVersionLabel\s*=/,
    "app.js must track the last rendered app version label",
  );
  // SPEC-3064 Phase 3 (E7): the static project chrome renderers and their
  // rendered-key slots live in the project shell surface.
  assert.match(
    projectShellSurfaceSource,
    /let\s+renderedProjectPickerKey\s*=/,
    "the project shell surface must track the last rendered Project Picker key",
  );
  assert.match(
    projectShellSurfaceSource,
    /let\s+renderedProjectOnboardingKey\s*=/,
    "the project shell surface must track the last rendered Project Onboarding key",
  );
  assert.match(
    projectShellSurfaceSource,
    /let\s+renderedActionAvailabilityKey\s*=/,
    "the project shell surface must track the last rendered action availability key",
  );

  const renderAppVersionBody = extractFunctionBody(appSource, "renderAppVersion");
  const versionGuardIndex = renderAppVersionBody.indexOf(
    "if (renderedAppVersionLabel === label)",
  );
  const versionHiddenIndex = renderAppVersionBody.indexOf("appVersionLabel.hidden");
  assert.ok(
    versionGuardIndex !== -1 && versionGuardIndex < versionHiddenIndex,
    "renderAppVersion must return before unchanged label DOM writes",
  );

  const renderProjectPickerBody = extractFunctionBody(
    projectShellSurfaceSource,
    "renderProjectPicker",
  );
  const pickerKeyIndex = renderProjectPickerBody.indexOf(
    "const nextProjectPickerKey = projectPickerRenderKey(activeTab);",
  );
  const pickerGuardIndex = renderProjectPickerBody.indexOf(
    "if (renderedProjectPickerKey === nextProjectPickerKey)",
  );
  const pickerClassIndex = renderProjectPickerBody.indexOf(
    "projectPicker.classList.toggle",
  );
  assert.ok(
    pickerKeyIndex !== -1 &&
      pickerGuardIndex > pickerKeyIndex &&
      pickerGuardIndex < pickerClassIndex,
    "renderProjectPicker must return before unchanged picker DOM writes",
  );

  const renderProjectOnboardingBody = extractFunctionBody(
    projectShellSurfaceSource,
    "renderProjectOnboarding",
  );
  const onboardingKeyIndex = renderProjectOnboardingBody.indexOf(
    "const nextProjectOnboardingKey = projectOnboardingRenderKey(tab);",
  );
  const onboardingGuardIndex = renderProjectOnboardingBody.indexOf(
    "if (renderedProjectOnboardingKey === nextProjectOnboardingKey)",
  );
  const onboardingClassIndex = renderProjectOnboardingBody.indexOf(
    "projectOnboarding.classList",
  );
  assert.ok(
    onboardingKeyIndex !== -1 &&
      onboardingGuardIndex > onboardingKeyIndex &&
      onboardingGuardIndex < onboardingClassIndex,
    "renderProjectOnboarding must return before unchanged onboarding DOM writes",
  );

  const updateActionAvailabilityBody = extractFunctionBody(
    projectShellSurfaceSource,
    "updateActionAvailability",
  );
  const actionKeyIndex = updateActionAvailabilityBody.indexOf(
    "const nextActionAvailabilityKey = actionAvailabilityRenderKey(activeTab);",
  );
  const actionGuardIndex = updateActionAvailabilityBody.indexOf(
    "if (renderedActionAvailabilityKey === nextActionAvailabilityKey)",
  );
  const disabledIndex = updateActionAvailabilityBody.indexOf("addButton.disabled");
  assert.ok(
    actionKeyIndex !== -1 &&
      actionGuardIndex > actionKeyIndex &&
      actionGuardIndex < disabledIndex,
    "updateActionAvailability must return before unchanged disabled-state DOM writes",
  );
});

test("App version label opens latest release notes when an update is available", () => {
  assert.match(
    appSource,
    /const\s+openReleaseNotesFromLabel\s*=\s*\(\)\s*=>\s*{\s*releaseNotesWindow\.open\(versionState\.latest\s*\|\|\s*versionState\.current\s*\|\|\s*null\);\s*};/s,
    "#app-version should focus the latest available release notes before falling back to the running version",
  );
});

test("Static project chrome keys ignore workspace geometry and include visible transition state", () => {
  const pickerKeyBody = extractFunctionBody(
    projectShellSurfaceSource,
    "projectPickerRenderKey",
  );
  assert.match(
    pickerKeyBody,
    /activeTab/,
    "Project Picker key must include visible active-tab state",
  );
  assert.match(
    pickerKeyBody,
    /projectError/,
    "Project Picker key must include picker error text",
  );
  assert.match(
    pickerKeyBody,
    /recentProjectsRenderKey\s*\(/,
    "Project Picker key must include Recent Projects only when visible",
  );

  const onboardingKeyBody = extractFunctionBody(
    projectShellSurfaceSource,
    "projectOnboardingRenderKey",
  );
  for (const field of ["kind", "project_root"]) {
    assert.match(
      onboardingKeyBody,
      new RegExp(`\\b${field}\\b`),
      `Project Onboarding key must include ${field}`,
    );
  }

  const actionKeyBody = extractFunctionBody(
    projectShellSurfaceSource,
    "actionAvailabilityRenderKey",
  );
  assert.match(
    actionKeyBody,
    /activeTab/,
    "Action Availability key must include active-tab availability",
  );

  for (const keyBody of [pickerKeyBody, onboardingKeyBody, actionKeyBody]) {
    for (const workspaceField of ["viewport", "windows", "geometry"]) {
      assert.doesNotMatch(
        keyBody,
        new RegExp(`\\b${workspaceField}\\b`),
        `Static project chrome keys must ignore ${workspaceField}`,
      );
    }
  }
});

test("Static project chrome keys avoid JSON stringify allocation", () => {
  for (const name of ["projectPickerRenderKey", "projectOnboardingRenderKey"]) {
    const keyBody = extractFunctionBody(projectShellSurfaceSource, name);
    assert.match(
      keyBody,
      /appendRenderKeyPart\s*\(/,
      `${name} must append primitive fields directly`,
    );
    assert.doesNotMatch(
      keyBody,
      /JSON\.stringify\s*\(/,
      `${name} must not serialize an object graph on every workspace_state`,
    );
  }
});

test("Static project chrome reuses the renderAppState active tab lookup", () => {
  const renderAppStateBody = extractFunctionBody(appSource, "renderAppState");
  const tabLookupIndex = renderAppStateBody.indexOf("const tab = activeProjectTab();");
  const pickerCallIndex = renderAppStateBody.indexOf("renderProjectPicker(tab);");
  const actionCallIndex = renderAppStateBody.indexOf("updateActionAvailability(tab);");
  const onboardingCallIndex = renderAppStateBody.indexOf("renderProjectOnboarding(tab);");
  const workspaceCallIndex = renderAppStateBody.indexOf(
    "renderWorkspace(tab?.workspace || emptyWorkspace());",
  );
  assert.ok(
    tabLookupIndex >= 0 &&
      pickerCallIndex > tabLookupIndex &&
      actionCallIndex > tabLookupIndex &&
      onboardingCallIndex > tabLookupIndex &&
      workspaceCallIndex > tabLookupIndex,
    "renderAppState must resolve the active tab once and pass it through static chrome and workspace renderers",
  );

  const pickerKeyBody = extractFunctionBody(
    projectShellSurfaceSource,
    "projectPickerRenderKey",
  );
  const actionKeyBody = extractFunctionBody(
    projectShellSurfaceSource,
    "actionAvailabilityRenderKey",
  );
  for (const keyBody of [pickerKeyBody, actionKeyBody]) {
    assert.doesNotMatch(
      keyBody,
      /activeProjectTab\s*\(\s*\)/,
      "static chrome key helpers must not rescan activeProjectTab on the workspace_state hot path",
    );
  }
});

test("Per-window renderer guards unchanged DOM writes after mount and preset sync", () => {
  assert.match(
    appSource,
    /const\s+renderedWindowElementKeys\s*=\s*new\s+Map\s*\(/,
    "app.js must track per-window element render keys",
  );
  assert.match(
    appSource,
    /function\s+windowElementRenderKey\s*\(/,
    "app.js must define a per-window element render key helper",
  );

  const ensureWindowBody = extractFunctionBody(appSource, "ensureWindow");
  const mountIndex = ensureWindowBody.indexOf("mountWindowBody(windowData, element);");
  const keyIndex = ensureWindowBody.indexOf(
    "const nextWindowElementKey = windowElementRenderKey(windowData);",
  );
  const guardIndex = ensureWindowBody.indexOf(
    "if (renderedWindowElementKeys.get(windowData.id) === nextWindowElementKey)",
  );
  const storeIndex = ensureWindowBody.indexOf(
    "renderedWindowElementKeys.set(windowData.id, nextWindowElementKey);",
  );
  const renderKeyCalls = [
    ...ensureWindowBody.matchAll(/windowElementRenderKey\(windowData\)/g),
  ];
  const titleIndex = ensureWindowBody.indexOf(".title-text");
  const roleBadgeIndex = ensureWindowBody.indexOf("setWindowRoleBadge");
  const tabsIndex = ensureWindowBody.indexOf("renderWindowTabs");
  const agentColorIndex = ensureWindowBody.indexOf("agentColor");
  // SPEC-2008 camera-focus: the minimized/maximized class toggles were removed;
  // the surviving guarded class write is the tab-group "tabbed" toggle.
  const classIndex = ensureWindowBody.indexOf('classList.toggle("tabbed"');
  const styleIndex = ensureWindowBody.indexOf("element.style.zIndex");
  const statusIndex = ensureWindowBody.indexOf("applyStatus");
  const fitIndex = ensureWindowBody.indexOf("scheduleTerminalFit");

  assert.notEqual(mountIndex, -1, "ensureWindow must keep preset/body mount logic");
  assert.ok(keyIndex > mountIndex, "per-window key must run after preset/body mounting");
  assert.ok(guardIndex > keyIndex, "ensureWindow must guard on the per-window key");
  assert.equal(
    renderKeyCalls.length,
    1,
    "ensureWindow must reuse the computed per-window key instead of recomputing it",
  );
  for (const [label, index] of [
    ["title", titleIndex],
    ["role badge", roleBadgeIndex],
    ["tab strip", tabsIndex],
    ["agent color", agentColorIndex],
    ["class", classIndex],
    ["style", styleIndex],
    ["status", statusIndex],
    ["terminal fit", fitIndex],
  ]) {
    assert.notEqual(index, -1, `ensureWindow must still contain ${label} writes`);
    assert.ok(
      guardIndex < index,
      `unchanged per-window key must return before ${label} writes`,
    );
  }
  assert.notEqual(
    storeIndex,
    -1,
    "ensureWindow must store the previously computed per-window key",
  );
  assert.ok(
    statusIndex < storeIndex,
    "ensureWindow must store changed keys after status/detail normalization",
  );
  assert.match(
    ensureWindowBody.slice(guardIndex, titleIndex),
    /return\s*;/,
    "unchanged per-window key guard must return before window DOM writes",
  );
});

test("Per-window render key covers DOM shell fields and removal cleanup", () => {
  const keyBody = extractFunctionBody(appSource, "windowElementRenderKey");
  assert.match(
    keyBody,
    /windowDisplayTitle\s*\(/,
    "per-window key must include displayed title",
  );
  assert.match(
    keyBody,
    /windowTitleTooltip\s*\(/,
    "per-window key must include title tooltip",
  );
  assert.match(
    keyBody,
    /windowGroupId\s*\(\s*windowData\s*\)/,
    "per-window key must derive the tab group for tab strip inputs",
  );
  assert.match(
    keyBody,
    /appendRenderKeyPart\s*\(/,
    "per-window key must append primitive key fields directly",
  );
  assert.match(
    keyBody,
    /detailMap\.get\s*\(\s*windowData\.id\s*\)/,
    "per-window key must include status detail text",
  );
  // SPEC-2008 camera-focus: the viewport-relative `maximized_fill` /
  // maximizedGeometry render-key input was removed (windows always render at
  // their own world geometry now, so per-client fill never enters the key).
  assert.doesNotMatch(
    keyBody,
    /maximizedGeometry/,
    "per-window key must not depend on the removed maximized fill geometry",
  );
  assert.doesNotMatch(
    keyBody,
    /\bmaximized_fill\b/,
    "per-window key must not carry the removed maximized_fill marker",
  );
  for (const field of [
    "id",
    "preset",
    "title",
    "dynamic_title",
    "dynamic_title_detail",
    "purpose_title",
    "agent_id",
    "agent_color",
    "status",
    "geometry",
    "x",
    "y",
    "width",
    "height",
    "z_index",
    "tab_group_id",
    "tab_group_active",
    "tabs",
  ]) {
    assert.match(
      keyBody,
      new RegExp(`\\b${field}\\b`),
      `per-window key must include ${field}`,
    );
  }

  const renderWorkspaceBody = extractFunctionBody(appSource, "renderWorkspace");
  const cleanupIndex = renderWorkspaceBody.indexOf("renderedWindowElementKeys.delete(windowId);");
  const removeIndex = renderWorkspaceBody.indexOf("element.remove();");
  assert.notEqual(cleanupIndex, -1, "window removal must clear per-window render key");
  assert.ok(
    cleanupIndex < removeIndex,
    "per-window render key cleanup must happen before element removal",
  );
});

test("Per-window render key avoids JSON stringify and mapped tab allocation", () => {
  const keyBody = extractFunctionBody(appSource, "windowElementRenderKey");
  assert.doesNotMatch(
    keyBody,
    /JSON\.stringify\s*\(/,
    "per-window key must not serialize an object graph for each changed window",
  );
  assert.doesNotMatch(
    keyBody,
    /windowTabsFor\s*\(\s*windowData\s*\)\.map\s*\(/,
    "per-window key must not allocate a mapped tab shell array",
  );
  assert.doesNotMatch(
    keyBody,
    /windowTabsFor\s*\(\s*windowData\s*\)/,
    "per-window key must avoid allocating a filtered tab array",
  );
  assert.match(
    keyBody,
    /for\s*\(\s*const\s+tab\s+of\s+activeWorkspace\s*\(\s*\)\.windows\s*\|\|\s*\[\]\s*\)/,
    "per-window key must iterate workspace tab candidates directly",
  );
  assert.match(
    keyBody,
    /windowGroupId\s*\(\s*tab\s*\)\s*!==\s*tabGroupId[\s\S]+continue\s*;/,
    "per-window key must keep only tabs in the same group",
  );
  for (const field of [
    "runtime_state",
    "detail",
    "display_title",
    "title_tooltip",
    "role_badge",
    "geometry_revision",
    "tabs",
    "tab_group_id",
    "tab_group_active",
  ]) {
    assert.match(
      keyBody,
      new RegExp(`\\b${field}\\b`),
      `per-window primitive key must keep ${field}`,
    );
  }
  // SPEC-2008 camera-focus: maximized_fill was removed from the primitive key.
  assert.doesNotMatch(
    keyBody,
    /\bmaximized_fill\b/,
    "per-window primitive key must not carry the removed maximized_fill marker",
  );
});

// SPEC-2008 camera-focus (UX amendment 2026-06-20): single click focuses
// WITHOUT moving the camera; only a deliberate double click frames it; the
// manual resize handle is restored.
test("window template restores the manual resize handle as a window-body sibling", () => {
  const ensureWindowBody = extractFunctionBody(appSource, "ensureWindow");
  // The resize handle is a sibling div of the window body, NOT a window-action
  // button (window-actions stays close-only).
  assert.match(
    ensureWindowBody,
    /<div class="window-body"><\/div>\s*<div class="resize-handle"><\/div>/,
    "the resize handle must be a sibling div after the window body",
  );
  // SPEC-2008 retired maximize/minimize; SPEC-2356 Anshin (FR-041/044) added
  // the STOP + RESTART kill-switch alongside close. Window-actions may carry
  // those, but never maximize/minimize.
  const windowActions = ensureWindowBody.match(
    /<div class="window-actions">[\s\S]*?<\/div>/,
  );
  assert.ok(windowActions, "window-actions block must exist");
  assert.match(windowActions[0], /data-action="close"/, "close must remain");
  assert.match(windowActions[0], /data-action="stop"/, "STOP kill-switch must be present");
  assert.match(windowActions[0], /data-action="restart"/, "RESTART must be present");
  assert.doesNotMatch(
    windowActions[0],
    /data-action="(maximize|minimize)"/,
    "window-actions must never reintroduce maximize/minimize buttons",
  );
  // The handle is wired to begin a local geometry edit and arm the resize state
  // (camera is untouched; only the window geometry changes).
  const handleWiringIndex = ensureWindowBody.indexOf(
    'resizeHandle.addEventListener("pointerdown"',
  );
  assert.notEqual(handleWiringIndex, -1, "resize handle must wire pointerdown");
  const handleWiring = ensureWindowBody.slice(handleWiringIndex);
  assert.match(
    handleWiring,
    /beginLocalGeometryEdit\(/,
    "resize pointerdown must begin a local geometry edit",
  );
  assert.match(
    handleWiring,
    /resizeState\s*=\s*\{/,
    "resize pointerdown must arm the resize state",
  );
  // The resize handle must NOT fly the camera — no frameWindow on resize.
  assert.doesNotMatch(
    handleWiring.slice(0, handleWiring.indexOf("});")),
    /frameWindow\(/,
    "resizing a window must never move the camera",
  );
});

test("resize geometry enforces the 420x260 floor and writes inline window size", () => {
  const applyBody = extractFunctionBody(appSource, "applyResizePointermove");
  // The pointer-state → geometry helper owns the min floor (default 420x260);
  // applyResizePointermove writes the result to the element's inline size.
  assert.match(
    applyBody,
    /resizeGeometryFromPointerState\(/,
    "resize apply must derive geometry from the shared pointer-state helper",
  );
  assert.match(
    applyBody,
    /element\.style\.width\s*=\s*`\$\{width\}px`/,
    "resize apply must write the inline window width",
  );
  assert.match(
    applyBody,
    /element\.style\.height\s*=\s*`\$\{height\}px`/,
    "resize apply must write the inline window height",
  );
});

test("titlebar click focuses on single click and only frames on double click", () => {
  const clickBody = extractFunctionBody(appSource, "handleTitlebarClick");
  // A double-click window + threshold gate the framing gesture.
  assert.match(
    appSource,
    /const\s+TITLEBAR_DOUBLE_CLICK_MS\s*=\s*\d+\s*;/,
    "a double-click threshold must gate the framing gesture",
  );
  assert.match(
    clickBody,
    /lastTitlebarClick[\s\S]*now\s*-\s*lastTitlebarClick\.at\s*<=\s*TITLEBAR_DOUBLE_CLICK_MS/,
    "handleTitlebarClick must detect a double click within the threshold",
  );
  // Double click → frame (camera moves). Single click → focus only (no camera).
  const doubleIndex = clickBody.indexOf("isDoubleClick");
  const frameIndex = clickBody.indexOf("frameWindow(windowId)");
  const focusIndex = clickBody.indexOf("focusWindowRemotely(windowId)");
  assert.ok(doubleIndex !== -1 && frameIndex !== -1 && focusIndex !== -1,
    "handleTitlebarClick must branch double→frame, single→focus");
  assert.ok(
    frameIndex < focusIndex,
    "the double-click frame branch must precede the single-click focus fallback",
  );
  // The single-click path uses focusWindowRemotely WITHOUT center, so no bounds
  // and no camera move.
  assert.doesNotMatch(
    clickBody,
    /focusWindowRemotely\(\s*windowId\s*,\s*\{\s*center/,
    "a single titlebar click must not center (move) the camera",
  );
});

test("body and terminal single click focus the window without moving the camera", () => {
  // focusWindowRemotely without {center:true} sends focus_window WITHOUT bounds
  // (camera unchanged); the body/terminal mousedown handlers use that path.
  const focusRemoteBody = extractFunctionBody(appSource, "focusWindowRemotely");
  assert.match(
    focusRemoteBody,
    /if\s*\(\s*center\s*\)\s*payload\.bounds\s*=\s*visibleBounds\(\)/,
    "focusWindowRemotely must only attach bounds when explicitly centering",
  );
  // The non-terminal body click and terminal-root / overlay click all focus
  // only (no center → no camera move). Pinned as source patterns since these
  // listeners live inside the window mount closures.
  assert.match(
    appSource,
    /terminalRoot\.addEventListener\("mousedown",\s*\(\)\s*=>\s*\{\s*focusWindowRemotely\(windowData\.id\);\s*\}\)/,
    "terminal click must focus only (no camera move)",
  );
  assert.match(
    appSource,
    /body\.addEventListener\("mousedown",\s*\(\)\s*=>\s*\{\s*focusWindowRemotely\(windowData\.id\);\s*\}\)/,
    "non-terminal body click must focus only (no camera move)",
  );
});

test("Focus class updates skip unchanged focus and avoid all-window scans", () => {
  const focusBody = extractFunctionBody(appSource, "focusWindowLocally");
  assert.match(
    focusBody,
    /const\s+targetElement\s*=\s*windowMap\.get\s*\(\s*windowId\s*\)/,
    "focusWindowLocally must resolve the requested window directly",
  );
  assert.match(
    focusBody,
    /focusedId\s*===\s*windowId[\s\S]+targetElement\?\.classList\.contains\s*\(\s*"focused"\s*\)[\s\S]+return\s*;/,
    "focusWindowLocally must return when focus is unchanged and the class is already applied",
  );
  assert.doesNotMatch(
    focusBody,
    /windowMap\.entries\s*\(/,
    "focusWindowLocally must not scan every mounted window",
  );
});

test("Focus class updates touch previous/current elements and keep no-topmost reset", () => {
  const focusBody = extractFunctionBody(appSource, "focusWindowLocally");
  assert.match(
    focusBody,
    /const\s+previousFocusedId\s*=\s*focusedId/,
    "focusWindowLocally must remember the previous focused window",
  );
  assert.match(
    focusBody,
    /focusedId\s*=\s*windowId/,
    "focusWindowLocally must update focusedId to the requested window",
  );
  assert.match(
    focusBody,
    /previousFocusedId\s*!==\s*windowId[\s\S]+previousElement\?\.classList\.remove\s*\(\s*"focused"\s*\)/,
    "focusWindowLocally must remove focus from the previous element only",
  );
  assert.match(
    focusBody,
    /targetElement\.classList\.add\s*\(\s*"focused"\s*\)/,
    "focusWindowLocally must add focus to the requested element",
  );

  const renderWorkspaceBody = extractFunctionBody(appSource, "renderWorkspace");
  assert.match(
    renderWorkspaceBody,
    /focusedId\s*=\s*null\s*;/,
    "renderWorkspace must still clear focusedId when no topmost active window exists",
  );
});

test("Workspace visibility classification reuses direct id sets", () => {
  const renderWorkspaceBody = extractFunctionBody(appSource, "renderWorkspace");
  assert.match(
    appSource,
    /function\s+workspaceWindowIdSet\s*\(/,
    "app.js must provide a direct-loop active window id set helper",
  );
  assert.match(
    appSource,
    /function\s+allProjectWindowIdSet\s*\(/,
    "app.js must provide a direct-loop all-project window id set helper",
  );
  assert.doesNotMatch(
    renderWorkspaceBody,
    /workspace\.windows\.map\s*\(\s*\(?\s*windowData\s*\)?\s*=>\s*windowData\.id\s*\)/,
    "renderWorkspace must not allocate an active window id array before classification",
  );
  assert.match(
    renderWorkspaceBody,
    /const\s+activeWindowIdSet\s*=\s*workspaceWindowIdSet\s*\(\s*workspace\s*\)/,
    "renderWorkspace must derive active window ids as a Set once",
  );
  assert.match(
    renderWorkspaceBody,
    /activeWindowIdSet\s*,[\s\S]*allProjectWindowIdSet:\s*allProjectWindowIdSet\s*\(\s*\)/,
    "renderWorkspace must pass id sets to classifyProjectWindowVisibility",
  );
  assert.match(
    renderWorkspaceBody,
    /topmostId\s*&&\s*activeWindowIdSet\.has\s*\(\s*topmostId\s*\)/,
    "renderWorkspace must reuse the active id set for topmost focus membership",
  );
});

test("Runtime status updates skip unchanged DOM and dependent surface writes", () => {
  assert.match(
    appSource,
    /const\s+renderedRuntimeStatusKeys\s*=\s*new\s+Map\s*\(/,
    "app.js must track per-window runtime status render keys",
  );
  assert.match(
    appSource,
    /function\s+windowRuntimeStatusRenderKey\s*\(/,
    "app.js must define a runtime status render key helper",
  );

  const statusBody = extractFunctionBody(appSource, "applyStatus");
  const keyIndex = statusBody.indexOf(
    "const nextRuntimeStatusKey = windowRuntimeStatusRenderKey(",
  );
  const guardIndex = statusBody.indexOf(
    "renderedRuntimeStatusKeys.get(windowId) === nextRuntimeStatusKey",
  );
  const chipIndex = statusBody.indexOf("chip.classList.remove(");
  const overlayIndex = statusBody.indexOf("const overlay = element.querySelector");
  const telemetryIndex = statusBody.indexOf("recomputeOperatorTelemetry");
  const windowListIndex = statusBody.lastIndexOf("renderWindowList()");
  const stateCuesIndex = statusBody.indexOf("refreshProjectTabStateCues()");

  assert.notEqual(keyIndex, -1, "applyStatus must compute a runtime status key");
  assert.notEqual(guardIndex, -1, "applyStatus must guard unchanged runtime status");
  for (const [label, index] of [
    ["status chip class writes", chipIndex],
    ["overlay lookup/writes", overlayIndex],
    ["telemetry recompute", telemetryIndex],
    ["Window List refresh", windowListIndex],
    ["project tab state cue refresh", stateCuesIndex],
  ]) {
    assert.notEqual(index, -1, `applyStatus must still contain ${label}`);
    assert.ok(guardIndex < index, `unchanged runtime status must return before ${label}`);
  }
  assert.match(
    statusBody.slice(guardIndex, chipIndex),
    /return\s*;/,
    "unchanged runtime status guard must return before DOM/dependent-surface writes",
  );
});

test("Runtime status updates repaint tab telemetry when the target window is hidden", () => {
  const statusBody = extractFunctionBody(appSource, "applyStatus");
  const branchMatch = statusBody.match(
    /if\s*\(\s*!element\s*\)\s*\{([\s\S]*?)return\s*;\s*\}/,
  );
  assert.ok(branchMatch, "applyStatus must handle status events for unmounted windows");

  const branchBody = branchMatch[1];
  assert.match(
    branchBody,
    /renderWindowList\s*\(\s*\)\s*;/,
    "hidden target status updates must still refresh the Window List",
  );
  assert.match(
    branchBody,
    /refreshWindowTabTelemetry\s*\(\s*windowData\s*\)\s*;/,
    "hidden target status updates must repaint visible sibling tab telemetry",
  );
  assert.match(
    branchBody,
    /refreshProjectTabStateCues\s*\(\s*\)\s*;/,
    "hidden target status updates must still refresh project tab state cues",
  );
});

test("Runtime status key covers state detail preset visibility and cleanup", () => {
  const keyBody = extractFunctionBody(appSource, "windowRuntimeStatusRenderKey");
  assert.match(
    appSource,
    /function\s+appendRenderKeyPart\s*\(/,
    "app.js must expose the primitive render-key append helper",
  );
  assert.match(
    keyBody,
    /appendRenderKeyPart\s*\(/,
    "Runtime status key must append primitive fields directly",
  );
  assert.doesNotMatch(
    keyBody,
    /JSON\.stringify\s*\(/,
    "Runtime status key must not serialize an object on every window_status event",
  );
  for (const pattern of [
    /windowId/,
    /windowMap\.has\s*\(\s*windowId\s*\)/,
    /runtimeState/,
    /effectiveDetail/,
    /windowData\?\.preset/,
    /shouldShowRuntimeStatus\s*\(/,
    /mapAgentTelemetryState\s*\(/,
  ]) {
    assert.match(keyBody, pattern, `runtime status key must include ${pattern}`);
  }

  const statusBody = extractFunctionBody(appSource, "applyStatus");
  const stateMapIndex = statusBody.indexOf("windowRuntimeStateMap.set(windowId, runtimeState);");
  const effectiveDetailIndex = statusBody.indexOf("const effectiveDetail = detailMap.get(windowId)");
  const keyIndex = statusBody.indexOf(
    "const nextRuntimeStatusKey = windowRuntimeStatusRenderKey(",
  );
  assert.ok(
    stateMapIndex !== -1 && stateMapIndex < keyIndex,
    "applyStatus must update runtime state before computing the status key",
  );
  assert.ok(
    effectiveDetailIndex !== -1 && effectiveDetailIndex < keyIndex,
    "applyStatus must compute effective detail before the status key",
  );

  const renderWorkspaceBody = extractFunctionBody(appSource, "renderWorkspace");
  const cleanupIndex = renderWorkspaceBody.indexOf("renderedRuntimeStatusKeys.delete(windowId);");
  const removeIndex = renderWorkspaceBody.indexOf("element.remove();");
  assert.notEqual(cleanupIndex, -1, "window removal must clear runtime status render key");
  assert.ok(
    cleanupIndex < removeIndex,
    "runtime status render key cleanup must happen before element removal",
  );
});

test("Terminal output decode is deferred out of the receive path", () => {
  const batcherConfig = appSource.match(
    /const\s+terminalOutputBatcher\s*=\s*createTerminalOutputBatcher\(\{([\s\S]*?)\n\s{6}\}\);/,
  );
  assert.ok(batcherConfig, "app.js must configure the terminal output batcher");
  assert.match(
    batcherConfig[1],
    /mergeChunks:\s*\(\s*chunks\s*,\s*windowId\s*\)/,
    "terminal output batcher must merge encoded chunks during scheduled flush",
  );
  assert.match(
    batcherConfig[1],
    /decoderMap\.get\s*\(\s*windowId\s*\)/,
    "flush merge must use the per-window TextDecoder",
  );
  assert.match(
    batcherConfig[1],
    /chunks\s*\.\s*map[\s\S]*decodeBase64\s*\(\s*chunk\s*\)/,
    "flush merge must decode each encoded chunk in order",
  );

  const writeOutputBody = extractFunctionBody(appSource, "writeOutput");
  assert.match(
    writeOutputBody,
    /terminalOutputBatcher\.enqueue\(\s*windowId,\s*base64\s*\)/,
    "ready terminal output must enqueue encoded chunks without eager decode",
  );
  const readyEnqueueIndex = writeOutputBody.indexOf(
    "terminalOutputBatcher.enqueue(",
  );
  const readyPath = writeOutputBody.slice(readyEnqueueIndex);
  assert.doesNotMatch(
    readyPath,
    /decoder\.decode\s*\(|decodeBase64\s*\(/,
    "writeOutput ready path must not decode before the scheduled flush",
  );
});

test("Terminal input arms one-shot output priority before sending", () => {
  function assertPriorityBeforeSend(functionName, message) {
    const body = extractFunctionBody(appSource, functionName);
    const priorityIndex = body.indexOf(
      "terminalOutputBatcher.prioritize(windowId);",
    );
    const sendIndex = body.indexOf(
      'send({ kind: "terminal_input", id: windowId, data });',
    );
    assert.notEqual(priorityIndex, -1, `${message} must arm output priority`);
    assert.notEqual(sendIndex, -1, `${message} must send terminal input`);
    assert.ok(
      priorityIndex < sendIndex,
      `${message} must arm priority before the wire send`,
    );
  }

  assertPriorityBeforeSend(
    "attachTerminalContainerBindings",
    "wheel fallback input",
  );
  assertPriorityBeforeSend("createTerminalRuntime", "xterm onData input");
});

test("Terminal output writes are gated while windows are hidden", () => {
  const batcherConfig = appSource.match(
    /const\s+terminalOutputBatcher\s*=\s*createTerminalOutputBatcher\(\{([\s\S]*?)\n\s{6}\}\);/,
  );
  assert.ok(batcherConfig, "app.js must configure the terminal output batcher");
  assert.match(
    batcherConfig[1],
    /canWrite:\s*canRefreshTerminalViewport/,
    "terminal output batcher must reuse the terminal visibility predicate before decode/write",
  );

  const renderWorkspaceBody = extractFunctionBody(appSource, "renderWorkspace");
  assert.match(
    renderWorkspaceBody,
    /onReveal:\s*\(\)\s*=>\s*\{[\s\S]*?terminalOutputBatcher\.schedulePending\(windowId\)[\s\S]*?rearmPendingTerminalViewportRefresh\(\s*windowId,\s*\{[\s\S]*?shouldPersistGeometry:\s*false[\s\S]*?\}\s*\)[\s\S]*?scheduleTerminalFocusActivation\(\s*windowId,\s*\{[\s\S]*?shouldPersistGeometry:\s*false[\s\S]*?reason:\s*"visibility_reveal"[\s\S]*?\}\s*\)/,
    "hidden project-tab reveal must re-arm pending output before viewport/focus activation",
  );
  assert.match(
    renderWorkspaceBody,
    /onReveal:\s*\(\)\s*=>\s*\{[\s\S]*?terminalOutputBatcher\.schedulePending\(windowData\.id\)[\s\S]*?rearmPendingTerminalViewportRefresh\(\s*windowData\.id,\s*\{[\s\S]*?shouldPersistGeometry:\s*false[\s\S]*?\}\s*\)[\s\S]*?scheduleTerminalFocusActivation\(\s*windowData\.id,\s*\{[\s\S]*?shouldPersistGeometry:\s*false[\s\S]*?reason:\s*"visibility_reveal"[\s\S]*?\}\s*\)/,
    "hidden window-tab reveal must re-arm pending output before viewport/focus activation",
  );
});

test("Operator telemetry skips unchanged counts before DOM writes", () => {
  assert.match(
    appSource,
    /let\s+renderedOperatorTelemetryKey\s*=\s*""/,
    "app.js must track the last rendered telemetry counts key",
  );
  assert.match(
    appSource,
    /function\s+operatorTelemetryRenderKey\s*\(/,
    "app.js must define a telemetry counts key helper",
  );
  assert.match(
    appSource,
    /function\s+applyOperatorTelemetryCounts\s*\(/,
    "app.js must route telemetry DOM writes through a guarded helper",
  );

  const helperBody = extractFunctionBody(appSource, "applyOperatorTelemetryCounts");
  const keyIndex = helperBody.indexOf("const nextOperatorTelemetryKey = operatorTelemetryRenderKey(counts);");
  const guardIndex = helperBody.indexOf(
    "renderedOperatorTelemetryKey === nextOperatorTelemetryKey",
  );
  const applyIndex = helperBody.indexOf("window.__operatorShell.applyTelemetryCounts(counts)");
  const storeIndex = helperBody.indexOf("renderedOperatorTelemetryKey = nextOperatorTelemetryKey");

  assert.notEqual(keyIndex, -1, "telemetry helper must compute a stable key");
  assert.ok(guardIndex > keyIndex, "telemetry helper must guard after key computation");
  assert.ok(
    guardIndex < applyIndex,
    "unchanged telemetry counts must return before applyTelemetryCounts DOM writes",
  );
  assert.ok(
    applyIndex < storeIndex,
    "telemetry key should be stored after applyTelemetryCounts succeeds",
  );
  assert.match(
    helperBody.slice(guardIndex, applyIndex),
    /return\s*;/,
    "unchanged telemetry guard must return before DOM writes",
  );
});

test("Operator telemetry key avoids JSON stringify allocation", () => {
  const keyBody = extractFunctionBody(appSource, "operatorTelemetryRenderKey");
  assert.match(
    appSource,
    /function\s+appendRenderKeyPart\s*\(/,
    "app.js must expose the primitive render-key append helper",
  );
  assert.match(
    keyBody,
    /appendRenderKeyPart\s*\(/,
    "telemetry key must append primitive count fields directly",
  );
  assert.doesNotMatch(
    keyBody,
    /JSON\.stringify\s*\(/,
    "telemetry key must not serialize a counts object graph",
  );
});

test("SPEC-3038 (2026-06-20): telemetry key includes windows so the badge updates on non-agent window changes", () => {
  // Without `windows` in the cache key, adding/removing a surface (non-agent)
  // window leaves the agent counts unchanged, so applyOperatorTelemetryCounts
  // short-circuits and the rail Windows badge never refreshes.
  const keyBody = extractFunctionBody(appSource, "operatorTelemetryRenderKey");
  assert.match(
    keyBody,
    /appendRenderKeyPart\(parts,\s*"windows"\)/,
    "telemetry render key must include the windows count",
  );
});

// SPEC-2356 — branch telemetry forwards only `branches` through the guarded
// applyOperatorTelemetryCounts helper; the dead Sidebar Layers `git` counter is
// retired (replaces develop's git-coupled branch telemetry assertion).
test("branch_entries telemetry uses the guarded helper without the retired git layer (SPEC-2356)", () => {
  const branchCase = appSource.match(/case\s+"branch_entries":[\s\S]*?break;/);
  assert.ok(branchCase, "expected branch_entries receive case");
  assert.match(
    branchCase[0],
    /applyOperatorTelemetryCounts\(\{\s*branches:\s*branchesCount,\s*\}\)/,
    "branch telemetry must route through the guarded applyOperatorTelemetryCounts helper",
  );
  assert.doesNotMatch(
    branchCase[0],
    /git:\s*branchesCount/,
    "the dead Sidebar Layers git counter must no longer be forwarded",
  );
  assert.doesNotMatch(
    branchCase[0],
    /window\.__operatorShell\?\.applyTelemetryCounts/,
    "branch telemetry must not call applyTelemetryCounts directly",
  );
});

test("Branch cleanup progress and result events are routed by app receive", () => {
  const cleanupCase = appSource.match(
    /case\s+"branch_cleanup_result":[\s\S]*?case\s+"branch_error":[\s\S]*?break;/,
  );
  assert.ok(
    cleanupCase,
    "expected top-level receive to route branch cleanup result/progress/error events",
  );
  assert.match(
    cleanupCase[0],
    /applyBranchCleanupReceiveEvent\(event\)/,
    "branch cleanup events must update the modal state instead of leaving rows queued",
  );
});

// Layout regression (2026-06-10 user verification): app.css carried a stale
// `.workspace-overview-shell { display: flex }` block that overrode the
// canonical grid layout in components.css. Under flex, long Workspace rows
// grow the list pane and crush the detail pane into a vertical sliver. The
// shell's display is owned by components.css (grid) alone.
test("app.css must not redeclare display for the Workspace overview shell", () => {
  const appCss = readFileSync(resolve(here, "../styles/app.css"), "utf8");
  const blocks = appCss.match(/\.workspace-overview-shell[^{]*\{[^}]*\}/g) ?? [];
  for (const block of blocks) {
    if (/\[hidden\]/.test(block)) continue;
    assert.doesNotMatch(
      block,
      /display\s*:\s*(?!none)/,
      `app.css must not override the overview shell display (grid lives in components.css): ${block}`,
    );
  }
  const componentsCss = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  const shellBlock = componentsCss.match(/\.workspace-overview-shell\s*\{[^}]*\}/)?.[0] ?? "";
  assert.match(shellBlock, /display\s*:\s*grid/, "components.css owns the grid layout");
});
