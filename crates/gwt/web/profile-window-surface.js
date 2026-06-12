// SPEC-3064 Phase 3 (E6e) — Profile window surface extracted from app.js.
// Owns the per-window profile state map (config snapshot, draft editing
// with debounced save, env var grid rows), the Profile window rendering,
// the Profile window mount, and the profile_snapshot / profile_error
// receive() bodies. Pure movement from app.js: behavior, DOM output, and
// WS protocol are unchanged; the moved code keeps its original app.js
// indentation. Textual changes are limited to: in-module self-references
// through `*` became direct local calls
// (hasEditableFocus → profileHasEditableFocus) and the mount's
// focus_window send goes through sendWindowFocus.
//
// deps:
// - send(message): forward a frontend event over the WebSocket bridge.
// - createNode(tag, className, textContent): shared DOM helper.
// - windowMap: workspace window element map owned by app.js.
// - focusWindowLocally(windowId) / sendWindowFocus(windowId): focus paths.
export function createProfileWindowSurface({
  send,
  createNode,
  windowMap,
  focusWindowLocally,
  sendWindowFocus,
}) {
      const profileStateMap = new Map();

      function ensureProfileState(windowId) {
        if (!profileStateMap.has(windowId)) {
          profileStateMap.set(windowId, {
            snapshot: null,
            loading: false,
            saving: false,
            error: "",
            selectedProfile: null,
            draft: null,
            saveTimer: null,
            saveInFlight: false,
          });
        }
        return profileStateMap.get(windowId);
      }

      function requestProfile(windowId) {
        const state = ensureProfileState(windowId);
        if (state.loading) {
          return;
        }
        state.loading = true;
        state.error = "";
        send({
          kind: "load_profile",
          id: windowId,
        });
      }

      function clearProfileSaveTimer(state) {
        if (state.saveTimer !== null) {
          clearTimeout(state.saveTimer);
          state.saveTimer = null;
        }
      }

      function profileDraftFromEntry(profile) {
        if (!profile) {
          return null;
        }
        return {
          currentName: profile.name,
          name: profile.name,
          description: profile.description || "",
          envVars: (profile.env_vars || []).map((entry) => ({
            key: entry.key || "",
            value: entry.value || "",
          })),
          disabledEnv: (profile.disabled_env || []).map((entry) => entry || ""),
        };
      }

      function normalizeProfileEnvKey(key) {
        return String(key || "").trim();
      }

      function profileDraftPayload(draft) {
        if (!draft) {
          return { envVars: [], disabledEnv: [] };
        }
        const envByKey = new Map();
        for (const entry of draft.envVars || []) {
          const key = normalizeProfileEnvKey(entry.key);
          if (!key) {
            continue;
          }
          envByKey.set(key, {
            key,
            value: entry.value ?? "",
          });
        }
        const disabledSet = new Set();
        for (const entry of draft.disabledEnv || []) {
          const key = normalizeProfileEnvKey(entry);
          if (key) {
            disabledSet.add(key);
          }
        }
        for (const key of disabledSet) {
          envByKey.delete(key);
        }
        return {
          envVars: Array.from(envByKey.values()).sort((left, right) =>
            left.key.localeCompare(right.key),
          ),
          disabledEnv: Array.from(disabledSet).sort((left, right) =>
            left.localeCompare(right),
          ),
        };
      }

      function removeProfileEnvOverride(draft, key) {
        const normalized = normalizeProfileEnvKey(key);
        draft.envVars = (draft.envVars || []).filter(
          (entry) => normalizeProfileEnvKey(entry.key) !== normalized,
        );
      }

      function removeProfileDisabledKey(draft, key) {
        const normalized = normalizeProfileEnvKey(key);
        draft.disabledEnv = (draft.disabledEnv || []).filter(
          (entry) => normalizeProfileEnvKey(entry) !== normalized,
        );
      }

      function setProfileEnvOverride(draft, key, value) {
        const normalized = normalizeProfileEnvKey(key);
        if (!normalized) {
          return null;
        }
        removeProfileDisabledKey(draft, normalized);
        const existing = (draft.envVars || []).find(
          (entry) => normalizeProfileEnvKey(entry.key) === normalized,
        );
        if (existing) {
          existing.key = normalized;
          existing.value = value ?? "";
          return existing;
        }
        const entry = { key: normalized, value: value ?? "" };
        draft.envVars.push(entry);
        return entry;
      }

      function setProfileRowMode(draft, key, mode) {
        const normalized = normalizeProfileEnvKey(key);
        if (!normalized) {
          return;
        }
        if (mode === "use_os") {
          removeProfileEnvOverride(draft, normalized);
          removeProfileDisabledKey(draft, normalized);
          return;
        }
        if (mode === "disabled") {
          removeProfileEnvOverride(draft, normalized);
          if (
            !(draft.disabledEnv || []).some(
              (entry) => normalizeProfileEnvKey(entry) === normalized,
            )
          ) {
            draft.disabledEnv.push(normalized);
          }
          return;
        }
        const existing = (draft.envVars || []).find(
          (entry) => normalizeProfileEnvKey(entry.key) === normalized,
        );
        setProfileEnvOverride(draft, normalized, existing?.value ?? "");
      }

      function profileEnvironmentRows(snapshot, draft) {
        const payload = profileDraftPayload(draft);
        const osEntries = (snapshot.os_env || [])
          .map((entry) => ({
            key: normalizeProfileEnvKey(entry.key),
            value: entry.value ?? "",
          }))
          .filter((entry) => entry.key)
          .sort((left, right) => left.key.localeCompare(right.key));
        const overrides = new Map(payload.envVars.map((entry) => [entry.key, entry]));
        const disabled = new Set(payload.disabledEnv);
        const osKeys = new Set();
        const rows = [];

        for (const entry of osEntries) {
          osKeys.add(entry.key);
          const mode = disabled.has(entry.key)
            ? "disabled"
            : overrides.has(entry.key)
              ? "override"
              : "use_os";
          const profileValue = overrides.get(entry.key)?.value ?? "";
          rows.push({
            kind: "os",
            key: entry.key,
            osValue: entry.value,
            mode,
            profileValue,
            result:
              mode === "disabled"
                ? "Disabled"
                : mode === "override"
                  ? profileValue
                  : entry.value,
          });
        }

        const addedKeys = new Set();
        for (const entry of payload.envVars) {
          if (osKeys.has(entry.key)) {
            continue;
          }
          addedKeys.add(entry.key);
          rows.push({
            kind: "added",
            key: entry.key,
            osValue: "",
            mode: "override",
            profileValue: entry.value,
            result: entry.value,
          });
        }
        for (const key of payload.disabledEnv) {
          if (osKeys.has(key) || addedKeys.has(key)) {
            continue;
          }
          rows.push({
            kind: "added",
            key,
            osValue: "",
            mode: "disabled",
            profileValue: "",
            result: "Disabled",
          });
        }

        rows.sort((left, right) => {
          if (left.kind !== right.kind) {
            return left.kind === "os" ? -1 : 1;
          }
          return left.key.localeCompare(right.key);
        });

        (draft.envVars || []).forEach((entry, index) => {
          if (normalizeProfileEnvKey(entry.key)) {
            return;
          }
          rows.push({
            kind: "pending",
            key: entry.key || "",
            osValue: "",
            mode: "override",
            profileValue: entry.value ?? "",
            result: entry.value ?? "",
            draftIndex: index,
          });
        });

        return rows;
      }

      function selectedProfileEntry(state) {
        const profiles = state.snapshot?.profiles || [];
        if (!state.selectedProfile) {
          return null;
        }
        return profiles.find((profile) => profile.name === state.selectedProfile) || null;
      }

      function syncProfileDraftFromSelection(state) {
        const selected = selectedProfileEntry(state);
        state.draft = profileDraftFromEntry(selected);
      }

      function profileDraftSignature(draft) {
        if (!draft) {
          return "";
        }
        const payload = profileDraftPayload(draft);
        return JSON.stringify({
          currentName: draft.currentName,
          name: draft.name,
          description: draft.description,
          envVars: payload.envVars,
          disabledEnv: payload.disabledEnv,
        });
      }

      function profileDraftIsDirty(state) {
        const selected = selectedProfileEntry(state);
        return profileDraftSignature(state.draft) !== profileDraftSignature(profileDraftFromEntry(selected));
      }

      function updateProfileStatus(windowId) {
        const element = windowMap.get(windowId);
        const status = element?.querySelector(".profile-status");
        if (!status) {
          return;
        }
        const state = ensureProfileState(windowId);
        const profileCount = state.snapshot?.profiles?.length || 0;
        const activeProfile = state.snapshot?.active_profile || "default";
        status.textContent = state.error
          ? state.error
          : state.loading
            ? state.saving
              ? "Saving profile..."
              : "Loading profiles..."
            : state.saving
              ? "Saving profile..."
              : `Active ${activeProfile} · ${profileCount} profile${profileCount === 1 ? "" : "s"}`;
        status.className = "profile-status";
        if (state.error) {
          status.classList.add("error");
        } else if (state.loading || state.saving) {
          status.classList.add("info");
        }
      }

      function profileHasEditableFocus(windowId) {
        const element = windowMap.get(windowId);
        const active = document.activeElement;
        return Boolean(
          element &&
            active &&
            element.contains(active) &&
            active.matches?.("input, textarea, select"),
        );
      }

      function flushProfileSave(windowId) {
        const state = ensureProfileState(windowId);
        clearProfileSaveTimer(state);
        if (!state.draft) {
          state.saving = false;
          updateProfileStatus(windowId);
          return;
        }
        if (!profileDraftIsDirty(state)) {
          state.saving = false;
          updateProfileStatus(windowId);
          return;
        }
        state.loading = true;
        state.saving = true;
        state.saveInFlight = true;
        state.error = "";
        updateProfileStatus(windowId);
        const payload = profileDraftPayload(state.draft);
        send({
          kind: "save_profile",
          id: windowId,
          current_name: state.draft.currentName,
          name: state.draft.name,
          description: state.draft.description,
          env_vars: payload.envVars,
          disabled_env: payload.disabledEnv,
        });
      }

      function scheduleProfileSave(windowId) {
        const state = ensureProfileState(windowId);
        clearProfileSaveTimer(state);
        state.saving = true;
        updateProfileStatus(windowId);
        state.saveTimer = setTimeout(() => {
          state.saveTimer = null;
          flushProfileSave(windowId);
        }, 250);
      }

      function selectProfile(windowId, profileName) {
        const state = ensureProfileState(windowId);
        if (state.selectedProfile === profileName) {
          return;
        }
        if (profileDraftIsDirty(state)) {
          flushProfileSave(windowId);
        } else {
          clearProfileSaveTimer(state);
        }
        state.loading = true;
        state.error = "";
        updateProfileStatus(windowId);
        send({
          kind: "select_profile",
          id: windowId,
          profile_name: profileName,
        });
      }

      function createProfile(windowId) {
        const state = ensureProfileState(windowId);
        if (profileDraftIsDirty(state)) {
          flushProfileSave(windowId);
        } else {
          clearProfileSaveTimer(state);
        }
        const name = window.prompt("Profile name", "review");
        if (!name) {
          return;
        }
        state.loading = true;
        state.error = "";
        updateProfileStatus(windowId);
        send({
          kind: "create_profile",
          id: windowId,
          name,
        });
      }

      function setActiveProfile(windowId) {
        const state = ensureProfileState(windowId);
        if (!state.selectedProfile) {
          return;
        }
        state.loading = true;
        state.error = "";
        updateProfileStatus(windowId);
        send({
          kind: "set_active_profile",
          id: windowId,
          profile_name: state.selectedProfile,
        });
      }

      function deleteProfile(windowId) {
        const state = ensureProfileState(windowId);
        if (!state.selectedProfile) {
          return;
        }
        if (!window.confirm(`Delete profile "${state.selectedProfile}"?`)) {
          return;
        }
        clearProfileSaveTimer(state);
        state.loading = true;
        state.saving = false;
        state.error = "";
        updateProfileStatus(windowId);
        send({
          kind: "delete_profile",
          id: windowId,
          profile_name: state.selectedProfile,
        });
      }

      function renderProfile(windowId, preserveDraft = false) {
        const element = windowMap.get(windowId);
        if (!element) {
          return;
        }
        const body = element.querySelector(".window-body");
        if (!body) {
          return;
        }
        const state = ensureProfileState(windowId);
        const snapshot = state.snapshot || {
          active_profile: "default",
          selected_profile: "default",
          profiles: [],
          os_env: [],
          merged_preview: [],
        };
        const profiles = snapshot.profiles || [];
        const status = body.querySelector(".profile-status");
        const list = body.querySelector(".profile-list");
        const editor = body.querySelector(".profile-editor-pane");
        if (!status || !list || !editor) {
          return;
        }

        if (
          state.selectedProfile &&
          !profiles.some((profile) => profile.name === state.selectedProfile)
        ) {
          state.selectedProfile = null;
        }
        if (!state.selectedProfile) {
          state.selectedProfile =
            snapshot.selected_profile || snapshot.active_profile || (profiles[0] ? profiles[0].name : null);
          preserveDraft = false;
        }

        if (!preserveDraft || !state.draft || state.draft.currentName !== state.selectedProfile) {
          syncProfileDraftFromSelection(state);
        }

        updateProfileStatus(windowId);
        list.innerHTML = "";
        if (!state.loading && profiles.length === 0) {
          const empty = createNode("div", "profile-empty workspace-empty-state");
          empty.appendChild(createNode("div", "mock-label", "No profiles yet"));
          empty.appendChild(
            createNode(
              "div",
              "profile-empty-copy",
              "Create a profile to track env overrides and disabled OS variables.",
            ),
          );
          const button = createNode("button", "wizard-button primary", "New profile");
          button.type = "button";
          button.addEventListener("click", () => createProfile(windowId));
          empty.appendChild(button);
          list.appendChild(empty);
        }

        for (const profile of profiles) {
          const row = createNode("button", "profile-row");
          row.type = "button";
          if (profile.name === state.selectedProfile) {
            row.classList.add("selected");
            row.setAttribute("aria-current", "true");
          } else {
            row.removeAttribute("aria-current");
          }
          row.addEventListener("click", () => selectProfile(windowId, profile.name));
          const header = createNode("div", "profile-row-header");
          header.appendChild(createNode("div", "profile-row-title", profile.name));
          const chips = createNode("div", "profile-row-chips");
          if (profile.is_active) {
            chips.appendChild(createNode("span", "profile-chip active", "Active"));
          }
          if (profile.is_default) {
            chips.appendChild(createNode("span", "profile-chip", "Default"));
          }
          header.appendChild(chips);
          row.appendChild(header);
          row.appendChild(
            createNode(
              "div",
              "profile-row-copy",
              profile.description || "No description yet",
            ),
          );
          const meta = createNode(
            "div",
            "profile-row-meta",
            `${profile.env_vars.length} env override${profile.env_vars.length === 1 ? "" : "s"} · ${profile.disabled_env.length} disabled variable${profile.disabled_env.length === 1 ? "" : "s"}`,
          );
          row.appendChild(meta);
          list.appendChild(row);
        }

        editor.innerHTML = "";
        const selected = selectedProfileEntry(state);
        if (!selected || !state.draft) {
          const empty = createNode("div", "profile-empty workspace-empty-state");
          empty.appendChild(createNode("div", "mock-label", "Select a profile"));
          empty.appendChild(
            createNode(
              "div",
              "profile-empty-copy",
              "Each profile keeps its own env overrides and disabled OS variables.",
            ),
          );
          editor.appendChild(empty);
          updateProfileStatus(windowId);
          return;
        }

        const actions = createNode("div", "profile-inline-actions");
        const activeButton = createNode("button", "wizard-button", "Set active");
        activeButton.type = "button";
        activeButton.disabled = selected.is_active || state.loading;
        activeButton.addEventListener("click", () => setActiveProfile(windowId));
        actions.appendChild(activeButton);

        const deleteButton = createNode("button", "wizard-button", "Delete");
        deleteButton.type = "button";
        deleteButton.disabled = selected.is_default || state.loading;
        deleteButton.addEventListener("click", () => deleteProfile(windowId));
        actions.appendChild(deleteButton);
        editor.appendChild(actions);

        const metadata = createNode("div", "profile-section");
        metadata.appendChild(createNode("div", "mock-label", "Profile metadata"));
        const nameField = createNode("label", "profile-field");
        nameField.appendChild(createNode("span", "", "Name"));
        const nameInput = document.createElement("input");
        nameInput.type = "text";
        nameInput.value = state.draft.name;
        nameInput.addEventListener("input", () => {
          state.draft.name = nameInput.value;
          scheduleProfileSave(windowId);
        });
        nameInput.addEventListener("blur", () => flushProfileSave(windowId));
        nameField.appendChild(nameInput);
        metadata.appendChild(nameField);

        const descriptionField = createNode("label", "profile-field");
        descriptionField.appendChild(createNode("span", "", "Description"));
        const descriptionInput = document.createElement("textarea");
        descriptionInput.className = "profile-textarea";
        descriptionInput.value = state.draft.description;
        descriptionInput.addEventListener("input", () => {
          state.draft.description = descriptionInput.value;
          scheduleProfileSave(windowId);
        });
        descriptionInput.addEventListener("blur", () => flushProfileSave(windowId));
        descriptionField.appendChild(descriptionInput);
        metadata.appendChild(descriptionField);
        editor.appendChild(metadata);

        const envSection = createNode("div", "profile-section profile-env-section");
        envSection.appendChild(createNode("div", "mock-label", "Environment Variables"));
        const envGrid = createNode("div", "profile-env-grid");
        const headerRow = createNode("div", "profile-env-grid-row profile-env-grid-head");
        for (const label of ["Key", "OS", "Mode", "Profile", "Result"]) {
          headerRow.appendChild(createNode("div", "", label));
        }
        envGrid.appendChild(headerRow);

        const rows = profileEnvironmentRows(snapshot, state.draft);
        rows.forEach((envRow, index) => {
          const row = createNode(
            "div",
            `profile-env-grid-row profile-env-row is-${envRow.mode}`,
          );

          if (envRow.kind === "os") {
            const keyCell = createNode("div", "profile-env-key", envRow.key);
            keyCell.title = envRow.key;
            row.appendChild(keyCell);
          } else {
            const keyInput = document.createElement("input");
            keyInput.type = "text";
            keyInput.placeholder = "KEY";
            keyInput.value = envRow.key;
            keyInput.setAttribute("aria-label", `Environment variable key, row ${index + 1}`);
            keyInput.addEventListener("input", () => {
              if (envRow.kind === "pending") {
                state.draft.envVars[envRow.draftIndex].key = keyInput.value;
                envRow.key = keyInput.value;
              } else if (envRow.mode === "disabled") {
                const previous = normalizeProfileEnvKey(envRow.key);
                const target = (state.draft.disabledEnv || []).findIndex(
                  (entry) => normalizeProfileEnvKey(entry) === previous,
                );
                if (target >= 0) {
                  state.draft.disabledEnv[target] = keyInput.value;
                  envRow.key = keyInput.value;
                }
              } else {
                const previous = normalizeProfileEnvKey(envRow.key);
                const target = (state.draft.envVars || []).find(
                  (entry) => normalizeProfileEnvKey(entry.key) === previous,
                );
                if (target) {
                  target.key = keyInput.value;
                  envRow.key = keyInput.value;
                }
              }
              scheduleProfileSave(windowId);
            });
            keyInput.addEventListener("blur", () => {
              flushProfileSave(windowId);
            });
            row.appendChild(keyInput);
          }

          const osCell = createNode("div", "profile-env-os-value", envRow.osValue || "-");
          osCell.title = envRow.osValue || "";
          row.appendChild(osCell);

          const modeSelect = document.createElement("select");
          modeSelect.setAttribute("aria-label", `Environment variable mode, row ${index + 1}`);
          const modeOptions =
            envRow.kind === "os"
              ? [
                  ["use_os", "Use OS"],
                  ["override", "Override"],
                  ["disabled", "Disabled"],
                ]
              : [
                  ["override", "Enabled"],
                  ["disabled", "Disabled"],
                ];
          for (const option of modeOptions) {
            const element = document.createElement("option");
            element.value = option[0];
            element.textContent = option[1];
            modeSelect.appendChild(element);
          }
          modeSelect.value = envRow.mode;
          modeSelect.addEventListener("change", () => {
            const rowKey =
              envRow.kind === "pending"
                ? state.draft.envVars[envRow.draftIndex]?.key
                : envRow.key;
            if (envRow.kind === "pending" && !normalizeProfileEnvKey(rowKey)) {
              if (modeSelect.value === "use_os") {
                state.draft.envVars.splice(envRow.draftIndex, 1);
              }
            } else {
              setProfileRowMode(state.draft, rowKey, modeSelect.value);
            }
            renderProfile(windowId, true);
            scheduleProfileSave(windowId);
          });
          row.appendChild(modeSelect);

          const valueInput = document.createElement("input");
          valueInput.type = "text";
          valueInput.placeholder = "Profile";
          valueInput.value = envRow.profileValue;
          valueInput.setAttribute("aria-label", `Profile value, row ${index + 1}`);
          const resultCell = createNode("div", "profile-env-result", envRow.result);
          resultCell.title = envRow.result;
          valueInput.addEventListener("input", () => {
            if (envRow.kind === "pending") {
              state.draft.envVars[envRow.draftIndex].value = valueInput.value;
              resultCell.textContent = valueInput.value;
            } else {
              setProfileEnvOverride(state.draft, envRow.key, valueInput.value);
              modeSelect.value = "override";
              resultCell.textContent = valueInput.value;
            }
            scheduleProfileSave(windowId);
          });
          valueInput.addEventListener("blur", () => flushProfileSave(windowId));
          row.appendChild(valueInput);
          row.appendChild(resultCell);
          envGrid.appendChild(row);
        });

        const addRow = createNode("button", "profile-env-add-row", "+ Add variable");
        addRow.type = "button";
        addRow.addEventListener("click", () => {
          state.draft.envVars.push({ key: "", value: "" });
          renderProfile(windowId, true);
        });
        envGrid.appendChild(addRow);
        envSection.appendChild(envGrid);
        editor.appendChild(envSection);

        updateProfileStatus(windowId);
      }
      // SPEC-3064 Phase 3 (E6e): Profile window mount moved verbatim from
      // app.js mountWindowBody (surface === "profile" branch).
      function mountProfileWindow(windowData, body) {
          body.innerHTML = `
            <div class="profile-root">
              <div class="workspace-toolbar is-stacked">
                <div class="workspace-toolbar-main">
                  <div class="knowledge-heading">Profiles</div>
                  <div class="profile-status"></div>
                </div>
                <div class="workspace-toolbar-actions">
                  <button class="wizard-button" type="button" data-action="new-profile">New profile</button>
                  <button class="icon-button" data-action="refresh-profile" aria-label="Refresh profiles">↻</button>
                </div>
              </div>
              <div class="profile-layout workspace-split">
                <div class="profile-list-pane">
                  <div class="profile-list"></div>
                </div>
                <div class="profile-editor-pane"></div>
              </div>
            </div>
          `;
          body.addEventListener("mousedown", () => {
            focusWindowLocally(windowData.id);
            sendWindowFocus(windowData.id);
          });
          body
            .querySelector("[data-action='refresh-profile']")
            .addEventListener("click", (event) => {
              event.stopPropagation();
              const state = ensureProfileState(windowData.id);
              state.error = "";
              requestProfile(windowData.id);
              renderProfile(windowData.id, true);
            });
          body
            .querySelector("[data-action='new-profile']")
            .addEventListener("click", (event) => {
              event.stopPropagation();
              createProfile(windowData.id);
            });
          const state = ensureProfileState(windowData.id);
          if (!state.snapshot && !state.loading && !state.error) {
            requestProfile(windowData.id);
          }
          renderProfile(windowData.id);
          return;
      }

      // SPEC-3064 Phase 3 (E6e): receive() bodies for profile_* events
      // moved verbatim from app.js; the case arms in app.js delegate here.
      function applyProfileReceiveEvent(event) {
        switch (event.kind) {
          case "profile_snapshot": {
            const state = ensureProfileState(event.id);
            const previousProfile = state.selectedProfile;
            const wasSaveInFlight = state.saveInFlight;
            state.snapshot = event.snapshot || null;
            state.loading = false;
            state.saving = Boolean(state.saveTimer);
            state.saveInFlight = false;
            state.error = "";
            state.selectedProfile = event.snapshot?.selected_profile || null;
            const selectedProfileUnchanged =
              !previousProfile || previousProfile === state.selectedProfile;
            if (
              wasSaveInFlight &&
              selectedProfileUnchanged &&
              profileHasEditableFocus(event.id)
            ) {
              updateProfileStatus(event.id);
              break;
            }
            renderProfile(event.id);
            break;
          }
          case "profile_error": {
            const state = ensureProfileState(event.id);
            state.loading = false;
            state.saving = Boolean(state.saveTimer);
            state.saveInFlight = false;
            state.error = event.message;
            renderProfile(event.id, true);
            break;
          }
          default:
            break;
        }
      }

      return {
        profileStateMap,
        ensureProfileState,
        requestProfile,
        renderProfile,
        createProfile,
        setActiveProfile,
        flushProfileSave,
        deleteProfile,
        updateProfileStatus,
        profileHasEditableFocus,
        syncProfileDraftFromSelection,
        clearProfileSaveTimer,
        mountProfileWindow,
        applyProfileReceiveEvent,
      };
}
