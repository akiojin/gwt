import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { parseHTML } from "linkedom";

import { createRecoveryCenterController } from "../recovery-center-modal.js";

const here = dirname(fileURLToPath(import.meta.url));

function fixture() {
  const { document } = parseHTML(`
    <button id="trigger">Recover</button>
    <div class="modal-backdrop" id="recovery-center-modal" aria-hidden="true">
      <div
        class="modal-shell recovery-center-shell"
        role="dialog"
        aria-modal="true"
        aria-label="Recovery Center"
        tabindex="-1"
      ></div>
    </div>
  `);
  const sent = [];
  const createNode = (tag, className, text) => {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  };
  const modalEl = document.getElementById("recovery-center-modal");
  const dialogEl = document.querySelector(".recovery-center-shell");
  const controller = createRecoveryCenterController({
    modalEl,
    dialogEl,
    createNode,
    sendAction: (request) => sent.push(request),
  });
  return { document, modalEl, dialogEl, controller, sent };
}

const candidates = [
  {
    action_handle: "rc1_intake_opaque",
    attention_required: true,
    purpose_preview: "Investigate <img src=x onerror=alert(1)>",
    kind: "intake",
    provider: "Codex",
    worktree_name: ".intake-5",
    last_checkpoint_at: "2026-07-16 10:42",
    coverage: "12 of 14 turns",
    capture_health: "degraded",
    board_pending: 2,
    board_delivery_failed: true,
    exact_available: false,
    exact_ambiguous: true,
    details: {
      "Base revision": "a1e0cb04",
      Status: "Provider identity needs confirmation",
    },
  },
  {
    action_handle: "rc1_execution_opaque",
    attention_required: true,
    purpose_preview: "Continue implementation",
    kind: "execution",
    provider: "Claude Code",
    worktree_name: "feature-x",
    last_checkpoint_at: "2026-07-16 09:10",
    coverage: { captured: 8, total: 8 },
    capture_health: "healthy",
    board_pending: false,
    exact_available: true,
    exact_ambiguity: false,
  },
  {
    action_handle: "rc1_restored_opaque",
    attention_required: false,
    purpose_preview: "Already restored automatically",
    kind: "intake",
    provider: "Codex",
  },
];

function click(document, node) {
  node.dispatchEvent(new document.defaultView.Event("click", { bubbles: true }));
}

function keydown(document, key) {
  const event = new document.defaultView.Event("keydown", { bubbles: true });
  Object.defineProperty(event, "key", { value: key });
  document.dispatchEvent(event);
}

test("renders attention candidates with recovery evidence and no HTML injection", () => {
  const deps = fixture();
  deps.controller.openAttention(candidates);

  const rows = deps.dialogEl.querySelectorAll(".recovery-center-row");
  assert.equal(rows.length, 2, "non-attention candidates stay out of the modal");
  const text = deps.dialogEl.textContent;
  for (const expected of [
    "Recovery Center",
    "Intake",
    "Execution",
    "Codex",
    "Claude Code",
    ".intake-5",
    "2026-07-16 10:42",
    "12 of 14 turns",
    "Capture degraded",
    "2 Board updates pending",
    "Board delivery needs attention",
    "Exact unavailable",
    "Exact match is ambiguous",
  ]) {
    assert.ok(text.includes(expected), `expected visible evidence: ${expected}`);
  }
  assert.equal(deps.dialogEl.querySelector("img"), null, "candidate text is textContent");
  assert.ok(text.includes("<img src=x onerror=alert(1)>"));
  assert.match(text, /2 need attention · 3 recoverable total/);
});

test("manual Recovery Center view includes every recovery candidate", () => {
  const deps = fixture();
  deps.controller.open(candidates);

  assert.equal(deps.dialogEl.querySelectorAll(".recovery-center-row").length, 3);
  assert.match(deps.dialogEl.textContent, /Review recoverable sessions/);
  assert.match(deps.dialogEl.textContent, /Already restored automatically/);
  assert.match(deps.dialogEl.textContent, /3 recoverable sessions/);
});

test("selecting a row exposes the complete action set", () => {
  const deps = fixture();
  deps.controller.open(candidates);
  const second = deps.dialogEl.querySelector('[data-action-handle="rc1_execution_opaque"]');
  click(deps.document, second);

  assert.equal(
    deps.dialogEl
      .querySelector('[data-action-handle="rc1_execution_opaque"]')
      .getAttribute("aria-selected"),
    "true",
  );
  const labels = Array.from(
    deps.dialogEl.querySelectorAll("[data-recovery-action]"),
    (node) => node.textContent,
  );
  assert.deepEqual(labels, [
    "Focus",
    "Confirm & Resume",
    "Continue Checkpoint",
    "Start Fresh",
    "Open Board",
    "Details",
    "Discard",
  ]);
  assert.match(deps.dialogEl.textContent, /8 of 8 captured/);
  assert.match(deps.dialogEl.textContent, /Exact available/);
});

test("backend action eligibility disables unavailable recovery choices", () => {
  const deps = fixture();
  deps.controller.open([
    {
      ...candidates[0],
      available_actions: ["continue_checkpoint", "details", "discard"],
    },
  ]);

  for (const action of ["focus", "confirm_resume", "start_fresh", "open_board"]) {
    assert.equal(
      deps.dialogEl.querySelector(`[data-recovery-action="${action}"]`).disabled,
      true,
      `${action} must follow backend eligibility`,
    );
  }
  for (const action of ["continue_checkpoint", "details", "discard"]) {
    assert.equal(
      deps.dialogEl.querySelector(`[data-recovery-action="${action}"]`).disabled,
      false,
      `${action} remains available`,
    );
  }
});

test("ambiguous provider roots require an explicit evidence-backed selection", () => {
  const deps = fixture();
  deps.controller.open([
    {
      ...candidates[0],
      available_actions: ["confirm_resume", "details", "discard"],
      provider_choices: [
        {
          choice_handle: "rp1_choice_a_opaque",
          label: "Candidate 1",
          evidence_count: 1,
        },
        {
          choice_handle: "rp1_choice_b_opaque",
          label: "Candidate 2",
          evidence_count: 2,
        },
      ],
    },
  ]);

  const roots = deps.dialogEl.querySelectorAll('[name="recovery-provider-choice"]');
  assert.equal(roots.length, 2);
  assert.match(deps.dialogEl.textContent, /Choose provider session/);
  assert.match(deps.dialogEl.textContent, /Candidate 1/);
  assert.match(deps.dialogEl.textContent, /1 recorded evidence signal/);
  assert.doesNotMatch(deps.dialogEl.textContent, /rp1_choice_[ab]_opaque/);
  const confirm = deps.dialogEl.querySelector('[data-recovery-action="confirm_resume"]');
  assert.equal(confirm.disabled, true, "exact resume stays disabled before selection");

  click(deps.document, roots[1]);
  assert.equal(
    deps.dialogEl.querySelector('[value="rp1_choice_b_opaque"]').checked,
    true,
  );
  const enabledConfirm = deps.dialogEl.querySelector(
    '[data-recovery-action="confirm_resume"]',
  );
  assert.equal(enabledConfirm.disabled, false);
  click(deps.document, enabledConfirm);
  assert.deepEqual(deps.sent, [
    {
      actionHandle: "rc1_intake_opaque",
      action: "confirm_resume",
      providerChoiceHandle: "rp1_choice_b_opaque",
    },
  ]);
});

test("pending action sends once and blocks every duplicate until settled", () => {
  const deps = fixture();
  deps.controller.open(candidates);
  const resume = deps.dialogEl.querySelector(
    '[data-recovery-action="confirm_resume"]',
  );

  click(deps.document, resume);
  click(deps.document, resume);

  assert.deepEqual(deps.sent, [
    { actionHandle: "rc1_intake_opaque", action: "confirm_resume" },
  ]);
  assert.equal(deps.dialogEl.getAttribute("aria-busy"), "true");
  for (const button of deps.dialogEl.querySelectorAll("[data-recovery-action]")) {
    assert.equal(button.disabled, true);
  }

  deps.controller.handleActionResult({
    action_handle: "rc1_intake_opaque",
    action: "confirm_resume",
    ok: false,
    message: "Provider is unavailable",
  });
  assert.equal(deps.dialogEl.getAttribute("aria-busy"), "false");
  assert.match(deps.dialogEl.textContent, /Provider is unavailable/);
  assert.equal(
    deps.dialogEl.querySelector('[data-recovery-action="confirm_resume"]').disabled,
    false,
  );
});

test("Discard requires an explicit second confirmation", () => {
  const deps = fixture();
  deps.controller.open(candidates);
  click(
    deps.document,
    deps.dialogEl.querySelector('[data-recovery-action="discard"]'),
  );

  assert.equal(deps.sent.length, 0, "first click only opens confirmation");
  assert.match(deps.dialogEl.textContent, /Discard this recovery candidate\?/);
  const confirm = deps.dialogEl.querySelector('[data-role="confirm-discard"]');
  assert.ok(confirm);
  click(deps.document, confirm);
  assert.deepEqual(deps.sent, [
    { actionHandle: "rc1_intake_opaque", action: "discard" },
  ]);
});

test("Escape cancels Discard confirmation first, then closes the modal", () => {
  const deps = fixture();
  deps.controller.open(candidates);
  click(
    deps.document,
    deps.dialogEl.querySelector('[data-recovery-action="discard"]'),
  );

  keydown(deps.document, "Escape");
  assert.equal(deps.controller.isOpen(), true);
  assert.equal(deps.dialogEl.querySelector('[data-role="confirm-discard"]'), null);

  keydown(deps.document, "Escape");
  assert.equal(deps.controller.isOpen(), false);
  assert.equal(deps.modalEl.getAttribute("aria-hidden"), "true");
});

test("close control dismisses and empty state remains useful", () => {
  const deps = fixture();
  deps.controller.openAttention([]);
  assert.match(deps.dialogEl.textContent, /No sessions need attention/);
  const close = deps.dialogEl.querySelector('[data-role="recovery-center-close"]');
  assert.ok(close);
  click(deps.document, close);
  assert.equal(deps.controller.isOpen(), false);
});

test("landing, styles, and renderer enforce the safe modal contract", () => {
  const index = readFileSync(resolve(here, "../index.html"), "utf8");
  const styles = readFileSync(resolve(here, "../styles/components.css"), "utf8");
  const source = readFileSync(resolve(here, "../recovery-center-modal.js"), "utf8");

  assert.match(index, /id="recovery-center-modal"/);
  assert.match(index, /class="modal-shell recovery-center-shell"/);
  assert.match(index, /aria-modal="true"/);
  assert.match(index, /aria-label="Recovery Center"/);
  assert.match(styles, /\.recovery-center-shell/);
  assert.equal(source.includes("innerHTML"), false, "renderer must build safe DOM nodes");
  assert.match(source, /createFocusTrap/);
});
