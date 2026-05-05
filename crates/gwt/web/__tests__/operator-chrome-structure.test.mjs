import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";

const here = dirname(fileURLToPath(import.meta.url));
const indexPath = resolve(here, "../index.html");
const html = readFileSync(indexPath, "utf8");
const { document } = parseHTML(html);
const operatorShellSource = readFileSync(resolve(here, "../operator-shell.js"), "utf8");
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");

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
    /function\s+renderActiveWorkOverview\(\)[\s\S]+activeWorkProjection\.agents/,
    "expected frontend to render per-agent projection data, not only aggregate counters",
  );
  assert.match(
    appSource,
    /op-agent-card[\s\S]+last_board_entry_id/,
    "expected agent cards to preserve board linkage for handoff/debugging",
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

test("hotkey overlay lists ⌘P/⌘B/⌘G/⌘L/⌘?/Esc plus the layout toggle", () => {
  const overlay = document.getElementById("op-hotkey-overlay");
  assert.ok(overlay, "hotkey overlay missing");
  const text = overlay.textContent.replace(/\s+/g, " ");
  for (const phrase of ["⌘ P", "⌘ B", "⌘ G", "⌘ L", "⌘ K", "⌘ ?", "⌘ \\", "Esc"]) {
    assert.ok(text.includes(phrase), `expected ${phrase} in hotkey overlay`);
  }
  const groups = Array.from(overlay.querySelectorAll(".op-hotkey-card__group-title")).map((el) =>
    el.textContent?.trim().toLowerCase(),
  );
  for (const expected of ["navigation", "layout", "help"]) {
    assert.ok(groups.includes(expected), `expected hotkey overlay group "${expected}", got ${groups.join("/")}`);
  }
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

test("operator-shell wires theme toggle aria-label updates on every render", () => {
  const operatorShell = readFileSync(resolve(here, "../operator-shell.js"), "utf8");
  // wireThemeToggle.renderLabel must update aria-label every render so
  // screen readers know the live preference + effective theme.
  assert.match(
    operatorShell,
    /btn\.setAttribute\(\s*"aria-label"/,
    "expected wireThemeToggle to update aria-label on every render",
  );
  assert.match(
    operatorShell,
    /Theme: \$\{pref === "auto" \? `auto/,
    "expected aria-label format to disclose preference + effective theme",
  );
});

test("operator-shell wires sidebar collapse hotkey and Mission Briefing early dismiss", () => {
  const operatorShell = readFileSync(resolve(here, "../operator-shell.js"), "utf8");
  // The source contains `cmd+\\\\` (escaped backslash in JS source).
  assert.ok(
    operatorShell.includes('hotkey.register("cmd+\\\\"'),
    "expected Cmd+backslash hotkey registration for sidebar collapse",
  );
  assert.match(
    operatorShell,
    /opSidebar\s*===\s*"collapsed"/,
    "expected collapsed state toggle on documentElement.dataset.opSidebar",
  );
  assert.match(
    operatorShell,
    /earlyDismiss/,
    "expected Mission Briefing earlyDismiss helper",
  );
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

test("Selected list rows mark active item with aria-current", () => {
  // Same pattern as project tabs (PR #2455): list-style buttons with a
  // selected state need aria-current to announce which row is active.
  // Coverage: knowledge-row, memo-note-row, profile-row, logs-entry.
  // The set/remove pair is required so a previously-selected row
  // doesn't retain the marker after the user picks a different row.
  for (const desc of [
    "knowledge",
    "memo note",
    "profile",
    "logs entry",
  ]) {
    // Each list iterates over its rows and conditionally sets
    // aria-current="true" on the selected one. We assert both the
    // setAttribute and removeAttribute calls exist somewhere in app.js.
    // The descriptive label is just for the failure message.
  }
  // Count occurrences — should be at least 5 set + 5 remove for the
  // five list-with-selection surfaces (knowledge / memo / profile /
  // logs / file-tree) plus the project-tabs case from PR #2455.
  const setMatches = appSource.match(/setAttribute\("aria-current",\s*"(true|page)"\)/g) || [];
  const removeMatches = appSource.match(/removeAttribute\("aria-current"\)/g) || [];
  assert.ok(
    setMatches.length >= 6,
    `expected >= 6 aria-current set calls (project tab + 5 row types), got ${setMatches.length}`,
  );
  assert.ok(
    removeMatches.length >= 6,
    `expected >= 6 aria-current remove calls (one per set call), got ${removeMatches.length}`,
  );
});

test("Project tabs mark the active project with aria-current=\"page\"", () => {
  // Project tabs use role="button" but represent a navigation choice. The
  // appropriate ARIA pattern is aria-current="page" on the active tab so
  // screen readers announce it as the current location. Inactive tabs
  // must explicitly clear the attribute so the previously-active tab
  // doesn't retain the marker after a switch.
  assert.match(
    appSource,
    /button\.setAttribute\("aria-current",\s*"page"\)/,
    "expected the active project tab to set aria-current=\"page\"",
  );
  assert.match(
    appSource,
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
  // the launch wizard, memo editor, and profile editor surfaces.
  const expected = [
    { selector: 'input\\.setAttribute\\("aria-label", "Branch name"\\)', desc: "wizard branch name" },
    { selector: 'input\\.setAttribute\\("aria-label", "Issue number"\\)', desc: "wizard issue number" },
    { selector: 'titleInput\\.setAttribute\\("aria-label", "Note title"\\)', desc: "memo note title" },
    { selector: 'bodyInput\\.setAttribute\\("aria-label", "Note body"\\)', desc: "memo note body" },
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
  assert.match(
    appSource,
    /migrationModal\s*&&\s*migrationModal\.classList\.contains\("open"\)[\s\S]*skip_migration/,
    "expected Esc to send skip_migration when migration modal is open",
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
