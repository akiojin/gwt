// SPEC-1921 Phase 81 / Issue #3547 — manual Launch Agent generation conflict
// contract. These structural checks complement the embedded Playwright test:
// the existing Launch Wizard modal remains the only dialog shell, while the
// body exposes backend-gated recovery choices and public-safe actions.

import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const surface = readFileSync(resolve(here, "../launch-wizard-surface.js"), "utf8");
const index = readFileSync(resolve(here, "../index.html"), "utf8");

test("generation conflict renders public-safe Move, Stop and Cancel choices", () => {
  assert.match(surface, /generation_conflict/);
  assert.match(surface, /holder_label/);
  assert.match(surface, /detail/);
  assert.match(surface, /Move to existing pane/);
  assert.match(surface, /Stop and start successor/);
  assert.match(surface, /focus_generation_holder/);
  assert.match(surface, /stop_and_start_generation_successor/);
  assert.match(surface, /can_focus/);
  assert.match(surface, /can_stop_and_start/);
});

test("generation conflict stays inside the shared accessible wizard primitive", () => {
  assert.equal(
    (index.match(/id="wizard-modal"/g) || []).length,
    1,
    "the conflict must reuse the one Launch Wizard modal",
  );
  assert.match(
    index,
    /id="wizard-modal"[^>]*>[\s\S]*?class="modal-shell is-wizard"[\s\S]*?role="dialog"[\s\S]*?aria-modal="true"[\s\S]*?class="modal-body" id="wizard-body"/,
  );
  assert.match(
    surface,
    /generation_conflict[\s\S]*?(launch-panel|launch-note)[\s\S]*?wizard-button/,
    "conflict UI must reuse Operator wizard panel/note/button primitives",
  );
});

test("generation conflict actions use pending suppression and safe cancellation", () => {
  assert.match(
    surface,
    /generation_conflict[\s\S]*?launchWizardPendingAction/,
    "conflict actions must share the wizard pending latch",
  );
  assert.match(
    surface,
    /generation_conflict[\s\S]*?wizardCancelButton\.focus/,
    "Cancel must be the conflict dialog's safe initial focus",
  );
  assert.match(
    surface,
    /handleWizardEscapeKeydown[\s\S]*?kind:\s*"cancel"/,
    "Escape must retain the existing safe Cancel action",
  );
});
