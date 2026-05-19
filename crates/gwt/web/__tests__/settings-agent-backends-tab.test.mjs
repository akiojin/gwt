// SPEC-1921 2026-05-18 amendment / FR-099 — Settings > Agent Backends tab.
//
// Source-pattern tests (matching the settings-system-tab.test.mjs pattern)
// validate that the new tab is wired into `renderSettingsWindow` alongside
// `Custom Agents`, that the panel hosts per-built-in sections for Claude
// Code / Codex, and that the socket dispatcher routes the new
// `agent_backend_*` events. Full DOM tests for add/edit/delete forms will
// land alongside the inline form work in T308 follow-up.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");

test("renderSettingsWindow declares an Agent Backends tab next to Custom Agents", () => {
  // The tab is a stable contract surface — frontend dispatch hits
  // `data-settings-tab='agent-backends'` and the body must mount the
  // matching panel.
  assert.match(
    appSource,
    /buildSettingsTab\("agent-backends",\s*"Agent Backends"/,
    "Agent Backends tab declaration missing from renderSettingsWindow",
  );
});

test("renderSettingsWindow mounts a panel with data-settings-panel='agent-backends'", () => {
  assert.match(
    appSource,
    /panelBackends\.dataset\.settingsPanel\s*=\s*"agent-backends"/,
    "Agent Backends panel must expose data-settings-panel='agent-backends'",
  );
});

test("renderSettingsWindow hydrates the Agent Backends panel on open", () => {
  assert.match(
    appSource,
    /renderAgentBackendsPanel\(panelBackends\)/,
    "Agent Backends panel must call renderAgentBackendsPanel on open",
  );
});

test("renderSettingsWindow sends list_agent_backends for both built-ins on open", () => {
  assert.match(
    appSource,
    /for\s*\(const agent of \["claudeCode",\s*"codex"\]\)/,
    "Agent Backends panel must enumerate both built-ins (FR-100 default targets)",
  );
  assert.match(
    appSource,
    /send\(\{\s*kind:\s*"list_agent_backends",\s*agent\s*\}\)/,
    "Agent Backends panel must dispatch list_agent_backends per built-in",
  );
});

test("renderAgentBackendsPanel builds per-built-in sections", () => {
  assert.match(
    appSource,
    /function renderAgentBackendsPanel\(panel\)\s*\{/,
    "expected renderAgentBackendsPanel function definition",
  );
  assert.match(
    appSource,
    /section\.dataset\.agent\s*=\s*agent/,
    "expected per-built-in section to carry data-agent attribute",
  );
});

test("renderAgentBackendsList renders empty-state helper text for each built-in", () => {
  assert.match(
    appSource,
    /No Claude Code backend profiles saved\./,
    "expected Claude Code empty-state copy",
  );
  assert.match(
    appSource,
    /No Codex backend profiles saved\./,
    "expected Codex empty-state copy",
  );
});

test("renderAgentBackendsList prints redacted profile rows when backends are loaded", () => {
  // The row should surface id (or display_name), base_url, and model — the
  // api_key is masked server-side by `redacted_for_wire` before reaching
  // this layer.
  assert.match(
    appSource,
    /row\.dataset\.backendId\s*=\s*profile\.id/,
    "agent backend row must declare data-backend-id",
  );
  assert.match(
    appSource,
    /profile\.base_url\s*\|\|\s*profile\.baseUrl\s*\|\|\s*""/,
    "agent backend row must read base_url",
  );
});

test("socket dispatcher routes agent_backend_list / saved / deleted / error events", () => {
  // 4 new BackendEvent cases must fall through the central dispatcher in
  // app.js so the panel re-renders, ephemeral status messages display, and
  // errors surface through setSettingsStatus.
  for (const kind of [
    "agent_backend_list",
    "agent_backend_saved",
    "agent_backend_deleted",
    "agent_backend_error",
  ]) {
    const pattern = new RegExp(`case\\s+"${kind}":`);
    assert.match(
      appSource,
      pattern,
      `socket dispatcher must handle BackendEvent::${kind}`,
    );
  }
});

test("knowledgeSettingsSurface exports renderAgentBackendsPanel", () => {
  // Same dispatcher hook the legacy custom-agents panel uses; exporting
  // through knowledgeSettingsSurface keeps the new surface reachable from
  // the dispatcher closure without leaking a new global symbol.
  assert.match(
    appSource,
    /renderAgentBackendsPanel,/,
    "renderAgentBackendsPanel must be re-exported from knowledgeSettingsSurface",
  );
  assert.match(
    appSource,
    /renderAgentBackendsPanelInAllSettingsWindows,/,
    "renderAgentBackendsPanelInAllSettingsWindows must be re-exported",
  );
});

test("agentBackendsState seeds empty lists for both built-ins", () => {
  assert.match(
    appSource,
    /const agentBackendsState\s*=\s*\{[\s\S]*?backends:\s*\{\s*claudeCode:\s*\[\],\s*codex:\s*\[\]\s*\}/,
    "agentBackendsState must seed backends.claudeCode and backends.codex",
  );
});
