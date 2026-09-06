// SPEC-3864 FR-005..FR-007 — the Launch Wizard renders one agent-independent
// setup affordance (`launchWizard.agent_setup`) instead of per-agent hint
// blocks. These source assertions lock in the wiring contract; the real
// rendering is covered by the headed Playwright spec.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const surface = readFileSync(resolve(here, "../launch-wizard-surface.js"), "utf8");
const css = readFileSync(resolve(here, "../styles/app.css"), "utf8");

test("wizard renders agent_setup through a single descriptor-driven helper", () => {
  assert.match(surface, /function appendAgentSetupNote\(parent, setup\)/);
  assert.match(surface, /appendAgentSetupNote\(section, launchWizard\.agent_setup\)/);
  assert.match(surface, /note\.dataset\.setupKind = setup\.kind/);
  assert.match(surface, /launch-agent-setup__title/);
  assert.match(surface, /launch-agent-setup__detail/);
});

test("setup action dispatches the generic run_agent_setup wire tag", () => {
  assert.match(surface, /kind:\s*"run_agent_setup"/);
  assert.doesNotMatch(surface, /run_opencode_setup/);
});

test("no per-agent needs-setup branch survives in the wizard surface", () => {
  assert.doesNotMatch(surface, /hermes_needs_setup/);
  assert.doesNotMatch(surface, /opencode_needs_setup/);
  assert.doesNotMatch(surface, /Run OpenCode setup/);
});

test("setup affordance styling uses Operator tokens only", () => {
  const start = css.indexOf(".launch-agent-setup {");
  assert.ok(start > 0, "launch-agent-setup block exists");
  const block = css.slice(start, css.indexOf(".launch-agent-setup__action", start) + 200);
  assert.doesNotMatch(block, /#[0-9a-fA-F]{3,8}\b|rgba?\(/);
  assert.match(block, /var\(--color-border\)/);
});
