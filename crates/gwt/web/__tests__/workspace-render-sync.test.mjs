// Issue #3365 — renderWorkspace exception safety.
//
// `renderWorkspace` used to commit `renderedWorkspaceWindowsKey` BEFORE the
// per-window sync loops and `recomputeOperatorTelemetry()` ran. A deterministic
// exception mid-sync left the key committed, so the next identical
// workspace_state short-circuited on the key diff and the Fleet Minimap /
// window list / telemetry stayed frozen until reload.
//
// `createWorkspaceRenderSync` owns the render-key lifecycle instead:
// - the key is committed only after a fully clean sync,
// - a failed sync leaves the key invalidated so the next workspace_state
//   retries (self-healing once the poisoned window goes away),
// - one window's failure is isolated and never blocks the other windows,
//   the telemetry recompute, or the post-sync focus step,
// - render() itself never throws, so `renderAppState` continues into
//   `renderWindowList()` even on a degraded render.

import assert from "node:assert/strict";
import test from "node:test";

import { createWorkspaceRenderSync } from "../workspace-render-sync.js";

function renderArgs(overrides = {}) {
  return {
    key: "k1",
    sync: () => {},
    recompute: () => {},
    afterSync: () => {},
    ...overrides,
  };
}

test("clean render runs sync, recompute, afterSync in order and commits the key", () => {
  const calls = [];
  const renderSync = createWorkspaceRenderSync();

  const result = renderSync.render(
    renderArgs({
      sync: (isolate) => {
        calls.push("sync");
        isolate("ensure", ["w1", "w2"], (id) => calls.push(`ensure:${id}`));
      },
      recompute: () => calls.push("recompute"),
      afterSync: () => calls.push("afterSync"),
    }),
  );

  assert.deepEqual(calls, ["sync", "ensure:w1", "ensure:w2", "recompute", "afterSync"]);
  assert.equal(result.skipped, false);
  assert.deepEqual(result.failures, []);
});

test("same key after a clean render skips without invoking callbacks", () => {
  const renderSync = createWorkspaceRenderSync();
  renderSync.render(renderArgs({ key: "k1" }));

  const calls = [];
  const result = renderSync.render(
    renderArgs({
      key: "k1",
      sync: () => calls.push("sync"),
      recompute: () => calls.push("recompute"),
      afterSync: () => calls.push("afterSync"),
    }),
  );

  assert.equal(result.skipped, true);
  assert.deepEqual(calls, [], "an unchanged window set must not re-sync");
});

test("a changed key re-renders after a clean render", () => {
  const renderSync = createWorkspaceRenderSync();
  renderSync.render(renderArgs({ key: "k1" }));

  const result = renderSync.render(renderArgs({ key: "k2" }));
  assert.equal(result.skipped, false);
});

test("one throwing window is isolated and the remaining windows still sync", () => {
  const synced = [];
  const renderSync = createWorkspaceRenderSync();

  const result = renderSync.render(
    renderArgs({
      sync: (isolate) => {
        isolate("ensure", ["good-1", "bad", "good-2"], (id) => {
          if (id === "bad") {
            throw new Error("poisoned window");
          }
          synced.push(id);
        });
      },
    }),
  );

  assert.deepEqual(synced, ["good-1", "good-2"]);
  assert.equal(result.failures.length, 1);
  assert.equal(result.failures[0].label, "ensure");
  assert.equal(result.failures[0].item, "bad");
  assert.match(String(result.failures[0].error), /poisoned window/);
});

test("a failed sync leaves the key uncommitted so the same key retries next render", () => {
  const renderSync = createWorkspaceRenderSync();
  let shouldThrow = true;

  const degraded = renderSync.render(
    renderArgs({
      sync: (isolate) => {
        isolate("ensure", ["bad"], () => {
          if (shouldThrow) {
            throw new Error("boom");
          }
        });
      },
    }),
  );
  assert.equal(degraded.skipped, false);
  assert.equal(degraded.failures.length, 1);

  // Same key again: must NOT be skipped — the failed sync never committed.
  shouldThrow = false;
  const retry = renderSync.render(
    renderArgs({
      sync: (isolate) => {
        isolate("ensure", ["bad"], () => {
          if (shouldThrow) {
            throw new Error("boom");
          }
        });
      },
    }),
  );
  assert.equal(retry.skipped, false, "a degraded render must retry on the next state");
  assert.deepEqual(retry.failures, []);

  // Clean retry committed the key: the third identical state skips.
  const settled = renderSync.render(renderArgs({}));
  assert.equal(settled.skipped, true);
});

test("recompute runs even when the sync callback itself throws", () => {
  const calls = [];
  const renderSync = createWorkspaceRenderSync();

  const result = renderSync.render(
    renderArgs({
      sync: () => {
        throw new TypeError("workspace.windows is not iterable");
      },
      recompute: () => calls.push("recompute"),
      afterSync: () => calls.push("afterSync"),
    }),
  );

  assert.deepEqual(calls, ["recompute", "afterSync"], "telemetry must stay live on a failed sync");
  assert.equal(result.failures.length, 1);
  assert.equal(result.failures[0].label, "sync");
});

test("a recompute failure is recorded but does not block the key commit", () => {
  const renderSync = createWorkspaceRenderSync();

  const first = renderSync.render(
    renderArgs({
      recompute: () => {
        throw new Error("telemetry exploded");
      },
    }),
  );
  assert.equal(first.failures.length, 1);
  assert.equal(first.failures[0].label, "recompute");

  const second = renderSync.render(renderArgs({}));
  assert.equal(second.skipped, true, "a clean sync commits the key even if recompute failed");
});

test("an afterSync failure is recorded but does not block the key commit", () => {
  const renderSync = createWorkspaceRenderSync();

  const first = renderSync.render(
    renderArgs({
      afterSync: () => {
        throw new Error("focus exploded");
      },
    }),
  );
  assert.equal(first.failures.length, 1);
  assert.equal(first.failures[0].label, "after_sync");

  const second = renderSync.render(renderArgs({}));
  assert.equal(second.skipped, true);
});

test("onDegraded fires once per degraded render with the failures", () => {
  const reports = [];
  const renderSync = createWorkspaceRenderSync({
    onDegraded: (failures) => reports.push(failures),
  });

  renderSync.render(renderArgs({}));
  assert.equal(reports.length, 0, "a clean render must not report degradation");

  renderSync.render(
    renderArgs({
      key: "k2",
      sync: (isolate) => {
        isolate("ensure", ["bad"], () => {
          throw new Error("boom");
        });
      },
    }),
  );
  assert.equal(reports.length, 1);
  assert.equal(reports[0].length, 1);
  assert.equal(reports[0][0].item, "bad");
});

test("render never throws even when every callback throws", () => {
  const renderSync = createWorkspaceRenderSync({
    onDegraded: () => {
      throw new Error("reporter exploded");
    },
  });

  assert.doesNotThrow(() =>
    renderSync.render({
      key: "k1",
      sync: () => {
        throw new Error("sync exploded");
      },
      recompute: () => {
        throw new Error("recompute exploded");
      },
      afterSync: () => {
        throw new Error("afterSync exploded");
      },
    }),
  );
});
