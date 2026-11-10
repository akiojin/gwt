// Project picker/onboarding, Open/Clone project dialogs and migration chrome.
// Shared catalog navigation lives in app.js and the Agents/Works surfaces.
import { renderMigrationModal as renderMigrationModalView } from "/migration-modal.js";
import { renderProjectCloneModal as renderProjectCloneModalView, createFolderPickerController, createOpenProjectPathForm, createOpenProjectPathDialog } from "/project-clone-modal.js";
import { createUiStateStore } from "/ui-state-store.js";

function shouldTriggerOpenFolderHotkey(event, { modalOpen = false } = {}) {
  const target = event?.target;
  return Boolean(event && !event.repeat && (event.metaKey || event.ctrlKey)
    && !event.shiftKey && !event.altKey && !modalOpen
    && !target?.closest?.('input, textarea, select, [contenteditable="true"]')
    && (String(event.key || '').toLowerCase() === 'o' || event.code === 'KeyO'));
}

export function createProjectShellSurface({
  send,
  createNode,
  getAppState,
  getProjectError,
  activeProjectTab,
  appendRenderKeyPart,


  cloneProjectModal,
  cloneProjectDialog,
  migrationModal,
  migrationDialog,
  scheduleFrame,
  isModalOpen = () => false,
}) {
      const projectPicker = document.getElementById("project-picker");
      const projectPickerError = document.getElementById("project-picker-error");
      const pickerOpenProjectButton = document.getElementById("picker-open-project");
      const pickerCloneProjectButton = document.getElementById("picker-clone-project");
      const recentProjectList = document.getElementById("recent-project-list");
      const projectOnboarding = document.getElementById("project-onboarding");
      const projectOnboardingTitle = document.getElementById(
        "project-onboarding-title",
      );
      const projectOnboardingCopy = document.getElementById(
        "project-onboarding-copy",
      );
      const onboardingOpenProjectButton = document.getElementById(
        "onboarding-open-project",
      );

      let renderedRecentProjectsKey = "";
      let renderedProjectPickerKey = "";
      let renderedProjectOnboardingKey = "";


      function recentProjectsRenderKey(state) {
        return JSON.stringify(
          (state?.recent_projects || []).map((project) => ({
            title: project?.title || "",
            kind: project?.kind || "",
            path: project?.path || "",
          })),
        );
      }

      function projectPickerRenderKey(activeTab = activeProjectTab()) {
        const appState = getAppState();
        const projectError = getProjectError();
        const shouldShow = !activeTab;
        const parts = [];
        appendRenderKeyPart(parts, "visible");
        appendRenderKeyPart(parts, shouldShow);
        appendRenderKeyPart(parts, "error");
        appendRenderKeyPart(parts, projectError || "");
        appendRenderKeyPart(parts, "recent_projects");
        appendRenderKeyPart(
          parts,
          shouldShow ? recentProjectsRenderKey(appState) : "",
        );
        return parts.join("");
      }

      function projectOnboardingRenderKey(tab) {
        const visible = Boolean(tab && tab.kind !== "git");
        const parts = [];
        appendRenderKeyPart(parts, "visible");
        appendRenderKeyPart(parts, visible);
        appendRenderKeyPart(parts, "kind");
        appendRenderKeyPart(parts, visible ? tab.kind || "" : "");
        appendRenderKeyPart(parts, "project_root");
        appendRenderKeyPart(parts, visible ? tab.project_root || "" : "");
        return parts.join("");
      }

      // SPEC-1934 US-6: state for the migration confirmation / progress modal.
      // `tabId` identifies which tab the active migration belongs to so a
      // multi-project frontend never mixes events from different repos.
      const migrationModel = createUiStateStore({
        open: false,
        stage: "confirm", // "confirm" | "running" | "error"
        tabId: null,
        projectRoot: "",
        branch: null,
        hasDirty: false,
        hasLocked: false,
        hasSubmodules: false,
        phase: "confirm",
        percent: 0,
        message: "",
        recovery: "",
      }, { scheduleFrame });
      let renderedMigrationSnapshot = null;
      // Draft edits are model updates too, but do not rebuild focused inputs.
      const cloneProjectModel = createUiStateStore({
        form: { url: "", parentPath: "", query: "" },
        modal: {
          open: false,
          mode: "url",
          repositories: [],
          selectedRepositoryUrl: "",
          searching: false,
          cloning: false,
          progress: "",
          error: "",
        },
      }, { scheduleFrame });
      let observedCloneState = null;
      let renderedCloneSnapshot = null;

      function sendOpenProjectDialog() {
        openPathDialog.open();
        folderPicker.begin("open");
      }

      const openPathDialog = createOpenProjectPathDialog(document, {
        onChoose: () => folderPicker.begin("open"),
        onOpen: (path, request_id) => send({ kind: "reopen_recent_project", path, request_id }),
      });

      const folderPicker = createFolderPickerController({
        send,
        onChange: (pending, message) => {
          openPathDialog.update(pending, message);
          for (const button of [pickerOpenProjectButton, onboardingOpenProjectButton]) {
            button.textContent = pending ? "Choosing folder…" : "Open Folder…";
            button.setAttribute("aria-busy", String(Boolean(pending)));
          }
          cloneProjectModel.update(current => ({ ...current,
            modal: { ...current.modal, pickerPending: Boolean(pending), pickerMessage: message },
          }));
        },
        onSelected: (event) => {
          if (event.purpose === "open") openPathDialog.waitForOpen(event.path, `picker:${event.request_id}`);
          if (event.purpose === "clone_parent") {
            cloneProjectModel.update(current => ({
              form: { ...current.form, parentPath: event.path },
              modal: { ...current.modal, error: "" },
            }));
          }
        },
      });

      function applyPickerReceiveEvent(event) { return folderPicker.receive(event); }
      function applyProjectOpenReceiveEvent(event) { openPathDialog.receive(event); }
      function clearPickerPending() { folderPicker.connectionLost(); }

      function openCloneProjectModal() {
        cloneProjectModel.update(current => ({ ...current,
          modal: { ...current.modal, open: true, error: "", progress: "" },
        }));
      }

      function closeCloneProjectModal() {
        if (cloneProjectModel.read().modal.cloning) return;
        cloneProjectModel.update(current => ({ ...current,
          modal: { ...current.modal, open: false, searching: false, error: "", progress: "" },
        }));
      }

      function renderProjectCloneModal() {
        if (!cloneProjectModal || !cloneProjectDialog) return;
        const { form, modal } = cloneProjectModel.read();
        renderProjectCloneModalView({
          modalEl: cloneProjectModal,
          dialogEl: cloneProjectDialog,
          state: { ...modal, ...form },
          createNode: (tagName, className, textContent) => {
            const node = document.createElement(tagName);
            if (className) node.className = className;
            if (textContent !== undefined) node.textContent = textContent;
            return node;
          },
          onClose: closeCloneProjectModal,
          onModeChange: (mode) => {
            cloneProjectModel.update(current => ({ ...current,
              modal: { ...current.modal, mode, error: "", progress: "" },
            }));
          },
          onUrlChange: (url) => {
            cloneProjectModel.update(current => ({ ...current, form: { ...current.form, url } }));
          },
          onParentSelect: () => { folderPicker.begin("clone_parent"); },
          onParentChange: (parentPath) => {
            cloneProjectModel.update(current => ({ ...current, form: { ...current.form, parentPath } }));
          },
          onSearchQueryChange: (query) => {
            cloneProjectModel.update(current => ({ ...current, form: { ...current.form, query } }));
          },
          onSearch: () => {
            const query = cloneProjectModel.read().form.query.trim();
            if (!query) return;
            cloneProjectModel.update(current => ({ ...current,
              modal: { ...current.modal, searching: true, error: "" },
            }));
            send({ kind: "github_repository_search", query });
          },
          onRepositorySelect: (url) => {
            cloneProjectModel.update(current => ({
              form: { ...current.form, url },
              modal: { ...current.modal, selectedRepositoryUrl: url, error: "" },
            }));
          },
          onClone: () => {
            const { form, modal } = cloneProjectModel.read();
            const url = (modal.mode === "search" ? modal.selectedRepositoryUrl : form.url).trim();
            const parentPath = form.parentPath.trim();
            if (!url || !parentPath) {
              cloneProjectModel.update(current => ({ ...current,
                modal: { ...current.modal, error: !url
                  ? "Select or enter a repository URL."
                  : "Choose a destination parent folder." },
              }));
              return;
            }
            cloneProjectModel.update(current => ({ ...current,
              modal: { ...current.modal, cloning: true, progress: "Cloning repository...", error: "" },
            }));
            send({ kind: "clone_project_start", url, parent_path: parentPath });
          },
        });
        renderedCloneSnapshot = modal;
      }

      function renderRecentProjects({ force = false } = {}) {
        const appState = getAppState();
        const nextKey = recentProjectsRenderKey(appState);
        if (!force && renderedRecentProjectsKey === nextKey) {
          return;
        }
        renderedRecentProjectsKey = nextKey;
        recentProjectList.replaceChildren();
        const recentProjects = appState?.recent_projects || [];
        if (recentProjects.length === 0) {
          const empty = document.createElement("div");
          empty.className = "file-tree-empty workspace-empty-state";
          empty.textContent = "No recent projects";
          recentProjectList.appendChild(empty);
        } else {
          for (const project of recentProjects) {
            const row = document.createElement("button");
            row.type = "button";
            row.className = "recent-project-row";
            // Issue #2746 follow-up — meta truncates with ellipsis, so the
            // full path lives in the native title tooltip on hover/focus.
            row.title = `${project.kind} · ${project.path}`;
            const titleEl = document.createElement("span");
            titleEl.className = "recent-project-title";
            titleEl.textContent = project.title;
            const metaEl = document.createElement("span");
            metaEl.className = "recent-project-meta";
            metaEl.textContent = `${project.kind} · ${project.path}`;
            row.append(titleEl, metaEl);
            row.addEventListener("click", () => {
              send({ kind: "reopen_recent_project", path: project.path });
            });
            recentProjectList.appendChild(row);
          }
        }
      }

      function renderProjectPicker(activeTab = activeProjectTab()) {
        const projectError = getProjectError();
        const nextProjectPickerKey = projectPickerRenderKey(activeTab);
        if (renderedProjectPickerKey === nextProjectPickerKey) {
          return;
        }
        renderedProjectPickerKey = nextProjectPickerKey;
        const shouldShow = !activeTab;
        projectPicker.classList.toggle("visible", shouldShow);
        projectPickerError.hidden = !projectError;
        projectPickerError.textContent = projectError;
        if (shouldShow) {
          renderRecentProjects();
        }
      }

      function renderProjectOnboarding(tab) {
        const nextProjectOnboardingKey = projectOnboardingRenderKey(tab);
        if (renderedProjectOnboardingKey === nextProjectOnboardingKey) {
          return;
        }
        renderedProjectOnboardingKey = nextProjectOnboardingKey;
        if (!tab || tab.kind === "git") {
          projectOnboarding.classList.remove("visible");
          return;
        }
        projectOnboarding.classList.add("visible");
        projectOnboardingTitle.textContent =
          tab.kind === "bare" ? "Bare repository selected" : "Project setup required";
        projectOnboardingCopy.textContent =
          tab.kind === "bare"
            ? `No develop worktree was found for ${tab.project_root}. Open a worktree folder or another project.`
            : `${tab.project_root} is not a Git workspace yet. Open another project or initialize this folder in your shell first.`;
      }

      // Migration receivers and local actions commit data; this view reads
      // the same immutable snapshot; only running progress waits for a frame.
      function renderMigrationModal() {
        if (!migrationModal || !migrationDialog) return;
        const migrationModalState = migrationModel.read();
        renderMigrationModalView({
          modalEl: migrationModal,
          dialogEl: migrationDialog,
          state: { migrationModal: migrationModalState },
          createNode,
          onMigrate: () => {
            if (!migrationModalState.tabId) return;
            const tabId = migrationModalState.tabId;
            migrationModel.update(current => ({
              ...current, stage: "running", phase: "validate", percent: 0,
            }));
            send({ kind: "start_migration", tab_id: tabId });
          },
          // SPEC-1934 US-7 / FR-032: only the error stage exposes a dismissal
          // affordance ("Close"), and it must not flip migration_pending —
          // the tab keeps its pending status so the next Open Project shows
          // the Accept-only modal again.
          onClose: () => {
            migrationModel.update(current => ({
              ...current, open: false, stage: "confirm", message: "", recovery: "",
            }));
          },
        });
        renderedMigrationSnapshot = migrationModalState;
      }

      // SPEC-3064 Phase 3 (E7): receive() bodies for the clone-project /
      // GitHub repository search events. The case arms stay in app.js as
      // thin delegates.
      function applyCloneProjectReceiveEvent(event) {
        cloneProjectModel.update(current => {
          const { form, modal } = current;
          switch (event.kind) {
            case "clone_project_parent_selected":
              return { form: { ...form, parentPath: event.path || "" },
                modal: { ...modal, error: "" } };
            case "github_repository_search_results":
              if (event.query !== form.query.trim()) return current;
              return { ...current, modal: { ...modal, repositories: event.repositories || [],
                selectedRepositoryUrl: "", searching: false, error: "" } };
            case "github_repository_search_error":
              if (event.query !== form.query.trim()) return current;
              return { ...current, modal: { ...modal, searching: false,
                error: event.message || "Repository search failed." } };
            case "clone_project_progress":
              return { ...current, modal: { ...modal, cloning: true,
                progress: event.message || "Cloning repository...", error: "" } };
            case "clone_project_done":
              return { ...current, modal: { ...modal, open: false, cloning: false,
                searching: false, progress: "", error: "" } };
            case "clone_project_error":
              return { ...current, modal: { ...modal, cloning: false,
                progress: "", error: event.message || "Clone failed." } };
            default:
              return current;
          }
        });
      }

      // SPEC-3064 Phase 3 (E7): receive() bodies for the migration events.
      // The case arms stay in app.js as thin delegates.
      function applyMigrationReceiveEvent(event) {
        const migrationModalState = migrationModel.read();
        switch (event.kind) {
          case "migration_detected": {
            // SPEC-1934 US-6.1: server says a project tab needs migration.
            migrationModel.update(() => ({
              open: true,
              stage: "confirm",
              tabId: event.tab_id,
              projectRoot: event.project_root || "",
              branch: event.branch || null,
              hasDirty: Boolean(event.has_dirty),
              hasLocked: Boolean(event.has_locked),
              hasSubmodules: Boolean(event.has_submodules),
              phase: "confirm",
              percent: 0,
              message: "",
              recovery: "",
            }));
            break;
          }
          case "migration_progress": {
            // Phase tick from execute_migration. Ignore if the modal already
            // closed (e.g. user pressed Quit) so we never re-open it.
            if (
              !migrationModalState.open ||
              migrationModalState.tabId !== event.tab_id
            ) {
              break;
            }
            migrationModel.update(current => ({
              ...current,
              stage: "running",
              phase: event.phase || "confirm",
              percent: Number.isFinite(event.percent) ? event.percent : 0,
            }));
            break;
          }
          case "migration_done": {
            if (migrationModalState.tabId === event.tab_id) {
              migrationModel.update(current => ({
                ...current, open: false, stage: "confirm", percent: 0,
              }));
            }
            break;
          }
          case "migration_error": {
            if (migrationModalState.tabId !== event.tab_id) break;
            migrationModel.update(current => ({
              ...current, open: true, stage: "error",
              phase: event.phase || "error",
              message: event.message || "", recovery: event.recovery || "",
            }));
            break;
          }
          default:
            break;
        }
      }

      // SPEC-3064 Phase 3 (E7): the migration branch of the app.js global
      // Esc handler. Returns true when the open migration modal consumed
      // the event so the caller stops walking the remaining branches.
      function handleMigrationModalEscape(event) {
        if (migrationModal && migrationModal.classList.contains("open")) {
          // SPEC-1934 US-7 / FR-032: the confirmation modal is Accept-only.
          // Esc no longer routes to skip — at the confirm stage it is
          // swallowed (so the user can't bypass migration), at the error
          // stage it dismisses the failure UI without flipping
          // migration_pending so the next Open Project re-presents Accept.
          if (migrationModel.read().stage === "error") {
            migrationModel.update(current => ({
              ...current, open: false, stage: "confirm", message: "", recovery: "",
            }));
          }
          event.preventDefault();
          return true;
        }
        return false;
      }

      function projectShellModalOrDropdownOpen() {
        return (
          Boolean(isModalOpen?.()) ||
          document.getElementById("close-project-modal")?.classList.contains("open") ||
          openPathDialog.modal.classList.contains("open") ||
          cloneProjectModal?.classList.contains("open") ||
          migrationModal?.classList.contains("open")
        );
      }

      // SPEC-2013 US-10 / FR-025: Cmd+O (mac) / Ctrl+O (Windows/Linux) opens the
      // native folder dialog directly, recovering the one-press Open Folder the
      // retired split button's primary half used to give.
      function handleOpenFolderHotkey(event) {
        if (
          !shouldTriggerOpenFolderHotkey(event, {
            modalOpen: projectShellModalOrDropdownOpen(),
          })
        ) {
          return false;
        }
        event.preventDefault();
        sendOpenProjectDialog();
        return true;
      }

      // Project picker and onboarding entry points share the existing dialogs.
      //
      // SPEC-2013 Phase 8: the standalone Open Project split-button was retired.
      // Open Folder / Clone now live inside the consolidated `Projects ▾`
      // switcher (onOpenFolder / onCloneFromGithub above) and Open Folder is
      // also reachable via the Cmd+O / Ctrl+O hotkey (handleOpenFolderHotkey).
      function installProjectShellChrome() {
        document.body.append(openPathDialog.modal);
        for (const button of [pickerOpenProjectButton, onboardingOpenProjectButton]) {
          button.parentElement.append(createOpenProjectPathForm(document, (path) => send({ kind: "reopen_recent_project", path })));
        }
        pickerOpenProjectButton.addEventListener("click", sendOpenProjectDialog);
        pickerCloneProjectButton.addEventListener("click", openCloneProjectModal);
        onboardingOpenProjectButton.addEventListener("click", sendOpenProjectDialog);

        window.addEventListener(
          "keydown",
          (event) => {
            handleOpenFolderHotkey(event);
          },
          true,
        );

      }

      // Keep open/close, actions and stage transitions synchronous. During a
      // run, progress changes share one frame and pull the latest snapshot.
      const unsubscribeMigration = migrationModel.subscribe(
        state => state.open && state.stage === "running" ? null : state,
        () => renderMigrationModal(),
      );
      const unsubscribeMigrationFrame = migrationModel.subscribeFrame(state => state, state => {
        if (state !== renderedMigrationSnapshot) renderMigrationModal();
      });
      // Only progress-only updates wait for a frame. Picker results and
      // actions stay synchronous, while draft-only changes never rebuild inputs.
      const unsubscribeClone = cloneProjectModel.subscribe(state => state, state => {
        const previous = observedCloneState;
        observedCloneState = state;
        if (previous && (state.modal === previous.modal || (
          state.form === previous.form && state.modal.open && previous.modal.open
          && state.modal.cloning && previous.modal.cloning
          && Object.keys(state.modal).every(key => key === "progress"
            || Object.is(state.modal[key], previous.modal[key]))
        ))) return;
        renderProjectCloneModal();
      });
      const unsubscribeCloneFrame = cloneProjectModel.subscribeFrame(state => state.modal, modal => {
        if (modal !== renderedCloneSnapshot) renderProjectCloneModal();
      });

      return {
        dispose() { unsubscribeMigration(); unsubscribeMigrationFrame(); unsubscribeClone(); unsubscribeCloneFrame(); folderPicker.dispose(); openPathDialog.dispose(); },
        sendOpenProjectDialog,
        closeCloneProjectModal,
        renderRecentProjects,
        renderProjectPicker,
        renderProjectOnboarding,
        applyCloneProjectReceiveEvent,
        applyPickerReceiveEvent,
        applyProjectOpenReceiveEvent,
        clearPickerPending,
        applyMigrationReceiveEvent,
        handleMigrationModalEscape,
        installProjectShellChrome,
      };
}
