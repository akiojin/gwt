// SPEC-2017 Kanban D&D — phase write-back through WebSocket
//
// HTML5 Drag and Drop wiring tests. The renderer + drop handler live in
// app.js, so we assert the source pattern (same approach as
// kanban-structure.test.mjs and operator-chrome-structure.test.mjs).
// Real DOM event simulation isn't useful here because dispatch happens
// through the WebSocket transport; we just need to verify the source
// has the right hook points and rollback discipline.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");

test("Kanban cards wire HTML5 dragstart so cards can be picked up", () => {
  // SPEC-2017 US-8: cards are .kanban-card buttons. The dragstart
  // listener marks them as the dragged card (DOM class) and stores
  // the issue number on dataTransfer so the drop handler can find
  // the entry. We assert addEventListener("dragstart", ...) hook.
  assert.match(
    appSource,
    /addEventListener\("dragstart"/,
    "expected app.js to wire dragstart for Kanban cards",
  );
  assert.match(
    appSource,
    /dataTransfer\.setData/,
    "expected dragstart to set dataTransfer payload",
  );
});

test("dragstart captures dndSnapshot for rollback on failure", () => {
  // SPEC-2017 US-8: on dragstart we snapshot the source phase so
  // a failed write-back can rollback the optimistic UI change. The
  // snapshot lives on state.dndSnapshot keyed by issue number plus
  // origin column. Without the snapshot rollback is impossible.
  assert.match(
    appSource,
    /state\.dndSnapshot\s*=/,
    "expected dragstart handler to set state.dndSnapshot",
  );
});

test("drop handler sends update_knowledge_bridge_phase WebSocket message", () => {
  // SPEC-2017 US-8: dropping on a target column triggers a
  // WebSocket request with kind="update_knowledge_bridge_phase"
  // carrying issue number, target_phase (or null for Backlog), and a
  // request id so the response can be matched back to the pending
  // update. The kind name mirrors the
  // FrontendEvent::UpdateKnowledgeBridgePhase serde rename.
  assert.match(
    appSource,
    /addEventListener\("drop"/,
    "expected app.js to wire drop on Kanban columns",
  );
  assert.match(
    appSource,
    /update_knowledge_bridge_phase/,
    "expected drop to dispatch update_knowledge_bridge_phase WebSocket message",
  );
});

test("Backlog drop sends target_phase=null", () => {
  // SPEC-2017 US-8: dropping a card on the Backlog column means
  // "remove every phase/* label". The protocol uses target_phase=null
  // to express that, so the handler can distinguish "set to draft"
  // from "remove all phase labels". Without this distinction Backlog
  // becomes unreachable through D&D.
  assert.match(
    appSource,
    /target_phase[^,;]*null|targetPhase[^,;]*null/,
    "expected Backlog drop to use target_phase=null",
  );
});

test("knowledge_bridge_phase_updated Ok response clears pendingPhaseUpdates", () => {
  // SPEC-2017 US-8: the optimistic UI keeps the moved card in
  // pendingPhaseUpdates until the server confirms. On success the
  // pending entry is removed and fresh_entry overwrites the card so
  // labels / counts reflect the GitHub source of truth. The response
  // event kind matches the BackendEvent::KnowledgeBridgePhaseUpdated
  // serde rename.
  assert.match(
    appSource,
    /knowledge_bridge_phase_updated/,
    "expected app.js to handle knowledge_bridge_phase_updated response",
  );
  assert.match(
    appSource,
    /pendingPhaseUpdates[\s\S]{0,300}?\.delete\(/,
    "expected success path to clear pendingPhaseUpdates entry",
  );
});

test("knowledge_phase_updated Error response triggers rollback from dndSnapshot", () => {
  // SPEC-2017 US-8: GitHub label write-back can fail (network,
  // permission, rate limit). The error path must restore the card
  // to its origin column using dndSnapshot — otherwise the user
  // sees the optimistic move stick around as a phantom write.
  assert.match(
    appSource,
    /dndSnapshot[\s\S]{0,300}?(rollback|restore|origin)|rollback[\s\S]{0,300}?dndSnapshot/,
    "expected error response to rollback from dndSnapshot",
  );
});

test("Plain Issue cards are not draggable", () => {
  // SPEC-2017 US-10: gwt-spec ラベル無し Issue は phase ラベルを持た
  // ないので D&D 不可。renderer は is_spec=false のカードに
  // draggable=false を設定する。これが無いと plain Issue が phase
  // カラムに入ってしまい、書き戻しで失敗する。
  assert.match(
    appSource,
    /draggable\s*=\s*!isPlain|draggable\s*=\s*false/,
    "expected plain Issue cards to set draggable=false",
  );
});

test("Drop target columns get visual feedback during dragover", () => {
  // SPEC-2017 US-8: the drop target column should show a hover
  // affordance (outline / background change) so the user knows
  // where the card will land. CSS class .is-drop-target is
  // toggled by dragenter / dragleave handlers.
  assert.match(
    appSource,
    /addEventListener\("dragover"|addEventListener\("dragenter"/,
    "expected dragover/dragenter wired for drop feedback",
  );
  assert.match(
    appSource,
    /is-drop-target/,
    "expected .is-drop-target class to appear in app.js drop feedback",
  );
});
