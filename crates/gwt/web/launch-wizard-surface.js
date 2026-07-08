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
  requestWorkAdvisory,
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

      // SPEC-2359 US-83: the launch-facing wizard abstracts the remote/local
      // distinction — show plain branch names ("feature-foo") by stripping the
      // `origin/` prefix. Display-only: the action payload keeps the raw name
      // (the backend `select_existing_branch` normalizes either form).
      const displayBranchName = (name) =>
        typeof name === "string" ? name.replace(/^origin\//, "") : name;

      let launchWizard = null;
      let launchWizardOpenError = null;
      let launchWizardOpening = null;
      let wizardWasOpen = false;
      let wizardBranchDraft = "";
      let wizardBranchBackendValue = "";
      let launchWizardPendingAction = null;
      // SPEC-2359 US-80 — Start Work duplicate-work advisory. The intake prompt
      // is always skippable; these locals drive the debounced query and the
      // non-blocking results panel. `wizardAdvisoryLatestRequestId` guards
      // against stale responses arriving out of order.
      let wizardPromptDraft = "";
      let wizardPromptBackendValue = "";
      let wizardAdvisoryResults = [];
      let wizardAdvisoryRequestSeq = 0;
      let wizardAdvisoryLatestRequestId = 0;
      let wizardAdvisoryTimer = 0;
      // The semantic search behind the advisory cold-loads the embedding model
      // (~several seconds), so show an in-flight indicator instead of an empty
      // panel while a non-empty prompt is being searched.
      let wizardAdvisoryLoading = false;
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
            if (!deferred.wizard?.launch_materialization_pending) {
              clearLaunchWizardPendingAction();
            }
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

      // SPEC-3152: free-text / numeric launch option (Hermes option fields).
      // Dispatches on change (blur/enter) so typing does not re-render mid-edit.
      function appendTextField(
        parent,
        label,
        value,
        placeholder,
        onChange,
        wide = false,
        inputType = "text",
      ) {
        const field = createLaunchField(label, wide);
        const input = document.createElement("input");
        input.type = inputType;
        input.className = "launch-input";
        input.setAttribute("aria-label", label);
        if (placeholder) {
          input.placeholder = placeholder;
        }
        input.value = value || "";
        input.addEventListener("change", () => onChange(input.value));
        field.appendChild(input);
        parent.appendChild(field);
        return field;
      }

      // SPEC-3152: Hermes provider picker. The choices are enumerated from the
      // user's own ~/.hermes/config.yaml (model.provider + providers: keys) and
      // passed in via launchWizard.hermes_provider_options — gwt does not
      // hardcode a provider list (it would go stale). A leading "use config
      // default" option and a trailing "Other…" free-text entry cover the
      // default path and the long tail (built-ins not present in config).
      function appendHermesProviderField(parent, currentValue, choices, onChange) {
        const providers = Array.isArray(choices) ? choices : [];
        const field = createLaunchField("Provider", false);
        const select = createNode("select", "launch-select");
        select.setAttribute("aria-label", "Provider");
        const addOption = (value, label) => {
          const option = document.createElement("option");
          option.value = value;
          option.textContent = label;
          select.appendChild(option);
        };
        addOption("", "(use config default)");
        for (const provider of providers) {
          addOption(provider, provider);
        }
        addOption("__other__", "Other…");
        const isKnown = Boolean(currentValue) && providers.includes(currentValue);
        const isOther = Boolean(currentValue) && !isKnown;
        select.value = currentValue ? (isKnown ? currentValue : "__other__") : "";

        const otherInput = document.createElement("input");
        otherInput.type = "text";
        otherInput.className = "launch-input";
        otherInput.placeholder = "custom provider id";
        otherInput.setAttribute("aria-label", "Custom provider");
        otherInput.value = isOther ? currentValue : "";
        otherInput.style.display = isOther ? "" : "none";
        otherInput.style.marginTop = "6px";
        otherInput.addEventListener("change", () => onChange(otherInput.value));

        select.addEventListener("change", () => {
          if (select.value === "__other__") {
            otherInput.style.display = "";
            otherInput.focus();
          } else {
            otherInput.style.display = "none";
            onChange(select.value);
          }
        });
        field.appendChild(select);
        field.appendChild(otherInput);
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

      // SPEC-2359 US-80 — is this a Start Work launch (where the work branch is
      // auto-created)? Start Work hides the branch controls; that is the signal
      // for showing the optional intake prompt + duplicate-work advisory.
      function isStartWorkLaunch() {
        return Boolean(launchWizard) && launchWizard.show_branch_controls === false;
      }

      function flushWizardPromptDraft() {
        if (!launchWizard || wizardPromptDraft === wizardPromptBackendValue) {
          return;
        }
        wizardPromptBackendValue = wizardPromptDraft;
        sendWizardAction({
          kind: "set_initial_prompt",
          value: wizardPromptDraft,
        });
      }

      function scheduleWorkAdvisory() {
        if (typeof requestWorkAdvisory !== "function") {
          return;
        }
        if (wizardAdvisoryTimer) {
          clearTimeout(wizardAdvisoryTimer);
          wizardAdvisoryTimer = 0;
        }
        const query = wizardPromptDraft.trim();
        if (!query) {
          // Empty prompt → quiet advisory (skippable, no noise). Advance the
          // request id so any response still in flight for previously typed
          // text is treated as stale and cannot repopulate the cleared panel.
          wizardAdvisoryRequestSeq += 1;
          wizardAdvisoryLatestRequestId = wizardAdvisoryRequestSeq;
          wizardAdvisoryLoading = false;
          wizardAdvisoryResults = [];
          renderWorkAdvisoryPanel();
          return;
        }
        // Show the in-flight indicator immediately so the multi-second model
        // load never looks like a frozen/unresponsive panel.
        wizardAdvisoryLoading = true;
        renderWorkAdvisoryPanel();
        wizardAdvisoryTimer = setTimeout(() => {
          wizardAdvisoryTimer = 0;
          wizardAdvisoryRequestSeq += 1;
          wizardAdvisoryLatestRequestId = wizardAdvisoryRequestSeq;
          requestWorkAdvisory({
            id: "launch-wizard",
            query,
            request_id: wizardAdvisoryRequestSeq,
          });
        }, 280);
      }

      // SPEC-2359 US-80 — apply a duplicate-work advisory result. Ignore stale
      // responses (older than the latest in-flight request); empty results keep
      // the panel quiet.
      function applyWorkAdvisoryResultEvent(event) {
        if (!event || typeof event !== "object") {
          return;
        }
        if (
          typeof event.request_id === "number"
          && event.request_id < wizardAdvisoryLatestRequestId
        ) {
          return;
        }
        wizardAdvisoryLoading = false;
        wizardAdvisoryResults = Array.isArray(event.results) ? event.results : [];
        renderWorkAdvisoryPanel();
      }

      function renderWorkAdvisoryPanel() {
        const panel = document.getElementById("wizard-work-advisory");
        if (!panel) {
          return;
        }
        panel.innerHTML = "";
        if (wizardAdvisoryLoading) {
          // In-flight: keep the panel visible with an animated indicator so the
          // multi-second semantic search never reads as an unresponsive UI.
          // Reuses the project index search loading-dots animation.
          panel.hidden = false;
          const loading = createNode("div", "launch-advisory-loading");
          const dots = createNode("span", "index-search-loading-dots");
          for (let i = 0; i < 3; i += 1) {
            dots.appendChild(createNode("span", "index-search-loading-dot"));
          }
          loading.appendChild(dots);
          loading.appendChild(
            createNode("span", "launch-advisory-loading-text", "Searching related work…"),
          );
          panel.appendChild(loading);
          return;
        }
        if (!wizardAdvisoryResults.length) {
          panel.hidden = true;
          return;
        }
        panel.hidden = false;
        panel.appendChild(
          createNode(
            "div",
            "launch-advisory-title",
            "Related prior work — review before starting",
          ),
        );
        for (const result of wizardAdvisoryResults.slice(0, 5)) {
          const row = createNode("div", "launch-advisory-item");
          row.appendChild(
            createNode("span", "launch-advisory-scope", result.scope || "work"),
          );
          row.appendChild(
            createNode("span", "launch-advisory-item-title", result.title || ""),
          );
          if (result.subtitle) {
            row.appendChild(
              createNode("span", "launch-advisory-item-sub", result.subtitle),
            );
          }
          panel.appendChild(row);
        }
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

      function openLaunchPendingWizard({ title, meta, message }) {
        clearLaunchWizardPendingAction();
        launchWizard = null;
        launchWizardOpenError = null;
        launchWizardOpening = {
          title,
          meta,
          message,
        };
        renderLaunchWizard();
      }

      // SPEC-3214 T-042: the standalone existing-branch picker keeps the
      // pending-wizard UX the removed Start Work entry used to provide.
      function openExistingBranchPendingWizard() {
        openLaunchPendingWizard({
          title: "Open existing branch",
          meta: "Branch picker",
          message: "Fetching remote branches...",
        });
      }

      // SPEC-3214 Phase 3: Intake replaced the old Start Work entry, but this
      // opener keeps the local pending wizard path alive while the backend
      // prepares the ephemeral branchless Plan Agent session.
      function openStartWorkPendingWizard() {
        openLaunchPendingWizard({
          title: "Plan Agent launch",
          meta: "Intake session",
          message: "Preparing Intake session...",
        });
      }

      function openLaunchAgentPendingWizard() {
        openLaunchPendingWizard({
          title: "Launch Agent",
          meta: "Agent launch",
          message: "Preparing Launch Agent...",
        });
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
        // Issue #3192 — this derivation runs BEFORE the opening/openError
        // early returns below, so it is reached for the Start Work /
        // Launch Agent pending states where `launchWizard` is null
        // (`launchWizardOpening` is set instead). Read the field null-safely:
        // a bare `launchWizard.launch_materialization_pending` throws and
        // renderLaunchWizard() never reaches the `.open` toggle, so the rail
        // button silently does nothing. Dereferences AFTER those early
        // returns are non-null by construction and stay bare.
        const isLaunchMaterializationPending = Boolean(
          launchWizard?.launch_materialization_pending,
        );
        const isLaunchActionPending =
          Boolean(launchWizardPendingAction) || isLaunchMaterializationPending;
        const isLaunchOpeningPending = Boolean(launchWizardOpening);
        const isLaunchSubmitPending =
          launchWizardPendingAction?.kind === "submit" || isLaunchMaterializationPending;
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
          wizardMeta.textContent = launchWizardOpening.meta || "Plan Agent launch";
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
              launchWizardOpening.message || "Preparing Plan Agent...",
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
              ? "Plan Agent launch"
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
          ? "Plan Agent launch"
          : `Selected branch · ${
            displayBranchName(
              launchWizard.selected_branch_name || launchWizard.branch_name || "Work",
            )
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
              launchWizard.launch_materialization_message || "Preparing worktree...",
            ),
          );
        }

        // SPEC-3165 — Start Work is now the Plan Agent entrypoint. The prompt
        // is still skippable and still drives the duplicate-work advisory.
        if (isStartWorkLaunch()) {
          const section = createLaunchSection(
            "Register an Issue",
            "Optional — describe the work for the Plan Agent to turn into an Issue or SPEC. You can skip this.",
          );
          const textarea = createNode("textarea", "launch-intake-input");
          textarea.placeholder = "e.g. register an issue for the login auth bug";
          textarea.rows = 2;
          textarea.value = wizardPromptDraft;
          textarea.addEventListener("input", () => {
            wizardPromptDraft = textarea.value;
            scheduleWorkAdvisory();
          });
          // Persist the prompt to wizard state on blur only (not per keystroke)
          // so typing never triggers a disruptive full wizard re-render.
          textarea.addEventListener("blur", () => {
            flushWizardPromptDraft();
          });
          section.appendChild(textarea);
          const advisory = createNode("div", "launch-advisory");
          advisory.id = "wizard-work-advisory";
          advisory.hidden = wizardAdvisoryResults.length === 0;
          section.appendChild(advisory);
          panel.appendChild(section);
          // Repaint any results already received into the fresh panel node.
          renderWorkAdvisoryPanel();
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
            "Pick the safest next step for this agent on the selected branch.",
          );

          const methodList = createNode("div", "start-method-list");
          const startMethodGroups = [
            {
              id: "recommended",
              title: "Recommended",
              copy: "Best next step for the current state.",
            },
            {
              id: "available",
              title: "Available",
              copy: "Other ways to start or resume this agent.",
            },
            {
              id: "unavailable",
              title: "Unavailable",
              copy: "Requires saved settings, saved sessions, or a running agent.",
            },
          ];
          const methodsByGroup = new Map(
            startMethodGroups.map((group) => [group.id, []]),
          );
          for (const method of launchWizard.start_methods || []) {
            const fallbackGroup =
              method.enabled === false ? "unavailable" : "available";
            const requestedGroup =
              method.group || (method.enabled === false ? "unavailable" : "available");
            const groupId = methodsByGroup.has(requestedGroup)
              ? requestedGroup
              : fallbackGroup;
            methodsByGroup.get(groupId).push(method);
          }
          for (const group of startMethodGroups) {
            const methods = methodsByGroup.get(group.id) || [];
            if (methods.length === 0) {
              continue;
            }
            const groupNode = createNode(
              "div",
              `start-method-group start-method-group--${group.id}`,
            );
            const groupHeader = createNode("div", "start-method-group-header");
            groupHeader.appendChild(
              createNode("div", "start-method-group-title", group.title),
            );
            groupHeader.appendChild(
              createNode("div", "start-method-group-copy", group.copy),
            );
            groupNode.appendChild(groupHeader);
            for (const method of methods) {
              const button = createNode("button", "start-method-button");
              button.type = "button";
              const isStartMethodPending =
                launchWizardPendingAction?.kind === "use_start_method"
                  && launchWizardPendingAction.method === method.kind;
              button.classList.toggle("is-pending", isStartMethodPending);
              if (method.recommended === true) {
                button.classList.add("start-method-button--recommended");
              }
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
              groupNode.appendChild(button);
            }
            methodList.appendChild(groupNode);
          }
          section.appendChild(methodList);

          panel.appendChild(section);
        }

        // SPEC-2359 US-83 / FR-444: "open an existing branch" picker. Lets the
        // user continue on an eligible remote branch (no new work/* branch)
        // instead of starting a fresh work branch. Each candidate dispatches
        // SelectExistingBranch, which flips the wizard to continue-on-branch.
        const openBranchCandidates = Array.isArray(launchWizard.open_branch_candidates)
          ? launchWizard.open_branch_candidates
          : [];
        // SPEC-3214 FR-010: the standalone picker mode always renders this
        // section — it IS the surface — including a fetching placeholder
        // while the candidate refresh is still running.
        const isExistingBranchMode = launchWizard.mode === "existing_branch";
        if (isExistingBranchMode && openBranchCandidates.length === 0) {
          const branchPickerSection = createLaunchSection(
            "Open an existing branch",
            "Continue on a remote branch instead of creating a new work branch.",
          );
          branchPickerSection.appendChild(
            createNode(
              "div",
              "start-method-summary",
              "Fetching remote branches...",
            ),
          );
          panel.appendChild(branchPickerSection);
        }
        if ((showStartMethods || isExistingBranchMode) && openBranchCandidates.length > 0) {
          const branchPickerSection = createLaunchSection(
            "Open an existing branch",
            "Continue on a remote branch instead of creating a new work branch.",
          );
          const candidateList = createNode("div", "start-method-list");
          for (const candidate of openBranchCandidates) {
            const button = createNode("button", "start-method-button");
            button.type = "button";
            button.setAttribute("data-existing-branch", candidate);
            button.appendChild(
              createNode("div", "start-method-title", displayBranchName(candidate)),
            );
            button.appendChild(
              createNode(
                "div",
                "start-method-summary",
                "Continue on this branch (tracks the remote)",
              ),
            );
            const openExistingBranch = () => {
              if (
                !releaseWizardInteractionGuardForChromeAction()
                || launchWizardPendingAction
              ) {
                return;
              }
              sendWizardAction({
                kind: "select_existing_branch",
                branch_name: candidate,
              });
            };
            button.addEventListener("click", openExistingBranch);
            candidateList.appendChild(button);
          }
          branchPickerSection.appendChild(candidateList);
          panel.appendChild(branchPickerSection);
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

        // SPEC-3152: Hermes-specific launch options, rendered only for the
        // Hermes agent. Provider is a curated dropdown sourced from the user's
        // config; the remaining fields are optional overrides (blank uses the
        // user's hermes setup / config.yaml). The wizard stays flat — no
        // nested disclosure — per the launch-wizard flatness invariant.
        if (showSetupForms && launchWizard.show_hermes_options) {
          const section = createLaunchSection(
            "Hermes options",
            "Provider and optional overrides for the Hermes agent. Blank fields use your hermes setup (config.yaml).",
          );
          if (launchWizard.hermes_needs_setup) {
            const note = createNode(
              "div",
              "launch-note",
              "Hermes is not set up yet (no credentials in ~/.hermes). Run `hermes setup` (or `hermes model`) in a terminal to choose a provider and sign in; gwt will then bridge it into every worktree. You can still launch — Hermes will prompt for setup.",
            );
            section.appendChild(note);
          }
          const grid = createNode("div", "launch-form-grid");
          appendHermesProviderField(
            grid,
            launchWizard.hermes_provider,
            launchWizard.hermes_provider_options || [],
            (value) =>
              sendWizardAction({
                kind: "set_hermes_option",
                field: "provider",
                value,
              }),
          );
          appendTextField(
            grid,
            "Model",
            launchWizard.selected_model,
            "e.g. anthropic/claude-sonnet-4 (blank = config.yaml)",
            (value) =>
              sendWizardAction({
                kind: "set_model",
                model: value,
              }),
          );
          appendTextField(
            grid,
            "Profile",
            launchWizard.hermes_profile,
            "Hermes profile name (optional)",
            (value) =>
              sendWizardAction({
                kind: "set_hermes_option",
                field: "profile",
                value,
              }),
          );
          appendTextField(
            grid,
            "Toolsets",
            launchWizard.hermes_toolsets,
            "comma-separated, e.g. fs,web (optional)",
            (value) =>
              sendWizardAction({
                kind: "set_hermes_option",
                field: "toolsets",
                value,
              }),
          );
          appendTextField(
            grid,
            "Skills",
            launchWizard.hermes_skills,
            "preloaded skills (optional)",
            (value) =>
              sendWizardAction({
                kind: "set_hermes_option",
                field: "skills",
                value,
              }),
          );
          appendTextField(
            grid,
            "Max turns",
            launchWizard.hermes_max_turns,
            "e.g. 40 (optional)",
            (value) =>
              sendWizardAction({
                kind: "set_hermes_option",
                field: "max_turns",
                value,
              }),
            false,
            "number",
          );
          appendToggleField(
            grid,
            "Safe mode",
            "Disable user config/plugins (also disables gwt hooks)",
            Boolean(launchWizard.hermes_safe_mode),
            (enabled) =>
              sendWizardAction({
                kind: "set_hermes_safe_mode",
                enabled,
              }),
          );
          section.appendChild(grid);
          panel.appendChild(section);
        }

        // SPEC-3151 FR-008/009/010: OpenCode-specific launch options, rendered
        // only for the OpenCode agent. OpenCode takes a single free-text
        // `provider/model` string (auth is host-global, so no provider bridge
        // is needed). When no AI provider is configured, a non-blocking note
        // offers an in-pane setup launcher that runs `opencode auth login`.
        if (showSetupForms && launchWizard.show_opencode_options) {
          const section = createLaunchSection(
            "OpenCode options",
            "Model and provider sign-in for the OpenCode agent. Blank model uses your OpenCode config.",
          );
          if (launchWizard.opencode_needs_setup) {
            const note = createNode(
              "div",
              "launch-note",
              "OpenCode has no AI provider configured yet. Run `/connect` inside OpenCode or `opencode auth login` to sign in. You can still launch — OpenCode will prompt for setup.",
            );
            const setupButton = createNode(
              "button",
              "launch-choice-button",
              "Run OpenCode setup",
            );
            setupButton.type = "button";
            setupButton.addEventListener("click", () =>
              sendWizardAction({ kind: "run_opencode_setup" }),
            );
            note.appendChild(setupButton);
            section.appendChild(note);
          }
          const grid = createNode("div", "launch-form-grid");
          appendTextField(
            grid,
            "Model",
            launchWizard.selected_model,
            "e.g. anthropic/claude-sonnet-4 (blank = config)",
            (value) =>
              sendWizardAction({
                kind: "set_model",
                model: value,
              }),
          );
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
        if (!event.wizard?.launch_materialization_pending) {
          clearLaunchWizardPendingAction();
        }
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
        openExistingBranchPendingWizard,
        openStartWorkPendingWizard,
        openLaunchAgentPendingWizard,
        applyLaunchWizardStateEvent,
        applyLaunchWizardOpenErrorEvent,
        applyWorkAdvisoryResultEvent,
        handleWizardEscapeKeydown,
        installWizardChrome,
      };
}
