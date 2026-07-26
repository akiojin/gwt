// SPEC-2359 W-17 (FR-398) — the Resume picker must not close optimistically.
//
// Before this phase, pick() sent `resume_workspace_agent` and immediately
// closed the modal, leaving the user staring at a blank canvas with no
// indication anything was happening (and no error surface if the socket was
// down). The picker now stays open in a pending state until the backend acks
// (`workspace_resume_agent_started`) or errors.

import { test } from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import { createWorkspaceResumePickerController } from "../workspace-resume-picker-modal.js";
import { createLaunchPendingController } from "../launch-pending-controller.js";

function createFixture() {
  const { document } = parseHTML(`
    <div id="modal"><div id="dialog"></div></div>
  `);
  let activeElement = null;
  Object.defineProperty(document, "activeElement", {
    configurable: true,
    get: () => activeElement,
  });
  const modalEl = document.getElementById("modal");
  const dialogEl = document.getElementById("dialog");
  const createNode = (tag, className, text) => {
    const node = document.createElement(tag);
    node.focus = () => {
      activeElement = node;
    };
    if (className) node.className = className;
    if (text !== undefined) node.textContent = text;
    return node;
  };
  dialogEl.focus = () => {
    activeElement = dialogEl;
  };
  return { document, modalEl, dialogEl, createNode };
}

function createPicker(fixture, { sent = [], launchPending, createOperationId } = {}) {
  const defaultOperationId = createOperationId ? "" : "resume-operation-test";
  const controller = createWorkspaceResumePickerController({
    modalEl: fixture.modalEl,
    dialogEl: fixture.dialogEl,
    createNode: fixture.createNode,
    send: (message) => sent.push(message),
    getResumeBounds: () => ({ x: 0, y: 0, width: 800, height: 600 }),
    launchPending,
    createOperationId: createOperationId || (() => defaultOperationId),
  });
  if (!defaultOperationId) return controller;
  const correlated = (event) => ({ operation_id: defaultOperationId, ...event });
  return {
    ...controller,
    handleAgentsList: (event) => controller.handleAgentsList(correlated(event)),
    handleStarted: (event) => controller.handleStarted(correlated(event)),
    handleError: (event) => controller.handleError(correlated(event)),
  };
}

const sampleAgent = {
  session_id: "work-1",
  agent_id: "codex",
  display_name: "Codex",
  branch: "feature/x",
};

test("native picker entries are labelled separately from fresh starts", () => {
  const fixture = createFixture();
  const picker = createPicker(fixture, {});

  picker.open("workspace-1");
  picker.handleAgentsList({
    workspace_id: "workspace-1",
    agents: [{ ...sampleAgent, resume_kind: "native_picker" }],
  });

  const tag = fixture.dialogEl.querySelector(".workspace-resume-picker-row-tag");
  assert.equal(tag?.textContent, "Open picker");
});

test("Resume picker exposes only inspection-capable Sessions, never a fresh-start fallback", () => {
  const fixture = createFixture();
  const picker = createPicker(fixture, {});

  picker.open("workspace-1");
  picker.handleAgentsList({
    workspace_id: "workspace-1",
    agents: [
      { ...sampleAgent, session_id: "metadata-only", resume_kind: "metadata_only" },
      { ...sampleAgent, session_id: "conversation", resume_kind: "session" },
    ],
  });

  const rows = fixture.dialogEl.querySelectorAll(".workspace-resume-picker-row");
  assert.equal(rows.length, 1, "metadata-only entries must fall back through Continue work");
  assert.equal(rows[0].dataset.sessionId, "conversation");
  assert.equal(rows[0].dataset.executionIntent, "inspection");
  assert.doesNotMatch(fixture.dialogEl.textContent, /Fresh start/);
  assert.match(
    fixture.dialogEl.textContent,
    /does not continue the Work/i,
    "the modal must explain that Resume is history-only",
  );
});

test("pick keeps the modal open in a pending state instead of closing", () => {
  const fixture = createFixture();
  const sent = [];
  const picker = createPicker(fixture, { sent });

  picker.open("workspace-1");
  picker.handleAgentsList({ workspace_id: "workspace-1", agents: [sampleAgent] });

  const row = fixture.dialogEl.querySelector(".workspace-resume-picker-row");
  assert.ok(row, "agent row rendered");
  row.click();

  assert.equal(sent.length, 1);
  assert.equal(sent[0].kind, "resume_workspace_agent");
  assert.equal(
    fixture.modalEl.classList.contains("open"),
    true,
    "modal must stay open while the resume is in flight",
  );
  const pendingRow = fixture.dialogEl.querySelector(
    ".workspace-resume-picker-row",
  );
  assert.equal(
    pendingRow.disabled,
    true,
    "rows are disabled while the resume is in flight",
  );
  assert.match(
    fixture.dialogEl.textContent,
    /Opening for inspection/,
    "pending state is visible to the user",
  );
});

test("a second click while pending does not send a duplicate request", () => {
  const fixture = createFixture();
  const sent = [];
  const picker = createPicker(fixture, { sent });

  picker.open("workspace-1");
  picker.handleAgentsList({ workspace_id: "workspace-1", agents: [sampleAgent] });
  fixture.dialogEl.querySelector(".workspace-resume-picker-row").click();
  const row = fixture.dialogEl.querySelector(".workspace-resume-picker-row");
  row.click();

  assert.equal(sent.length, 1, "pending pick must not re-send");
});

test("handleStarted closes the modal once the backend acks", () => {
  const fixture = createFixture();
  const picker = createPicker(fixture, {});

  picker.open("workspace-1");
  picker.handleAgentsList({ workspace_id: "workspace-1", agents: [sampleAgent] });
  fixture.dialogEl.querySelector(".workspace-resume-picker-row").click();

  picker.handleStarted({ session_id: "work-1" });

  assert.equal(fixture.modalEl.classList.contains("open"), false);
});

test("handleError clears pending and shows the reason in place", () => {
  const fixture = createFixture();
  const picker = createPicker(fixture, {});

  picker.open("workspace-1");
  picker.handleAgentsList({ workspace_id: "workspace-1", agents: [sampleAgent] });
  fixture.dialogEl.querySelector(".workspace-resume-picker-row").click();

  picker.handleError({ session_id: "work-1", message: "Worktree missing" });

  assert.equal(fixture.modalEl.classList.contains("open"), true);
  assert.match(fixture.dialogEl.textContent, /Worktree missing/);
  const row = fixture.dialogEl.querySelector(".workspace-resume-picker-row");
  assert.equal(row.disabled, false, "rows re-enable after an error");
});

test("pick consults the shared launch-pending controller as a global guard", () => {
  const fixture = createFixture();
  const sent = [];
  const launchPending = createLaunchPendingController({
    setTimeoutFn: () => 1,
    clearTimeoutFn: () => {},
  });
  const picker = createPicker(fixture, { sent, launchPending });

  // Another surface already started this Work's resume.
  launchPending.begin("session:work-1", "Resume");

  picker.open("workspace-1");
  picker.handleAgentsList({ workspace_id: "workspace-1", agents: [sampleAgent] });
  fixture.dialogEl.querySelector(".workspace-resume-picker-row").click();

  assert.equal(
    sent.length,
    0,
    "picker must not double-send a Work that is already resuming elsewhere",
  );
});

test("picker ignores a stale agents list from another Workspace", () => {
  const fixture = createFixture();
  const picker = createPicker(fixture);

  picker.open("workspace-1");
  picker.handleAgentsList({
    workspace_id: "workspace-stale",
    agents: [sampleAgent],
  });
  assert.equal(
    fixture.dialogEl.querySelectorAll(".workspace-resume-picker-row").length,
    0,
    "a response for a previously opened Work must have zero effect",
  );

  picker.handleAgentsList({
    workspace_id: "workspace-1",
    agents: [sampleAgent],
  });
  assert.equal(
    fixture.dialogEl.querySelectorAll(".workspace-resume-picker-row").length,
    1,
  );
});

test("picker ignores an earlier list response after reopening the same Workspace", () => {
  const fixture = createFixture();
  const operationIds = ["list-operation-1", "list-operation-2"];
  const picker = createPicker(fixture, {
    createOperationId: () => operationIds.shift(),
  });

  assert.equal(picker.open("workspace-1"), "list-operation-1");
  assert.equal(picker.open("workspace-1"), "list-operation-2");
  picker.handleAgentsList({
    operation_id: "list-operation-1",
    workspace_id: "workspace-1",
    agents: [sampleAgent],
  });
  assert.equal(
    fixture.dialogEl.querySelectorAll(".workspace-resume-picker-row").length,
    0,
  );
  picker.handleAgentsList({
    operation_id: "list-operation-2",
    workspace_id: "workspace-1",
    agents: [sampleAgent],
  });
  assert.equal(
    fixture.dialogEl.querySelectorAll(".workspace-resume-picker-row").length,
    1,
  );
});

test("picker settles only the exact pending Session response", () => {
  const fixture = createFixture();
  const picker = createPicker(fixture);

  picker.open("workspace-1");
  picker.handleAgentsList({ workspace_id: "workspace-1", agents: [sampleAgent] });
  fixture.dialogEl.querySelector(".workspace-resume-picker-row").click();

  picker.handleStarted({ session_id: "work-stale" });
  assert.equal(
    fixture.modalEl.classList.contains("open"),
    true,
    "a stale success must not close the current picker",
  );
  picker.handleError({
    session_id: "work-stale",
    message: "Stale failure",
  });
  assert.doesNotMatch(
    fixture.dialogEl.textContent,
    /Stale failure/,
    "a stale error must not replace the current pending state",
  );

  picker.handleStarted({ session_id: "work-1" });
  assert.equal(fixture.modalEl.classList.contains("open"), false);

  picker.open("workspace-1");
  picker.handleStarted({ session_id: "work-1" });
  assert.equal(
    fixture.modalEl.classList.contains("open"),
    true,
    "an unsolicited success without a pending request must be ignored",
  );
});

test("same-Session retry ignores the timed-out operation's late response", () => {
  const fixture = createFixture();
  const sent = [];
  let timeoutCallback = null;
  const operationIds = [
    "list-operation-1",
    "resume-operation-1",
    "resume-operation-2",
  ];
  const launchPending = createLaunchPendingController({
    setTimeoutFn: (callback) => {
      timeoutCallback = callback;
      return operationIds.length;
    },
    clearTimeoutFn: () => {},
  });
  const picker = createPicker(fixture, {
    sent,
    launchPending,
    createOperationId: () => operationIds.shift(),
  });

  picker.open("workspace-1");
  picker.handleAgentsList({
    operation_id: "list-operation-1",
    workspace_id: "workspace-1",
    agents: [sampleAgent],
  });
  fixture.dialogEl.querySelector(".workspace-resume-picker-row").click();
  assert.equal(sent[0].operation_id, "resume-operation-1");

  timeoutCallback();
  picker.render();
  fixture.dialogEl.querySelector(".workspace-resume-picker-row").click();
  assert.equal(sent[1].operation_id, "resume-operation-2");

  const staleError = {
    session_id: "work-1",
    operation_id: "resume-operation-1",
    message: "Old request failed",
  };
  picker.handleError(staleError);
  assert.equal(launchPending.settleAck(staleError), false);
  assert.doesNotMatch(fixture.dialogEl.textContent, /Old request failed/);
  assert.equal(launchPending.isPending("session:work-1"), true);

  const stale = {
    session_id: "work-1",
    operation_id: "resume-operation-1",
  };
  picker.handleStarted(stale);
  assert.equal(launchPending.settleAck(stale), false);
  assert.equal(fixture.modalEl.classList.contains("open"), true);
  assert.equal(launchPending.isPending("session:work-1"), true);

  const current = {
    session_id: "work-1",
    operation_id: "resume-operation-2",
  };
  picker.handleStarted(current);
  launchPending.settleAck(current);
  assert.equal(fixture.modalEl.classList.contains("open"), false);
});

test("shared pending timeout re-enables the picker and ignores its late ack", () => {
  const fixture = createFixture();
  let timeoutCallback = null;
  const launchPending = createLaunchPendingController({
    setTimeoutFn: (callback) => {
      timeoutCallback = callback;
      return 1;
    },
    clearTimeoutFn: () => {},
  });
  const picker = createPicker(fixture, { launchPending });

  picker.open("workspace-1");
  picker.handleAgentsList({ workspace_id: "workspace-1", agents: [sampleAgent] });
  fixture.dialogEl.querySelector(".workspace-resume-picker-row").click();
  assert.equal(typeof timeoutCallback, "function");

  timeoutCallback();
  picker.render();

  const row = fixture.dialogEl.querySelector(".workspace-resume-picker-row");
  assert.equal(row.disabled, false, "timeout must release the local pending row");
  const error = fixture.dialogEl.querySelector(".workspace-resume-picker-error");
  assert.match(error?.textContent || "", /timed out/i);
  assert.equal(error?.getAttribute("role"), "alert");

  picker.handleStarted({ session_id: "work-1" });
  assert.equal(
    fixture.modalEl.classList.contains("open"),
    true,
    "a late ack after timeout must not close the retryable picker",
  );
});

test("picker exposes dialog status semantics and preserves focus across rerenders", () => {
  const fixture = createFixture();
  const picker = createPicker(fixture);

  picker.open("workspace-1");
  picker.handleAgentsList({ workspace_id: "workspace-1", agents: [sampleAgent] });

  assert.equal(fixture.dialogEl.getAttribute("role"), "dialog");
  assert.equal(fixture.dialogEl.getAttribute("aria-modal"), "true");
  assert.equal(
    fixture.dialogEl.getAttribute("aria-labelledby"),
    "workspace-resume-picker-title",
  );

  const row = fixture.dialogEl.querySelector(".workspace-resume-picker-row");
  row.focus();
  picker.handleAgentsList({
    workspace_id: "workspace-1",
    agents: [{ ...sampleAgent, lifecycle_status: "interrupted" }],
  });
  assert.equal(
    fixture.document.activeElement?.dataset?.sessionId,
    "work-1",
    "an async list rerender must restore the focused Session row",
  );

  fixture.dialogEl.querySelector(".workspace-resume-picker-row").click();
  const pending = fixture.dialogEl.querySelector(".workspace-resume-picker-pending");
  assert.equal(pending?.getAttribute("role"), "status");
  assert.equal(pending?.getAttribute("aria-live"), "polite");
  assert.equal(
    fixture.document.activeElement?.classList.contains(
      "workspace-resume-picker-cancel",
    ),
    true,
    "when a focused row becomes disabled, focus moves to the enabled Cancel action",
  );
});
