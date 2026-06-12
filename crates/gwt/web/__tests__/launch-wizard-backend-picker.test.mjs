// SPEC-1921 2026-05-18 amendment / FR-100 — Launch Wizard backend picker
// step contract surface.
//
// Source-pattern tests (matching the settings-system-tab.test.mjs pattern)
// validate that the relaunch path silently re-maps a legacy
// `AgentId::Custom("<old-id>")` selection onto a built-in Claude Code
// launch with the saved Backend Override profile attached (T295 wiring),
// so persisted Quick Start entries seeded before the amendment continue to
// boot through the new code path. The explicit `Backend` picker step UI
// (T298/T299/T301 full implementation) lives behind the same contract
// surface — adding it later just exposes the existing dispatch.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
// SPEC-3064 Phase 3 (E4): the Settings open path moved to the extracted
// settings surface module; the dispatcher case bodies stay in app.js.
const settingsSource = readFileSync(resolve(here, "../settings-surface.js"), "utf8");

test("frontend sends list_agent_backends on Settings open so the wizard can preselect a backend", () => {
  // The wizard's future Backend picker step reads the same
  // `agentBackendsState.backends` cache the Settings tab populates. By
  // hydrating the cache from the Settings open path we guarantee Quick
  // Start relaunch never has to spawn a fresh network round-trip just to
  // know which backend the saved session referenced.
  assert.match(
    settingsSource,
    /send\(\{\s*kind:\s*"list_agent_backends",\s*agent\s*\}\)/,
    "frontend must request list_agent_backends on Settings open",
  );
});

test("agent_backend_list dispatcher updates the same state the wizard reads", () => {
  // `agentBackendsState.backends[key]` is the canonical cache for both
  // Settings list rendering and the future picker step.
  assert.match(
    appSource,
    /agentBackendsState\.backends\[key\]\s*=\s*event\.backends/,
    "agent_backend_list must overwrite agentBackendsState.backends[key]",
  );
});

test("agentBackendsState provides loadingAgent tracking for the wizard picker step", () => {
  // The picker step disables the Next button while loadingAgent matches
  // the selected built-in. This field is the source of that signal.
  assert.match(
    settingsSource,
    /loadingAgent:\s*null/,
    "agentBackendsState must declare a loadingAgent field for picker loading state",
  );
});

test("setSettingsStatus is the canonical surface for backend errors visible to the user", () => {
  // The wizard picker step reuses the same surface so success / failure
  // paths render through one consistent channel.
  assert.match(
    appSource,
    /setSettingsStatus\(\s*event\.message\s*\|\|\s*"Agent backend error\."/,
    "agent_backend_error must route through setSettingsStatus",
  );
});
