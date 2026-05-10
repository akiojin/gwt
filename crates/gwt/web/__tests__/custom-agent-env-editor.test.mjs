import { test } from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";

import { renderCustomAgentEnvEditor } from "../custom-agent-env-editor.js";

function createContext() {
  const { document, window } = parseHTML("<!doctype html><body></body>");
  const saved = [];
  const cancelled = [];
  const agent = {
    id: "claude-proxy",
    display_name: "Claude Proxy",
    command: "claude",
    default_args: ["--model", "sonnet"],
    mode_args: {
      normal: ["--print"],
      continue: ["--continue"],
      resume: ["--resume"],
    },
    skip_permissions_args: ["--dangerously-skip-permissions"],
    env: {
      ANTHROPIC_API_KEY: "redacted",
      ANTHROPIC_BASE_URL: "http://proxy.local:32768",
    },
  };
  const editor = renderCustomAgentEnvEditor({
    document,
    agent,
    onSave: (updated) => saved.push(updated),
    onCancel: () => cancelled.push(true),
  });
  document.body.appendChild(editor);
  return { document, window, agent, editor, saved, cancelled };
}

function click(window, element) {
  element.dispatchEvent(new window.Event("click", { bubbles: true }));
}

test("custom agent env editor can add rename remove and save env rows", () => {
  const { document, window, agent, saved } = createContext();

  const rows = () => [...document.querySelectorAll("[data-role='custom-agent-env-row']")];
  assert.equal(rows().length, 2);

  const firstKey = rows()[0].querySelector("[data-role='custom-agent-env-key']");
  firstKey.value = "ANTHROPIC_AUTH_TOKEN";

  click(window, rows()[1].querySelector("[data-role='custom-agent-env-remove']"));
  assert.equal(rows().length, 1);

  click(window, document.querySelector("[data-role='custom-agent-env-add']"));
  const added = rows()[1];
  added.querySelector("[data-role='custom-agent-env-key']").value = "ANTHROPIC_BASE_URL";
  added.querySelector("[data-role='custom-agent-env-value']").value = "https://api.example.test";

  click(window, document.querySelector("[data-role='custom-agent-env-save']"));

  assert.equal(saved.length, 1);
  assert.deepEqual(saved[0].env, {
    ANTHROPIC_AUTH_TOKEN: "redacted",
    ANTHROPIC_BASE_URL: "https://api.example.test",
  });
  assert.deepEqual(agent.env, {
    ANTHROPIC_API_KEY: "redacted",
    ANTHROPIC_BASE_URL: "http://proxy.local:32768",
  });
});

test("custom agent env editor ignores blank keys and preserves agent fields", () => {
  const { document, window, saved } = createContext();

  click(window, document.querySelector("[data-role='custom-agent-env-add']"));
  const rows = [...document.querySelectorAll("[data-role='custom-agent-env-row']")];
  const added = rows[rows.length - 1];
  added.querySelector("[data-role='custom-agent-env-key']").value = "   ";
  added.querySelector("[data-role='custom-agent-env-value']").value = "ignored";

  click(window, document.querySelector("[data-role='custom-agent-env-save']"));

  assert.equal(saved.length, 1);
  assert.equal(saved[0].id, "claude-proxy");
  assert.equal(saved[0].display_name, "Claude Proxy");
  assert.deepEqual(saved[0].default_args, ["--model", "sonnet"]);
  assert.deepEqual(saved[0].mode_args, {
    normal: ["--print"],
    continue: ["--continue"],
    resume: ["--resume"],
  });
  assert.deepEqual(saved[0].skip_permissions_args, ["--dangerously-skip-permissions"]);
  assert.equal(saved[0].env.ANTHROPIC_API_KEY, "redacted");
  assert.equal(saved[0].env.ANTHROPIC_BASE_URL, "http://proxy.local:32768");
  assert.equal(Object.keys(saved[0].env).length, 2);
});
