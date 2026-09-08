// Issue #2698 PR 1 (B7) — verify the launch-wizard surface wires up the
// interaction-guard primitive so destructive `renderLaunchWizard()` calls
// cannot fire while a user has a native `<select>` dropdown open. The
// integration is too entangled with the rest of the bundle to exercise via
// a full DOM harness here, so we assert structural invariants on the
// source text instead.
//
// SPEC-3064 Phase 3 (E5): the wizard surface (state, guard, chrome
// listeners) moved from app.js to launch-wizard-surface.js. Guard wiring
// patterns are pinned against the extracted module; the receive() case
// arms in app.js are pinned as thin delegates into the surface appliers.
//
// If these patterns ever stop matching, run the wizard manually on
// Windows / macOS and confirm a native `<select>` dropdown still
// commits its selection even when `launch_wizard_state` arrives
// mid-interaction. The patterns are minimal markers — they should
// remain stable across reasonable refactors.

import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { parseHTML } from "linkedom";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
const wizardSource = readFileSync(
  resolve(here, "../launch-wizard-surface.js"),
  "utf8",
);

const wizardModuleSource = wizardSource
  .replace(
    'from "/interaction-guard.js"',
    `from "${pathToFileURL(resolve(here, "../interaction-guard.js")).href}"`,
  )
  .replace(
    'from "/focus-trap.js"',
    `from "${pathToFileURL(resolve(here, "../focus-trap.js")).href}"`,
  )
  .replace(
    'from "/launch-controls.js"',
    `from "${pathToFileURL(resolve(here, "../launch-controls.js")).href}"`,
  );
const { createLaunchWizardSurface } = await import(
  `data:text/javascript;base64,${Buffer.from(wizardModuleSource).toString("base64")}`
);

function holderWizard(overrides = {}) {
  return {
    title: "Launch Agent",
    branch_name: "work/holder-decision",
    selected_branch_name: "work/holder-decision",
    branch_mode: "use_selected",
    show_back_button: true,
    show_branch_controls: true,
    show_manual_setup: false,
    show_runtime_confirmation: false,
    show_confirm: false,
    show_start_methods: false,
    runtime_context_resolved: true,
    primary_action_enabled: true,
    launch_summary: [],
    progress_steps: [],
    ...overrides,
  };
}

function createWizardHarness({ sendResult = "sent" } = {}) {
  const { document } = parseHTML(`<!doctype html><html><body>
    <div class="modal-backdrop" id="wizard-modal" aria-hidden="true">
      <div class="modal-shell is-wizard" role="dialog" aria-modal="true" aria-labelledby="wizard-title" tabindex="-1">
        <div class="modal-header wizard-header">
          <h2 id="wizard-title">Launch Agent</h2>
          <div id="wizard-meta"></div>
        </div>
        <div id="wizard-error" role="alert" hidden></div>
        <div id="wizard-summary"></div>
        <div class="modal-body" id="wizard-body"></div>
        <div class="modal-footer wizard-footer">
          <div class="wizard-actions">
            <button class="wizard-button" id="wizard-back-button" hidden>Back</button>
            <button class="wizard-button" id="wizard-cancel-button">Cancel</button>
            <button class="wizard-button primary" id="wizard-submit-button">Launch</button>
          </div>
        </div>
      </div>
    </div>
  </body></html>`);
  const previousDocument = globalThis.document;
  globalThis.document = document;
  // linkedom does not keep document.activeElement in sync with focus().
  // Model the browser contract explicitly so holder-decision focus changes
  // can be asserted without emitting an enormous data-URL module stack.
  let activeElement = null;
  Object.defineProperty(document, "activeElement", {
    configurable: true,
    get: () => activeElement,
  });
  for (const node of document.querySelectorAll("button, [role='dialog']")) {
    node.focus = () => {
      activeElement = node;
    };
  }
  const sent = [];
  const deliveries = [];
  const createNode = (tag, className, textContent) => {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (textContent != null) node.textContent = textContent;
    return node;
  };
  const surface = createLaunchWizardSurface({
    createNode,
    closeModal: () => {},
    sendWizardAction: (action, options) => {
      sent.push(action);
      deliveries.push({ action, options });
      return sendResult;
    },
    requestWorkAdvisory: () => {},
  });
  surface.installWizardChrome();
  return {
    document,
    deliveries,
    sent,
    surface,
    restore: () => {
      globalThis.document = previousDocument;
    },
  };
}

function dispatchPointerActivation(document, button) {
  const pointer = new document.defaultView.Event("pointerup", {
    bubbles: true,
    cancelable: true,
  });
  Object.defineProperty(pointer, "button", { value: 0 });
  button.dispatchEvent(pointer);
  button.dispatchEvent(
    new document.defaultView.Event("click", { bubbles: true, cancelable: true }),
  );
}

test("holder decision replaces normal launch chrome with availability-gated actions and reasons", () => {
  const harness = createWizardHarness();
  try {
    harness.surface.applyLaunchWizardStateEvent({
      wizard: holderWizard({
        holder_decision: {
          fingerprint: "fp-local",
          holder_session_id: "session-local",
          holder_window_id: "window-local",
          holder_summary: "Remote holder · Unknown runtime",
          stop_available: false,
          stop_unavailable_reason: "Remote holders cannot be stopped here.",
          move_available: false,
          move_unavailable_reason: "Unknown windows cannot be moved.",
        },
      }),
    });

    const body = harness.document.getElementById("wizard-body");
    const move = harness.document.getElementById("wizard-back-button");
    const stop = harness.document.getElementById("wizard-submit-button");
    const cancel = harness.document.getElementById("wizard-cancel-button");
    assert.match(body.textContent, /Remote holder · Unknown runtime/);
    assert.match(body.textContent, /Remote holders cannot be stopped here\./);
    assert.match(body.textContent, /Unknown windows cannot be moved\./);
    assert.equal(move.textContent, "Move existing pane");
    assert.equal(move.hidden, false);
    assert.equal(move.disabled, true);
    assert.equal(stop.textContent, "Stop and start successor");
    assert.equal(stop.hidden, false);
    assert.equal(stop.disabled, true);
    assert.equal(stop.classList.contains("destructive"), true);
    assert.equal(move.classList.contains("destructive"), false);
    assert.equal(cancel.textContent, "Cancel");
    assert.doesNotMatch(move.textContent, /Launch/);
    cancel.dispatchEvent(
      new harness.document.defaultView.Event("click", {
        bubbles: true,
        cancelable: true,
      }),
    );
    assert.deepEqual(harness.sent, [{ kind: "cancel" }]);
  } finally {
    harness.restore();
  }
});

test("holder decision requires exact identifiers even when backend marks actions available", () => {
  const harness = createWizardHarness();
  try {
    harness.surface.applyLaunchWizardStateEvent({
      wizard: holderWizard({
        holder_decision: {
          fingerprint: "fp-without-window",
          holder_session_id: "session-local",
          holder_window_id: null,
          holder_summary: "Local holder",
          stop_available: true,
          move_available: true,
        },
      }),
    });

    assert.equal(
      harness.document.getElementById("wizard-back-button").disabled,
      true,
    );
    assert.equal(
      harness.document.getElementById("wizard-submit-button").disabled,
      true,
    );
  } finally {
    harness.restore();
  }
});

test("holder recovery dispatches exact payloads once while each action is pending", () => {
  const harness = createWizardHarness();
  const decision = {
    fingerprint: "fp/exact value",
    holder_session_id: "session-exact",
    holder_window_id: "window/exact value",
    holder_summary: "Local holder in another window",
    stop_available: true,
    move_available: true,
  };
  try {
    harness.surface.applyLaunchWizardStateEvent({
      wizard: holderWizard({ holder_decision: decision }),
    });
    dispatchPointerActivation(
      harness.document,
      harness.document.getElementById("wizard-submit-button"),
    );
    assert.deepEqual(harness.sent, [
      {
        kind: "stop_and_start_successor",
        fingerprint: "fp/exact value",
        window_id: "window/exact value",
      },
    ]);

    harness.surface.applyLaunchWizardStateEvent({
      wizard: holderWizard({
        holder_decision: decision,
        error: "The holder stop was not confirmed.",
      }),
    });
    dispatchPointerActivation(
      harness.document,
      harness.document.getElementById("wizard-back-button"),
    );
    assert.deepEqual(harness.sent, [
      {
        kind: "stop_and_start_successor",
        fingerprint: "fp/exact value",
        window_id: "window/exact value",
      },
      {
        kind: "move_existing_pane",
        fingerprint: "fp/exact value",
        window_id: "window/exact value",
      },
    ]);
  } finally {
    harness.restore();
  }
});

test("holder action stays pending through same-decision echoes and disables cancellation", () => {
  const harness = createWizardHarness();
  const decision = {
    fingerprint: "fp-pending",
    holder_session_id: "session-pending",
    holder_window_id: "window-pending",
    holder_summary: "Local holder",
    stop_available: true,
    move_available: true,
  };
  try {
    harness.surface.applyLaunchWizardStateEvent({
      wizard: holderWizard({ holder_decision: decision }),
    });
    dispatchPointerActivation(
      harness.document,
      harness.document.getElementById("wizard-submit-button"),
    );
    harness.surface.applyLaunchWizardStateEvent({
      wizard: holderWizard({ holder_decision: decision }),
    });

    const stop = harness.document.getElementById("wizard-submit-button");
    const cancel = harness.document.getElementById("wizard-cancel-button");
    assert.equal(stop.textContent, "Stopping...");
    assert.equal(stop.disabled, true);
    assert.equal(cancel.disabled, true);
    dispatchPointerActivation(harness.document, stop);
    cancel.dispatchEvent(
      new harness.document.defaultView.Event("click", {
        bubbles: true,
        cancelable: true,
      }),
    );
    assert.deepEqual(harness.sent, [
      {
        kind: "stop_and_start_successor",
        fingerprint: "fp-pending",
        window_id: "window-pending",
      },
    ]);
  } finally {
    harness.restore();
  }
});

test("holder successor materialization blocks Cancel and Escape", () => {
  const harness = createWizardHarness();
  const decision = {
    fingerprint: "fp-materializing",
    holder_session_id: "session-materializing",
    holder_window_id: "window-materializing",
    holder_summary: "Local holder",
    stop_available: true,
    move_available: true,
  };
  try {
    harness.surface.applyLaunchWizardStateEvent({
      wizard: holderWizard({ holder_decision: decision }),
    });
    dispatchPointerActivation(
      harness.document,
      harness.document.getElementById("wizard-submit-button"),
    );
    harness.surface.applyLaunchWizardStateEvent({
      wizard: holderWizard({
        holder_decision: decision,
        launch_materialization_pending: true,
        launch_materialization_message: "Starting successor...",
      }),
    });

    const cancel = harness.document.getElementById("wizard-cancel-button");
    assert.equal(cancel.disabled, true);
    cancel.dispatchEvent(
      new harness.document.defaultView.Event("click", {
        bubbles: true,
        cancelable: true,
      }),
    );
    let escapePrevented = false;
    assert.equal(
      harness.surface.handleWizardEscapeKeydown({
        preventDefault: () => {
          escapePrevented = true;
        },
      }),
      true,
    );
    assert.equal(escapePrevented, true);
    assert.deepEqual(harness.sent, [
      {
        kind: "stop_and_start_successor",
        fingerprint: "fp-materializing",
        window_id: "window-materializing",
      },
    ]);
  } finally {
    harness.restore();
  }
});

test("holder action becomes manually retryable after reconnect and a non-pending echo", () => {
  const harness = createWizardHarness();
  const decision = {
    fingerprint: "fp-reconnect",
    holder_session_id: "session-reconnect",
    holder_window_id: "window-reconnect",
    holder_summary: "Local holder",
    stop_available: true,
    move_available: true,
  };
  try {
    assert.equal(
      typeof harness.surface.handleLaunchWizardTransportChange,
      "function",
    );
    harness.surface.applyLaunchWizardStateEvent({
      wizard: holderWizard({ holder_decision: decision }),
    });
    const stop = harness.document.getElementById("wizard-submit-button");
    dispatchPointerActivation(harness.document, stop);
    harness.surface.handleLaunchWizardTransportChange(false);
    harness.surface.handleLaunchWizardTransportChange(true);
    harness.surface.applyLaunchWizardStateEvent({
      wizard: holderWizard({ holder_decision: decision }),
    });

    assert.equal(stop.disabled, false);
    assert.equal(stop.textContent, "Stop and start successor");
    assert.equal(harness.sent.length, 1);
    dispatchPointerActivation(harness.document, stop);
    assert.deepEqual(harness.sent, [
      {
        kind: "stop_and_start_successor",
        fingerprint: "fp-reconnect",
        window_id: "window-reconnect",
      },
      {
        kind: "stop_and_start_successor",
        fingerprint: "fp-reconnect",
        window_id: "window-reconnect",
      },
    ]);
  } finally {
    harness.restore();
  }
});

test("holder actions bypass the generic reconnect queue and become retryable after an unsent attempt", () => {
  const harness = createWizardHarness({ sendResult: "unavailable" });
  const decision = {
    fingerprint: "fp-queued-reconnect",
    holder_session_id: "session-queued-reconnect",
    holder_window_id: "window-queued-reconnect",
    holder_summary: "Local holder",
    stop_available: true,
    move_available: true,
  };
  try {
    harness.surface.applyLaunchWizardStateEvent({
      wizard: holderWizard({ holder_decision: decision }),
    });
    const stop = harness.document.getElementById("wizard-submit-button");
    dispatchPointerActivation(harness.document, stop);
    harness.surface.handleLaunchWizardTransportChange(false);
    harness.surface.handleLaunchWizardTransportChange(true);
    harness.surface.applyLaunchWizardStateEvent({
      wizard: holderWizard({ holder_decision: decision }),
    });

    assert.deepEqual(harness.deliveries[0].options, {
      queueIfDisconnected: false,
    });
    assert.equal(stop.disabled, false);
    assert.equal(stop.textContent, "Stop and start successor");
    dispatchPointerActivation(harness.document, stop);
    assert.equal(harness.sent.length, 2);
  } finally {
    harness.restore();
  }
});

test("new holder decision moves keyboard focus to safe Cancel action", () => {
  const harness = createWizardHarness();
  try {
    harness.surface.applyLaunchWizardStateEvent({ wizard: holderWizard() });
    harness.document.getElementById("wizard-submit-button").focus();
    harness.surface.applyLaunchWizardStateEvent({
      wizard: holderWizard({
        holder_decision: {
          fingerprint: "fp-safe-focus",
          holder_session_id: "session-safe-focus",
          holder_window_id: "window-safe-focus",
          holder_summary: "Local holder",
          stop_available: true,
          move_available: true,
        },
      }),
    });

    assert.equal(
      harness.document.activeElement,
      harness.document.getElementById("wizard-cancel-button"),
    );
  } finally {
    harness.restore();
  }
});

test("reconnect keeps holder actions locked when the server reports materialization", () => {
  const harness = createWizardHarness();
  const decision = {
    fingerprint: "fp-reconnect-pending",
    holder_session_id: "session-reconnect-pending",
    holder_window_id: "window-reconnect-pending",
    holder_summary: "Local holder",
    stop_available: true,
    move_available: true,
  };
  try {
    assert.equal(
      typeof harness.surface.handleLaunchWizardTransportChange,
      "function",
    );
    harness.surface.applyLaunchWizardStateEvent({
      wizard: holderWizard({ holder_decision: decision }),
    });
    const stop = harness.document.getElementById("wizard-submit-button");
    dispatchPointerActivation(harness.document, stop);
    harness.surface.handleLaunchWizardTransportChange(false);
    harness.surface.handleLaunchWizardTransportChange(true);
    harness.surface.applyLaunchWizardStateEvent({
      wizard: holderWizard({
        holder_decision: decision,
        launch_materialization_pending: true,
      }),
    });

    assert.equal(stop.disabled, true);
    dispatchPointerActivation(harness.document, stop);
    assert.equal(harness.sent.length, 1);
  } finally {
    harness.restore();
  }
});

test("holder summary falls back to the exact Session identity", () => {
  const harness = createWizardHarness();
  try {
    harness.surface.applyLaunchWizardStateEvent({
      wizard: holderWizard({
        holder_decision: {
          fingerprint: "fp-summary",
          holder_session_id: "session-summary",
          holder_window_id: null,
          holder_summary: "",
          stop_available: false,
          move_available: false,
        },
      }),
    });
    assert.match(
      harness.document.getElementById("wizard-body").textContent,
      /Session session-summary/,
    );
  } finally {
    harness.restore();
  }
});

test("holder chrome keeps Move neutral first, Cancel second, and destructive Stop last", () => {
  const harness = createWizardHarness();
  try {
    harness.surface.applyLaunchWizardStateEvent({
      wizard: holderWizard({
        holder_decision: {
          fingerprint: "fp-order",
          holder_session_id: "session-order",
          holder_window_id: "window-order",
          holder_summary: "Local holder",
          stop_available: true,
          move_available: true,
        },
      }),
    });
    const actions = [...harness.document.querySelector(".wizard-actions").children];
    assert.deepEqual(
      actions.map((button) => button.textContent),
      ["Move existing pane", "Cancel", "Stop and start successor"],
    );
    assert.equal(actions[2].classList.contains("destructive"), true);
    const dialog = harness.document.querySelector("[role='dialog']");
    assert.equal(dialog.getAttribute("aria-modal"), "true");
    assert.equal(dialog.getAttribute("aria-labelledby"), "wizard-title");
    assert.ok(dialog.querySelector(".modal-header"));
  } finally {
    harness.restore();
  }
});

test("wizard without holder decision preserves normal Back, Launch, and Cancel chrome", () => {
  const harness = createWizardHarness();
  try {
    harness.surface.applyLaunchWizardStateEvent({ wizard: holderWizard() });
    const back = harness.document.getElementById("wizard-back-button");
    const submit = harness.document.getElementById("wizard-submit-button");
    const cancel = harness.document.getElementById("wizard-cancel-button");
    assert.equal(back.textContent, "Back");
    assert.equal(back.hidden, false);
    assert.equal(submit.textContent, "Launch");
    assert.equal(submit.hidden, false);
    assert.equal(submit.disabled, false);
    assert.equal(cancel.textContent, "Cancel");
  } finally {
    harness.restore();
  }
});

test("wizard surface imports createInteractionGuard from /interaction-guard.js", () => {
  assert.match(
    wizardSource,
    /import\s*\{\s*createInteractionGuard\s*\}\s*from\s*["']\/interaction-guard\.js["']/,
    "expected named import of createInteractionGuard from /interaction-guard.js",
  );
});

test("wizard surface instantiates wizardInteractionGuard with an onFlush callback", () => {
  assert.match(
    wizardSource,
    /wizardInteractionGuard\s*=\s*createInteractionGuard\(\s*\{\s*[\s\S]{0,400}?onFlush\s*:/,
    "expected `wizardInteractionGuard = createInteractionGuard({ onFlush: ... })`",
  );
});

test("launch_wizard_state applier defers via wizardInteractionGuard before mutating state", () => {
  // The app.js case arm delegates into the surface applier, which must
  // contain a guard.defer({...}) check that returns early before
  // assigning `launchWizard = ...`.
  assert.match(
    appSource,
    /case\s+"launch_wizard_state":[\s\S]{0,300}?applyLaunchWizardStateEvent\(event\);\s*break;/,
    "expected app.js launch_wizard_state case to delegate into the wizard surface",
  );
  assert.match(
    wizardSource,
    /function\s+applyLaunchWizardStateEvent\(event\)\s*\{[\s\S]{0,400}?wizardInteractionGuard\.defer\([\s\S]{0,200}?\)\s*\)\s*\{\s*return;\s*\}[\s\S]{0,400}?launchWizard\s*=\s*event\.wizard/,
    "expected guard.defer() short-circuit before launchWizard mutation",
  );
});

test("launch_wizard_open_error applier defers via wizardInteractionGuard", () => {
  assert.match(
    appSource,
    /case\s+"launch_wizard_open_error":[\s\S]{0,300}?applyLaunchWizardOpenErrorEvent\(event\);\s*break;/,
    "expected app.js launch_wizard_open_error case to delegate into the wizard surface",
  );
  assert.match(
    wizardSource,
    /function\s+applyLaunchWizardOpenErrorEvent\(event\)\s*\{[\s\S]{0,400}?wizardInteractionGuard\.defer\([\s\S]{0,200}?\)\s*\)\s*\{\s*return;\s*\}/,
    "expected guard.defer() short-circuit in the launch_wizard_open_error applier",
  );
});

test("closeLaunchWizardLocal discards pending guard state before re-render", () => {
  // Local user-initiated close must not be undone by replaying a
  // deferred backend event — discard() drops both pending value
  // and active flag without invoking onFlush.
  assert.match(
    wizardSource,
    /function\s+closeLaunchWizardLocal\(\)\s*\{[\s\S]{0,500}?wizardInteractionGuard\.discard\(\)[\s\S]{0,140}?renderLaunchWizard\(\)/,
    "expected closeLaunchWizardLocal() to discard guard before render",
  );
});

test("wizardBody activates the guard on pointerdown over a <select>", () => {
  assert.match(
    wizardSource,
    /wizardBody\.addEventListener\(\s*"pointerdown"[\s\S]{0,400}?tagName\s*===\s*"SELECT"[\s\S]{0,200}?wizardInteractionGuard\.activate\(\)/,
    "expected delegated pointerdown listener that activates the guard",
  );
});

test("wizardBody releases the guard on change over a <select>", () => {
  assert.match(
    wizardSource,
    /wizardBody\.addEventListener\(\s*"change"[\s\S]{0,400}?tagName\s*===\s*"SELECT"[\s\S]{0,200}?wizardInteractionGuard\.release\(\)/,
    "expected delegated change listener that releases the guard",
  );
});

test("wizardBody releases the guard on focusout over a <select>", () => {
  assert.match(
    wizardSource,
    /wizardBody\.addEventListener\(\s*"focusout"[\s\S]{0,400}?tagName\s*===\s*"SELECT"[\s\S]{0,200}?wizardInteractionGuard\.release\(\)/,
    "expected delegated focusout listener that releases the guard",
  );
});

test("wizardBody guards segmented choices until focus leaves the committed option", () => {
  assert.match(
    wizardSource,
    /const\s+isGuardedSegmentedOption[\s\S]{0,240}?launch-segmented__option/,
    "expected a shared segmented-option interaction guard predicate",
  );
  assert.match(
    wizardSource,
    /wizardBody\.addEventListener\(\s*"pointerdown"[\s\S]{0,500}?isGuardedSegmentedOption\(target\)[\s\S]{0,200}?wizardInteractionGuard\.activate\(\)/,
    "expected segmented pointerdown to activate the interaction guard",
  );
  assert.match(
    wizardSource,
    /wizardBody\.addEventListener\(\s*"focusout"[\s\S]{0,500}?isGuardedSegmentedOption\(target\)[\s\S]{0,200}?wizardInteractionGuard\.release\(\)/,
    "expected segmented focusout to flush the authoritative state",
  );
});

test("wizardModal releases the guard when Escape is pressed during interaction", () => {
  assert.match(
    wizardSource,
    /wizardModal\.addEventListener\(\s*"keydown"[\s\S]{0,400}?key\s*===\s*"Escape"[\s\S]{0,200}?wizardInteractionGuard\.release\(\)/,
    "expected Escape keydown to release the guard",
  );
});

test("wizard chrome actions release any active guard before dispatch", () => {
  assert.match(
    wizardSource,
    /function\s+releaseWizardInteractionGuardForChromeAction\(\)\s*\{[\s\S]{0,300}?wizardInteractionGuard\.isActive\(\)[\s\S]{0,150}?wizardInteractionGuard\.release\(\)[\s\S]{0,200}?return\s+Boolean\(launchWizard\s*\|\|\s*launchWizardOpenError\)/,
    "expected a chrome-action helper that releases the guard and reports whether wizard state remains",
  );
  assert.match(
    wizardSource,
    /function\s+closeLaunchWizardFromChrome\(\)\s*\{[\s\S]{0,160}?releaseWizardInteractionGuardForChromeAction\(\)/,
    "expected Cancel/Close to release pending guard state before dispatching",
  );
  assert.match(
    wizardSource,
    /wizardBackButton\.addEventListener\(\s*"click"[\s\S]{0,220}?releaseWizardInteractionGuardForChromeAction\(\)[\s\S]{0,300}?kind:\s*"back"/,
    "expected Back to release pending guard state before dispatching",
  );
  assert.match(
    wizardSource,
    /function\s+handleLaunchWizardSubmitFromChrome\(\)[\s\S]{0,260}?releaseWizardInteractionGuardForChromeAction\(\)[\s\S]{0,320}?kind:\s*"submit"/,
    "expected Submit handler to release pending guard state before dispatching",
  );
  assert.match(
    wizardSource,
    /if\s*\(wizardModal\.classList\.contains\("open"\)\)\s*\{[\s\S]{0,260}?releaseWizardInteractionGuardForChromeAction\(\)[\s\S]{0,350}?kind:\s*"cancel"/,
    "expected Escape-close to release pending guard state before dispatching",
  );
});

test("wizard start method actions release guard before dispatch", () => {
  assert.match(
    wizardSource,
    /const\s+handleStartMethodLaunchAction\s*=\s*\(\)\s*=>\s*\{[\s\S]{0,260}?releaseWizardInteractionGuardForChromeAction\(\)[\s\S]{0,360}?kind:\s*"use_start_method"/,
    "expected Start methods direct actions to release pending guard state before dispatching",
  );
});

test("wizard launch pointer fallback routes submit and start methods", () => {
  assert.match(
    wizardSource,
    /function\s+handleLaunchWizardSubmitFromChrome\(\)[\s\S]{0,500}?kind:\s*"submit"/,
    "expected Launch Wizard submit to be centralized for click and pointer fallback",
  );
  assert.match(
    wizardSource,
    /wizardSubmitButton\.addEventListener\(\s*"pointerup"[\s\S]{0,420}?handleLaunchWizardSubmitFromChrome\(\)/,
    "expected Create and Launch pointerup fallback to route through submit handler",
  );
  assert.match(
    wizardSource,
    /button\.addEventListener\(\s*"pointerup"[\s\S]{0,420}?handleStartMethodLaunchAction\(\)/,
    "expected Start method pointerup fallback to route through the same action handler",
  );
});

test("wizard launch actions expose local pending feedback", () => {
  assert.match(
    wizardSource,
    /let\s+launchWizardPendingAction\s*=\s*null/,
    "expected Launch Wizard to track a local pending action",
  );
  assert.match(
    wizardSource,
    /function\s+setLaunchWizardPendingAction\(\s*action[\s\S]{0,500}?launchWizardPendingAction\s*=/,
    "expected a helper to set Launch Wizard pending state",
  );
  assert.match(
    wizardSource,
    /function\s+clearLaunchWizardPendingAction\(\)[\s\S]{0,300}?launchWizardPendingAction\s*=\s*null/,
    "expected a helper to clear Launch Wizard pending state",
  );
  assert.match(
    wizardSource,
    /wizardModal\.classList\.toggle\(\s*"is-launch-pending"[\s\S]{0,220}?wizardDialog\.setAttribute\(\s*"aria-busy"/,
    "expected modal busy class and aria-busy to mirror pending launch actions",
  );
  assert.match(
    wizardSource,
    /wizardSubmitButton\.textContent\s*=\s*isLaunchSubmitPending[\s\S]{0,160}?"Launching\.\.\."/,
    "expected final launch submit to show an immediate Launching label",
  );
  assert.match(
    wizardSource,
    /createNode\(\s*"div",\s*"launch-note launch-pending-note",\s*launchWizard\.launch_materialization_message\s*\|\|\s*"Preparing worktree\.\.\."/,
    "expected pending submit to render visible progress copy in the modal",
  );
});

test("wizard backend launch materialization state preserves pending feedback", () => {
  assert.match(
    wizardSource,
    /function\s+applyLaunchWizardStateEvent\(event\)[\s\S]{0,700}?shouldClearLaunchWizardPendingAction\(event\.wizard\)[\s\S]{0,180}?clearLaunchWizardPendingAction\(\)/,
    "backend launch materialization state must not clear local pending chrome",
  );
  assert.match(
    wizardSource,
    /function\s+shouldClearLaunchWizardPendingAction\(nextWizard\)[\s\S]{0,700}?return\s+!nextWizard\?\.launch_materialization_pending/,
    "pending acknowledgement must distinguish materialization from stale state echoes",
  );
  assert.match(
    wizardSource,
    /const\s+isLaunchMaterializationPending\s*=\s*Boolean\(\s*launchWizard\?\.launch_materialization_pending,?\s*\)/,
    "renderer must read backend launch materialization pending state null-safely",
  );
  assert.doesNotMatch(
    wizardSource,
    /Boolean\(\s*launchWizard\.launch_materialization_pending,?\s*\)/,
    "renderer must not dereference launchWizard while local opening state has no backend wizard",
  );
  assert.match(
    wizardSource,
    /launchWizard\.launch_materialization_message\s*\|\|\s*"Preparing worktree\.\.\."/,
    "expected backend materialization message to render as visible progress copy",
  );
});

test("app forwards WebSocket connection state to the launch wizard surface", () => {
  assert.match(
    appSource,
    /function\s+setConnectionState\(connected\)[\s\S]{0,500}?handleLaunchWizardTransportChange\(connected\)/,
    "expected reconnect epochs to reach the launch wizard pending-action state",
  );
});
