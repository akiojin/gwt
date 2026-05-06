// SPEC-2041 Phase 13 — GUI 自動更新ポーリング刷新 (FR-035/036/037)
//
// 5min ポーリング中、Available 検出時に「初回はトースト + ボタン、以降は同一
// latest をスキップしてボタン永続」という UI 状態遷移をフロントエンドが満たす
// ことをソースレベルで検証する。kanban-structure.test.mjs と同じ手法で
// renderer / handler を直接呼ばずに app.js / index.html の宣言を確認する。

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
const indexHtml = readFileSync(resolve(here, "../index.html"), "utf8");

test("app.js tracks the first-seen latest version to dedup poll detections", () => {
  // FR-036: 5min ポーリングで同一 latest を繰り返し受信してもトースト/ボタンを
  // 二重表示しないよう、最後に表示した latest を保持する必要がある。
  assert.match(
    appSource,
    /firstSeenUpdateVersion|firstSeenVersion|lastSeenUpdateLatest/,
    "app.js must track the last-seen `latest` to avoid duplicate UI updates",
  );
});

test("app.js defines a showUpdateButton renderer", () => {
  // FR-036: 初回トースト 8s が消えても永続的に残るボタンを描画する関数が必要。
  assert.match(appSource, /showUpdateButton\s*\(/, "missing showUpdateButton");
});

test("app.js update-button click handler runs window.confirm then apply_update", () => {
  // FR-005 を踏襲しつつ FR-036 の永続ボタン経路にも適用する。
  // ボタン作成箇所からほどなくして confirm → apply_update を発火する。
  const buttonBlock = appSource.match(
    /update-button[\s\S]{0,1200}?apply_update/,
  );
  assert.ok(
    buttonBlock,
    "expected update-button click path to reach apply_update",
  );
  assert.match(
    buttonBlock[0],
    /window\.confirm|confirm\s*\(/,
    "update-button click must guard apply_update with window.confirm",
  );
});

test("app.js update toast click handler uses the same apply_update path", () => {
  // トーストと永続ボタンで backend の適用入口を分岐させない。
  const toastBlock = appSource.match(
    /function showUpdateToast[\s\S]{0,1600}?apply_update/,
  );
  assert.ok(toastBlock, "expected update toast click path to reach apply_update");
  assert.match(
    toastBlock[0],
    /window\.confirm|confirm\s*\(/,
    "update toast click must guard apply_update with window.confirm",
  );
});

test("update_state handler delegates to showUpdateButton for persistence", () => {
  // case "update_state" が新しいボタン renderer を呼ぶことで FR-036 の永続表示
  // が成立する。トースト呼び出しは初回のみ条件分岐される必要がある。
  const handlerSlice = appSource.match(
    /case "update_state"[\s\S]{0,2500}/,
  );
  assert.ok(handlerSlice, "expected case \"update_state\" handler in app.js");
  assert.match(
    handlerSlice[0],
    /showUpdateButton/,
    "update_state handler must call showUpdateButton",
  );
});

test("app.js surfaces backend update apply failures", () => {
  const handlerSlice = appSource.match(
    /case "update_apply_error"[\s\S]{0,500}/,
  );
  assert.ok(handlerSlice, "expected update_apply_error handler in app.js");
  assert.match(
    handlerSlice[0],
    /alert\s*\(/,
    "update apply failures must be visible to the user",
  );
});

test("index.html declares a fixed bottom-right .update-button style", () => {
  // FR-036: 右下フローティングで永続表示。
  const styleMatch = indexHtml.match(/\.update-button\s*\{[^}]+\}/);
  assert.ok(styleMatch, "expected .update-button rule inside <style>");
  assert.match(
    styleMatch[0],
    /position:\s*fixed/,
    ".update-button must be position: fixed",
  );
  assert.match(
    styleMatch[0],
    /bottom:\s*\d+px/,
    ".update-button must anchor to bottom",
  );
  assert.match(
    styleMatch[0],
    /right:\s*\d+px/,
    ".update-button must anchor to right",
  );
});

test("index.html offsets .update-button while the toast is visible", () => {
  // 初回トースト 8s 表示中は .has-toast クラスでボタンを上方向にずらし、
  // 重なりを避けてからトースト消滅後に通常位置へ戻す。
  assert.match(
    indexHtml,
    /\.update-button\.has-toast\s*\{[^}]*bottom:\s*\d+px/,
    "expected .update-button.has-toast rule offsetting `bottom`",
  );
});
