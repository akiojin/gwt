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

import { createPmSettingsPanel } from "../pm-settings-panel.js";

const here = dirname(fileURLToPath(import.meta.url));
const html = readFileSync(resolve(here, "../index.html"), "utf8");
const componentsCss = readFileSync(resolve(here, "../styles/components.css"), "utf8");
const appJs = readFileSync(resolve(here, "../app.js"), "utf8");
const operatorShellJs = readFileSync(resolve(here, "../operator-shell.js"), "utf8");

const RUNNING_STATUS = {
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

const GROK_COMMON_EFFORTS = [
  "",
  "none",
  "minimal",
  "low",
  "medium",
  "high",
  "xhigh",
  "max",
];

function fixture({ confirmAnswer = true, mounts = 1 } = {}) {
  const { document } = parseHTML(html);
  document.getElementById("pm-settings-panel")?.remove();
  const sent = [];
  const confirmed = [];
  const panel = createPmSettingsPanel({
    document,
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
  return { document, sent, confirmed, panel, views };
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

/** Select `value` the way the DOM stores a selection (option-level), then fire
 *  the change the user's click would. */
function chooseAgent(document, select, value) {
  const option = select.querySelector(`option[value="${value}"]`);
  assert.ok(option, `option ${value} must exist`);
  option.selected = true;
  select.dispatchEvent(changeEvent(document));
}

test("FR-132: Settings mount は英語の PM controls と shared primitives を使う", () => {
  const { views } = fixture();
  const view = views[0];

  assert.ok(view.querySelector(".settings-section"));
  const agent = required(view, '[data-role="pm-agent-select"]');
  const model = required(view, '[data-role="pm-model-input"]');
  const effort = required(view, '[data-role="pm-effort-select"]');
  const interval = required(view, '[data-role="pm-loop-interval"]');
  const autoStart = required(view, '[data-role="pm-auto-start"]');
  assert.ok(agent.classList.contains("settings-select"));
  assert.ok(model.classList.contains("settings-input"));
  assert.ok(effort.classList.contains("settings-select"));
  assert.ok(interval.classList.contains("settings-input"));
  assert.ok(autoStart.classList.contains("settings-checkbox"));
  assert.match(accessibleName(view, agent), /Agent/i);
  assert.match(accessibleName(view, model), /Model/i);
  assert.match(accessibleName(view, effort), /Effort/i);
  assert.match(accessibleName(view, interval), /Loop interval/i);
  assert.match(accessibleName(view, autoStart), /Auto start/i);
  assert.doesNotMatch(
    view.textContent,
    /[\u3040-\u30ff\u3400-\u9fff]/,
    "PM Settings copy must stay English",
  );
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
    /for \(const id of \["op-pm-entry", "canvas-pm-launcher"\]\)/,
    "PM ランチャーの click 配線は維持する",
  );
  assert.match(appJs, /send\(\{\s*kind:\s*"open_pm_agent"\s*\}\)/);
});

test("FR-132: shared controller routes gear and pm-settings command to Settings", () => {
  const { document, panel, sent } = fixture({ mounts: 0 });
  assert.equal(typeof panel.bindEntryPoints, "function");
  panel.bindEntryPoints({ document });

  const openedTargets = [];
  document.addEventListener("settings:open", (event) => {
    openedTargets.push(event.detail?.target);
  });

  required(document, "#op-pm-settings-button").click();
  assert.deepEqual(openedTargets, ["project-manager"]);
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

test("FR-132: late and existing mounts share the controller's current snapshot", () => {
  const { document, panel, views } = fixture();
  panel.applyStatus({
    ...RUNNING_STATUS,
    auto_start: false,
    configured_agent_id: "codex",
    configured_model: "gpt-5.1-codex-max",
    loop_interval_secs: 42,
  });

  const lateView = document.createElement("section");
  lateView.className = "settings-panel";
  lateView.dataset.settingsPanel = "project-manager";
  document.body.appendChild(lateView);
  panel.mount(lateView);
  views.push(lateView);

  for (const view of views) {
    assert.equal(required(view, '[data-role="pm-auto-start"]').checked, false);
    assert.equal(required(view, '[data-role="pm-agent-select"]').value, "codex");
    assert.equal(
      required(view, '[data-role="pm-model-input"]').value,
      "gpt-5.1-codex-max",
    );
    assert.equal(required(view, '[data-role="pm-loop-interval"]').value, "42");
  }

  panel.applyStatus({ ...RUNNING_STATUS, loop_interval_secs: 84 });
  for (const view of views) {
    assert.equal(required(view, '[data-role="pm-loop-interval"]').value, "84");
  }
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
    assert.equal(required(view, '[data-role="pm-agent-select"]').value, "codex");
    assert.equal(
      required(view, '[data-role="pm-model-input"]').value,
      "gpt-5.1-codex-max",
    );
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

test("FR-026: agent select は pm_status の選択肢から作られ set_pm_launch_profile を送る", () => {
  const { document, sent, panel } = fixture();
  panel.applyStatus(RUNNING_STATUS);

  const select = document.querySelector('[data-role="pm-agent-select"]');
  assert.ok(select, "agent select が必要");
  assert.deepEqual(
    [...select.querySelectorAll("option")].map((option) => option.value),
    ["claude", "codex", "grok"],
    "選択肢は backend の agent_options だけ",
  );
  assert.equal(select.value, "claude", "現在の設定値が選択されている");

  chooseAgent(document, select, "codex");

  assert.deepEqual(sent.at(-1), {
    kind: "set_pm_launch_profile",
    agent_id: "codex",
    model: null,
    reasoning: null,
  });
});

test("FR-120: Grok Build を Agent/Model/Effort の同一プロファイルとして送る", () => {
  const { document, sent, panel } = fixture();
  panel.applyStatus({
    ...GROK_RUNNING_STATUS,
    configured_agent_id: "codex",
    configured_model: "gpt-5.6-sol",
    configured_reasoning: "xhigh",
    running_agent_id: "codex",
    running_model: "gpt-5.6-sol",
    running_reasoning: "xhigh",
  });

  const select = document.querySelector('[data-role="pm-agent-select"]');
  assert.equal(
    select.querySelector('option[value="grok"]')?.textContent,
    "Grok Build",
    "backend が投影した Grok Build を canonical label で選べる",
  );

  chooseAgent(document, select, "grok");

  assert.deepEqual(sent.at(-1), {
    kind: "set_pm_launch_profile",
    agent_id: "grok",
    model: "gpt-5.6-sol",
    reasoning: "xhigh",
  });
});

test("FR-120: model の変更も reasoning を保った full profile event に載る", () => {
  const { document, sent, panel } = fixture();
  panel.applyStatus({
    ...RUNNING_STATUS,
    configured_agent_id: "codex",
    configured_reasoning: "high",
  });

  const model = document.querySelector('[data-role="pm-model-input"]');
  assert.ok(model, "model の入力が必要");
  model.value = "gpt-5.1-codex-max";
  model.dispatchEvent(changeEvent(document));

  assert.deepEqual(sent.at(-1), {
    kind: "set_pm_launch_profile",
    agent_id: "codex",
    model: "gpt-5.1-codex-max",
    reasoning: "high",
  });
});

test("FR-120: agent 固有 catalog と保存済み未列挙 effort を full profile event で保持する", () => {
  const { document, sent, panel } = fixture();
  panel.applyStatus({
    ...RUNNING_STATUS,
    configured_agent_id: "codex",
    configured_model: "gpt-5.6-sol",
    configured_reasoning: "provider-experimental",
  });

  const effort = document.querySelector('[data-role="pm-effort-select"]');
  const values = [...effort.querySelectorAll("option")].map((option) => option.value);
  assert.ok(values.includes("ultra"), "Codex catalog は ultra を含む");
  assert.ok(!values.includes("none"), "Grok 固有 none を Codex に提示しない");
  assert.ok(!values.includes("minimal"), "Grok 固有 minimal を Codex に提示しない");
  assert.equal(
    effort.value,
    "provider-experimental",
    "未列挙の保存値も current option として表現する",
  );

  const model = document.querySelector('[data-role="pm-model-input"]');
  model.value = "gpt-5.6-sol-next";
  model.dispatchEvent(changeEvent(document));

  assert.deepEqual(sent.at(-1), {
    kind: "set_pm_launch_profile",
    agent_id: "codex",
    model: "gpt-5.6-sol-next",
    reasoning: "provider-experimental",
  });

  panel.applyStatus({
    ...RUNNING_STATUS,
    configured_agent_id: "claude",
    configured_reasoning: "xhigh",
  });
  assert.equal(
    document.querySelector('[data-role="pm-effort-select"]').value,
    "xhigh",
    "Claude の union catalog でも xhigh を新規選択・保持できる",
  );
});

test("FR-120: Effort は Auto と公式 common values を持ち同じ full profile event を送る", () => {
  const { document, sent, panel } = fixture();
  panel.applyStatus(GROK_RUNNING_STATUS);

  const effort = document.querySelector('[data-role="pm-effort-select"]');
  assert.ok(effort, "PM 設定に Effort control が必要");
  assert.equal(
    effort.closest("label")?.querySelector(".settings-label")?.textContent,
    "Effort",
    "ユーザー向けラベルは Effort とする",
  );
  assert.deepEqual(
    [...effort.querySelectorAll("option")].map((option) => option.value),
    GROK_COMMON_EFFORTS,
    "Auto は空値、残りは Grok CLI の common effort values",
  );
  assert.match(
    effort.querySelector('option[value=""]')?.textContent || "",
    /Auto/i,
    "空値は provider default を保つ Auto と表示する",
  );
  assert.equal(effort.value, "high", "configured reasoning を Effort に反映する");

  const xhigh = effort.querySelector('option[value="xhigh"]');
  assert.ok(xhigh, "xhigh option が必要");
  xhigh.selected = true;
  effort.dispatchEvent(changeEvent(document));

  assert.deepEqual(sent.at(-1), {
    kind: "set_pm_launch_profile",
    agent_id: "grok",
    model: "team/grok-code-fast",
    reasoning: "xhigh",
  });
});

test("FR-120: Effort の Auto は reasoning null として full profile event を送る", () => {
  const { document, sent, panel } = fixture();
  panel.applyStatus(GROK_RUNNING_STATUS);

  const effort = document.querySelector('[data-role="pm-effort-select"]');
  assert.ok(effort, "PM 設定に Effort control が必要");
  const auto = effort.querySelector('option[value=""]');
  assert.ok(auto, "Auto option が必要");
  auto.selected = true;
  effort.dispatchEvent(changeEvent(document));

  assert.deepEqual(sent.at(-1), {
    kind: "set_pm_launch_profile",
    agent_id: "grok",
    model: "team/grok-code-fast",
    reasoning: null,
  });
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

  assert.equal(
    document.querySelector('[data-role="pm-model-input"]').value,
    "team/grok-code-quality",
    "編集欄は configured model を表示する",
  );
  assert.equal(
    document.querySelector('[data-role="pm-effort-select"]').value,
    "max",
    "編集欄は configured effort を表示する",
  );
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
    assert.equal(
      required(view, '[data-role="pm-agent-select"]').querySelectorAll("option").length,
      0,
    );
    assert.equal(required(view, '[data-role="pm-model-input"]').value, "");
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
    /case "pm_status":[\s\S]{0,200}pmSettingsPanel\.applyStatus\(/,
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
