// SPEC-3064 Phase 3 (E5) — Launch Wizard surface extracted from app.js.
// Owns the wizard state (launchWizard / launchWizardOpenError /
// launchWizardOpening / branch draft / pending action), the
// wizardInteractionGuard that defers destructive re-renders while a native
// <select> dropdown or the reasoning slider is mid-interaction, the field
// builders, the state transitions, renderLaunchWizard, the wizard chrome
// listeners (installWizardChrome), and the wizard branch of the global Esc
// handler. Pure movement from app.js: behavior, DOM output, and WS protocol
// are unchanged; the moved code keeps its original app.js indentation. The
// only textual change: in-module self-references through
// `frontendUnits.launchWizardSurface.*` became direct local calls
// (render → renderLaunchWizard, flushBranchDraft → flushWizardBranchDraft,
// sendAction → sendWizardAction).
//
// deps:
// - createNode(tag, className, textContent): shared DOM helper owned by
//   app.js (also used by Board / settings surfaces).
// - closeModal(): closes the Add Window preset modal before the wizard
//   takes over the screen.
// - sendWizardAction(action): wizard action transport (owned by app.js so
//   the frontendUnits registry and visibleBounds() stay there).
import { createInteractionGuard } from "/interaction-guard.js";
import { createFocusTrap } from "/focus-trap.js";
import {
  buildChoiceOrSelectField,
  buildReasoningField,
  buildToggleField,
} from "/launch-controls.js";

export function createLaunchWizardSurface({
  createNode,
  closeModal,
  sendWizardAction,
}) {
      const wizardModal = document.getElementById("wizard-modal");
      const wizardDialog = wizardModal.querySelector(".modal-shell");
      const wizardTitle = document.getElementById("wizard-title");
      const wizardMeta = document.getElementById("wizard-meta");
      const wizardSummary = document.getElementById("wizard-summary");
      const wizardBody = document.getElementById("wizard-body");
      const wizardError = document.getElementById("wizard-error");
      const wizardBackButton = document.getElementById("wizard-back-button");
      const wizardCancelButton = document.getElementById("wizard-cancel-button");
      const wizardSubmitButton = document.getElementById("wizard-submit-button");

      let launchWizard = null;
      let launchWizardOpenError = null;
      let launchWizardOpening = null;
      let wizardWasOpen = false;
      let wizardBranchDraft = "";
      let wizardBranchBackendValue = "";
      let launchWizardPendingAction = null;
      // Issue #2698 PR 1 (B7) — defer destructive wizard re-renders
      // while the user has a native <select> dropdown open. The OS
      // dropdown overlay is anchored to the original DOM node; if
      // renderLaunchWizard() swaps `wizardBody` mid-interaction, the
      // user's selection commit lands on a destroyed element and is
      // silently lost. `wizardInteractionGuard` coalesces inbound
      // `launch_wizard_state` / `launch_wizard_open_error` messages
      // while active and replays the latest pending event on release.
      const wizardInteractionGuard = createInteractionGuard({
        onFlush: (deferred) => {
          if (!deferred || typeof deferred !== "object") {
            return;
          }
          if (deferred.kind === "launch_wizard_state") {
            clearLaunchWizardPendingAction();
            clearLaunchWizardOpening();
            if (deferred.wizard) {
              launchWizardOpenError = null;
            }
            launchWizard = deferred.wizard;
          } else if (deferred.kind === "launch_wizard_open_error") {
            clearLaunchWizardPendingAction();
            clearLaunchWizardOpening();
            launchWizard = null;
            launchWizardOpenError = {
              title: deferred.title || "Launch Agent",
              message: deferred.message || "Unable to open Launch Wizard",
            };
          }
          renderLaunchWizard();
        },
      });
      let branchCleanupWindowId = null;

      function setLaunchWizardPendingDisabled(root, disabled) {
        if (!disabled) return;
        const selector =
          "input, textarea, select, button, [role='button'], [contenteditable='true']";
        for (const element of root.querySelectorAll(selector)) {
          if ("disabled" in element) {
            element.disabled = true;
          }
          element.setAttribute("aria-disabled", "true");
          element.setAttribute("tabindex", "-1");
        }
      }


      function createLaunchSection(title, copy) {
        const section = createNode("section", "launch-section");
        const header = createNode("div", "launch-section-header");
        const text = createNode("div");
        text.appendChild(createNode("div", "launch-section-title", title));
        if (copy) {
          text.appendChild(createNode("div", "launch-section-copy", copy));
        }
        header.appendChild(text);
        section.appendChild(header);
        return section;
      }

      function createLaunchField(label, wide = false) {
        const field = createNode(
          "div",
          wide ? "launch-field wide" : "launch-field",
        );
        field.appendChild(createNode("div", "launch-field-label", label));
        return field;
      }

      function createChoiceButton(option, selected, onSelect) {
        const button = createNode("button", "launch-choice-button");
        button.type = "button";
        // SPEC-2356 — choice buttons toggle between mutually-exclusive
        // options (which agent to launch / which preset). aria-pressed
        // exposes the toggled state so screen readers announce which
        // option is currently selected without relying on the visual
        // .selected class alone.
        button.setAttribute("aria-pressed", selected ? "true" : "false");
        if (selected) {
          button.classList.add("selected");
        }
        const title = createNode("span", "launch-choice-title");
        if (option.color) {
          button.dataset.agentColor = option.color;
          title.appendChild(createNode("span", "agent-dot"));
        }
        title.appendChild(document.createTextNode(option.label));
        button.appendChild(title);
        if (option.description) {
          button.appendChild(
            createNode("span", "launch-choice-detail", option.description),
          );
        }
        button.addEventListener("click", onSelect);
        return button;
      }

      function appendChoiceField(
        parent,
        label,
        options,
        selectedValue,
        onSelect,
        wide = false,
      ) {
        const field = createLaunchField(label, wide);
        const row = createNode("div", "launch-choice-row");
        for (const option of options) {
          row.appendChild(
            createChoiceButton(option, option.value === selectedValue, () =>
              onSelect(option.value),
            ),
          );
        }
        field.appendChild(row);
        parent.appendChild(field);
        return field;
      }

      function appendSelectField(
        parent,
        label,
        options,
        selectedValue,
        onChange,
        wide = false,
        emptyLabel = "Unavailable",
      ) {
        const field = createLaunchField(label, wide);
        const select = createNode("select", "launch-select");
        // SPEC-2356 — launch-field labels are non-<label> divs, so set
        // aria-label directly. Reuses the visible label text so screen
        // readers and visual users see the same name.
        select.setAttribute("aria-label", label);
        if (options.length === 0) {
          select.disabled = true;
          const option = document.createElement("option");
          option.value = "";
          option.textContent = emptyLabel;
          select.appendChild(option);
        } else {
          for (const item of options) {
            const option = document.createElement("option");
            option.value = item.value;
            option.textContent = item.label;
            select.appendChild(option);
          }
          const hasSelected = options.some((item) => item.value === selectedValue);
          select.value = hasSelected ? selectedValue : options[0].value;
          select.addEventListener("change", () => onChange(select.value));
        }
        field.appendChild(select);
        parent.appendChild(field);
        return field;
      }

      function appendCheckboxField(
        parent,
        label,
        copy,
        checked,
        onChange,
        wide = false,
      ) {
        const field = createLaunchField(label, wide);
        const checkboxLabel = createNode("label", "launch-inline-check");
        const input = document.createElement("input");
        input.type = "checkbox";
        input.checked = checked;
        input.addEventListener("change", () => onChange(input.checked));
        checkboxLabel.appendChild(input);
        checkboxLabel.appendChild(createNode("span", "", copy));
        field.appendChild(checkboxLabel);
        parent.appendChild(field);
        return field;
      }

      // SPEC-2014 2026-05-29 amendment — operation-appropriate controls.
      // These delegate to the standalone launch-controls.js builders so the
      // control logic stays unit-testable; the wizard action payloads are
      // unchanged from the prior native <select> / checkbox controls.
      function appendReasoningField(parent, label, options, selectedValue, onChange) {
        const field = buildReasoningField(document, {
          label,
          options,
          selectedValue,
          onChange,
        });
        parent.appendChild(field);
        return field;
      }

      function appendToggleField(parent, label, copy, checked, onChange, wide = false) {
        const field = buildToggleField(document, {
          label,
          copy,
          checked,
          onChange,
          wide,
        });
        parent.appendChild(field);
        return field;
      }

      function appendChoiceOrSelectField(
        parent,
        label,
        options,
        selectedValue,
        onChange,
        wide = false,
      ) {
        const field = buildChoiceOrSelectField(document, {
          label,
          options,
          selectedValue,
          onChange,
          wide,
        });
        parent.appendChild(field);
        return field;
      }

      function runtimeTargetPayload(value) {
        return value === "docker" ? "Docker" : "Host";
      }

      function dockerLifecyclePayload(value) {
        switch (value) {
          case "connect":
            return "Connect";
          case "start":
            return "Start";
          case "restart":
            return "Restart";
          case "recreate":
            return "Recreate";
          case "create_and_start":
            return "CreateAndStart";
          default:
            return "Connect";
        }
      }

      function syncWizardDraftState() {
        if (!launchWizard) {
          wizardWasOpen = false;
          wizardBranchDraft = "";
          wizardBranchBackendValue = "";
          return;
        }

        if (!wizardWasOpen) {
          wizardWasOpen = true;
          wizardBranchDraft = launchWizard.branch_name || "";
          wizardBranchBackendValue = wizardBranchDraft;
          return;
        }

        if ((launchWizard.branch_name || "") !== wizardBranchBackendValue) {
          wizardBranchDraft = launchWizard.branch_name || "";
          wizardBranchBackendValue = wizardBranchDraft;
        }
      }

      function flushWizardBranchDraft() {
        if (!launchWizard || launchWizard.branch_mode !== "create_new") {
          return;
        }
        if (wizardBranchDraft === wizardBranchBackendValue) {
          return;
        }
        wizardBranchBackendValue = wizardBranchDraft;
        sendWizardAction({
          kind: "set_branch_name",
          value: wizardBranchDraft,
        });
      }

      function renderWizardSummary() {
        wizardSummary.innerHTML = "";
        for (const item of launchWizard.launch_summary || []) {
          const card = createNode("div", "wizard-summary-item");
          card.appendChild(createNode("div", "wizard-summary-label", item.label));
          card.appendChild(createNode("div", "wizard-summary-value", item.value));
          wizardSummary.appendChild(card);
        }
      }

      // SPEC-2014 FR-128: progress rail step key → wizard phase. Clicking a
      // reachable step dispatches goto_step with the mapped phase so the
      // backend can jump the ManualSetup (Setup 3-step) wizard between
      // Path / Settings / Runtime / Confirm without re-walking each step.
      const WIZARD_RAIL_STEP_PHASE = Object.freeze({
        path: "path",
        setup: "settings",
        runtime: "runtime",
        start: "confirm",
      });

      function gotoWizardStep(phase) {
        if (
          !releaseWizardInteractionGuardForChromeAction()
          || launchWizardOpenError
          || launchWizardPendingAction
        ) {
          return;
        }
        flushWizardBranchDraft();
        sendWizardAction({ kind: "goto_step", phase });
      }

      function renderWizardProgressRail() {
        const rail = createNode("aside", "wizard-progress-rail");
        rail.setAttribute("aria-label", "Launch progress");
        for (const step of launchWizard.progress_steps || []) {
          const item = createNode("div", "wizard-progress-step");
          const state = step.state || "pending";
          item.dataset.state = state;
          // SPEC-2014 FR-128 — only reached steps (active/done) are
          // navigable; pending steps stay inert. A null phase mapping
          // (unknown key) also stays inert.
          const targetPhase = WIZARD_RAIL_STEP_PHASE[step.key];
          const isClickable =
            Boolean(targetPhase) && (state === "active" || state === "done");
          if (isClickable) {
            item.dataset.clickable = "true";
            item.setAttribute("role", "button");
            item.setAttribute("tabindex", "0");
            item.setAttribute(
              "aria-label",
              `Go to ${step.label} step`,
            );
            const jump = () => gotoWizardStep(targetPhase);
            item.addEventListener("click", jump);
            item.addEventListener("keydown", (event) => {
              if (event.key === "Enter" || event.key === " " || event.key === "Spacebar") {
                event.preventDefault();
                jump();
              }
            });
          }
          item.appendChild(createNode("span", "wizard-progress-marker"));
          const copy = createNode("span", "wizard-progress-copy");
          copy.appendChild(createNode("span", "wizard-progress-label", step.label));
          if (step.detail) {
            copy.appendChild(createNode("span", "wizard-progress-detail", step.detail));
          }
          item.appendChild(copy);
          rail.appendChild(item);
        }
        return rail;
      }

      let wizardFocusReturn = null;
      let wizardFocusTrapRelease = null;

      function closeLaunchWizardLocal() {
        clearLaunchWizardPendingAction();
        clearLaunchWizardOpening();
        launchWizard = null;
        launchWizardOpenError = null;
        // Issue #2698 PR 1 (B7) — local close wins over any pending
        // backend state. Discard (do not replay) the deferred event
        // so we don't undo the user-initiated close.
        wizardInteractionGuard.discard();
        renderLaunchWizard();
      }

      function releaseWizardInteractionGuardForChromeAction() {
        if (wizardInteractionGuard.isActive()) {
          wizardInteractionGuard.release();
        }
        return Boolean(launchWizard || launchWizardOpenError);
      }

      function setLaunchWizardPendingAction(action) {
        launchWizardPendingAction = action || null;
        renderLaunchWizard();
      }

      function clearLaunchWizardPendingAction() {
        launchWizardPendingAction = null;
      }

      function openStartWorkPendingWizard() {
        clearLaunchWizardPendingAction();
        launchWizard = null;
        launchWizardOpenError = null;
        launchWizardOpening = {
          title: "Start Work",
          meta: "Work launch",
          message: "Preparing Start Work...",
        };
        renderLaunchWizard();
      }

      function clearLaunchWizardOpening() {
        launchWizardOpening = null;
      }

      function syncLaunchWizardPendingChrome(isPending) {
        wizardModal.classList.toggle("is-launch-pending", isPending);
        if (wizardDialog) {
          wizardDialog.classList.toggle("is-launch-pending", isPending);
          wizardDialog.setAttribute("aria-busy", isPending ? "true" : "false");
        }
      }

      function closeLaunchWizardFromChrome() {
        if (!releaseWizardInteractionGuardForChromeAction()) {
          return;
        }
        clearLaunchWizardPendingAction();
        if (launchWizardOpenError) {
          closeLaunchWizardLocal();
          return;
        }
        sendWizardAction({ kind: "cancel" });
      }

      function isPrimaryPointerActivation(event) {
        return !event || event.button === 0 || event.button === undefined;
      }

      function handleLaunchWizardSubmitFromChrome() {
        if (
          !releaseWizardInteractionGuardForChromeAction()
          || launchWizardOpenError
          || wizardSubmitButton.disabled
        ) {
          return;
        }
        flushWizardBranchDraft();
        setLaunchWizardPendingAction({ kind: "submit" });
        sendWizardAction({ kind: "submit" });
      }

      function renderLaunchWizard() {
        if (!launchWizard && !launchWizardOpenError && !launchWizardOpening) {
          clearLaunchWizardPendingAction();
          syncLaunchWizardPendingChrome(false);
          const wasOpenBeforeClose = wizardModal.classList.contains("open");
          wizardModal.classList.remove("open");
          // SPEC-2356 — keep aria-hidden in lockstep with .open so screen
          // readers stop announcing the wizard when it slides closed.
          wizardModal.setAttribute("aria-hidden", "true");
          // SPEC-2356 — release the focus trap before restoring focus so
          // the trap doesn't intercept the focus move and pull it back in.
          if (wasOpenBeforeClose && typeof wizardFocusTrapRelease === "function") {
            wizardFocusTrapRelease();
            wizardFocusTrapRelease = null;
          }
          // SPEC-2356 — restore focus to the trigger that opened the wizard
          // so keyboard users land back on Start Work / Launch Agent / etc.
          if (wasOpenBeforeClose && wizardFocusReturn && typeof wizardFocusReturn.focus === "function") {
            try { wizardFocusReturn.focus({ preventScroll: true }); }
            catch { wizardFocusReturn.focus(); }
            wizardFocusReturn = null;
          }
          wizardSummary.innerHTML = "";
          wizardBody.innerHTML = "";
          wizardError.hidden = true;
          wizardError.textContent = "";
          if (wizardTitle) wizardTitle.textContent = "Launch Agent";
          wizardSubmitButton.textContent = "Launch";
          wizardSubmitButton.disabled = false;
          wizardSubmitButton.hidden = false;
          wizardBackButton.hidden = true;
          wizardBackButton.disabled = false;
          wizardCancelButton.textContent = "Cancel";
          wizardCancelButton.disabled = false;
          syncWizardDraftState();
          return;
        }

        syncWizardDraftState();
        closeModal();
        const isLaunchActionPending = Boolean(launchWizardPendingAction);
        const isLaunchOpeningPending = Boolean(launchWizardOpening);
        const isLaunchSubmitPending = launchWizardPendingAction?.kind === "submit";
        syncLaunchWizardPendingChrome(isLaunchActionPending || isLaunchOpeningPending);
        const wasOpenWizard = wizardModal.classList.contains("open");
        if (!wasOpenWizard) {
          // Capture trigger BEFORE flipping .open so render-driven focus
          // moves don't overwrite our save.
          wizardFocusReturn = document.activeElement;
        }
        wizardModal.classList.add("open");
        wizardModal.removeAttribute("aria-hidden");
        if (!wasOpenWizard && wizardDialog && typeof wizardDialog.focus === "function") {
          // SPEC-2356 — move focus into the dialog so screen readers
          // announce "Launch Agent dialog" and keyboard users land inside.
          try { wizardDialog.focus({ preventScroll: true }); }
          catch { wizardDialog.focus(); }
        }
        if (!wasOpenWizard && wizardDialog) {
          // SPEC-2356 — trap Tab inside the wizard while it's open so
          // keyboard users can't escape into background content.
          wizardFocusTrapRelease = createFocusTrap(wizardDialog, { document });
        }

        if (launchWizardOpening) {
          if (wizardTitle) {
            wizardTitle.textContent = launchWizardOpening.title || "Start Work";
          }
          wizardMeta.textContent = launchWizardOpening.meta || "Work launch";
          wizardBackButton.hidden = true;
          wizardBackButton.disabled = true;
          wizardSubmitButton.hidden = true;
          wizardSubmitButton.disabled = true;
          wizardCancelButton.textContent = "Cancel";
          wizardCancelButton.disabled = true;
          wizardError.hidden = true;
          wizardError.textContent = "";
          wizardSummary.innerHTML = "";
          wizardBody.innerHTML = "";
          const openingPanel = createNode("div", "launch-panel wizard-disabled");
          openingPanel.appendChild(
            createNode(
              "div",
              "launch-note launch-pending-note",
              launchWizardOpening.message || "Preparing Start Work...",
            ),
          );
          wizardBody.appendChild(openingPanel);
          return;
        }

        if (launchWizardOpenError) {
          if (wizardTitle) {
            wizardTitle.textContent = launchWizardOpenError.title || "Launch Agent";
          }
          wizardMeta.textContent =
            launchWizardOpenError.title === "Start Work"
              ? "Work launch"
              : "Launch Agent";
          wizardBackButton.hidden = true;
          wizardBackButton.disabled = false;
          wizardSubmitButton.hidden = true;
          wizardSubmitButton.disabled = true;
          wizardCancelButton.textContent = "Close";
          wizardCancelButton.disabled = false;
          wizardError.hidden = false;
          wizardError.textContent =
            launchWizardOpenError.message || "Unable to open Launch Wizard";
          wizardSummary.innerHTML = "";
          wizardBody.innerHTML = "";
          return;
        }

        wizardSubmitButton.hidden = false;
        wizardBackButton.hidden = !launchWizard.show_back_button;
        wizardBackButton.disabled = Boolean(
          isLaunchActionPending
            || launchWizard.is_hydrating
            || launchWizard.runtime_resolution_pending
            || !launchWizard.show_back_button,
        );
        wizardCancelButton.textContent = "Cancel";
        if (wizardTitle) wizardTitle.textContent = launchWizard.title || "Launch Agent";
        wizardMeta.textContent = launchWizard.show_branch_controls === false
          ? "Work launch"
          : `Selected branch · ${
            launchWizard.selected_branch_name || launchWizard.branch_name || "Work"
          }`;
        wizardSubmitButton.textContent = isLaunchSubmitPending
          ? "Launching..."
          : launchWizard.primary_action_label || (
          launchWizard.is_hydrating
            ? "Loading..."
            : launchWizard.runtime_context_resolved === false
              ? "Continue"
              : launchWizard.branch_mode === "create_new"
                ? "Create and launch"
                : "Launch"
        );
        wizardSubmitButton.disabled = Boolean(
          isLaunchActionPending
            || launchWizard.is_hydrating
            || launchWizard.runtime_resolution_pending
            || launchWizard.primary_action_enabled === false,
        );
        wizardCancelButton.disabled = false;

        if (launchWizard.error || launchWizard.hydration_error) {
          wizardError.hidden = false;
          wizardError.textContent =
            launchWizard.error || launchWizard.hydration_error;
        } else {
          wizardError.hidden = true;
          wizardError.textContent = "";
        }

        renderWizardSummary();
        // SPEC-2014 FR-126/FR-127 — the backend now drives four mutually
        // exclusive wizard phases through dedicated flags:
        //   show_manual_setup       → Settings form
        //   show_runtime_confirmation→ Runtime step
        //   show_confirm             → Confirm (read-only summary + Launch)
        //   show_start_methods       → entry (start method picker)
        // The backend already strictly clears show_manual_setup during
        // Runtime / Confirm, but we re-derive exclusive locals here so the
        // renderer never paints two phases at once.
        const showConfirm = Boolean(launchWizard.show_confirm);
        const isRuntimeConfirmation = Boolean(
          launchWizard.runtime_context_resolved
          && launchWizard.show_runtime_confirmation
          && !showConfirm
        );
        const showManualSetup =
          launchWizard.show_manual_setup !== false
          && !isRuntimeConfirmation
          && !showConfirm;
        const showStartMethods = Boolean(
          launchWizard.show_start_methods
            && !isRuntimeConfirmation
            && !showConfirm
            && !launchWizard.runtime_resolution_pending
            && (launchWizard.start_methods || []).length > 0,
        );
        const showSetupForms = showManualSetup && !isRuntimeConfirmation;
        wizardBody.innerHTML = "";
        const wizardMain = createNode("div", "wizard-main");
        wizardMain.appendChild(renderWizardProgressRail());
        const wizardContentPane = createNode("div", "wizard-content-pane");
        const panel = createNode("div", "launch-panel");
        const isRuntimeResolutionPending = Boolean(launchWizard.runtime_resolution_pending);
        panel.classList.toggle(
          "wizard-disabled",
          isRuntimeResolutionPending || isLaunchActionPending,
        );
        if (launchWizard.is_hydrating) {
          panel.appendChild(
            createNode(
              "div",
              "launch-note",
              "Loading branch workspace, recent sessions, and Docker options...",
            ),
          );
        }
        if (launchWizard.runtime_resolution_pending) {
          panel.appendChild(
            createNode(
              "div",
              "launch-note",
              launchWizard.runtime_resolution_message || "Preparing runtime...",
            ),
          );
        }
        if (isLaunchSubmitPending) {
          panel.appendChild(
            createNode(
              "div",
              "launch-note launch-pending-note",
              "Creating agent window...",
            ),
          );
        }

        // SPEC-2014 FR-127 — Confirm step: a read-only review of the
        // resolved launch configuration plus the footer Launch button.
        // No editable controls are rendered here; the user revisits an
        // earlier phase (via the progress rail or Back) to change anything.
        if (showConfirm) {
          const section = createLaunchSection(
            "Confirm",
            "Review the launch configuration. Use the steps above or Back to change anything.",
          );
          const summaryList = createNode("div", "wizard-confirm-summary");
          for (const item of launchWizard.launch_summary || []) {
            const card = createNode("div", "wizard-summary-item");
            card.appendChild(createNode("div", "wizard-summary-label", item.label));
            card.appendChild(createNode("div", "wizard-summary-value", item.value));
            summaryList.appendChild(card);
          }
          section.appendChild(summaryList);
          panel.appendChild(section);
        }

        if (showStartMethods) {
          const section = createLaunchSection(
            "Start methods",
            "Choose how this agent should start on the selected branch.",
          );

          const methodList = createNode("div", "start-method-list");
          for (const method of launchWizard.start_methods || []) {
            const button = createNode("button", "start-method-button");
            button.type = "button";
            const isStartMethodPending =
              launchWizardPendingAction?.kind === "use_start_method"
                && launchWizardPendingAction.method === method.kind;
            button.classList.toggle("is-pending", isStartMethodPending);
            button.disabled = method.enabled === false || isLaunchActionPending;
            const head = createNode("div", "start-method-head");
            head.appendChild(
              createNode(
                "div",
                "start-method-title",
                isStartMethodPending ? "Preparing..." : method.label,
              ),
            );
            if (method.badge) {
              head.appendChild(createNode("div", "start-method-badge", method.badge));
            }
            button.appendChild(head);
            button.appendChild(
              createNode("div", "start-method-summary", method.summary || ""),
            );
            const detail = method.enabled === false
              ? method.disabled_reason
              : method.detail;
            if (detail) {
              button.appendChild(createNode("div", "start-method-detail", detail));
            }
            const handleStartMethodLaunchAction = () => {
              if (
                !releaseWizardInteractionGuardForChromeAction()
                || button.disabled
                || launchWizardPendingAction
              ) {
                return;
              }
              setLaunchWizardPendingAction({
                kind: "use_start_method",
                method: method.kind,
              });
              sendWizardAction({
                kind: "use_start_method",
                method: method.kind,
              });
            };
            button.addEventListener("pointerup", (event) => {
              if (!isPrimaryPointerActivation(event)) {
                return;
              }
              event.preventDefault();
              handleStartMethodLaunchAction();
            });
            button.addEventListener("click", () => {
              handleStartMethodLaunchAction();
            });
            methodList.appendChild(button);
          }
          section.appendChild(methodList);

          panel.appendChild(section);
        }

        if (showSetupForms && launchWizard.show_branch_controls !== false) {
          const section = createLaunchSection(
            "Branch",
            "Choose the selected branch or create a new branch from it.",
          );
          const grid = createNode("div", "launch-form-grid");
          appendChoiceField(
            grid,
            "Branch target",
            [
              {
                value: "use_selected",
                label: "Use selected",
                description:
                  launchWizard.selected_branch_name || "Launch on the selected branch",
              },
              {
                value: "create_new",
                label: "Create new",
                description: `Base · ${
                  launchWizard.selected_branch_name || "selected branch"
                }`,
              },
            ],
            launchWizard.branch_mode,
            (value) => {
              sendWizardAction({
                kind: "set_branch_mode",
                create_new: value === "create_new",
              });
            },
            true,
          );

          if (launchWizard.branch_mode === "create_new") {
            appendChoiceField(
              grid,
              "Branch type",
              launchWizard.branch_type_options || [],
              launchWizard.selected_branch_type,
              (value) => {
                flushWizardBranchDraft();
                sendWizardAction({
                  kind: "set_branch_type",
                  prefix: value,
                });
              },
              true,
            );
            const field = createLaunchField("Branch name", true);
            const input = createNode("input", "launch-input");
            input.type = "text";
            // SPEC-2356 — launch-field labels are <div>s (not <label>s)
            // so screen readers can't programmatically associate them
            // with the input. Set aria-label directly so the input
            // announces with its purpose ("Branch name, edit text").
            input.setAttribute("aria-label", "Branch name");
            input.value = wizardBranchDraft;
            input.placeholder = "feature/my-task";
            input.addEventListener("input", () => {
              wizardBranchDraft = input.value;
            });
            input.addEventListener("blur", () => {
              flushWizardBranchDraft();
            });
            field.appendChild(input);
            field.appendChild(
              createNode(
                "div",
                "launch-field-help",
                `Base branch · ${launchWizard.selected_branch_name || "selected"}`,
              ),
            );
            grid.appendChild(field);
          } else {
            const note = createLaunchField("Resolved target", true);
            note.appendChild(
              createNode(
                "div",
                "launch-note",
                launchWizard.selected_branch_name || launchWizard.branch_name,
              ),
            );
            grid.appendChild(note);
          }

          section.appendChild(grid);
          panel.appendChild(section);
        }

        if (showSetupForms) {
          const section = createLaunchSection(
            "Launch",
            "Choose what to launch on the selected branch.",
          );
          const grid = createNode("div", "launch-form-grid");
          appendChoiceOrSelectField(
            grid,
            "Target",
            launchWizard.launch_target_options || [],
            launchWizard.selected_launch_target,
            (value) =>
              sendWizardAction({
                kind: "set_launch_target",
                target: value === "shell" ? "shell" : "agent",
              }),
          );
          if (launchWizard.show_agent_settings) {
            appendChoiceOrSelectField(
              grid,
              "Agent",
              launchWizard.agent_options || [],
              launchWizard.selected_agent_id,
              (value) =>
                sendWizardAction({
                  kind: "set_agent",
                  agent_id: value,
                }),
            );
            if ((launchWizard.model_options || []).length > 0) {
              appendSelectField(
                grid,
                "Model",
                launchWizard.model_options || [],
                launchWizard.selected_model,
                (value) =>
                  sendWizardAction({
                    kind: "set_model",
                    model: value,
                  }),
              );
            }
            if (launchWizard.show_reasoning) {
              appendReasoningField(
                grid,
                "Reasoning",
                launchWizard.reasoning_options || [],
                launchWizard.selected_reasoning,
                (value) =>
                  sendWizardAction({
                    kind: "set_reasoning",
                    reasoning: value,
                  }),
              );
            }
          } else {
            const note = createLaunchField("Shell", true);
            note.appendChild(
              createNode(
                "div",
                "launch-note",
                "Open a plain shell in the selected branch and runtime.",
              ),
            );
            grid.appendChild(note);
          }
          if (launchWizard.show_windows_shell) {
            appendChoiceOrSelectField(
              grid,
              "Shell",
              launchWizard.windows_shell_options || [],
              launchWizard.selected_windows_shell,
              (value) =>
                sendWizardAction({
                  kind: "set_windows_shell",
                  shell: value,
                }),
            );
          }
          section.appendChild(grid);
          panel.appendChild(section);
        }

        if (
          showSetupForms &&
          (
            launchWizard.show_version ||
            launchWizard.show_skip_permissions ||
            launchWizard.show_fast_mode ||
            launchWizard.show_codex_fast_mode
          )
        ) {
          const showFastMode = Boolean(
            launchWizard.show_fast_mode ?? launchWizard.show_codex_fast_mode,
          );
          const section = createLaunchSection(
            "Launch settings",
            "Version, permissions, and tool-specific launch behavior.",
          );
          const grid = createNode("div", "launch-form-grid");
          if (launchWizard.show_version) {
            appendSelectField(
              grid,
              "Version",
              launchWizard.version_options || [],
              launchWizard.selected_version,
              (value) =>
                sendWizardAction({
                  kind: "set_version",
                  version: value,
                }),
            );
          }
          if (launchWizard.show_skip_permissions) {
            appendToggleField(
              grid,
              "Permissions",
              "Skip permission prompts",
              launchWizard.skip_permissions,
              (enabled) =>
                sendWizardAction({
                  kind: "set_skip_permissions",
                  enabled,
                }),
            );
          }
          if (showFastMode) {
            appendToggleField(
              grid,
              "Fast mode",
              "Use the agent's Fast mode",
              Boolean(launchWizard.fast_mode ?? launchWizard.codex_fast_mode),
              (enabled) =>
                sendWizardAction({
                  kind: "set_fast_mode",
                  enabled,
                }),
            );
          }
          section.appendChild(grid);
          panel.appendChild(section);
        }

        // SPEC-2014 Amendment 2026-05-20 (FR-057 / FR-058):
        // The Linked issue section renders only when the wizard was opened
        // through the Knowledge Issue Bridge. The issue number is shown as
        // read-only text instead of an editable input.
        if (showSetupForms && launchWizard.show_linked_issue) {
          const section = createLaunchSection(
            "Linked issue",
            "Read-only: this agent will be linked to the originating issue.",
          );
          const grid = createNode("div", "launch-form-grid");
          const field = createLaunchField("Issue number", false);
          // The launch-field label already announces "Issue number"; the
          // static value div is read alongside that label so SR users hear
          // "Issue number, #N" without needing a per-value aria-label.
          const value = createNode("div", "launch-static-value");
          value.textContent = `#${launchWizard.linked_issue_number}`;
          field.appendChild(value);
          grid.appendChild(field);
          section.appendChild(grid);
          panel.appendChild(section);
        }

        const hasRuntimeControls =
          launchWizard.show_runtime_target ||
          (launchWizard.show_docker_service &&
            (launchWizard.docker_service_options || []).length > 0) ||
          (launchWizard.show_docker_lifecycle &&
            (launchWizard.docker_lifecycle_options || []).length > 0);
        if (
          launchWizard.show_runtime_confirmation &&
          !showConfirm &&
          (hasRuntimeControls || isRuntimeConfirmation || !showManualSetup)
        ) {
          const section = createLaunchSection(
            "Runtime",
            "Choose where the session runs and how Docker services are used.",
          );
          const grid = createNode("div", "launch-form-grid");
          let appendedRuntimeControl = false;
          if (launchWizard.show_runtime_target) {
            appendChoiceOrSelectField(
              grid,
              "Runtime target",
              launchWizard.runtime_target_options || [],
              launchWizard.selected_runtime_target,
              (value) =>
                sendWizardAction({
                  kind: "set_runtime_target",
                  target: runtimeTargetPayload(value),
                }),
            );
            appendedRuntimeControl = true;
          }
          if (
            launchWizard.show_docker_service &&
            (launchWizard.docker_service_options || []).length > 0
          ) {
            appendSelectField(
              grid,
              "Docker service",
              launchWizard.docker_service_options || [],
              launchWizard.selected_docker_service,
              (value) =>
                sendWizardAction({
                  kind: "set_docker_service",
                  service: value,
                }),
            );
            appendedRuntimeControl = true;
          }
          if (
            launchWizard.show_docker_lifecycle &&
            (launchWizard.docker_lifecycle_options || []).length > 0
          ) {
            appendSelectField(
              grid,
              "Docker lifecycle",
              launchWizard.docker_lifecycle_options || [],
              launchWizard.selected_docker_lifecycle,
              (value) =>
                sendWizardAction({
                  kind: "set_docker_lifecycle",
                  intent: dockerLifecyclePayload(value),
                }),
            );
            appendedRuntimeControl = true;
          }
          if (!appendedRuntimeControl) {
            const note = createLaunchField("Runtime target", true);
            note.appendChild(
              createNode(
                "div",
                "launch-note",
                launchWizard.selected_runtime_target === "docker"
                  ? "Docker"
                  : "Host",
              ),
            );
            grid.appendChild(note);
          }
          section.appendChild(grid);

          panel.appendChild(section);
        }

        setLaunchWizardPendingDisabled(
          panel,
          isRuntimeResolutionPending || isLaunchActionPending,
        );
        wizardContentPane.appendChild(panel);
        wizardMain.appendChild(wizardContentPane);
        wizardBody.appendChild(wizardMain);
      }

      // SPEC-3064 Phase 3 (E5): receive() delegates. The guard defer and
      // the wizard state mutation live here because launchWizard /
      // launchWizardOpenError are module-level lets now.
      function applyLaunchWizardStateEvent(event) {
        // Issue #2698 PR 1 (B7) — defer when user is mid-dropdown.
        if (
          wizardInteractionGuard.defer({
            kind: "launch_wizard_state",
            wizard: event.wizard,
          })
        ) {
          return;
        }
        clearLaunchWizardPendingAction();
        clearLaunchWizardOpening();
        if (event.wizard) {
          launchWizardOpenError = null;
        }
        launchWizard = event.wizard;
        renderLaunchWizard();
      }

      function applyLaunchWizardOpenErrorEvent(event) {
        // Issue #2698 PR 1 (B7) — defer when user is mid-dropdown.
        if (
          wizardInteractionGuard.defer({
            kind: "launch_wizard_open_error",
            title: event.title,
            message: event.message,
          })
        ) {
          return;
        }
        clearLaunchWizardPendingAction();
        clearLaunchWizardOpening();
        launchWizard = null;
        launchWizardOpenError = {
          title: event.title || "Launch Agent",
          message: event.message || "Unable to open Launch Wizard",
        };
        renderLaunchWizard();
      }

      // SPEC-3064 Phase 3 (E5): wizard branch of the global Esc handler.
      // preventDefaults and returns true when the wizard modal consumed the
      // event; app.js falls through to the other modals when it returns
      // false.
      function handleWizardEscapeKeydown(event) {
        if (wizardModal.classList.contains("open")) {
          // Wizard cancel is the explicit cancellation path; map Esc to
          // the same action so the modal isn't a keyboard trap.
          if (!releaseWizardInteractionGuardForChromeAction()) {
            event.preventDefault();
            return true;
          }
          if (launchWizardOpenError) {
            closeLaunchWizardLocal();
          } else {
            sendWizardAction({ kind: "cancel" });
          }
          event.preventDefault();
          return true;
        }
        return false;
      }

      // SPEC-3064 Phase 3 (E5): wizard chrome wiring (footer buttons +
      // interaction-guard listeners), called once from app.js boot.
      function installWizardChrome() {
      wizardCancelButton.addEventListener("click", closeLaunchWizardFromChrome);
      wizardBackButton.addEventListener("click", () => {
        if (
          !releaseWizardInteractionGuardForChromeAction()
          || launchWizardOpenError
          || wizardBackButton.disabled
        ) {
          return;
        }
        flushWizardBranchDraft();
        sendWizardAction({ kind: "back" });
      });
      wizardSubmitButton.addEventListener("pointerup", (event) => {
        if (!isPrimaryPointerActivation(event)) {
          return;
        }
        event.preventDefault();
        handleLaunchWizardSubmitFromChrome();
      });
      wizardSubmitButton.addEventListener("click", handleLaunchWizardSubmitFromChrome);
      // Issue #2698 PR 1 (B7) — interaction-guard wiring for native
      // <select> dropdowns inside the wizard body. We register
      // delegated listeners on `wizardBody` so they survive the
      // destructive re-render of its children. Activation on
      // pointerdown matches when the OS overlay opens; release on
      // `change` (commit), `focusout` (cancel), and `Escape` covers
      // every common termination path.
      // SPEC-2014 2026-05-29: the reasoning slider (`.launch-range__input`)
      // needs the same guard. Each value change commits set_reasoning, and the
      // backend echoes launch_wizard_state; without deferral that re-render
      // destroys and recreates the slider mid-drag (breaking the drag) and
      // drops keyboard focus between Arrow steps. Activate on focusin (covers
      // mouse press and keyboard tab-in) and release on focusout so re-renders
      // are coalesced for the whole interaction, not committed on every step.
      const isGuardedRange = (el) =>
        Boolean(el && el.classList && el.classList.contains("launch-range__input"));
      wizardBody.addEventListener("pointerdown", (event) => {
        const target = event.target;
        if (target && (target.tagName === "SELECT" || isGuardedRange(target))) {
          wizardInteractionGuard.activate();
        }
      });
      wizardBody.addEventListener("focusin", (event) => {
        if (isGuardedRange(event.target)) {
          wizardInteractionGuard.activate();
        }
      });
      wizardBody.addEventListener("change", (event) => {
        const target = event.target;
        // <select> commits on change; the range keeps the guard active across
        // multiple Arrow steps / the post-drag focused state until focusout.
        if (target && target.tagName === "SELECT") {
          wizardInteractionGuard.release();
        }
      });
      wizardBody.addEventListener("focusout", (event) => {
        const target = event.target;
        if (target && (target.tagName === "SELECT" || isGuardedRange(target))) {
          wizardInteractionGuard.release();
        }
      });
      wizardModal.addEventListener("keydown", (event) => {
        if (event.key === "Escape" && wizardInteractionGuard.isActive()) {
          wizardInteractionGuard.release();
        }
      });
      }

      return {
        syncWizardDraftState,
        flushWizardBranchDraft,
        renderLaunchWizard,
        openStartWorkPendingWizard,
        applyLaunchWizardStateEvent,
        applyLaunchWizardOpenErrorEvent,
        handleWizardEscapeKeydown,
        installWizardChrome,
      };
}
