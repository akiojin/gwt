/* SPEC-3431 FR-018〜FR-022 — Project Manager surfaces.
 *
 * The PM is the user's single conversational window, so the frontend owes it
 * three things the rest of the canvas does not get: a permanent launcher, a
 * one-click path that always lands on it, and chrome that tells it apart from
 * ordinary agent panes. These are structure/contract assertions over the real
 * index.html + components.css, in the style of operator-chrome-structure.
 */

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";
import { parseHTML } from "linkedom";

const here = dirname(fileURLToPath(import.meta.url));
const html = readFileSync(resolve(here, "../index.html"), "utf8");
const componentsCss = readFileSync(resolve(here, "../styles/components.css"), "utf8");
const tokensCss = readFileSync(resolve(here, "../styles/tokens.css"), "utf8");
const appJs = readFileSync(resolve(here, "../app.js"), "utf8");

function doc() {
  return parseHTML(html).document;
}

test("FR-018: PM launcher は rail の Navigate グループ先頭にある", () => {
  const document = doc();
  const entry = document.getElementById("op-pm-entry");
  assert.ok(entry, "rail に PM ランチャーが必要");
  assert.ok(
    entry.classList.contains("op-rail__item"),
    "既存 rail の文法に乗ること",
  );

  const navigateGroup = entry.closest(".op-rail__group");
  assert.equal(navigateGroup?.getAttribute("aria-label"), "Navigate");
  const items = [...navigateGroup.querySelectorAll(".op-rail__item")];
  assert.equal(items[0]?.id, "op-pm-entry", "PM が Navigate の先頭に来ること");
});

test("FR-018: PM launcher はアクセシブルな名前を持つ", () => {
  const document = doc();
  const entry = document.getElementById("op-pm-entry");
  assert.equal(entry.getAttribute("aria-label"), "Project Manager");
  // アイコンは装飾なので支援技術から隠す。
  assert.equal(
    entry.querySelector(".op-rail__icon")?.getAttribute("aria-hidden"),
    "true",
  );

  const floating = document.getElementById("canvas-pm-launcher");
  assert.ok(floating, "キャンバス側のランチャーが必要");
  assert.equal(floating.getAttribute("aria-label"), "Open Project Manager");
});

test("FR-018: キャンバスの PM ランチャーは既定で非表示", () => {
  const document = doc();
  const floating = document.getElementById("canvas-pm-launcher");
  // PM が見えているときに重複 CTA を出さないため、初期状態は hidden。
  assert.ok(floating.hasAttribute("hidden"));
});

test("FR-018: フローティング PM は minimap と同じ角を奪わない", () => {
  // fleet-minimap は右下・z-index 60 を占有している。PM を同じ角に置くと
  // 重なるので、左下に固定されていることを構造で固定する。
  const minimap = /\.fleet-minimap\s*\{[^}]*\}/.exec(componentsCss)?.[0] ?? "";
  assert.match(minimap, /right:/, "前提: minimap は右下にある");

  const launcher = /\.canvas-pm-launcher\s*\{[^}]*\}/.exec(componentsCss)?.[0] ?? "";
  assert.match(launcher, /left:/, "PM ランチャーは左端に置く");
  assert.doesNotMatch(
    launcher,
    /\bright:/,
    "右下は minimap のものなので PM は使わない",
  );
});

test("FR-020: PM の role トークンは両テーマに定義され raw color を使わない", () => {
  const themes = tokensCss.match(/--color-role-pm:\s*[^;]+;/g) ?? [];
  // dark / light / forced-colors の 3 ブロック。
  assert.equal(themes.length, 3, `--color-role-pm は 3 テーマ必要: ${themes}`);

  // PM の面は必ずトークン経由で塗る（生 hex/rgb の直書き禁止）。
  const pmRules = componentsCss
    .split("\n")
    .filter((line) => /canvas-pm-launcher|op-rail__pm-dot|data-pm="true"|op-rail__item--pm/.test(line));
  assert.ok(pmRules.length > 0, "PM の CSS が存在すること");
  const pmBlockStart = componentsCss.indexOf(".op-rail__item--pm");
  const pmBlock = componentsCss.slice(pmBlockStart);
  assert.doesNotMatch(
    pmBlock,
    /#[0-9a-fA-F]{3,8}\b|\brgba?\(/,
    "PM の CSS は Operator トークンのみを使う",
  );
});

test("FR-020: PM ウィンドウは『何であるか』を文字で名乗る", () => {
  // 実機レビュー（2026-08-05）で判明した欠陥の回帰固定。
  // 初版は既存 role badge に色を付けただけで、表示文字列は通常の
  // エージェント窓と同じ "Claude Code" のままだった。色は補助であって
  // 識別子ではない — タイトルと badge が PM だと名乗る必要がある。
  assert.match(
    appJs,
    /function windowDisplayTitle\(windowData\)\s*\{[\s\S]*?windowData\?\.is_pm/,
    "PM 窓のタイトルは PM だと分かる文字列にする",
  );
  assert.match(appJs, /PM_WINDOW_TITLE\s*=\s*"Project Manager"/);
  // role badge は agent 名ではなく PM を出す。
  assert.match(
    appJs,
    /function windowRoleBadgeLabel\(windowData\)\s*\{[\s\S]*?windowData\?\.is_pm[\s\S]*?PM_ROLE_BADGE/,
  );
  assert.match(appJs, /PM_ROLE_BADGE\s*=\s*"PM"/);
});

test("FR-020: PM ウィンドウは lane badge を出さない", () => {
  // PM は Execution/Intake という実行レーンの住人ではないので、lane badge は
  // 意味を持たないノイズになる（実機で "Execution" が出て PM 表示と競合した）。
  const laneIdentity = readFileSync(resolve(here, "../window-lane-identity.js"), "utf8");
  assert.match(
    laneIdentity,
    /shouldShowWindowLaneBadge[\s\S]*?is_pm/,
    "PM 窓では lane badge を抑止する",
  );
});

test("FR-020: PM ウィンドウは左アクセント帯と塗り badge で識別する", () => {
  assert.match(
    componentsCss,
    /\.workspace-window\[data-pm="true"\]\s*\{[^}]*border-left:[^}]*var\(--color-role-pm\)/,
    "左アクセント帯が必要",
  );
  assert.match(
    componentsCss,
    /\.workspace-window\[data-pm="true"\]\s+\.window-role-badge\s*\{[^}]*background:\s*var\(--color-role-pm\)/,
    "role badge は塗りつぶしにする",
  );
  // 色だけに頼らない（形でも分かる）ことを固定。
  assert.match(componentsCss, /\.workspace-window\[data-pm="true"\][^{]*\{[^}]*border-left/);
});

test("FR-021: rail の PM 状態は running / stopped / absent を持つ", () => {
  const document = doc();
  assert.equal(
    document.getElementById("op-pm-entry").dataset.pmState,
    "absent",
    "PM 未起動が初期状態",
  );
  for (const state of ["running", "stopped"]) {
    assert.ok(
      componentsCss.includes(`[data-pm-state="${state}"]`),
      `${state} のスタイルが必要`,
    );
  }
  // light テーマで surface 差だけに頼らないため、状態はドットで表す。
  assert.match(
    componentsCss,
    /\[data-pm-state="running"\]\s+\.op-rail__pm-dot\s*\{[^}]*background:\s*var\(--color-role-pm\)/,
  );
});

test("FR-019: PM クリックは既存なら中央フレーミング、無ければ起動要求", () => {
  // 既存ウィンドウは camera-focus の center 経路を再利用する。
  assert.match(
    appJs,
    /function openPmAgent\(\)\s*\{[\s\S]*focusWindowRemotely\(pmWindowId,\s*\{\s*center:\s*true\s*\}\)/,
  );
  // 無ければバックエンドへ起動要求（bounds 付きで中央に開かせる）。
  assert.match(
    appJs,
    /function openPmAgent\(\)[\s\S]*send\(\{\s*kind:\s*"open_pm_agent",\s*bounds:\s*visibleBounds\(\)\s*\}\)/,
  );
});

test("FR-022: 閉じる確認は PM の停止が自動復帰しないことを明示する", () => {
  const modal = readFileSync(resolve(here, "../window-close-confirm-modal.js"), "utf8");
  assert.match(modal, /state\.isPm/, "PM かどうかで文言を分岐する");
  assert.match(
    modal,
    /will not restart on its own/,
    "クラッシュ時の自動復帰と区別できる文言にする",
  );
  // 実際に PM フラグが確認ダイアログへ渡っていること。
  assert.match(appJs, /isPm:\s*Boolean\(windowData\.is_pm\)/);
});
