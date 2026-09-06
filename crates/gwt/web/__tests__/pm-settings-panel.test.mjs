/* SPEC-3431 FR-026 — PM settings panel.
 *
 * The PM is the one agent the user never launches through the wizard, so the
 * only place it can be configured is next to its launcher. The panel owes the
 * user four things: what the PM is running as right now, what it will run as
 * next, an auto-start opt-out, and a restart that is honest about starting a
 * NEW conversation (a Claude history cannot be carried into Codex).
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
  auto_start: true,
  configured_agent_id: "claude",
  configured_model: null,
  configured_reasoning: null,
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

function fixture({ confirmAnswer = true } = {}) {
  const { document } = parseHTML(html);
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
  panel.mount();
  return { document, sent, confirmed, panel };
}

function changeEvent(document) {
  return new document.defaultView.Event("change");
}

/** Select `value` the way the DOM stores a selection (option-level), then fire
 *  the change the user's click would. */
function chooseAgent(document, select, value) {
  const option = select.querySelector(`option[value="${value}"]`);
  assert.ok(option, `option ${value} must exist`);
  option.selected = true;
  select.dispatchEvent(changeEvent(document));
}

test("FR-026: パネルは既定で非表示、かつ PM ランチャーに隣接する", () => {
  const { document } = fixture();
  const panelEl = document.getElementById("pm-settings-panel");
  assert.ok(panelEl, "PM 設定パネルの器が必要");
  assert.ok(panelEl.hasAttribute("hidden"), "既定は非表示");

  // アンカー: PM rail item と同じシェルに属していること。別の場所に浮かぶと
  // 「どの PM の設定か」が読めなくなる。
  const shell = panelEl.closest(".pm-launcher-shell");
  assert.ok(shell, "パネルは .pm-launcher-shell の中に置く");
  assert.equal(
    shell.querySelector("#op-pm-entry")?.id,
    "op-pm-entry",
    "同じシェルに PM ランチャーが居ること",
  );
});

test("FR-026: 歯車は PM ボタンの外側にあり、PM クリックを奪わない", () => {
  const { document } = fixture();
  const gear = document.getElementById("op-pm-settings-button");
  assert.ok(gear, "設定を開く歯車が必要");
  // 歯車が PM ボタンの子だと、歯車クリックが PM の click に必ず bubble して
  // 設定を開くたびに PM が起動してしまう。構造で排除する。
  assert.equal(
    document.getElementById("op-pm-entry").contains(gear),
    false,
    "歯車は PM ボタンの内側に置かない",
  );
  // 歯車は rail の item 語彙に入らない（Navigate 先頭は PM のまま）。
  assert.equal(gear.classList.contains("op-rail__item"), false);

  // PM ボタン自身の click は open_pm_agent のまま（回帰固定）。
  assert.match(
    appJs,
    /for \(const id of \["op-pm-entry", "canvas-pm-launcher"\]\)/,
    "PM ランチャーの click 配線は維持する",
  );
  // bounds は付けない。バックエンドの center 計算は viewport-sync に
  // 捨てられるため、フレーミングはローカルの pendingPmFrame が担う。
  assert.match(appJs, /send\(\{\s*kind:\s*"open_pm_agent"\s*\}\)/);
});

test("FR-026: 歯車でパネルが開閉する", () => {
  const { document, panel } = fixture();
  const gear = document.getElementById("op-pm-settings-button");
  const panelEl = document.getElementById("pm-settings-panel");

  assert.equal(gear.getAttribute("aria-expanded"), "false");
  gear.click();
  assert.equal(panelEl.hasAttribute("hidden"), false, "歯車でパネルが開く");
  assert.equal(gear.getAttribute("aria-expanded"), "true");
  assert.equal(panel.isOpen(), true);

  gear.click();
  assert.equal(panelEl.hasAttribute("hidden"), true, "もう一度押すと閉じる");
  assert.equal(gear.getAttribute("aria-expanded"), "false");
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
    effort.closest("label")?.querySelector(".pm-settings-panel__label")?.textContent,
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

test("FR-026: Restart は confirm を通ったときだけ restart_pm_agent を送る", () => {
  const declined = fixture({ confirmAnswer: false });
  declined.panel.applyStatus(RUNNING_STATUS);
  declined.document.querySelector('[data-role="pm-restart"]').click();
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
  accepted.document.querySelector('[data-role="pm-restart"]').click();
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

test("FR-026: パネルは app.js に mount され pm_status を受け取る", () => {
  // 実装されていても配線されていなければ機能は死んでいる。
  assert.match(appJs, /import \{ createPmSettingsPanel \} from "\/pm-settings-panel\.js"/);
  assert.match(appJs, /createPmSettingsPanel\(\{[\s\S]*?\}\)/);
  assert.match(appJs, /pmSettingsPanel\.mount\(\)/);
  assert.match(
    appJs,
    /case "pm_status":[\s\S]{0,200}pmSettingsPanel\.applyStatus\(/,
    "pm_status が受信ディスパッチに繋がっていること",
  );
});

test("FR-026: pm-settings はコマンドパレットから開ける", () => {
  assert.match(operatorShellJs, /id: "pm-settings"/);
  assert.match(
    appJs,
    /case "pm-settings":[\s\S]{0,200}pmSettingsPanel\.open\(\)/,
    "op:command が実際にパネルを開くこと",
  );
});
