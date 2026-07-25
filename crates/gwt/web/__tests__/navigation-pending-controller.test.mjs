import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";

import { createNavigationPendingController } from "../navigation-pending-controller.js";

const here = dirname(fileURLToPath(import.meta.url));

function projectState(activeTabId = "tab-a") {
  return {
    app_version: "test",
    active_tab_id: activeTabId,
    recent_projects: [],
    tabs: [
      {
        id: "tab-a",
        title: "A",
        workspace: {
          viewport: { x: 0, y: 0, zoom: 1 },
          windows: [
            {
              id: "window-a",
              z_index: 2,
              placement: { kind: "canvas" },
            },
          ],
        },
      },
      {
        id: "tab-b",
        title: "B",
        workspace: {
          viewport: { x: 0, y: 0, zoom: 1 },
          windows: [
            {
              id: "window-b1",
              z_index: 1,
              tab_group_id: "group-b",
              tab_group_active: true,
              placement: { kind: "canvas" },
            },
            {
              id: "window-b2",
              z_index: 1,
              tab_group_id: "group-b",
              tab_group_active: false,
              placement: { kind: "canvas" },
            },
            {
              id: "window-b3",
              z_index: 2,
              placement: { kind: "canvas" },
            },
          ],
        },
      },
    ],
  };
}

function setup(initialState = projectState()) {
  const published = [];
  let sequence = 0;
  const controller = createNavigationPendingController({
    initialState,
    onState: (state) => published.push(state),
    createInteractionId: () => `interaction-${++sequence}`,
  });
  return { controller, published };
}

test("project navigation publishes the optimistic tab before transport acknowledgement", () => {
  const { controller, published } = setup();

  const message = controller.begin({
    kind: "select_project_tab",
    tab_id: "tab-b",
  });

  assert.deepEqual(message, {
    kind: "select_project_tab",
    tab_id: "tab-b",
    interaction_id: "interaction-1",
  });
  assert.equal(published.at(-1).active_tab_id, "tab-b");
  assert.equal(controller.pendingCount(), 1);
});

test("rapid B then A navigation never replays the older B result over pending A", () => {
  const { controller, published } = setup();
  const toB = controller.begin({
    kind: "select_project_tab",
    tab_id: "tab-b",
  });
  const toA = controller.begin({
    kind: "select_project_tab",
    tab_id: "tab-a",
  });

  controller.handleResult({
    kind: "navigation_result",
    interaction_id: toB.interaction_id,
    revision: 1,
    scope: "project_tab",
    outcome: "accepted",
    canonical: {
      active_tab_id: "tab-b",
      target_id: "tab-b",
      window_updates: [],
    },
  });

  assert.equal(published.at(-1).active_tab_id, "tab-a");
  assert.equal(controller.pendingCount(), 1);

  controller.handleResult({
    kind: "navigation_result",
    interaction_id: toA.interaction_id,
    revision: 2,
    scope: "project_tab",
    outcome: "accepted",
    canonical: {
      active_tab_id: "tab-a",
      target_id: "tab-a",
      window_updates: [],
    },
  });

  assert.equal(published.at(-1).active_tab_id, "tab-a");
  assert.equal(controller.pendingCount(), 0);
  assert.equal(controller.revision(), 2);
});

test("a later A result retires an earlier pending B before its late result", () => {
  const { controller, published } = setup();
  const toB = controller.begin({
    kind: "select_project_tab",
    tab_id: "tab-b",
  });
  const toA = controller.begin({
    kind: "select_project_tab",
    tab_id: "tab-a",
  });

  assert.equal(
    controller.handleResult({
      kind: "navigation_result",
      interaction_id: toA.interaction_id,
      revision: 2,
      scope: "project_tab",
      outcome: "accepted",
      canonical: {
        active_tab_id: "tab-a",
        target_id: "tab-a",
        window_updates: [],
      },
    }),
    true,
  );

  assert.equal(controller.pendingCount(), 0);
  assert.equal(published.at(-1).active_tab_id, "tab-a");
  assert.equal(controller.revision(), 2);
  assert.equal(
    controller.handleResult({
      kind: "navigation_result",
      interaction_id: toB.interaction_id,
      revision: 1,
      scope: "project_tab",
      outcome: "accepted",
      canonical: {
        active_tab_id: "tab-b",
        target_id: "tab-b",
        window_updates: [],
      },
    }),
    false,
  );
  assert.equal(published.at(-1).active_tab_id, "tab-a");
  assert.equal(controller.revision(), 2);
});

test("rejected navigation removes the optimistic overlay and restores canonical state", () => {
  const { controller, published } = setup();
  const pending = controller.begin({
    kind: "select_project_tab",
    tab_id: "tab-b",
  });
  assert.equal(published.at(-1).active_tab_id, "tab-b");

  controller.handleResult({
    kind: "navigation_result",
    interaction_id: pending.interaction_id,
    revision: 0,
    scope: "project_tab",
    outcome: "not_found",
    canonical: {
      active_tab_id: "tab-a",
      target_id: "tab-b",
      window_updates: [],
    },
  });

  assert.equal(published.at(-1).active_tab_id, "tab-a");
  assert.equal(controller.pendingCount(), 0);
});

test("window-tab navigation reveals and raises the selected group member locally", () => {
  const { controller, published } = setup(projectState("tab-b"));

  const message = controller.begin({
    kind: "activate_window_tab",
    id: "window-b2",
  });

  assert.equal(message.interaction_id, "interaction-1");
  const windows = published.at(-1).tabs[1].workspace.windows;
  assert.equal(windows.find((window) => window.id === "window-b1").tab_group_active, false);
  assert.equal(windows.find((window) => window.id === "window-b2").tab_group_active, true);
  assert.equal(windows.find((window) => window.id === "window-b2").z_index, 3);
});

test("cross-project window focus reveals a hidden grouped member locally", () => {
  const { controller, published } = setup(projectState("tab-a"));

  controller.begin({
    kind: "focus_window",
    id: "window-b2",
  });

  const state = published.at(-1);
  const windows = state.tabs[1].workspace.windows;
  assert.equal(state.active_tab_id, "tab-b");
  assert.equal(
    windows.find((window) => window.id === "window-b1").tab_group_active,
    false,
  );
  assert.equal(
    windows.find((window) => window.id === "window-b2").tab_group_active,
    true,
  );
  assert.equal(windows.find((window) => window.id === "window-b2").z_index, 3);
});

test("window focus allocates optimistic z-order without rescanning every window", () => {
  let zReads = 0;
  const windows = Array.from({ length: 100 }, (_, index) => {
    const windowData = {
      id: `window-${index + 1}`,
      placement: { kind: "canvas" },
    };
    Object.defineProperty(windowData, "z_index", {
      enumerable: true,
      get() {
        zReads += 1;
        return index + 1;
      },
    });
    return windowData;
  });
  const state = {
    app_version: "test",
    active_tab_id: "tab-large",
    recent_projects: [],
    tabs: [
      {
        id: "tab-large",
        workspace: {
          viewport: { x: 0, y: 0, zoom: 1 },
          windows,
        },
      },
    ],
  };
  const { controller } = setup(state);
  zReads = 0;

  controller.begin({
    kind: "focus_window",
    id: "window-100",
  });

  assert.ok(
    zReads <= 2,
    `the input path may read the target z-index, not all 100 windows (reads=${zReads})`,
  );
  assert.equal(
    controller.state().tabs[0].workspace.windows.at(-1).z_index,
    101,
  );
});

test("a stale workspace revision cannot overwrite newer canonical navigation", () => {
  const { controller, published } = setup();

  assert.equal(
    controller.handleWorkspace({
      revision: 3,
      workspace: projectState("tab-b"),
    }),
    true,
  );
  assert.equal(
    controller.handleWorkspace({
      revision: 2,
      workspace: projectState("tab-a"),
    }),
    false,
  );

  assert.equal(published.at(-1).active_tab_id, "tab-b");
  assert.equal(controller.revision(), 3);
});

test("a newer foreign workspace cannot retire a local operation before its matching result", () => {
  const { controller, published } = setup();
  controller.begin({
    kind: "select_project_tab",
    tab_id: "tab-b",
  });
  assert.equal(published.at(-1).active_tab_id, "tab-b");

  controller.handleWorkspace({
    revision: 1,
    workspace: projectState("tab-a"),
  });

  assert.equal(controller.pendingCount(), 1);
  assert.equal(published.at(-1).active_tab_id, "tab-b");
});

test("a same-revision topology update rebases pending focus above a newly raised peer", () => {
  const { controller, published } = setup(projectState("tab-b"));
  controller.begin({
    kind: "focus_window",
    id: "window-b2",
  });

  const updated = projectState("tab-b");
  updated.tabs[1].workspace.windows.find(
    (windowData) => windowData.id === "window-b3",
  ).z_index = 100;
  controller.handleWorkspace({
    revision: 0,
    workspace: updated,
  });

  const windows = published.at(-1).tabs[1].workspace.windows;
  assert.equal(
    windows.find((windowData) => windowData.id === "window-b2").z_index,
    101,
  );
});

test("rapid focus replay keeps topology traversal bounded per click", () => {
  let idReads = 0;
  const windows = Array.from({ length: 100 }, (_, index) => {
    const windowData = {
      z_index: index + 1,
      placement: { kind: "canvas" },
    };
    Object.defineProperty(windowData, "id", {
      enumerable: true,
      get() {
        idReads += 1;
        return `window-${index + 1}`;
      },
    });
    return windowData;
  });
  const state = {
    app_version: "test",
    active_tab_id: "tab-large",
    recent_projects: [],
    tabs: [
      {
        id: "tab-large",
        workspace: {
          viewport: { x: 0, y: 0, zoom: 1 },
          windows,
        },
      },
    ],
  };
  const { controller } = setup(state);
  idReads = 0;
  const readsPerClick = [];

  for (let index = 0; index < 5; index += 1) {
    const before = idReads;
    controller.begin({
      kind: "focus_window",
      id: "window-100",
    });
    readsPerClick.push(idReads - before);
  }

  assert.ok(
    Math.max(...readsPerClick) <= readsPerClick[0] + 10,
    `pending replay must stay linear in topology, reads=${readsPerClick.join(",")}`,
  );
});

test("workspace reconciliation indexes topology once for many pending operations", () => {
  const initial = projectState("tab-b");
  const { controller } = setup(initial);
  for (let index = 0; index < 100; index += 1) {
    controller.begin({
      kind: "focus_window",
      id: "window-b3",
    });
  }

  let idReads = 0;
  const windows = Array.from({ length: 100 }, (_, index) => {
    const windowData = {
      z_index: index + 1,
      placement: { kind: "canvas" },
    };
    Object.defineProperty(windowData, "id", {
      enumerable: true,
      get() {
        idReads += 1;
        return `window-${index + 1}`;
      },
    });
    return windowData;
  });
  const workspace = {
    app_version: "test",
    active_tab_id: "tab-large",
    recent_projects: [],
    tabs: [
      {
        id: "tab-large",
        workspace: {
          viewport: { x: 0, y: 0, zoom: 1 },
          windows,
        },
      },
      ...initial.tabs,
    ],
  };

  idReads = 0;
  controller.handleWorkspace({ revision: 1, workspace });

  assert.ok(
    idReads <= 500,
    `workspace reconciliation must be O(topology + pending), reads=${idReads}`,
  );
});

test("revision zero establishes the versioned workspace fence", () => {
  const { controller, published } = setup();

  assert.equal(
    controller.handleWorkspace({
      revision: 0,
      workspace: projectState("tab-b"),
    }),
    true,
  );
  assert.equal(
    controller.handleWorkspace({
      workspace: projectState("tab-a"),
    }),
    false,
  );
  assert.equal(published.at(-1).active_tab_id, "tab-b");
});

test("an earlier result advances the fence of later pending navigation", () => {
  const { controller, published } = setup();
  const toB = controller.begin({
    kind: "select_project_tab",
    tab_id: "tab-b",
  });
  const toA = controller.begin({
    kind: "select_project_tab",
    tab_id: "tab-a",
  });

  controller.handleResult({
    kind: "navigation_result",
    interaction_id: toB.interaction_id,
    revision: 1,
    canonical: {
      active_tab_id: "tab-b",
      target_id: "tab-b",
      window_updates: [],
    },
  });
  controller.handleWorkspace({
    revision: 1,
    workspace: projectState("tab-b"),
  });

  assert.equal(
    published.at(-1).active_tab_id,
    "tab-a",
    "the workspace paired with B must not retire the later A operation",
  );
  assert.equal(controller.pendingCount(), 1);

  controller.handleResult({
    kind: "navigation_result",
    interaction_id: toA.interaction_id,
    revision: 2,
    canonical: {
      active_tab_id: "tab-a",
      target_id: "tab-a",
      window_updates: [],
    },
  });
  assert.equal(controller.pendingCount(), 0);
  assert.equal(published.at(-1).active_tab_id, "tab-a");
});

test("legacy unversioned workspace retires pending navigation", () => {
  const { controller, published } = setup();
  controller.begin({
    kind: "select_project_tab",
    tab_id: "tab-b",
  });

  controller.handleWorkspace({
    workspace: projectState("tab-a"),
  });

  assert.equal(controller.pendingCount(), 0);
  assert.equal(published.at(-1).active_tab_id, "tab-a");
});

test("clearing reconnect-era pending work restores the last canonical snapshot", () => {
  const { controller, published } = setup();
  controller.begin({
    kind: "select_project_tab",
    tab_id: "tab-b",
  });

  controller.clearPending();

  assert.equal(published.at(-1).active_tab_id, "tab-a");
  assert.equal(controller.pendingCount(), 0);
});

test("connection reset keeps the optimistic paint until a low-revision bootstrap arrives", () => {
  const { controller, published } = setup();
  assert.equal(
    controller.handleWorkspace({
      revision: 8,
      workspace: projectState("tab-a"),
    }),
    true,
  );
  controller.begin({
    kind: "select_project_tab",
    tab_id: "tab-b",
  });
  const publishCountBeforeReset = published.length;

  controller.resetConnection();

  assert.equal(controller.pendingCount(), 0);
  assert.equal(controller.revision(), 0);
  assert.equal(published.length, publishCountBeforeReset);
  assert.equal(controller.state().active_tab_id, "tab-b");
  assert.equal(
    controller.handleWorkspace({
      revision: 1,
      workspace: projectState("tab-a"),
    }),
    true,
  );
  assert.equal(published.at(-1).active_tab_id, "tab-a");
});

test("an authoritative workspace that removes a target retires its pending navigation", () => {
  const { controller, published } = setup();
  controller.begin({
    kind: "select_project_tab",
    tab_id: "tab-b",
  });

  const withoutTabB = projectState("tab-a");
  withoutTabB.tabs = withoutTabB.tabs.filter((tab) => tab.id !== "tab-b");
  controller.handleWorkspace({
    revision: 1,
    workspace: withoutTabB,
  });

  assert.equal(controller.pendingCount(), 0);
  assert.equal(published.at(-1).active_tab_id, "tab-a");

  controller.handleWorkspace({
    revision: 2,
    workspace: projectState("tab-a"),
  });
  assert.equal(
    published.at(-1).active_tab_id,
    "tab-a",
    "a reused target id must not resurrect an operation retired by canonical removal",
  );
});

test("duplicate or unknown navigation results cannot mutate canonical state", () => {
  const { controller, published } = setup();
  const pending = controller.begin({
    kind: "select_project_tab",
    tab_id: "tab-b",
  });
  controller.handleResult({
    kind: "navigation_result",
    interaction_id: pending.interaction_id,
    revision: 1,
    canonical: {
      active_tab_id: "tab-b",
      target_id: "tab-b",
      window_updates: [],
    },
  });

  assert.equal(
    controller.handleResult({
      kind: "navigation_result",
      interaction_id: pending.interaction_id,
      revision: 2,
      canonical: {
        active_tab_id: "tab-a",
        target_id: "tab-a",
        window_updates: [],
      },
    }),
    false,
  );
  assert.equal(
    controller.handleResult({
      kind: "navigation_result",
      interaction_id: "another-client",
      revision: 3,
      canonical: {
        active_tab_id: "tab-a",
        target_id: "tab-a",
        window_updates: [],
      },
    }),
    false,
  );
  assert.equal(published.at(-1).active_tab_id, "tab-b");
  assert.equal(controller.revision(), 1);
});

test("invalid-present workspace revisions are rejected instead of treated as legacy", () => {
  for (const revision of [
    Number.MAX_SAFE_INTEGER + 1,
    -1,
    "1",
  ]) {
    const { controller } = setup();
    assert.equal(
      controller.handleWorkspace({
        revision,
        workspace: projectState("tab-b"),
      }),
      false,
    );
    assert.equal(controller.state().active_tab_id, "tab-a");
    assert.equal(controller.revision(), 0);
  }

  const { controller } = setup();
  assert.equal(
    controller.handleWorkspace({
      workspace: projectState("tab-b"),
    }),
    true,
  );
  assert.equal(controller.state().active_tab_id, "tab-b");
});

test("unsafe result revisions settle pending work without poisoning the revision fence", () => {
  const { controller, published } = setup();
  const pending = controller.begin({
    kind: "select_project_tab",
    tab_id: "tab-b",
  });

  controller.handleResult({
    kind: "navigation_result",
    interaction_id: pending.interaction_id,
    revision: Number.MAX_SAFE_INTEGER + 1,
    canonical: {
      active_tab_id: "tab-b",
      target_id: "tab-b",
      window_updates: [],
    },
  });

  assert.equal(controller.pendingCount(), 0);
  assert.equal(controller.revision(), 0);
  assert.equal(published.at(-1).active_tab_id, "tab-a");
  assert.equal(
    controller.handleWorkspace({
      revision: 1,
      workspace: projectState("tab-b"),
    }),
    true,
  );
  assert.equal(published.at(-1).active_tab_id, "tab-b");
});

test("app routes navigation sends and ordered backend events through the controller", () => {
  const source = readFileSync(resolve(here, "../app.js"), "utf8");
  const frontendRunner = readFileSync(
    resolve(here, "../../../../scripts/run-frontend-unit-tests.sh"),
    "utf8",
  );
  const projectShellSource = readFileSync(
    resolve(here, "../project-shell-surface.js"),
    "utf8",
  );
  const fleetMinimapSource = readFileSync(
    resolve(here, "../fleet-minimap.js"),
    "utf8",
  );

  assert.match(
    source,
    /from "\/navigation-pending-controller\.js"/,
    "the shipped app must import the local-first controller",
  );
  assert.match(
    source,
    /isNavigationRequest\(message\)[\s\S]*navigationPendingController\.begin\(message\)[\s\S]*sendRaw\(outgoing\)/,
    "navigation must publish its optimistic state before entering the raw socket path",
  );
  assert.match(
    source,
    /function send\(message\)[\s\S]*navigationWindowTargetId\(message\)[\s\S]*focusWindowLocally\(localWindowTargetId\)[\s\S]*navigationPendingController\.begin\(message\)/,
    "window navigation must establish the initiating viewer's input target before optimistic rendering",
  );
  assert.match(
    source,
    /case "workspace_state":[\s\S]*navigationPendingController\.handleWorkspace\(event\)/,
  );
  assert.match(
    source,
    /case "navigation_result":[\s\S]*navigationPendingController\.handleResult\(event\)/,
  );
  assert.match(
    source,
    /function sendRaw\(message\)[\s\S]*isNavigationRequest\(message\)[\s\S]*return;/,
    "offline navigation must not enter the generic reconnect resend queue",
  );
  assert.match(
    source,
    /function handleSocketOpen\(\)[\s\S]*navigationPendingController\?\.resetConnection\(\)/,
    "a new socket generation must silently reset pending operations and its revision fence",
  );
  const raiseStart = source.indexOf("function raiseWindowElementLocally");
  const raiseEnd = source.indexOf("function focusWindowRemotely", raiseStart);
  const raiseBody = source.slice(raiseStart, raiseEnd);
  assert.doesNotMatch(
    raiseBody,
    /windowMap\.values\(\)/,
    "a focus click must not scan every mounted window to allocate z-order",
  );
  assert.match(
    raiseBody,
    /localWindowZCounter/,
    "focus z-order must use the incrementally maintained local counter",
  );
  assert.match(
    source,
    /function cycleFocus\([\s\S]*frameWindow\(windows\[nextIndex\]\.id,\s*\{\s*animate:\s*false\s*\}\)/,
    "keyboard focus cycling must commit the local camera without a 180ms tween",
  );
  assert.match(
    projectShellSource,
    /row\.addEventListener\("click"[\s\S]*frameWindow\(entry\.id,\s*\{\s*animate:\s*false\s*\}\)/,
    "Windows list selection must commit the local camera immediately",
  );
  assert.match(
    fleetMinimapSource,
    /frameWindow\(windowId,\s*\{\s*animate:\s*false\s*\}\)/,
    "minimap selection must commit the local camera immediately",
  );
  assert.match(
    source,
    /const focusedWindowStillVisible[\s\S]*visibleWindowData\(focusedWindow\)[\s\S]*if \(!focusedWindowStillVisible\)[\s\S]*topmostWindowId\(workspace\)/,
    "remote workspace updates must preserve an existing visible local input focus",
  );
  assert.match(
    frontendRunner,
    /crates\/gwt\/web\/__tests__\/navigation-pending-controller\.test\.mjs/,
    "the canonical frontend suite must execute the navigation controller contract",
  );
});
