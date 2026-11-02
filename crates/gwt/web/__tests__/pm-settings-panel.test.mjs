/* SPEC-3431 FR-026 / FR-132 — shared Project Manager settings editor.
 *
 * The PM is the one agent the user never launches through the wizard, so the
 * canonical editor lives in Settings and may be mounted in more than one
 * Settings window. The rail gear and command palette only route to that tab;
 * they do not own a second editor or state path.
 *
 * These are DOM-level contract assertions over the real index.html +
 * components.css plus wiring assertions over app.js / operator-shell.js — a
 * handler that exists but is never reached is the failure mode this file
 * exists to prevent.
 */

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";

import { createMemberTuningModel } from "../agent-activity-model.js";
import { createPmSettingsPanel } from "../pm-settings-panel.js";

const here = dirname(fileURLToPath(import.meta.url));
const html = readFileSync(resolve(here, "../index.html"), "utf8");
const componentsCss = readFileSync(resolve(here, "../styles/components.css"), "utf8");
const appJs = readFileSync(resolve(here, "../app.js"), "utf8");
const operatorShellJs = readFileSync(resolve(here, "../operator-shell.js"), "utf8");

const RUNNING_STATUS = {
  project_key: "project", configured_member_id: "pm-member", profile_revision: "profile-token",
  available: true,
  auto_start: true,
  configured_agent_id: "claude",
  configured_model: null,
  configured_reasoning: null,
  loop_interval_secs: 60,
  running_agent_id: "claude",
  running_model: null,
  running_reasoning: null,
  is_running: true,
  agent_options: [
    { id: "claude", name: "Claude Code" },
    { id: "codex", name: "Codex" },
    { id: "grok", name: "Grok Build" },
  ],
};

const GROK_RUNNING_STATUS = {
  ...RUNNING_STATUS,
  configured_agent_id: "grok",
  configured_model: "team/grok-code-fast",
  configured_reasoning: "high",
  running_agent_id: "grok",
  running_model: "team/grok-code-fast",
  running_reasoning: "high",
};

function fixture({ confirmAnswer = true, mounts = 1 } = {}) {
  const { document } = parseHTML(html);
  document.getElementById("pm-settings-panel")?.remove();
  const sent = [];
  const confirmed = [], openedMembers = [], reloaded = [];
  const frames = new Set();
  const flush = () => {
    const callbacks = [...frames];
    frames.clear();
    callbacks.forEach(callback => callback());
  };
  const tuningController = createMemberTuningModel("project", { onSave: event => sent.push(event), scheduleFrame: callback => { frames.add(callback); return () => frames.delete(callback); } });
  const panel = createPmSettingsPanel({
    projectKey: "project", tuningController, onOpenMember: memberId => openedMembers.push(memberId), onReloadProfile: () => reloaded.push(true),
    document,
    scheduleFrame(callback) { frames.add(callback); return () => frames.delete(callback); },
    send: (event) => sent.push(event),
    confirm: (message) => {
      confirmed.push(message);
      return confirmAnswer;
    },
  });
  const views = [];
  for (let index = 0; index < mounts; index += 1) {
    const view = document.createElement("section");
    view.className = "settings-panel";
    view.dataset.settingsPanel = "project-manager";
    document.body.appendChild(view);
    panel.mount(view);
    views.push(view);
  }
  const applyStatus = panel.applyStatus.bind(panel);
  // Existing interaction assertions observe a completed display frame.
  panel.applyStatus = status => { applyStatus(status); flush(); };
  return { document, sent, confirmed, panel, views, applyStatus, frames, flush, tuningController, openedMembers, reloaded };
}

function changeEvent(document) {
  return new document.defaultView.Event("change");
}

function required(root, selector) {
  const node = root.querySelector(selector);
  assert.ok(node, `expected ${selector}`);
  return node;
}

function sharedPrimitiveRule(className) {
  const match = componentsCss.match(
    new RegExp(`:root\\[data-theme\\]\\s+\\.${className}\\s*\\{([^}]*)\\}`),
  );
  assert.ok(match, `expected shared .${className} rule`);
  return match[1];
}

function accessibleName(root, control) {
  const ariaLabel = control.getAttribute("aria-label")?.trim();
  if (ariaLabel) return ariaLabel;
  const wrappingLabel = control.closest("label")?.textContent?.trim();
  if (wrappingLabel) return wrappingLabel;
  const id = control.getAttribute("id");
  return id ? root.querySelector(`label[for="${id}"]`)?.textContent?.trim() || "" : "";
}

function assertIntervalError(view, copyPattern) {
  const error = required(view, '[data-role="pm-loop-interval-error"]');
  assert.ok(error.classList.contains("settings-status"));
  assert.equal(error.dataset.kind, "error");
  assert.match(error.textContent, copyPattern);
  assert.doesNotMatch(error.textContent, /[\u3040-\u30ff\u3400-\u9fff]/);
}

test("FR-132: Settings mount は英語の PM controls と shared primitives を使う", () => {
  const { views } = fixture();
  const view = views[0];

  assert.ok(view.querySelector(".settings-section"));
  const interval = required(view, '[data-role="pm-loop-interval"]');
  const autoStart = required(view, '[data-role="pm-auto-start"]');
  assert.ok(interval.classList.contains("settings-input"));
  assert.ok(autoStart.classList.contains("settings-checkbox"));
  assert.match(accessibleName(view, interval), /Loop interval/i);
  assert.match(accessibleName(view, autoStart), /Auto start/i);
  assert.doesNotMatch(
    view.textContent,
    /[\u3040-\u30ff\u3400-\u9fff]/,
    "PM Settings copy must stay English",
  );
});

test("AC-6: PM Settings explains existing controls on focus and hover without acting", () => {
  const { document, sent, confirmed, panel, views } = fixture({ mounts: 2 });
  panel.applyStatus(RUNNING_STATUS);
  const view = views[0];
  for (const [role, meaning] of [
    ["pm-autonomy-status", /coordination.*Issue Monitor/i],
    ["pm-pause-toggle", /keeping the Project Manager available/i],
    ["pm-loop-interval", /Minimum: 10 seconds/i],
    ["pm-auto-start", /automatically.*opens/i], ["pm-restart", /new conversation/i],
  ]) {
    const control = required(view, `[data-role="${role}"]`);
    const text = control.getAttribute("aria-description") || "";
    assert.match(text, meaning, role);
    control.dispatchEvent(new document.defaultView.Event("focusin", { bubbles: true }));
    const popup = document.getElementById(control.getAttribute("aria-describedby")?.split(/\s+/).at(-1));
    assert.ok(popup, "focus associates the explanation with its control");
    assert.equal(popup.hidden, false);
    assert.equal(popup.textContent, text);
    control.dispatchEvent(new document.defaultView.Event("focusout", { bubbles: true }));
    control.dispatchEvent(new document.defaultView.Event("mouseover", { bubbles: true }));
    assert.equal(popup.hidden, false);
    assert.equal(popup.textContent, text);
    control.dispatchEvent(new document.defaultView.Event("mouseout", { bubbles: true }));
  }
  const interval = required(view, '[data-role="pm-loop-interval"]');
  interval.dispatchEvent(new document.defaultView.Event("focusin", { bubbles: true }));
  assert.match(interval.getAttribute("aria-describedby"), /pm-loop-interval-error-\d+ /,
    "tooltip association preserves the existing validation description");
  const firstPopupId = interval.getAttribute("aria-describedby").split(/\s+/).at(-1);
  const second = required(views[1], '[data-role="pm-auto-start"]');
  second.dispatchEvent(new document.defaultView.Event("focusin", { bubbles: true }));
  assert.notEqual(second.getAttribute("aria-describedby"), firstPopupId, "Settings mounts have separate tooltip identities");
  assert.equal(typeof panel.unmount, "function", "window deletion needs explicit tooltip cleanup");
  panel.unmount(views[0]);
  views[0].remove();
  assert.equal(document.getElementById(firstPopupId), null, "unmount disposes immediately without another status");
  assert.ok(document.getElementById(second.getAttribute("aria-describedby")), "other Settings mounts remain live");
  panel.applyStatus({ ...RUNNING_STATUS, loop_interval_secs: 84 });
  assert.equal(interval.value, "60", "unmounted controls no longer receive snapshots");
  assert.equal(required(views[1], '[data-role="pm-loop-interval"]').value, "84");
  document.body.appendChild(view);
  panel.mount(view);
  assert.equal(required(view, '[data-role="pm-loop-interval"]').value, "84",
    "remount pulls the latest committed snapshot");
  assert.match(appJs, /pmSettingsPanel\.unmount\(element\.querySelector\('\[data-settings-panel="project-manager"\]'\)\)/);
  assert.deepEqual(sent, []);
  assert.deepEqual(confirmed, []);
});

test("FR-132: rail gear remains separate from the PM launcher without a standalone overlay", () => {
  const { document } = parseHTML(html);
  const gear = document.getElementById("op-pm-settings-button");
  assert.ok(gear, "設定を開く歯車が必要");
  // 歯車が PM ボタンの子だと、歯車クリックが PM の click に必ず bubble して
  // 設定を開くたびに PM が起動してしまう。構造で排除する。
  assert.equal(
    document.getElementById("op-pm-entry").contains(gear),
    false,
    "歯車は PM ボタンの内側に置かない",
  );
  assert.equal(gear.classList.contains("op-rail__item"), false);
  assert.equal(
    document.getElementById("pm-settings-panel"),
    null,
    "standalone PM overlay must be removed",
  );
  assert.equal(gear.getAttribute("aria-controls"), null);

  assert.match(
    appJs,
    /document\.getElementById\("op-pm-entry"\)/,
    "PM ランチャーの click 配線は維持する",
  );
  assert.match(appJs, /send\(\{\s*kind:\s*"open_pm_agent"\s*\}\)/);
});

test("FR-132: shared controller routes gear and pm-settings command to Settings", () => {
  const { document, panel, sent } = fixture({ mounts: 0 });
  assert.equal(typeof panel.bindEntryPoints, "function");
  panel.bindEntryPoints({ document });

  const openedTargets = [];
  const openedOwners = [];
  document.addEventListener("settings:open", (event) => {
    openedTargets.push(event.detail?.target);
    openedOwners.push(event.detail?.surface_owner);
  });

  required(document, "#op-pm-settings-button").click();
  assert.deepEqual(openedTargets, ["project-manager"]);
  assert.deepEqual(openedOwners, ["pm"], "PM Settings must retain its opening surface");
  assert.deepEqual(sent, [], "opening Settings must not start or mutate the PM");

  openedTargets.length = 0;
  let observedCommands = 0;
  document.addEventListener("op:command", () => {
    observedCommands += 1;
  });
  document.dispatchEvent(
    new document.defaultView.CustomEvent("op:command", {
      detail: { id: "pm-settings" },
    }),
  );
  assert.deepEqual(openedTargets, ["project-manager"]);
  assert.equal(observedCommands, 1, "routing must not hide the command from observers");
  assert.deepEqual(sent, [], "the PM settings command must not mutate the PM");
});

test("FR-132: late and existing mounts share the latest frame snapshot while actions read synchronously", () => {
  const { document, sent, panel, views, applyStatus, frames, flush } = fixture();
  applyStatus({ ...RUNNING_STATUS, loop_interval_secs: 70 });
  applyStatus({
    ...RUNNING_STATUS,
    auto_start: false,
    configured_agent_id: "codex",
    configured_model: "gpt-5.1-codex-max",
    loop_interval_secs: 42,
  });
  assert.equal(required(views[0], '[data-role="pm-loop-interval"]').value, "60",
    "received updates must wait for the display frame");
  assert.equal(frames.size, 1, "PM operational status reserves one frame for every mount");

  const lateView = document.createElement("section");
  lateView.className = "settings-panel";
  lateView.dataset.settingsPanel = "project-manager";
  document.body.appendChild(lateView);
  panel.mount(lateView);
  views.push(lateView);
  assert.equal(required(lateView, '[data-role="pm-loop-interval"]').value, "42",
    "a late mount immediately reads the latest model");
  flush();

  for (const view of views) {
    assert.equal(required(view, '[data-role="pm-auto-start"]').checked, false);
    assert.match(required(view, '[data-role="pm-configured-profile"]').textContent, /Codex/);
    assert.match(required(view, '[data-role="pm-configured-profile"]').textContent, /gpt-5.1-codex-max/);
    assert.equal(required(view, '[data-role="pm-loop-interval"]').value, "42");
  }

  panel.applyStatus({ ...RUNNING_STATUS, loop_interval_secs: 84 });
  for (const view of views) {
    assert.equal(required(view, '[data-role="pm-loop-interval"]').value, "84");
  }

  applyStatus({ available: false });
  required(views[0], '[data-role="pm-pause-toggle"]').click();
  assert.deepEqual(sent, [], "an action rejects unavailable state before its frame renders");
  panel.unmount(views[0]);
  lateView.remove();
  applyStatus(RUNNING_STATUS); // Synchronous pruning releases the detached view.
  assert.equal(frames.size, 0, "unmount/prune cancels the last pending display frame");
  flush();
  assert.equal(required(views[0], '[data-role="pm-loop-interval"]').value, "84");
});

test("a failing PM settings mount does not leave another mount on the old snapshot", () => {
  const { panel, views } = fixture({ mounts: 2 });
  const runningLine = required(views[0], '[data-role="pm-running-as"]');
  Object.defineProperty(runningLine, "textContent", {
    configurable: true,
    set() { throw new Error("failed settings renderer"); },
  });

  assert.throws(
    () => panel.applyStatus({ ...RUNNING_STATUS, loop_interval_secs: 84 }),
    /failed settings renderer/,
  );
  assert.equal(required(views[1], '[data-role="pm-loop-interval"]').value, "84");
});

test("FR-132: active-project status replacement updates every mount without leaking the prior project", () => {
  const { panel, views } = fixture({ mounts: 2 });
  panel.applyStatus({
    ...RUNNING_STATUS,
    configured_agent_id: "claude",
    configured_model: "claude-opus-4-1",
    loop_interval_secs: 30,
  });

  panel.applyStatus({
    ...RUNNING_STATUS,
    configured_agent_id: "codex",
    configured_model: "gpt-5.1-codex-max",
    loop_interval_secs: 75,
    running_agent_id: "codex",
  });

  for (const view of views) {
    assert.match(required(view, '[data-role="pm-configured-profile"]').textContent, /Codex/);
    assert.match(required(view, '[data-role="pm-configured-profile"]').textContent, /gpt-5.1-codex-max/);
    assert.equal(required(view, '[data-role="pm-loop-interval"]').value, "75");
    assert.doesNotMatch(view.textContent, /claude-opus-4-1/);
  }
});

test("FR-026: Running as は稼働中エージェントを名乗る", () => {
  const { document, panel } = fixture();
  panel.applyStatus({ ...RUNNING_STATUS, running_agent_id: "codex" });
  const running = document.querySelector('[data-role="pm-running-as"]');
  assert.ok(running, "Running as 行が必要");
  assert.match(running.textContent, /Running as:/);
  assert.match(running.textContent, /Codex/, "稼働中の agent 名を出す");

  panel.applyStatus({ ...RUNNING_STATUS, is_running: false, running_agent_id: null });
  assert.match(
    document.querySelector('[data-role="pm-running-as"]').textContent,
    /Not running/,
    "停止中は稼働中を名乗らない",
  );
});

test("PM status preserves unlisted saved profile facts without mounting another editor", () => {
  const { document, sent, panel, tuningController } = fixture();
  panel.applyStatus({ ...GROK_RUNNING_STATUS, configured_reasoning: "provider-experimental" });
  const profile = document.querySelector('[data-role="pm-configured-profile"]').textContent;
  assert.match(profile, /team\/grok-code-fast/); assert.match(profile, /provider-experimental/);
  assert.equal(tuningController.model.read().targets["pm:pm-member"].draft.reasoning, "provider-experimental");
  assert.equal(Boolean(document.querySelector('[data-member-tuning-field]')), false); assert.equal(sent.length, 0);
});

test("FR-121: Running as は稼働中の agent/model/effort を configured 値と分けて表示する", () => {
  const { document, panel } = fixture();
  panel.applyStatus({
    ...GROK_RUNNING_STATUS,
    configured_model: "team/grok-code-quality",
    configured_reasoning: "max",
    running_model: "team/grok-code-fast",
    running_reasoning: "medium",
  });

  const running = document.querySelector('[data-role="pm-running-as"]');
  assert.match(running.textContent, /Grok Build/, "running agent を表示する");
  assert.match(
    running.textContent,
    /team\/grok-code-fast/,
    "configured model ではなく running model を表示する",
  );
  assert.match(running.textContent, /medium/i, "running effort を表示する");

  assert.match(document.querySelector('[data-role="pm-configured-profile"]').textContent, /team\/grok-code-quality.*max/);

});

test("FR-026: Auto start トグルは set_pm_auto_start を送る", () => {
  const { document, sent, panel } = fixture();
  panel.applyStatus(RUNNING_STATUS);

  const toggle = document.querySelector('[data-role="pm-auto-start"]');
  assert.ok(toggle, "Auto start トグルが必要");
  assert.equal(toggle.checked, true, "backend の値を反映する");

  toggle.checked = false;
  toggle.dispatchEvent(changeEvent(document));
  assert.deepEqual(sent.at(-1), { kind: "set_pm_auto_start", enabled: false });

  toggle.checked = true;
  toggle.dispatchEvent(changeEvent(document));
  assert.deepEqual(sent.at(-1), { kind: "set_pm_auto_start", enabled: true });
});

test("#3812 AC-5: Pause/Resume is unavailable before project hydration", () => {
  const { views, sent } = fixture();
  const toggle = required(views[0], '[data-role="pm-pause-toggle"]');
  assert.ok(toggle.classList.contains("wizard-button"));
  assert.equal(toggle.disabled, true);
  toggle.click();
  assert.deepEqual(sent, []);
});

test("#3812 AC-5: Pause/Resume shares backend state without changing PM residency or Monitor", () => {
  const { document, views, sent, panel } = fixture({ mounts: 2 });
  panel.applyStatus({ ...RUNNING_STATUS, is_running: false, paused: false });
  const toggle = required(views[0], '[data-role="pm-pause-toggle"]');
  assert.equal(toggle.disabled, false, "pause is independent of PM registration");
  assert.equal(toggle.textContent, "Pause");
  toggle.click();
  assert.deepEqual(sent, [{ kind: "set_pm_paused", paused: true }]);

  panel.applyStatus({ ...RUNNING_STATUS, paused: true });
  const lateView = document.createElement("section");
  document.body.appendChild(lateView);
  panel.mount(lateView);
  views.push(lateView);
  for (const view of views) {
    const pausedToggle = required(view, '[data-role="pm-pause-toggle"]');
    assert.equal(pausedToggle.textContent, "Resume");
    assert.equal(pausedToggle.getAttribute("aria-pressed"), "true");
    const status = required(view, '[data-role="pm-autonomy-status"]');
    assert.ok(status.classList.contains("settings-status"));
    assert.equal(status.textContent, "Autonomous loop paused");
    assert.equal(status.getAttribute("role"), "status");
    assert.equal(status.getAttribute("aria-live"), "polite");
    assert.match(required(view, '[data-role="pm-running-as"]').textContent, /Running as:/);
  }

  required(lateView, '[data-role="pm-pause-toggle"]').click();
  assert.deepEqual(sent, [
    { kind: "set_pm_paused", paused: true },
    { kind: "set_pm_paused", paused: false },
  ]);
  panel.applyStatus({ ...RUNNING_STATUS, paused: false });
  for (const view of views) {
    assert.equal(required(view, '[data-role="pm-pause-toggle"]').textContent, "Pause");
    assert.equal(required(view, '[data-role="pm-pause-toggle"]').getAttribute("aria-pressed"), "false");
    assert.equal(required(view, '[data-role="pm-autonomy-status"]').textContent, "Autonomous loop active");
  }
});

test("FR-132: missing loop interval displays the effective 60 second default", () => {
  const { views, panel } = fixture();
  const statusWithoutInterval = { ...RUNNING_STATUS };
  delete statusWithoutInterval.loop_interval_secs;
  panel.applyStatus(statusWithoutInterval);

  const input = views[0].querySelector('[data-role="pm-loop-interval"]');
  assert.ok(input, "loop interval input is required");
  assert.equal(input.type, "number");
  assert.equal(input.min, "10");
  assert.equal(input.value, "60");
});

test("FR-132: unavailable project clears and disables every PM editor mount", () => {
  const { document, sent, views, panel } = fixture({ mounts: 2 });
  panel.applyStatus(RUNNING_STATUS);

  panel.applyStatus({ available: false });

  for (const view of views) {
    assert.match(
      required(view, '[data-role="pm-running-as"]').textContent,
      /unavailable/i,
    );
    assert.match(required(view, '[data-role="pm-configured-profile"]').textContent, /unavailable/i);
    assert.equal(required(view, '[data-role="pm-loop-interval"]').value, "60");
    for (const control of view.querySelectorAll("input, select, button")) {
      assert.equal(control.disabled, true);
    }
  }

  required(views[0], '[data-role="pm-auto-start"]').dispatchEvent(changeEvent(document));
  assert.deepEqual(sent, []);
});

test("FR-132: interval 9 emits no event and explains the 10 second minimum", () => {
  const { document, sent, views, panel } = fixture();
  panel.applyStatus(RUNNING_STATUS);
  const input = required(views[0], '[data-role="pm-loop-interval"]');

  input.value = "9";
  input.dispatchEvent(changeEvent(document));
  assert.deepEqual(sent, []);
  assertIntervalError(views[0], /at least 10 seconds/i);
  const error = required(views[0], '[data-role="pm-loop-interval-error"]');
  assert.equal(input.getAttribute("aria-invalid"), "true");
  assert.equal(error.getAttribute("role"), "alert");
  assert.equal(error.getAttribute("aria-live"), "polite");
  assert.equal(input.getAttribute("aria-describedby"), error.id);
});

test("FR-132: fractional interval emits no event and requests integer seconds", () => {
  const { document, sent, views, panel } = fixture();
  panel.applyStatus(RUNNING_STATUS);
  const input = required(views[0], '[data-role="pm-loop-interval"]');

  input.value = "10.5";
  input.dispatchEvent(changeEvent(document));
  assert.deepEqual(sent, []);
  assertIntervalError(
    views[0],
    /(?:whole|integer)[^.]*seconds|seconds[^.]*(?:whole|integer)/i,
  );
});

test("FR-132: boundary loop interval 10 emits set_pm_loop_interval", () => {
  const { document, sent, views, panel } = fixture();
  panel.applyStatus(RUNNING_STATUS);
  const input = required(views[0], '[data-role="pm-loop-interval"]');
  input.value = "10";
  input.dispatchEvent(changeEvent(document));

  assert.deepEqual(sent, [
    { kind: "set_pm_loop_interval", loop_interval_secs: 10 },
  ]);
  assert.equal(input.getAttribute("aria-invalid"), "false");
});

test("FR-132: u64 status and edits stay exact beyond JavaScript's safe integer range", () => {
  const { document, sent, views, panel } = fixture();
  panel.applyStatus({
    ...RUNNING_STATUS,
    loop_interval_secs: 18446744073709552000,
    loop_interval_secs_decimal: "18446744073709551615",
  });
  const input = required(views[0], '[data-role="pm-loop-interval"]');
  assert.equal(input.value, "18446744073709551615");

  input.dispatchEvent(changeEvent(document));
  assert.deepEqual(sent, [
    {
      kind: "set_pm_loop_interval",
      loop_interval_secs: "18446744073709551615",
    },
  ]);
});

test("FR-026: Restart は confirm を通ったときだけ restart_pm_agent を送る", () => {
  const declined = fixture({ confirmAnswer: false });
  declined.panel.applyStatus(RUNNING_STATUS);
  required(declined.document, '[data-role="pm-restart"]').click();
  assert.equal(declined.confirmed.length, 1, "確認は必ず出す");
  assert.equal(
    declined.sent.some((event) => event.kind === "restart_pm_agent"),
    false,
    "キャンセルしたら再起動しない",
  );
  // 会話が引き継がれないことを明示する（Claude の履歴は Codex に載らない）。
  assert.match(declined.confirmed[0], /new conversation/i);

  const accepted = fixture({ confirmAnswer: true });
  accepted.panel.applyStatus(RUNNING_STATUS);
  required(accepted.document, '[data-role="pm-restart"]').click();
  assert.deepEqual(accepted.sent.at(-1), { kind: "restart_pm_agent" });
});

test("FR-026: Pending チップは configured != running のときだけ出る", () => {
  const { document, panel } = fixture();

  panel.applyStatus(RUNNING_STATUS);
  const chip = document.querySelector('[data-role="pm-pending-chip"]');
  assert.ok(chip, "Pending チップの器が必要");
  assert.equal(chip.hidden, true, "設定と稼働が一致していれば出さない");

  panel.applyStatus({ ...RUNNING_STATUS, configured_agent_id: "codex" });
  assert.equal(chip.hidden, false, "設定を変えたら再起動待ちを出す");
  assert.match(chip.textContent, /restart/i);

  // 停止中は「反映待ち」ではなく単に止まっているだけなので出さない。
  panel.applyStatus({
    ...RUNNING_STATUS,
    configured_agent_id: "codex",
    is_running: false,
    running_agent_id: null,
  });
  assert.equal(chip.hidden, true, "停止中に pending は意味を持たない");
});

test("FR-132: theme-scoped shared Settings primitives consume tokens without raw colors", () => {
  for (const className of [
    "settings-section",
    "settings-label",
    "settings-select",
    "settings-input",
    "settings-checkbox",
    "settings-help",
    "settings-status",
  ]) {
    const body = sharedPrimitiveRule(className);
    assert.match(body, /var\(--/, `.${className} must consume an Operator token`);
    assert.doesNotMatch(
      body,
      /#[0-9a-fA-F]{3,8}\b|\brgba?\(/,
      `.${className} must not introduce a raw color`,
    );
  }
});

test("FR-121: Pending チップは agent が同じでも model 差分を検出する", () => {
  const { document, panel } = fixture();
  panel.applyStatus({
    ...GROK_RUNNING_STATUS,
    configured_model: "team/grok-code-quality",
    running_model: "team/grok-code-fast",
  });

  const chip = document.querySelector('[data-role="pm-pending-chip"]');
  assert.equal(chip.hidden, false, "model だけの変更も再起動待ちになる");
});

test("FR-121: Pending チップは agent/model が同じでも effort 差分を検出する", () => {
  const { document, panel } = fixture();
  panel.applyStatus({
    ...GROK_RUNNING_STATUS,
    configured_reasoning: "max",
    running_reasoning: "high",
  });

  const chip = document.querySelector('[data-role="pm-pending-chip"]');
  assert.equal(chip.hidden, false, "effort だけの変更も再起動待ちになる");
});

test("FR-026: PM 設定の CSS は Operator トークンのみを使う", () => {
  const start = componentsCss.indexOf(".pm-launcher-shell");
  assert.ok(start >= 0, "PM 設定の CSS が存在すること");
  const block = componentsCss.slice(start);
  assert.doesNotMatch(
    block,
    /#[0-9a-fA-F]{3,8}\b|\brgba?\(/,
    "PM 設定の CSS は Operator トークンのみを使う",
  );
  // 歯車は常時表示すると rail のノイズになるので hover/focus で現れる。
  assert.match(block, /\.pm-launcher-shell:hover \.pm-launcher-gear/);
  assert.match(block, /\.pm-launcher-shell:focus-within \.pm-launcher-gear/);
});

test("FR-132: app owns one controller and applies pm_status to it", () => {
  assert.match(appJs, /import \{ createPmSettingsPanel \} from "\/pm-settings-panel\.js"/);
  assert.equal(
    appJs.match(/\bcreatePmSettingsPanel\s*\(/g)?.length ?? 0,
    1,
    "app must instantiate exactly one shared PM settings controller",
  );
  assert.match(appJs, /pmSettingsPanel\.bindEntryPoints\(\{\s*document\s*\}\)/);
  assert.match(
    appJs,
    /case "pm_status":[\s\S]{0,200}applyPmSettingsReceiveEvent\(/,
    "pm_status が受信ディスパッチに繋がっていること",
  );
  assert.match(
    appJs,
    /__gwtPmSettingsTestApi\s*=\s*Object\.freeze\(\{[\s\S]{0,160}pmSettingsPanel\.mount\(container\)/,
    "Playwright bridge must mount the app-owned controller, not a test-only duplicate",
  );
});

test("FR-132: command palette exposes the pm-settings command", () => {
  assert.match(operatorShellJs, /id: "pm-settings"/);
  assert.match(appJs, /case "pm-settings":[\s\S]{0,320}return;/);
});


test("PM Settings routes its known member to Agents without sending a profile write", () => {
  const { document, sent, panel, openedMembers } = fixture(); panel.applyStatus(RUNNING_STATUS);
  document.querySelector('[data-role="pm-open-member"]').click();
  assert.deepEqual(openedMembers, ["pm-member"]); assert.equal(sent.length, 0);
});

test("foreign PM status routes to its project before editing and cannot send through the current socket", () => {
  const { document, sent, panel } = fixture(); panel.applyStatus({ ...RUNNING_STATUS, project_key: 'foreign' });
  assert.equal(document.querySelector('[data-role="pm-open-member"]').disabled, true);
  document.querySelector('[data-role="pm-auto-start"]').dispatchEvent(changeEvent(document));
  document.querySelector('[data-role="pm-pause-toggle"]').click();
  assert.equal(sent.length, 0);
});

test("unknown PM status disables shared profile authority, preserves the draft and offers explicit reload", () => {
  const { document, sent, panel, tuningController: c, reloaded } = fixture(); panel.applyStatus(RUNNING_STATUS);
  c.editTuning("pm", "pm-member", { model: "unfinished" });
  panel.applyStatus({ ...RUNNING_STATUS, configured_member_id: null, profile_revision: null, configured_agent_id: null });
  const target = c.model.read().targets["pm:pm-member"];
  assert.equal(target.draft.model, "unfinished"); assert.equal(target.available, false);
  assert.match(document.querySelector('[data-role="pm-configured-profile"]').textContent, /unavailable/i);
  document.querySelector('[data-role="pm-reload-profile"]').click(); assert.deepEqual(reloaded, [true]);
  c.editTuning("pm", "pm-member", { model: "unsafe" }); assert.equal(sent.length, 1);
});

test("PM pending change includes member identity even when tool/model/reasoning match", () => {
  const { document, panel } = fixture(); panel.applyStatus({ ...RUNNING_STATUS, running_member_id: 'previous-pm' });
  assert.equal(document.querySelector('[data-role="pm-pending-chip"]').hidden, false);
});

test("PM Settings keeps operational controls and routes profile editing to Agents", () => {
  const { document, panel } = fixture(); panel.applyStatus(RUNNING_STATUS);
  assert.equal(Boolean(document.querySelector('[data-member-tuning-field]')), false);
  assert.ok(document.querySelector('[data-role="pm-open-member"]'));
  for (const role of ['pm-running-as', 'pm-pending-chip', 'pm-auto-start', 'pm-pause-toggle', 'pm-loop-interval', 'pm-restart']) assert.ok(document.querySelector(`[data-role="${role}"]`));
});
