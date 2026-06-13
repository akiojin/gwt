// SPEC-3064 Phase 3 (E4) — Settings windows surface extracted from app.js.
// Owns the Settings window body (tabbed System / Custom Agents / Agent
// Backends / Usage & Limits surface), the customAgentsState /
// agentBackendsState / systemSettingsState stores, the Teams channel link
// converters, the add-from-preset flow, the autostart appliers, and the
// systemSettingsInteractionGuard that defers destructive System panel
// re-renders while a native <select> dropdown is open. Pure movement from
// app.js: behavior, DOM output, and WS protocol are unchanged; the moved
// code keeps its original app.js indentation.
//
// deps:
// - send(message): forward a frontend event over the WebSocket bridge.
// - createNode(tag, className, textContent): shared DOM helper owned by
//   app.js (also used by Board / wizard surfaces).
// - focusWindowLocally(windowId): local z-order focus bookkeeping.
// - activeProjectTab(): active project tab accessor.
// - focusOrSpawnPreset(preset): focus-or-spawn a preset window (used by the
//   document-level "settings:open" dispatch).
// - renderUsagePanel(panel): Usage & Limits panel renderer. Instance
//   function created by createProviderUsageSurface in app.js (its usage
//   snapshot state lives there), so it must be injected rather than
//   imported as a sibling module.
// - indexStatusByProjectRoot: per-project index status Map owned by the
//   Project Index surface instance created in app.js (E3).
import { createInteractionGuard } from "/interaction-guard.js";
import { renderIndexSettingsPanel } from "/index-settings-panel.js";
import { renderCustomAgentEnvEditor } from "/custom-agent-env-editor.js";

export function createSettingsSurface({
  send,
  createNode,
  focusWindowLocally,
  activeProjectTab,
  focusOrSpawnPreset,
  renderUsagePanel,
  indexStatusByProjectRoot,
}) {
      // Issue #2698 PR 4 — same guard applied to the System Settings
      // Output Language `<select>`. Backend echoes `system_settings`
      // and `system_settings_updated` events; if either arrives while
      // the user has the dropdown open, `renderSystemPanel()` does a
      // `while (panel.firstChild) panel.removeChild(panel.firstChild)`
      // pass that destroys the live `<select>` and breaks the user's
      // commit. Delegated listeners scope to `select.settings-select`
      // so the guard covers every Settings window without per-window
      // wiring.
      function applyAutostartStatus(event, statusMessage = "") {
        systemSettingsState.autostartEnabled = event.enabled === true;
        systemSettingsState.autostartPreviousEnabled =
          systemSettingsState.autostartEnabled;
        systemSettingsState.autostartMechanism = event.mechanism || "";
        systemSettingsState.autostartInstallPath = event.install_path || "";
        systemSettingsState.autostartLoaded = true;
        systemSettingsState.autostartPending = false;
        systemSettingsState.statusMessage = statusMessage;
        systemSettingsState.statusKind = statusMessage ? "success" : "";
      }

      function applyAutostartError(event) {
        systemSettingsState.autostartEnabled =
          systemSettingsState.autostartPreviousEnabled === true;
        systemSettingsState.autostartPending = false;
        systemSettingsState.statusMessage =
          event.message || "Failed to update login launch setting.";
        systemSettingsState.statusKind = "error";
      }

      const systemSettingsInteractionGuard = createInteractionGuard({
        onFlush: (deferred) => {
          if (!deferred || typeof deferred !== "object") {
            return;
          }
          if (deferred.kind === "system_settings") {
            systemSettingsState.language = deferred.language || "auto";
            systemSettingsState.codexTrustManagedHooks =
              deferred.codex_trust_managed_hooks !== false;
            systemSettingsState.boardProvider =
              deferred.board_provider || systemSettingsState.boardProvider || "local";
            systemSettingsState.loaded = true;
            if (
              !systemSettingsState.statusMessage
              || systemSettingsState.statusKind === "info"
            ) {
              systemSettingsState.statusMessage = "";
              systemSettingsState.statusKind = "";
            }
          } else if (deferred.kind === "system_settings_updated") {
            systemSettingsState.language = deferred.language
              || systemSettingsState.language;
            systemSettingsState.codexTrustManagedHooks =
              deferred.codex_trust_managed_hooks !== false;
            systemSettingsState.boardProvider =
              deferred.board_provider || systemSettingsState.boardProvider || "local";
            systemSettingsState.statusMessage = "Saved system settings.";
            systemSettingsState.statusKind = "success";
          } else if (deferred.kind === "system_settings_error") {
            systemSettingsState.statusMessage = deferred.message
              || "Failed to update system settings.";
            systemSettingsState.statusKind = "error";
          } else if (deferred.kind === "autostart_status") {
            applyAutostartStatus(
              deferred,
              deferred.from_update ? "Saved login launch setting." : "",
            );
          } else if (deferred.kind === "autostart_error") {
            applyAutostartError(deferred);
          }
          renderSystemPanelInAllSettingsWindows();
        },
      });


      // All DOM nodes are built via createElement + textContent to avoid
      // innerHTML with mixed trust. Secrets in env tables are redacted by
      // the backend before reaching this layer (see redact_secrets_in_agent).
      const customAgentsState = {
        agents: [],
        loading: false,
        statusMessage: "",
        statusKind: "",
      };
      // SPEC-1921 2026-05-18 amendment / FR-099: Settings > Agent Backends
      // per-built-in backend profile state. `backends` is keyed by
      // BuiltinAgentId string ("claudeCode" / "codex"). Mirrors
      // customAgentsState shape so dispatch + status messages can share
      // helpers like setSettingsStatus.
      const agentBackendsState = {
        backends: { claudeCode: [], codex: [] },
        loadingAgent: null,
        statusMessage: "",
        statusKind: "",
      };
      // SPEC-1933 US-4: System tab state. `language` is the raw stored value
      // (auto/en/ja); the backend `system_settings` reply seeds it.
      const systemSettingsState = {
        language: "auto",
        codexTrustManagedHooks: true,
        // SPEC-2959/2963: selected Board backend (local/slack/teams).
        boardProvider: "local",
        // SPEC-2963: remote provider sign-in state + last sign-in message.
        boardAuth: { slack: false, teams: false },
        boardAuthMessage: "",
        // SPEC-2963: editable (non-secret) provider config for the settings UI.
        // Secrets are never echoed back; `*HasSecret` flags reflect store state.
        boardConfig: {
          slackClientId: "",
          slackDefaultChannel: "",
          slackHasSecret: false,
          teamsClientId: "",
          teamsTenantId: "",
          teamsDefaultChannel: "",
          oauthRedirectPort: 8765,
        },
        autostartEnabled: false,
        autostartPreviousEnabled: false,
        autostartMechanism: "",
        autostartInstallPath: "",
        autostartLoaded: false,
        autostartPending: false,
        loaded: false,
        statusMessage: "",
        statusKind: "",
      };
      const settingsWindowBodies = new Set();
      let pendingAddFromPreset = null;
      let editingCustomAgentId = null;

      function createDiv(className) {
        const el = document.createElement("div");
        if (className) el.className = className;
        return el;
      }

      function purgeDetachedSettingsBodies() {
        for (const body of Array.from(settingsWindowBodies)) {
          if (!body.isConnected) settingsWindowBodies.delete(body);
        }
      }

      // SPEC-1933 Phase: System Settings (Output Language).
      // Build a tabbed Settings surface (System | Custom Agents) using
      // Operator Design tokens. Existing renderSettingsAgentList continues
      // to populate the Custom Agents panel via [data-role='settings-scroll'].
      function renderSettingsWindow(body, windowData) {
        // Sweep detached bodies up-front so repeated open/close cycles do
        // not accumulate references.
        purgeDetachedSettingsBodies();
        while (body.firstChild) body.removeChild(body.firstChild);

        const root = createDiv("settings-root");

        const toolbar = document.createElement("header");
        toolbar.className = "settings-toolbar";
        const heading = document.createElement("h2");
        heading.className = "settings-heading";
        heading.textContent = windowData.title || "Settings";

        const tabs = document.createElement("nav");
        tabs.className = "settings-tabs";
        tabs.setAttribute("role", "tablist");
        tabs.appendChild(buildSettingsTab("system", "System", true));
        tabs.appendChild(buildSettingsTab("custom-agents", "Custom Agents", false));
        // SPEC-1921 2026-05-18 amendment / FR-099: Agent Backends tab is the
        // dedicated surface for Claude Code / Codex Backend Override profiles.
        // Kept distinct from `custom-agents` so External CLI rows and
        // built-in LLM redirection have separate physical UI.
        tabs.appendChild(buildSettingsTab("agent-backends", "Agent Backends", false));
        // SPEC-2970: provider usage display preferences (Claude opt-in).
        tabs.appendChild(buildSettingsTab("usage", "Usage & Limits", false));

        toolbar.appendChild(heading);
        toolbar.appendChild(tabs);

        const bodyEl = createDiv("settings-body");

        const panelSystem = document.createElement("section");
        panelSystem.className = "settings-panel";
        panelSystem.setAttribute("role", "tabpanel");
        panelSystem.dataset.settingsPanel = "system";

        const panelAgents = document.createElement("section");
        panelAgents.className = "settings-panel hidden";
        panelAgents.setAttribute("role", "tabpanel");
        panelAgents.dataset.settingsPanel = "custom-agents";
        // Existing renderSettingsAgentList queries this attribute to inject
        // the Add button and agent rows.
        panelAgents.dataset.role = "settings-scroll";

        const panelBackends = document.createElement("section");
        panelBackends.className = "settings-panel hidden";
        panelBackends.setAttribute("role", "tabpanel");
        panelBackends.dataset.settingsPanel = "agent-backends";
        panelBackends.dataset.role = "settings-scroll";

        const panelUsage = document.createElement("section");
        panelUsage.className = "settings-panel hidden";
        panelUsage.setAttribute("role", "tabpanel");
        panelUsage.dataset.settingsPanel = "usage";
        panelUsage.dataset.role = "settings-scroll";

        bodyEl.appendChild(panelSystem);
        bodyEl.appendChild(panelAgents);
        bodyEl.appendChild(panelBackends);
        bodyEl.appendChild(panelUsage);

        root.appendChild(toolbar);
        root.appendChild(bodyEl);
        body.appendChild(root);

        tabs.addEventListener("click", (e) => {
          const btn = e.target.closest("[data-settings-tab]");
          if (!btn) return;
          switchSettingsTab(body, btn.dataset.settingsTab);
        });
        tabs.addEventListener("keydown", (e) => {
          if (e.key !== "Enter" && e.key !== " ") return;
          const btn = e.target.closest("[data-settings-tab]");
          if (!btn) return;
          e.preventDefault();
          switchSettingsTab(body, btn.dataset.settingsTab);
        });

        body.addEventListener("mousedown", () => {
          focusWindowLocally(windowData.id);
          send({ kind: "focus_window", id: windowData.id });
        });
        settingsWindowBodies.add(body);

        renderSystemPanel(panelSystem);
        renderUsagePanel(panelUsage);
        // Always request fresh system settings on open so the dropdown
        // reflects the on-disk config, even if the user changed it from a
        // different gwt instance.
        send({ kind: "get_system_settings" });
        // SPEC-2963: also fetch remote Board provider sign-in state.
        send({ kind: "get_board_auth_status" });
        send({ kind: "get_autostart_status" });

        renderSettingsAgentList();
        if (!customAgentsState.loading && customAgentsState.agents.length === 0) {
          customAgentsState.loading = true;
          send({ kind: "list_custom_agents" });
        }

        // SPEC-1921 2026-05-18 amendment / FR-099: hydrate the Agent
        // Backends panel for both built-in agents that support Backend
        // Override (Claude Code, Codex). The backend returns the redacted
        // list (api_key replaced with `***REDACTED***`).
        renderAgentBackendsPanel(panelBackends);
        for (const agent of ["claudeCode", "codex"]) {
          send({ kind: "list_agent_backends", agent });
        }

        // Honour any pending settings:open dispatch (e.g. from the badge
        // click) by switching to the requested tab once the panel is mounted.
        if (pendingSettingsTabTarget) {
          switchSettingsTab(body, pendingSettingsTabTarget);
          pendingSettingsTabTarget = null;
        }
      }

      let pendingSettingsTabTarget = null;

      function renderIndexPanel(panel) {
        const activeProjectRoot = activeProjectTab()?.project_root || "";
        const status =
          (activeProjectRoot && indexStatusByProjectRoot.get(activeProjectRoot)) || null;
        renderIndexSettingsPanel({
          panel,
          status,
          projectRoot: activeProjectRoot,
          send,
        });
      }

      // SPEC-1921 2026-05-18 amendment / FR-099: render the `Agent Backends`
      // Settings tab body. Two `[data-agent]` sections (`claudeCode` /
      // `codex`) host the per-built-in backend lists; each saved backend
      // renders as a row with the redacted profile shape. The Add /
      // Edit / Delete affordances will land alongside the protocol
      // dispatch when the inline forms move out of the legacy
      // `Custom Agents` tab (T308 follow-up). Today the panel exposes a
      // read-only mirror that confirms FR-101 silent migration produced
      // the expected `[builtinAgents.claudeCode.backends.*]` rows.
      function renderAgentBackendsPanel(panel) {
        while (panel.firstChild) panel.removeChild(panel.firstChild);

        for (const agent of ["claudeCode", "codex"]) {
          const section = createDiv("settings-section");
          section.dataset.agent = agent;

          const heading = document.createElement("h3");
          heading.className = "settings-section-heading";
          heading.textContent =
            agent === "claudeCode" ? "Claude Code" : "Codex";
          section.appendChild(heading);

          const list = createDiv("agent-backends-list");
          list.dataset.role = "agent-backends-list";
          list.dataset.agent = agent;
          renderAgentBackendsList(list, agent);
          section.appendChild(list);

          panel.appendChild(section);
        }
      }

      function renderAgentBackendsList(container, agent) {
        while (container.firstChild) container.removeChild(container.firstChild);
        const profiles = agentBackendsState.backends[agent] || [];
        if (profiles.length === 0) {
          const empty = document.createElement("p");
          empty.className = "settings-help";
          empty.textContent =
            agent === "claudeCode"
              ? "No Claude Code backend profiles saved. Default Anthropic upstream is used."
              : "No Codex backend profiles saved. Default OpenAI upstream is used.";
          container.appendChild(empty);
          return;
        }
        for (const profile of profiles) {
          const row = createDiv("agent-backend-row");
          row.dataset.backendId = profile.id;
          const title = document.createElement("strong");
          title.textContent =
            profile.display_name || profile.displayName || profile.id;
          row.appendChild(title);
          const detail = document.createElement("span");
          detail.className = "settings-help";
          const baseUrl = profile.base_url || profile.baseUrl || "";
          const model = profile.model || "";
          detail.textContent = ` · ${baseUrl} · ${model}`;
          row.appendChild(detail);
          container.appendChild(row);
        }
      }

      function renderAgentBackendsPanelInAllSettingsWindows() {
        for (const settingsBody of Array.from(settingsWindowBodies)) {
          if (!settingsBody.isConnected) {
            settingsWindowBodies.delete(settingsBody);
            continue;
          }
          const panel = settingsBody.querySelector(
            "[data-settings-panel='agent-backends']",
          );
          if (panel) renderAgentBackendsPanel(panel);
        }
      }

      function renderIndexPanelInAllSettingsWindows() {
        for (const settingsBody of Array.from(settingsWindowBodies)) {
          if (!settingsBody.isConnected) {
            settingsWindowBodies.delete(settingsBody);
            continue;
          }
          const panel = settingsBody.querySelector(
            "[data-settings-panel='index']",
          );
          if (panel) renderIndexPanel(panel);
        }
      }

      function requestFullIndexStatusRefresh() {
        const activeProjectRoot = activeProjectTab()?.project_root || "";
        if (!activeProjectRoot) return;
        send({ kind: "refresh_index_status", project_root: activeProjectRoot });
      }

      document.addEventListener("settings:open", (event) => {
        const target = event?.detail?.target || "system";
        if (target === "index") {
          focusOrSpawnPreset("index");
          return;
        }
        const existingBody = Array.from(settingsWindowBodies).find(
          (settingsBody) => settingsBody.isConnected,
        );
        if (existingBody) {
          switchSettingsTab(existingBody, target);
          return;
        }
        pendingSettingsTabTarget = target;
        focusOrSpawnPreset("settings");
      });

      function buildSettingsTab(id, label, selected) {
        const btn = document.createElement("button");
        btn.type = "button";
        btn.className = selected ? "settings-tab active" : "settings-tab";
        btn.setAttribute("role", "tab");
        btn.setAttribute("aria-selected", String(selected));
        btn.dataset.settingsTab = id;
        btn.textContent = label;
        return btn;
      }

      function switchSettingsTab(body, target) {
        const tabs = body.querySelectorAll(".settings-tab");
        tabs.forEach((tab) => {
          const isSelected = tab.dataset.settingsTab === target;
          tab.setAttribute("aria-selected", String(isSelected));
          tab.classList.toggle("active", isSelected);
        });
        const panels = body.querySelectorAll(".settings-panel");
        panels.forEach((panel) => {
          panel.classList.toggle(
            "hidden",
            panel.dataset.settingsPanel !== target,
          );
        });
      }

      function composeTeamsDefaultChannel(teamId, channelId) {
        const team = String(teamId || "").trim();
        const channel = String(channelId || "").trim();
        if (!team && !channel) return "";
        if (!team) return channel;
        if (!channel) return team;
        return `${team}/${channel}`;
      }

      function parseTeamsDefaultChannel(value) {
        const raw = String(value || "").trim();
        if (!raw) return { teamId: "", channelId: "" };
        const slash = raw.indexOf("/");
        if (slash === -1) return { teamId: "", channelId: raw };
        return {
          teamId: raw.slice(0, slash).trim(),
          channelId: raw.slice(slash + 1).trim(),
        };
      }

      function formatTeamsChannelLink(defaultChannel, tenantId) {
        const { teamId, channelId } = parseTeamsDefaultChannel(defaultChannel);
        if (!teamId || !channelId) return "";
        const tenant = String(tenantId || "").trim();
        const params = new URLSearchParams({ groupId: teamId });
        if (tenant) params.set("tenantId", tenant);
        return `https://teams.microsoft.com/l/channel/${encodeURIComponent(channelId)}/configured-channel?${params.toString()}`;
      }

      function parseTeamsChannelLink(value) {
        const raw = String(value || "").trim();
        if (!raw) return null;
        let url;
        try {
          url = new URL(raw);
        } catch (_) {
          return null;
        }
        const teamId = (url.searchParams.get("groupId") || "").trim();
        const segments = url.pathname.split("/").filter(Boolean);
        const channelIndex = segments.findIndex(
          (segment) => segment.toLowerCase() === "channel",
        );
        const encodedChannel =
          channelIndex >= 0 ? segments[channelIndex + 1] || "" : "";
        let channelId = encodedChannel.trim();
        if (channelId) {
          try {
            channelId = decodeURIComponent(channelId);
          } catch (_) {
            // Keep the raw segment; it is still a better hint than clearing it.
          }
        }
        if (!teamId && !channelId) return null;
        return { teamId, channelId };
      }

      function renderSystemPanel(panel) {
        while (panel.firstChild) panel.removeChild(panel.firstChild);

        const section = createDiv("settings-section");

        const label = document.createElement("label");
        label.className = "settings-label";
        label.setAttribute("for", "settings-system-language");
        label.textContent = "Output Language";
        section.appendChild(label);

        const select = document.createElement("select");
        select.className = "settings-select";
        select.id = "settings-system-language";
        for (const opt of [
          { value: "auto", text: "Auto (OS locale)" },
          { value: "en", text: "English" },
          { value: "ja", text: "日本語" },
        ]) {
          const option = document.createElement("option");
          option.value = opt.value;
          option.textContent = opt.text;
          select.appendChild(option);
        }
        select.value = systemSettingsState.language || "auto";
        select.addEventListener("change", (e) => {
          const next = e.target.value;
          systemSettingsState.language = next;
          systemSettingsState.statusMessage = "Saving…";
          systemSettingsState.statusKind = "info";
          renderSystemPanelStatus(panel);
          send({ kind: "update_system_settings", language: next });
        });
        section.appendChild(select);

        const help = document.createElement("p");
        help.className = "settings-help";
        help.textContent =
          "Used for narrative outputs (Work summaries and Board post bodies). " +
          "Settings UI text and gwtd subcommands stay English.";
        section.appendChild(help);

        const trustSection = createDiv("settings-section");
        const trustLabel = document.createElement("label");
        trustLabel.className = "settings-checkbox-label";
        trustLabel.setAttribute("for", "settings-system-codex-hooks");

        const trustCheckbox = document.createElement("input");
        trustCheckbox.type = "checkbox";
        trustCheckbox.className = "settings-checkbox";
        trustCheckbox.id = "settings-system-codex-hooks";
        trustCheckbox.checked = systemSettingsState.codexTrustManagedHooks !== false;
        trustCheckbox.addEventListener("change", (e) => {
          const next = e.target.checked === true;
          systemSettingsState.codexTrustManagedHooks = next;
          systemSettingsState.statusMessage = "Saving…";
          systemSettingsState.statusKind = "info";
          renderSystemPanelStatus(panel);
          send({
            kind: "update_system_settings",
            language: systemSettingsState.language || "auto",
            codex_trust_managed_hooks: next,
          });
        });

        const trustText = document.createElement("span");
        trustText.textContent = "Trust gwt-managed Codex hooks";
        trustLabel.appendChild(trustCheckbox);
        trustLabel.appendChild(trustText);
        trustSection.appendChild(trustLabel);

        const trustHelp = document.createElement("p");
        trustHelp.className = "settings-help";
        trustHelp.textContent =
          "Enabled by default. Registers only generated gwt hook commands in Codex hook trust state.";
        trustSection.appendChild(trustHelp);

        // SPEC-2959/2963: Board provider selector. `local` keeps the Board
        // offline; `slack` / `teams` are network-backed and selectable. Picking
        // a remote provider reveals its config form (client id / channel /
        // secret) and a sign-in affordance below.
        const boardSection = createDiv("settings-section");
        const boardLabel = document.createElement("label");
        boardLabel.className = "settings-label";
        boardLabel.setAttribute("for", "settings-system-board-provider");
        boardLabel.textContent = "Board provider";
        boardSection.appendChild(boardLabel);

        const boardSelect = document.createElement("select");
        boardSelect.className = "settings-select";
        boardSelect.id = "settings-system-board-provider";
        for (const opt of [
          { value: "local", text: "Local (offline)" },
          { value: "slack", text: "Slack" },
          { value: "teams", text: "Teams" },
        ]) {
          const option = document.createElement("option");
          option.value = opt.value;
          option.textContent = opt.text;
          boardSelect.appendChild(option);
        }
        boardSelect.value = systemSettingsState.boardProvider || "local";
        boardSelect.addEventListener("change", (e) => {
          const next = e.target.value;
          systemSettingsState.boardProvider = next;
          systemSettingsState.statusMessage = "Saving…";
          systemSettingsState.statusKind = "info";
          renderSystemPanelStatus(panel);
          send({
            kind: "update_system_settings",
            language: systemSettingsState.language || "auto",
            board_provider: next,
          });
          renderSystemPanelInAllSettingsWindows();
        });
        boardSection.appendChild(boardSelect);

        const boardHelp = document.createElement("p");
        boardHelp.className = "settings-help";
        boardHelp.textContent =
          "Where the coordination Board is stored. Local keeps the Board offline and " +
          "on this machine. Slack / Teams require sign-in and are network-backed.";
        boardSection.appendChild(boardHelp);

        // SPEC-2963 FR-011/FR-012: sign-in affordance + auth status for the
        // selected remote provider. Local needs no sign-in.
        const selectedProvider = systemSettingsState.boardProvider || "local";
        if (selectedProvider === "slack" || selectedProvider === "teams") {
          // SPEC-2963 FR-006: provider config form. Non-secret fields persist to
          // config.toml; the client secret is routed to the secure credential
          // store and never echoed back (placeholder shows "configured").
          const cfg = systemSettingsState.boardConfig || {};
          const configForm = createDiv("settings-section board-config-form");
          configForm.dataset.provider = selectedProvider;

          const makeField = (id, labelText, value, opts = {}) => {
            const wrap = createDiv("settings-field");
            const fieldLabel = document.createElement("label");
            fieldLabel.className = "settings-label";
            fieldLabel.setAttribute("for", id);
            fieldLabel.textContent = labelText;
            wrap.appendChild(fieldLabel);
            const input = document.createElement("input");
            input.className = "settings-input";
            input.id = id;
            input.type = opts.password ? "password" : "text";
            input.value = value || "";
            if (opts.placeholder) input.placeholder = opts.placeholder;
            if (opts.autocomplete) input.autocomplete = opts.autocomplete;
            wrap.appendChild(input);
            configForm.appendChild(wrap);
            return input;
          };

          let clientIdInput;
          let defaultChannelInput;
          let tenantIdInput;
          let teamsChannelLinkInput;
          let secretInput;
          if (selectedProvider === "slack") {
            clientIdInput = makeField(
              "settings-board-slack-client-id",
              "Client ID",
              cfg.slackClientId,
              { placeholder: "e.g. 1234567890.1234567890" },
            );
            defaultChannelInput = makeField(
              "settings-board-slack-channel",
              "Default channel ID",
              cfg.slackDefaultChannel,
              { placeholder: "e.g. C0123456789" },
            );
            secretInput = makeField(
              "settings-board-slack-secret",
              "Client secret",
              "",
              {
                password: true,
                autocomplete: "new-password",
                placeholder: cfg.slackHasSecret
                  ? "configured — leave blank to keep"
                  : "required for Slack sign-in",
              },
            );
            // The secret is stored securely and never echoed back, so the
            // field intentionally clears after Save. Show an explicit saved
            // state so it is obvious the secret persisted (id used by tests).
            const secretState = createNode(
              "p",
              "settings-help board-secret-state",
              cfg.slackHasSecret
                ? "✓ A client secret is saved (the field stays blank for security)."
                : "No client secret saved yet.",
            );
            secretState.dataset.hasSecret = cfg.slackHasSecret ? "true" : "false";
            configForm.appendChild(secretState);
          } else {
            clientIdInput = makeField(
              "settings-board-teams-client-id",
              "Application (client) ID",
              cfg.teamsClientId,
              { placeholder: "Entra app id" },
            );
            tenantIdInput = makeField(
              "settings-board-teams-tenant-id",
              "Tenant ID",
              cfg.teamsTenantId,
              { placeholder: "tenant id / common / organizations" },
            );
            teamsChannelLinkInput = makeField(
              "settings-board-teams-channel-link",
              "Teams channel link",
              formatTeamsChannelLink(
                cfg.teamsDefaultChannel,
                cfg.teamsTenantId,
              ),
              { placeholder: "https://teams.microsoft.com/l/channel/..." },
            );
            const teamsChannelHelp = createNode(
              "p",
              "settings-help",
              cfg.teamsDefaultChannel
                ? "Saved channel link is shown here. Paste a new channel link to change it."
                : "Paste the link from Teams > Get link to channel. gwt extracts the team and channel IDs when saving.",
            );
            configForm.appendChild(teamsChannelHelp);
          }

          const saveBtn = createNode(
            "button",
            "wizard-button",
            "Save configuration",
          );
          saveBtn.type = "button";
          saveBtn.addEventListener("click", () => {
            if (selectedProvider === "teams") {
              const teamsChannelLinkValue = teamsChannelLinkInput
                ? teamsChannelLinkInput.value.trim()
                : "";
              let nextTeamsDefaultChannel = cfg.teamsDefaultChannel || "";
              if (teamsChannelLinkValue) {
                const parsedTeamsChannel = parseTeamsChannelLink(
                  teamsChannelLinkValue,
                );
                if (
                  !parsedTeamsChannel ||
                  !parsedTeamsChannel.teamId ||
                  !parsedTeamsChannel.channelId
                ) {
                  systemSettingsState.statusMessage =
                    "Paste a valid Teams channel link with groupId and /channel/...";
                  systemSettingsState.statusKind = "error";
                  renderSystemPanelStatus(panel);
                  return;
                }
                nextTeamsDefaultChannel = composeTeamsDefaultChannel(
                  parsedTeamsChannel.teamId,
                  parsedTeamsChannel.channelId,
                );
              }
              send({
                kind: "update_board_provider_config",
                provider: selectedProvider,
                client_id: clientIdInput ? clientIdInput.value.trim() : "",
                default_channel: nextTeamsDefaultChannel,
                tenant_id: tenantIdInput ? tenantIdInput.value.trim() : "",
              });
              return;
            }
            const payload = {
              kind: "update_board_provider_config",
              provider: selectedProvider,
              client_id: clientIdInput ? clientIdInput.value.trim() : "",
              default_channel: defaultChannelInput
                ? defaultChannelInput.value.trim()
                : "",
            };
            if (selectedProvider === "slack" && secretInput) {
              // Only send the secret when the user typed one, so an empty box
              // does not clear an already-configured secret.
              if (secretInput.value.length > 0) {
                payload.client_secret = secretInput.value;
              }
            }
            send(payload);
          });
          configForm.appendChild(saveBtn);
          boardSection.appendChild(configForm);

          // SPEC-2963 FR-005: fixed OAuth callback port. The redirect_uri must
          // exactly match a URL registered in the provider app; gwt binds this
          // loopback port so sign-in works regardless of the (ephemeral) GUI
          // server port. Editable so a busy 8765 can be changed.
          const oauthPort = Number(cfg.oauthRedirectPort) || 8765;
          const oauthForm = createDiv("settings-section board-oauth-port-form");
          const portField = createDiv("settings-field");
          const portLabel = document.createElement("label");
          portLabel.className = "settings-label";
          portLabel.setAttribute("for", "settings-board-oauth-port");
          portLabel.textContent = "OAuth callback port";
          portField.appendChild(portLabel);
          const portInput = document.createElement("input");
          portInput.className = "settings-input";
          portInput.id = "settings-board-oauth-port";
          portInput.type = "number";
          portInput.min = "1";
          portInput.max = "65535";
          portInput.value = String(oauthPort);
          portField.appendChild(portInput);
          oauthForm.appendChild(portField);

          const redirectHint = createNode(
            "p",
            "settings-help board-oauth-redirect-hint",
            "",
          );
          const renderRedirectHint = () => {
            const p = Number(portInput.value) || oauthPort;
            redirectHint.textContent =
              "Register this exact Redirect URL in the Slack/Teams app: " +
              `http://127.0.0.1:${p}/oauth/callback`;
          };
          renderRedirectHint();
          portInput.addEventListener("input", renderRedirectHint);
          oauthForm.appendChild(redirectHint);

          const savePortBtn = createNode("button", "wizard-button", "Save port");
          savePortBtn.type = "button";
          savePortBtn.addEventListener("click", () => {
            const next = Number(portInput.value);
            send({
              kind: "update_board_oauth_port",
              port: Number.isFinite(next) && next > 0 ? Math.floor(next) : 0,
            });
          });
          oauthForm.appendChild(savePortBtn);
          boardSection.appendChild(oauthForm);

          const auth = systemSettingsState.boardAuth || { slack: false, teams: false };
          const signedIn = auth[selectedProvider] === true;
          const authRow = createDiv("settings-section board-auth-row");
          const statusText = createNode(
            "span",
            "board-auth-status",
            signedIn
              ? `Signed in to ${selectedProvider}`
              : `Not signed in to ${selectedProvider}`,
          );
          statusText.dataset.signedIn = signedIn ? "true" : "false";
          authRow.appendChild(statusText);

          const signInBtn = createNode(
            "button",
            "wizard-button",
            signedIn ? "Re-sign in" : "Sign in",
          );
          signInBtn.type = "button";
          signInBtn.addEventListener("click", () => {
            send({ kind: "board_provider_sign_in", provider: selectedProvider });
          });
          authRow.appendChild(signInBtn);

          if (signedIn) {
            const signOutBtn = createNode("button", "text-button", "Sign out");
            signOutBtn.type = "button";
            signOutBtn.addEventListener("click", () => {
              send({ kind: "board_provider_sign_out", provider: selectedProvider });
            });
            authRow.appendChild(signOutBtn);
          }

          const refreshBtn = createNode("button", "text-button", "Refresh");
          refreshBtn.type = "button";
          refreshBtn.addEventListener("click", () => {
            send({ kind: "get_board_auth_status" });
          });
          authRow.appendChild(refreshBtn);
          boardSection.appendChild(authRow);

          if (systemSettingsState.boardAuthMessage) {
            boardSection.appendChild(
              createNode("p", "settings-help", systemSettingsState.boardAuthMessage),
            );
          }
        }

        const autostartSection = createDiv("settings-section");
        const autostartLabel = document.createElement("label");
        autostartLabel.className = "settings-checkbox-label";
        autostartLabel.setAttribute("for", "settings-system-autostart");

        const autostartCheckbox = document.createElement("input");
        autostartCheckbox.type = "checkbox";
        autostartCheckbox.className = "settings-checkbox";
        autostartCheckbox.id = "settings-system-autostart";
        autostartCheckbox.checked = systemSettingsState.autostartEnabled === true;
        autostartCheckbox.disabled = systemSettingsState.autostartPending === true;
        autostartCheckbox.addEventListener("change", (e) => {
          const next = e.target.checked === true;
          systemSettingsState.autostartPreviousEnabled =
            systemSettingsState.autostartEnabled === true;
          systemSettingsState.autostartEnabled = next;
          systemSettingsState.autostartPending = true;
          systemSettingsState.statusMessage = "Saving…";
          systemSettingsState.statusKind = "info";
          renderSystemPanelInAllSettingsWindows();
          send({ kind: "update_autostart", enabled: next });
        });

        const autostartText = document.createElement("span");
        autostartText.textContent = "Launch GWT at login";
        autostartLabel.appendChild(autostartCheckbox);
        autostartLabel.appendChild(autostartText);
        autostartSection.appendChild(autostartLabel);

        const autostartHelp = document.createElement("p");
        autostartHelp.className = "settings-help";
        autostartHelp.textContent =
          "Starts GWT in the menu bar when you log in. The browser does not open automatically.";
        autostartSection.appendChild(autostartHelp);

        if (systemSettingsState.autostartLoaded) {
          const autostartDetail = document.createElement("p");
          autostartDetail.className = "settings-help";
          const mechanism = systemSettingsState.autostartMechanism || "Unknown";
          const installPath = systemSettingsState.autostartInstallPath || "";
          autostartDetail.textContent = installPath
            ? `Autostart: ${mechanism} · ${installPath}`
            : `Autostart: ${mechanism}`;
          autostartSection.appendChild(autostartDetail);
        }

        const status = document.createElement("p");
        status.className = "settings-status";
        status.dataset.role = "system-settings-status";
        autostartSection.appendChild(status);

        panel.appendChild(section);
        panel.appendChild(trustSection);
        panel.appendChild(boardSection);
        panel.appendChild(autostartSection);
        renderSystemPanelStatus(panel);
      }

      function renderSystemPanelStatus(panel) {
        const status = panel.querySelector(
          "[data-role='system-settings-status']",
        );
        if (!status) return;
        status.textContent = systemSettingsState.statusMessage || "";
        if (systemSettingsState.statusKind) {
          status.dataset.kind = systemSettingsState.statusKind;
        } else {
          delete status.dataset.kind;
        }
      }

      function renderSystemPanelInAllSettingsWindows() {
        for (const body of Array.from(settingsWindowBodies)) {
          if (!body.isConnected) {
            settingsWindowBodies.delete(body);
            continue;
          }
          const panel = body.querySelector(
            "[data-settings-panel='system']",
          );
          if (panel) renderSystemPanel(panel);
        }
      }

      function renderSettingsAgentList() {
        for (const body of Array.from(settingsWindowBodies)) {
          if (!body.isConnected) {
            settingsWindowBodies.delete(body);
            continue;
          }
          const scroll = body.querySelector("[data-role='settings-scroll']");
          if (!scroll) continue;
          while (scroll.firstChild) scroll.removeChild(scroll.firstChild);

          const addBtn = document.createElement("button");
          addBtn.className = "wizard-button";
          addBtn.style.margin = "8px 0";
          // SPEC-1921 Phase 63H / T326: the legacy
          // `+ Add Claude Code (OpenAI-compat backend)` button now points
          // users at the new `Agent Backends` tab. The underlying
          // `add_custom_agent_from_preset` dispatch is preserved for
          // existing callers (Phase 52 contract), but the entry point
          // visible in Custom Agents redirects to the proper surface so
          // External CLI rows and Backend Override profiles never get
          // conflated again.
          addBtn.textContent = "＋ Add Claude Code backend (moved to Agent Backends)";
          addBtn.addEventListener("click", (e) => {
            e.stopPropagation();
            // Switch the Settings window to the Agent Backends tab.
            const body = scroll.closest(".settings-body")?.parentElement;
            if (body) switchSettingsTab(body, "agent-backends");
            setSettingsStatus(
              "Backend Override moved to Agent Backends. Add your Claude Code / Codex backend there.",
              "success",
            );
          });
          scroll.appendChild(addBtn);

          if (customAgentsState.statusMessage) {
            const section = createDiv("mock-section");
            const label = createDiv("mock-label");
            label.textContent = "Status";
            label.style.color =
              customAgentsState.statusKind === "error"
                ? "#ff6b6b"
                : customAgentsState.statusKind === "success"
                  ? "#7abf7a"
                  : "#999";
            section.appendChild(label);
            const row = createDiv("mock-row");
            const text = document.createElement("span");
            text.textContent = customAgentsState.statusMessage;
            row.appendChild(text);
            section.appendChild(row);
            scroll.appendChild(section);
          }

          if (customAgentsState.loading && customAgentsState.agents.length === 0) {
            const section = createDiv("mock-section");
            const row = createDiv("mock-row");
            const text = document.createElement("span");
            text.textContent = "Loading custom agents…";
            row.appendChild(text);
            section.appendChild(row);
            scroll.appendChild(section);
            continue;
          }

          if (customAgentsState.agents.length === 0) {
            const section = createDiv("mock-section");
            const row = createDiv("mock-row");
            const text = document.createElement("span");
            text.textContent = "No custom agents configured yet.";
            row.appendChild(text);
            section.appendChild(row);
            scroll.appendChild(section);
            continue;
          }

          for (const agent of customAgentsState.agents) {
            const section = createDiv("mock-section");
            const label = createDiv("mock-label");
            label.textContent = agent.display_name || agent.id;
            section.appendChild(label);
            const row = createDiv("mock-row");
            const text = document.createElement("span");
            const envCount = Object.keys(agent.env || {}).length;
            const baseUrl =
              agent.env && agent.env.ANTHROPIC_BASE_URL
                ? ` · ${agent.env.ANTHROPIC_BASE_URL}`
                : "";
            text.textContent = `${agent.id} · ${agent.command} · ${envCount} env entries${baseUrl}`;
            row.appendChild(text);
            const delBtn = document.createElement("button");
            delBtn.className = "icon-button";
            delBtn.setAttribute("aria-label", "Delete agent");
            delBtn.textContent = "×";
            delBtn.addEventListener("click", (e) => {
              e.stopPropagation();
              if (window.confirm(`Delete custom agent "${agent.id}"?`)) {
                send({ kind: "delete_custom_agent", agent_id: agent.id });
              }
            });
            const editBtn = document.createElement("button");
            editBtn.className = "icon-button";
            editBtn.setAttribute("aria-label", "Edit agent environment");
            editBtn.title = "Edit environment";
            editBtn.textContent = "✎";
            editBtn.addEventListener("click", (e) => {
              e.stopPropagation();
              editingCustomAgentId =
                editingCustomAgentId === agent.id ? null : agent.id;
              renderSettingsAgentList();
            });
            row.appendChild(editBtn);
            row.appendChild(delBtn);
            section.appendChild(row);
            if (editingCustomAgentId === agent.id) {
              section.appendChild(
                renderCustomAgentEnvEditor({
                  document,
                  agent,
                  onSave: (updatedAgent) => {
                    editingCustomAgentId = null;
                    setSettingsStatus("Saving custom agent…", "info");
                    send({ kind: "update_custom_agent", agent: updatedAgent });
                  },
                  onCancel: () => {
                    editingCustomAgentId = null;
                    renderSettingsAgentList();
                  },
                }),
              );
            }
            scroll.appendChild(section);
          }
        }
      }

      function startAddClaudeCodeOpenaiCompatFlow() {
        const baseUrl = window.prompt(
          "Upstream base_url (http:// or https://)\n\nExample: http://192.168.100.166:32768",
          "http://",
        );
        if (!baseUrl) return;
        const apiKey = window.prompt(
          "API key (forwarded as Bearer on /v1/models probe and ANTHROPIC_API_KEY at launch):",
        );
        if (!apiKey) return;
        setSettingsStatus("Probing /v1/models…", "info");
        pendingAddFromPreset = { baseUrl, apiKey };
        send({ kind: "test_backend_connection", base_url: baseUrl, api_key: apiKey });
      }

      function setSettingsStatus(message, kind) {
        customAgentsState.statusMessage = message;
        customAgentsState.statusKind = kind;
        renderSettingsAgentList();
      }

      function completeAddFromPreset(discoveredModels) {
        if (!pendingAddFromPreset) return;
        if (!discoveredModels || discoveredModels.length === 0) {
          setSettingsStatus("Upstream /v1/models returned no entries.", "error");
          pendingAddFromPreset = null;
          return;
        }
        const modelList = discoveredModels.join("\n");
        const model = window.prompt(
          `Discovered ${discoveredModels.length} model(s). Choose default_model (copy one ID):\n\n${modelList}`,
          discoveredModels[0],
        );
        if (!model) {
          pendingAddFromPreset = null;
          setSettingsStatus("Cancelled.", "info");
          return;
        }
        const id = window.prompt("Custom agent id (alphanumeric + `-`):", "claude-code-openai");
        if (!id) {
          pendingAddFromPreset = null;
          setSettingsStatus("Cancelled.", "info");
          return;
        }
        const displayName = window.prompt("Display name:", "Claude Code (OpenAI-compat)");
        if (!displayName) {
          pendingAddFromPreset = null;
          setSettingsStatus("Cancelled.", "info");
          return;
        }
        setSettingsStatus("Saving preset…", "info");
        send({
          kind: "add_custom_agent_from_preset",
          input: {
            id,
            display_name: displayName,
            base_url: pendingAddFromPreset.baseUrl,
            api_key: pendingAddFromPreset.apiKey,
            default_model: model,
          },
        });
        pendingAddFromPreset = null;
      }


      // SPEC-3064 Phase 3 (E4): receive() delegates for the two settings
      // cases whose bodies reassign module-level state
      // (editingCustomAgentId / pendingAddFromPreset) and therefore cannot
      // stay inline in app.js once that state lives here.
      function applyCustomAgentDeleted(event) {
        customAgentsState.agents = customAgentsState.agents.filter(
          (a) => a.id !== event.agent_id,
        );
        if (editingCustomAgentId === event.agent_id) {
          editingCustomAgentId = null;
        }
        setSettingsStatus(`Deleted custom agent "${event.agent_id}".`, "success");
      }

      function applyCustomAgentError(event) {
        customAgentsState.loading = false;
        pendingAddFromPreset = null;
        setSettingsStatus(
          `Error [${event.code}]: ${event.message}`,
          "error",
        );
      }

      return {
        customAgentsState,
        agentBackendsState,
        systemSettingsState,
        systemSettingsInteractionGuard,
        applyAutostartStatus,
        applyAutostartError,
        applyCustomAgentDeleted,
        applyCustomAgentError,
        renderSettingsWindow,
        renderSettingsAgentList,
        renderAgentBackendsPanel,
        renderAgentBackendsPanelInAllSettingsWindows,
        renderSystemPanelInAllSettingsWindows,
        renderIndexPanelInAllSettingsWindows,
        requestFullIndexStatusRefresh,
        setSettingsStatus,
        completeAddFromPreset,
      };
}
