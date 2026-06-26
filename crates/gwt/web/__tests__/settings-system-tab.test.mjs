// SPEC-1933 Phase: System Settings (Output Language) — DOM / CSS structure
//
// Settings ウィンドウのタブ化と System タブの Language select が
// Operator Design tokens を使い、つぎはぎ実装になっていないことを
// CSS とレンダラのソースパターンで検証する（kanban-structure と同じ手法）。
//
// SPEC-3064 Phase 3 (E4): the Settings window renderer moved from app.js to
// settings-surface.js. Renderer/source patterns are pinned against the
// extracted module; receive() dispatch case arms stay pinned to app.js.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
const settingsSource = readFileSync(resolve(here, "../settings-surface.js"), "utf8");
const componentsCss = readFileSync(
  resolve(here, "../styles/components.css"),
  "utf8",
);

function extractFunctionSource(source, name) {
  const start = source.indexOf(`function ${name}(`);
  assert.notEqual(start, -1, `missing function ${name}`);
  const braceStart = source.indexOf("{", start);
  assert.notEqual(braceStart, -1, `missing body for ${name}`);
  let depth = 0;
  for (let i = braceStart; i < source.length; i += 1) {
    const ch = source[i];
    if (ch === "{") depth += 1;
    if (ch === "}") {
      depth -= 1;
      if (depth === 0) return source.slice(start, i + 1);
    }
  }
  assert.fail(`unterminated function ${name}`);
}

function loadFunction(name) {
  return Function(`"use strict"; return (${extractFunctionSource(settingsSource, name)});`)();
}

function loadFunctionWithDeps(name, deps) {
  const dependencySources = deps
    .map((dep) => extractFunctionSource(settingsSource, dep))
    .join("\n");
  return Function(
    `"use strict"; ${dependencySources}; return (${extractFunctionSource(settingsSource, name)});`,
  )();
}

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

test("settings toolbar lets tabs wrap inside compact tiled windows", () => {
  const toolbarBlock = componentsCss.match(
    /:root\[data-theme\]\s+\.settings-toolbar\s*\{([\s\S]*?)\}/,
  );
  assert.ok(toolbarBlock, "expected .settings-toolbar block in components.css");
  assert.match(
    toolbarBlock[1],
    /flex-wrap:\s*wrap/,
    "settings toolbar must wrap instead of clipping the tab strip",
  );

  const tabsBlock = componentsCss.match(
    /:root\[data-theme\]\s+\.settings-tabs\s*\{([\s\S]*?)\}/,
  );
  assert.ok(tabsBlock, "expected .settings-tabs block in components.css");
  assert.match(tabsBlock[1], /flex-wrap:\s*wrap/);
  assert.match(tabsBlock[1], /min-width:\s*0/);
  assert.match(tabsBlock[1], /max-width:\s*100%/);
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
  assert.match(settingsSource, /toolbar\.className\s*=\s*"settings-toolbar"/);
  assert.match(settingsSource, /tabs\.setAttribute\("role",\s*"tablist"\)/);
  assert.match(settingsSource, /buildSettingsTab\("system",\s*"System"/);
  assert.match(
    settingsSource,
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
      settingsSource,
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
    settingsSource,
    /send\(\{\s*kind:\s*"update_system_settings",\s*language:\s*next\s*\}\)/,
    "expected select onChange to send update_system_settings",
  );
});

test("renderSystemPanel exposes Codex managed hook trust opt-in", () => {
  assert.match(
    settingsSource,
    /trustText\.textContent\s*=\s*"Trust gwt-managed Codex hooks"/,
    "expected System tab Codex hook trust checkbox label",
  );
  assert.match(
    settingsSource,
    /codexTrustManagedHooks:\s*true/,
    "expected Codex hook trust UI state to default on",
  );
  assert.match(
    settingsSource,
    /trustCheckbox\.checked\s*=\s*systemSettingsState\.codexTrustManagedHooks\s*!==\s*false/,
    "expected unchecked only when config explicitly disables Codex hook trust",
  );
  assert.match(
    settingsSource,
    /send\(\{\s*kind:\s*"update_system_settings",\s*language:\s*systemSettingsState\.language\s*\|\|\s*"auto",\s*codex_trust_managed_hooks:\s*next,\s*\}\)/,
    "expected checkbox onChange to send codex_trust_managed_hooks",
  );
});

test("System tab Board provider select offers Local/Slack/Teams as selectable (SPEC-2963)", () => {
  // SPEC-2963 Phase 5: slack/teams are now real, selectable options (sign-in
  // gated rather than disabled "coming soon").
  assert.match(
    settingsSource,
    /id\s*=\s*"settings-system-board-provider"/,
    "expected a Board provider select with the canonical id",
  );
  for (const provider of ["local", "slack", "teams"]) {
    assert.match(
      settingsSource,
      new RegExp(`value:\\s*"${provider}"`),
      `${provider} must be a Board provider option`,
    );
  }
});

test("System tab exposes remote provider sign-in affordance + auth status (SPEC-2963)", () => {
  assert.match(settingsSource, /"board-auth-status"/, "expected an auth status element");
  assert.match(
    settingsSource,
    /kind:\s*"board_provider_sign_in",\s*provider:\s*selectedProvider/,
    "Sign in button must send board_provider_sign_in",
  );
  assert.match(
    settingsSource,
    /kind:\s*"board_provider_sign_out",\s*provider:\s*selectedProvider/,
    "Sign out button must send board_provider_sign_out",
  );
  assert.match(
    settingsSource,
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
    settingsSource,
    /board-config-form/,
    "expected a provider config form container",
  );
  assert.match(
    settingsSource,
    /"settings-board-slack-client-id"/,
    "Slack client id input must exist",
  );
  assert.match(
    settingsSource,
    /"settings-board-slack-secret"/,
    "Slack client secret input must exist",
  );
  assert.match(
    settingsSource,
    /"settings-board-teams-tenant-id"/,
    "Teams tenant id input must exist",
  );
  assert.match(
    settingsSource,
    /kind:\s*"update_board_provider_config"/,
    "Save button must send update_board_provider_config",
  );
  // The secret must only be sent when the user typed one (empty box keeps the
  // stored secret), so it is conditional rather than always included.
  assert.match(
    settingsSource,
    /secretInput\.value\.length\s*>\s*0/,
    "secret must be sent only when the user typed a value",
  );
  // Dispatch must store the non-secret config view for prefill.
  assert.match(
    appSource,
    /slackClientId:\s*event\.slack_client_id/,
    "dispatch must prefill slack client id from board_auth_status",
  );
  // The secret field clears after Save by design; an explicit saved-state note
  // must make persistence obvious instead of looking like data loss.
  assert.match(
    settingsSource,
    /board-secret-state/,
    "a client-secret saved-state indicator must exist",
  );
  assert.match(
    settingsSource,
    /A client secret is saved/,
    "the saved-state indicator must confirm the secret persisted",
  );
});

test("System tab accepts a Teams channel link without exposing Team ID fields (SPEC-2963 Phase 9)", () => {
  assert.match(
    settingsSource,
    /function\s+parseTeamsChannelLink\(/,
    "expected a helper that parses a copied Teams channel link",
  );
  assert.match(
    settingsSource,
    /function\s+composeTeamsDefaultChannel\(/,
    "expected the parsed Teams link to save as team_id/channel_id",
  );
  assert.match(
    settingsSource,
    /function\s+formatTeamsChannelLink\(/,
    "expected saved team_id/channel_id config to render back into the link field",
  );
  assert.match(
    settingsSource,
    /"settings-board-teams-channel-link"/,
    "Teams channel link paste input must exist",
  );
  assert.doesNotMatch(
    settingsSource,
    /"settings-board-teams-team-id"/,
    "Teams Team ID must not be exposed as a separate input",
  );
  assert.doesNotMatch(
    settingsSource,
    /"settings-board-teams-channel-id"/,
    "Teams Channel ID must not be exposed as a separate input",
  );
  assert.match(
    settingsSource,
    /const\s+parsedTeamsChannel\s*=\s*parseTeamsChannelLink\(\s*teamsChannelLinkValue\s*,?\s*\)/,
    "Save must parse the Teams channel link",
  );
  assert.match(
    settingsSource,
    /default_channel:\s*nextTeamsDefaultChannel/,
    "Save payload must use the parsed or preserved Teams channel",
  );
  assert.match(
    settingsSource,
    /let\s+nextTeamsDefaultChannel\s*=\s*cfg\.teamsDefaultChannel\s*\|\|\s*""/,
    "saving client/tenant settings without a new link must preserve the existing channel",
  );
  assert.match(
    settingsSource,
    /formatTeamsChannelLink\(\s*cfg\.teamsDefaultChannel,\s*cfg\.teamsTenantId,?\s*\)/,
    "saved channel must remain visible in the Teams channel link field after rerender",
  );
});

test("Teams channel helpers parse and rehydrate channel links without losing the saved channel (SPEC-2963 Phase 9)", () => {
  const parseTeamsChannelLink = loadFunction("parseTeamsChannelLink");
  const formatTeamsChannelLink = loadFunctionWithDeps("formatTeamsChannelLink", [
    "parseTeamsDefaultChannel",
  ]);
  const composeTeamsDefaultChannel = loadFunction("composeTeamsDefaultChannel");
  const teamId = "0abf2b52-ada6-40ee-96ba-c8010f5be083";
  const channelId = "19:04685a0564cd4f7eb712f8152167360a@thread.skype";
  const tenantId = "0ff7b59c-80f2-4925-99ca-f6d55b11d31c";
  const defaultChannel = `${teamId}/${channelId}`;

  assert.deepEqual(
    parseTeamsChannelLink(
      `https://teams.microsoft.com/l/channel/${encodeURIComponent(channelId)}/gwt-test?groupId=${teamId}&tenantId=${tenantId}`,
    ),
    { teamId, channelId },
  );
  assert.equal(composeTeamsDefaultChannel(teamId, channelId), defaultChannel);
  assert.deepEqual(parseTeamsChannelLink(formatTeamsChannelLink(defaultChannel, tenantId)), {
    teamId,
    channelId,
  });
  assert.equal(parseTeamsChannelLink("not a Teams link"), null);
});

test("System tab exposes an editable OAuth callback port + redirect URL hint (SPEC-2963 FR-005)", () => {
  // The OAuth redirect must use a fixed, registerable loopback port; the UI
  // exposes it (default 8765) and shows the exact URL to register.
  assert.match(
    settingsSource,
    /"settings-board-oauth-port"/,
    "OAuth callback port input must exist",
  );
  assert.match(
    settingsSource,
    /127\.0\.0\.1:\$\{[^}]+\}\/oauth\/callback/,
    "the redirect URL hint must show http://127.0.0.1:<port>/oauth/callback",
  );
  assert.match(
    settingsSource,
    /kind:\s*"update_board_oauth_port"/,
    "Save port must send update_board_oauth_port",
  );
  assert.match(
    appSource,
    /oauthRedirectPort:\s*event\.oauth_redirect_port/,
    "dispatch must prefill the OAuth port from board_auth_status",
  );
});

test("renderSystemPanel sends update_system_settings with board_provider on change (SPEC-2959)", () => {
  assert.match(
    settingsSource,
    /kind:\s*"update_system_settings",[\s\S]*?board_provider:\s*next/,
    "Board provider select onChange must send update_system_settings with board_provider",
  );
});

test("System tab exposes Launch GWT at login autostart control", () => {
  assert.match(
    settingsSource,
    /autostartEnabled:\s*false/,
    "expected System settings state to track autostart enabled status",
  );
  assert.match(
    settingsSource,
    /autostartPreviousEnabled:\s*false/,
    "expected System settings state to retain the previous autostart value for failed updates",
  );
  assert.match(
    settingsSource,
    /systemSettingsState\.autostartEnabled\s*=\s*systemSettingsState\.autostartPreviousEnabled\s*===\s*true/,
    "expected autostart update errors to restore the last confirmed value",
  );
  assert.match(
    settingsSource,
    /autostartText\.textContent\s*=\s*"Launch GWT at login"/,
    "expected System tab autostart checkbox label",
  );
  assert.match(
    settingsSource,
    /send\(\{\s*kind:\s*"update_autostart",\s*enabled:\s*next\s*\}\)/,
    "expected autostart checkbox onChange to send update_autostart",
  );
});

test("renderSettingsWindow requests current settings via get_system_settings", () => {
  assert.match(
    settingsSource,
    /send\(\{\s*kind:\s*"get_system_settings"\s*\}\)/,
    "expected renderSettingsWindow to send get_system_settings on open",
  );
  assert.match(
    settingsSource,
    /send\(\{\s*kind:\s*"get_autostart_status"\s*\}\)/,
    "expected renderSettingsWindow to send get_autostart_status on open",
  );
});

test("Frontend dispatches all system_settings and autostart backend events", () => {
  // The dispatch must handle the three reply variants from
  // crates/gwt/src/system_settings.rs: SystemSettings (load),
  // SystemSettingsUpdated (save success), SystemSettingsError (failure).
  for (const kind of [
    /case\s+"system_settings":/,
    /case\s+"system_settings_updated":/,
    /case\s+"system_settings_error":/,
    /case\s+"autostart_status":/,
    /case\s+"autostart_error":/,
  ]) {
    assert.match(
      appSource,
      kind,
      `expected backend event dispatch case: ${kind}`,
    );
  }
});

test("app.js consumes #about hash by opening the About GWT version surface", () => {
  assert.match(
    appSource,
    /window\.addEventListener\(\s*"hashchange",\s*consumeAboutHash/,
    "expected app.js to listen for #about hash changes",
  );
  assert.match(
    appSource,
    /window\.location\.hash\s*===\s*"#about"/,
    "expected app.js to detect #about hash exactly",
  );
  assert.match(
    appSource,
    /releaseNotesWindow\.openAbout\(versionState\.current\s*\|\|\s*null\)/,
    "expected #about to open the About GWT version surface",
  );
});

test("Custom Agents panel keeps the data-role='settings-scroll' hook for the legacy renderer", () => {
  // renderSettingsAgentList queries [data-role='settings-scroll'] to find
  // its target. Moving it onto the Custom Agents panel preserves the
  // existing list/edit flow without rewriting that surface.
  assert.match(
    settingsSource,
    /panelAgents\.dataset\.role\s*=\s*"settings-scroll"/,
    "Custom Agents panel must carry data-role='settings-scroll'",
  );
});

test("Custom Agents panel wires the env editor to update_custom_agent", () => {
  assert.match(
    settingsSource,
    /from "\/custom-agent-env-editor\.js"/,
    "settings surface must import the custom agent env editor module",
  );
  assert.match(
    settingsSource,
    /renderCustomAgentEnvEditor\(\{/,
    "renderSettingsAgentList must mount the env editor",
  );
  assert.match(
    settingsSource,
    /kind:\s*"update_custom_agent",\s*agent:\s*updatedAgent/,
    "env editor save must send update_custom_agent with the edited agent",
  );
});

test("switchSettingsTab toggles aria-selected and hidden together", () => {
  // The active / hidden contract: aria-selected on the tab and the .hidden
  // class on every panel except the matching one must change in lockstep.
  // Without this, screen readers and the visual state can desync.
  assert.match(
    settingsSource,
    /aria-selected/,
    "switchSettingsTab must update aria-selected",
  );
  assert.match(
    settingsSource,
    /panel\.classList\.toggle\(\s*"hidden"/,
    "switchSettingsTab must toggle hidden on non-active panels",
  );
});
