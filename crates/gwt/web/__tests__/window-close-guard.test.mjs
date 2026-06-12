// Agent-window close confirmation guard (user verification 2026-06-12):
// the agent window `×` must route through a confirm dialog while the agent
// process is live, and close silently otherwise. Pure-logic contract for
// the decision helper used by app.js `requestCloseWindow`.

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  agentWindowDisplayName,
  isAgentPaneWindow,
  shouldConfirmAgentWindowClose,
} from "../window-close-guard.js";

test("agent panes are detected by agent_id or agent presets", () => {
  assert.ok(isAgentPaneWindow({ preset: "agent" }));
  assert.ok(isAgentPaneWindow({ preset: "claude" }));
  assert.ok(isAgentPaneWindow({ preset: "codex" }));
  assert.ok(isAgentPaneWindow({ preset: "shell", agent_id: "custom-agent" }));
  assert.ok(!isAgentPaneWindow({ preset: "shell" }));
  assert.ok(!isAgentPaneWindow({ preset: "work" }));
  assert.ok(!isAgentPaneWindow(null));
});

test("live agent windows require close confirmation", () => {
  const agentWindow = { preset: "agent" };
  assert.ok(shouldConfirmAgentWindowClose(agentWindow, "running"));
  assert.ok(shouldConfirmAgentWindowClose(agentWindow, "waiting"));
  assert.ok(shouldConfirmAgentWindowClose(agentWindow, "starting"));
});

test("settled agent windows and non-agent windows close without confirmation", () => {
  const agentWindow = { preset: "agent" };
  assert.ok(!shouldConfirmAgentWindowClose(agentWindow, "idle"));
  assert.ok(!shouldConfirmAgentWindowClose(agentWindow, "stopped"));
  assert.ok(!shouldConfirmAgentWindowClose(agentWindow, "error"));
  assert.ok(!shouldConfirmAgentWindowClose({ preset: "work" }, "running"));
  assert.ok(!shouldConfirmAgentWindowClose({ preset: "shell" }, "running"));
});

test("display name follows dynamic_title → purpose_title → title precedence", () => {
  assert.equal(
    agentWindowDisplayName({
      dynamic_title: "fixing tests",
      purpose_title: "Claude Code",
      title: "Agent",
    }),
    "fixing tests",
  );
  assert.equal(
    agentWindowDisplayName({ purpose_title: "Claude Code", title: "Agent" }),
    "Claude Code",
  );
  assert.equal(agentWindowDisplayName({ title: "Agent" }), "Agent");
  assert.equal(agentWindowDisplayName(null), "agent");
});
