// SPEC-1933 Phase: System Settings (Output Language) — DOM / CSS structure
//
// Settings ウィンドウのタブ化と System タブの Language select が
// Operator Design tokens を使い、つぎはぎ実装になっていないことを
// CSS とレンダラのソースパターンで検証する（kanban-structure と同じ手法）。

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
const componentsCss = readFileSync(
  resolve(here, "../styles/components.css"),
  "utf8",
);

test("components.css declares the settings tab and panel surfaces under :root[data-theme]", () => {
  // Every Settings primitive must live inside the dual-theme scope so it
  // honors Dark Operator / Light Drafting alongside the rest of the shell.
  for (const klass of [
    ".settings-root",
    ".settings-toolbar",
    ".settings-heading",
    ".settings-tabs",
    ".settings-tab",
    ".settings-panel",
    ".settings-section",
    ".settings-select",
    ".settings-checkbox-label",
    ".settings-checkbox",
    ".settings-help",
    ".settings-status",
  ]) {
    const pattern = new RegExp(
      `:root\\[data-theme\\]\\s+${klass.replace(".", "\\.")}\\b`,
    );
    assert.match(
      componentsCss,
      pattern,
      `expected components.css to define ${klass} under :root[data-theme]`,
    );
  }
});

test("settings-tab uses Operator color/space tokens (no hardcoded colors)", () => {
  // Pull the .settings-tab declaration block and verify it threads design
  // tokens, not raw color literals — this is the contract that prevents
  // patchwork-style additions from drifting away from the design system.
  const block = componentsCss.match(
    /:root\[data-theme\]\s+\.settings-tab\s*\{([\s\S]*?)\}/,
  );
  assert.ok(block, "expected .settings-tab block in components.css");
  const body = block[1];
  assert.match(body, /var\(--color-surface-elevated\)/);
  assert.match(body, /var\(--color-border\)/);
  assert.match(body, /var\(--color-text\)/);
  assert.match(body, /var\(--font-body\)/);
  assert.match(body, /var\(--radius-md\)/);
  // No raw hex / rgb literals snuck in.
  assert.doesNotMatch(body, /#[0-9a-fA-F]{3,8}\b/, "no hex color literals");
  assert.doesNotMatch(body, /\brgb\(/, "no raw rgb() literals");
});

test("settings-tab[aria-selected=true] uses --color-state-active tint", () => {
  // The active state must inherit the Operator accent so the tab strip
  // picks up the same accent as Project Bar / Sidebar Layers / etc.
  const block = componentsCss.match(
    /:root\[data-theme\]\s+\.settings-tab\[aria-selected="true"\]\s*\{([\s\S]*?)\}/,
  );
  assert.ok(
    block,
    "expected .settings-tab[aria-selected=true] block in components.css",
  );
  assert.match(block[1], /var\(--color-state-active\)/);
});

test("settings-select uses --color-focus-ring on focus-visible", () => {
  // Focus visibility must reuse the same accent as the rest of the app.
  const block = componentsCss.match(
    /:root\[data-theme\]\s+\.settings-select:focus-visible\s*\{([\s\S]*?)\}/,
  );
  assert.ok(block, "expected .settings-select:focus-visible block");
  assert.match(block[1], /var\(--color-focus-ring\)/);
});

test("renderSettingsWindow builds a tablist toolbar with System and Custom Agents tabs", () => {
  // The renderer must declare role="tablist" so assistive tech sees the tab
  // group, and emit both data-settings-tab values that switchSettingsTab
  // expects.
  assert.match(appSource, /toolbar\.className\s*=\s*"settings-toolbar"/);
  assert.match(appSource, /tabs\.setAttribute\("role",\s*"tablist"\)/);
  assert.match(appSource, /buildSettingsTab\("system",\s*"System"/);
  assert.match(
    appSource,
    /buildSettingsTab\("custom-agents",\s*"Custom Agents"/,
  );
});

test("System tab Language select offers Auto / English / 日本語", () => {
  // SPEC-1933 NFR-005: Language select labels are the single approved
  // exception to the English-only UI rule. Verify all three options exist
  // and use the expected raw values.
  for (const opt of [
    /value:\s*"auto",\s*text:\s*"Auto \(OS locale\)"/,
    /value:\s*"en",\s*text:\s*"English"/,
    /value:\s*"ja",\s*text:\s*"日本語"/,
  ]) {
    assert.match(
      appSource,
      opt,
      `Language select missing option: ${opt}`,
    );
  }
});

test("renderSystemPanel sends update_system_settings on select change", () => {
  // The select listener must propagate the new raw value to the backend
  // via the WebSocket protocol (SPEC-1933 FR-007). Verify both the kind
  // and the payload field name.
  assert.match(
    appSource,
    /send\(\{\s*kind:\s*"update_system_settings",\s*language:\s*next\s*\}\)/,
    "expected select onChange to send update_system_settings",
  );
});

test("renderSystemPanel exposes Codex managed hook trust opt-in", () => {
  assert.match(
    appSource,
    /trustText\.textContent\s*=\s*"Trust gwt-managed Codex hooks"/,
    "expected System tab Codex hook trust checkbox label",
  );
  assert.match(
    appSource,
    /codexTrustManagedHooks:\s*true/,
    "expected Codex hook trust UI state to default on",
  );
  assert.match(
    appSource,
    /trustCheckbox\.checked\s*=\s*systemSettingsState\.codexTrustManagedHooks\s*!==\s*false/,
    "expected unchecked only when config explicitly disables Codex hook trust",
  );
  assert.match(
    appSource,
    /send\(\{\s*kind:\s*"update_system_settings",\s*language:\s*systemSettingsState\.language\s*\|\|\s*"auto",\s*codex_trust_managed_hooks:\s*next,\s*\}\)/,
    "expected checkbox onChange to send codex_trust_managed_hooks",
  );
});

test("System tab Board provider select offers Local/Slack/Teams as selectable (SPEC-2963)", () => {
  // SPEC-2963 Phase 5: slack/teams are now real, selectable options (sign-in
  // gated rather than disabled "coming soon").
  assert.match(
    appSource,
    /id\s*=\s*"settings-system-board-provider"/,
    "expected a Board provider select with the canonical id",
  );
  for (const provider of ["local", "slack", "teams"]) {
    assert.match(
      appSource,
      new RegExp(`value:\\s*"${provider}"`),
      `${provider} must be a Board provider option`,
    );
  }
});

test("System tab exposes remote provider sign-in affordance + auth status (SPEC-2963)", () => {
  assert.match(appSource, /"board-auth-status"/, "expected an auth status element");
  assert.match(
    appSource,
    /kind:\s*"board_provider_sign_in",\s*provider:\s*selectedProvider/,
    "Sign in button must send board_provider_sign_in",
  );
  assert.match(
    appSource,
    /kind:\s*"board_provider_sign_out",\s*provider:\s*selectedProvider/,
    "Sign out button must send board_provider_sign_out",
  );
  assert.match(
    appSource,
    /kind:\s*"get_board_auth_status"/,
    "panel must fetch board auth status",
  );
  assert.match(
    appSource,
    /case\s+"board_auth_status":/,
    "dispatch must handle board_auth_status",
  );
});

test("System tab exposes remote provider config form that saves via update_board_provider_config (SPEC-2963)", () => {
  // FR-006: client_id / default_channel / tenant_id / secret editable from the
  // UI instead of hand-editing config.toml.
  assert.match(
    appSource,
    /board-config-form/,
    "expected a provider config form container",
  );
  assert.match(
    appSource,
    /"settings-board-slack-client-id"/,
    "Slack client id input must exist",
  );
  assert.match(
    appSource,
    /"settings-board-slack-secret"/,
    "Slack client secret input must exist",
  );
  assert.match(
    appSource,
    /"settings-board-teams-tenant-id"/,
    "Teams tenant id input must exist",
  );
  assert.match(
    appSource,
    /kind:\s*"update_board_provider_config"/,
    "Save button must send update_board_provider_config",
  );
  // The secret must only be sent when the user typed one (empty box keeps the
  // stored secret), so it is conditional rather than always included.
  assert.match(
    appSource,
    /secretInput\.value\.length\s*>\s*0/,
    "secret must be sent only when the user typed a value",
  );
  // Dispatch must store the non-secret config view for prefill.
  assert.match(
    appSource,
    /slackClientId:\s*event\.slack_client_id/,
    "dispatch must prefill slack client id from board_auth_status",
  );
});

test("renderSystemPanel sends update_system_settings with board_provider on change (SPEC-2959)", () => {
  assert.match(
    appSource,
    /kind:\s*"update_system_settings",[\s\S]*?board_provider:\s*next/,
    "Board provider select onChange must send update_system_settings with board_provider",
  );
});

test("renderSettingsWindow requests current settings via get_system_settings", () => {
  assert.match(
    appSource,
    /send\(\{\s*kind:\s*"get_system_settings"\s*\}\)/,
    "expected renderSettingsWindow to send get_system_settings on open",
  );
});

test("Frontend dispatches all three system_settings backend events", () => {
  // The dispatch must handle the three reply variants from
  // crates/gwt/src/system_settings.rs: SystemSettings (load),
  // SystemSettingsUpdated (save success), SystemSettingsError (failure).
  for (const kind of [
    /case\s+"system_settings":/,
    /case\s+"system_settings_updated":/,
    /case\s+"system_settings_error":/,
  ]) {
    assert.match(
      appSource,
      kind,
      `expected backend event dispatch case: ${kind}`,
    );
  }
});

test("Custom Agents panel keeps the data-role='settings-scroll' hook for the legacy renderer", () => {
  // renderSettingsAgentList queries [data-role='settings-scroll'] to find
  // its target. Moving it onto the Custom Agents panel preserves the
  // existing list/edit flow without rewriting that surface.
  assert.match(
    appSource,
    /panelAgents\.dataset\.role\s*=\s*"settings-scroll"/,
    "Custom Agents panel must carry data-role='settings-scroll'",
  );
});

test("Custom Agents panel wires the env editor to update_custom_agent", () => {
  assert.match(
    appSource,
    /from "\/custom-agent-env-editor\.js"/,
    "app.js must import the custom agent env editor module",
  );
  assert.match(
    appSource,
    /renderCustomAgentEnvEditor\(\{/,
    "renderSettingsAgentList must mount the env editor",
  );
  assert.match(
    appSource,
    /kind:\s*"update_custom_agent",\s*agent:\s*updatedAgent/,
    "env editor save must send update_custom_agent with the edited agent",
  );
});

test("switchSettingsTab toggles aria-selected and hidden together", () => {
  // The active / hidden contract: aria-selected on the tab and the .hidden
  // class on every panel except the matching one must change in lockstep.
  // Without this, screen readers and the visual state can desync.
  assert.match(
    appSource,
    /aria-selected/,
    "switchSettingsTab must update aria-selected",
  );
  assert.match(
    appSource,
    /panel\.classList\.toggle\(\s*"hidden"/,
    "switchSettingsTab must toggle hidden on non-active panels",
  );
});
