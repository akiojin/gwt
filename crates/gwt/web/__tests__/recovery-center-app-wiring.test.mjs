import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");

test("app wires Recovery Center transport, automatic attention, and manual access", () => {
  assert.match(
    appSource,
    /import \{ createRecoveryCenterController \} from "\/recovery-center-modal\.js";/,
  );
  assert.match(appSource, /createRecoveryCenterController\s*\(\s*\{/);
  assert.match(appSource, /kind:\s*"list_recovery_center"/);
  assert.match(appSource, /kind:\s*"recovery_center_action"/);
  assert.match(appSource, /action_handle:\s*actionHandle/);
  assert.match(
    appSource,
    /provider_choice_handle:\s*providerChoiceHandle\s*\|\|\s*undefined/,
  );
  assert.doesNotMatch(appSource, /expected_generation:/);
  assert.doesNotMatch(appSource, /provider_root_id:/);
  assert.doesNotMatch(appSource, /recovery_id:/);
  assert.match(appSource, /bounds:\s*visibleBounds\(\)/);
  assert.match(appSource, /case "recovery_center_state":/);
  assert.match(appSource, /openAttention\(event\.center\?\.candidates/);
  assert.match(appSource, /case "recovery_center_action_result":/);
  assert.match(appSource, /case "recovery_center_error":/);
  assert.match(appSource, /case "recovery-center":/);
});
