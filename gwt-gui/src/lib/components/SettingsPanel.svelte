<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import type {
    SkillRegistrationStatus,
    SkillRegistrationScope,
    ProfilesConfig,
    Profile,
    SettingsData,
    ShellInfo,
    VoiceInputSettings,
  } from "../types";

  let { onClose }: { onClose: () => void } = $props();

  type SettingsTabId =
    | "appearance"
    | "voiceInput"
    | "githubIntegration"
    | "profiles"
    | "terminal";
  let activeSettingsTab: SettingsTabId = $state("appearance");

  let availableShells: ShellInfo[] = $state([]);

  let settings: SettingsData | null = $state(null);
  let profiles: ProfilesConfig | null = $state(null);

  let loadingSettings: boolean = $state(true);
  let loadingProfiles: boolean = $state(true);
  let saving: boolean = $state(false);
  let errorMessage: string | null = $state(null);
  let newBranch: string = $state("");
  let saveMessage: string = $state("");

  let selectedProfileKey: string = $state("");
  let newProfileName: string = $state("");
  let newEnvKey: string = $state("");
  let newEnvValue: string = $state("");

  let savedUiFontSize: number = $state(13);
  let savedTerminalFontSize: number = $state(13);
  let savedUiFontFamily: string = $state(
    'system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif'
  );
  let savedTerminalFontFamily: string = $state(
    '"JetBrains Mono", "Fira Code", "SF Mono", Menlo, Consolas, monospace'
  );
  let skillStatus: SkillRegistrationStatus | null = $state(null);
  let skillStatusLoading: boolean = $state(false);
  let skillStatusRepairing: boolean = $state(false);
  let skillStatusMessage: string = $state("");

  const DEFAULT_VOICE_INPUT: VoiceInputSettings = {
    enabled: false,
    hotkey: "Mod+Shift+M",
    language: "auto",
    model: "base",
  };
  const DEFAULT_UI_FONT_FAMILY =
    'system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif';
  const DEFAULT_TERMINAL_FONT_FAMILY =
    '"JetBrains Mono", "Fira Code", "SF Mono", Menlo, Consolas, monospace';
  type FontPreset = { label: string; value: string };
  const UI_FONT_PRESETS: FontPreset[] = [
    { label: "System UI (Default)", value: DEFAULT_UI_FONT_FAMILY },
    {
      label: "Inter",
      value: '"Inter", system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif',
    },
    {
      label: "Noto Sans",
      value: '"Noto Sans", system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif',
    },
    {
      label: "Source Sans 3",
      value:
        '"Source Sans 3", system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif',
    },
  ];
  const TERMINAL_FONT_PRESETS: FontPreset[] = [
    { label: "JetBrains Mono (Default)", value: DEFAULT_TERMINAL_FONT_FAMILY },
    {
      label: "Cascadia Mono",
      value: '"Cascadia Mono", "Cascadia Code", Consolas, monospace',
    },
    {
      label: "Fira Code",
      value: '"Fira Code", "JetBrains Mono", Menlo, Consolas, monospace',
    },
    {
      label: "SF Mono",
      value: '"SF Mono", Menlo, Monaco, Consolas, monospace',
    },
    {
      label: "Ubuntu Mono",
      value: '"Ubuntu Mono", "DejaVu Sans Mono", Consolas, monospace',
    },
  ];
  const DEFAULT_APP_LANGUAGE: SettingsData["app_language"] = "auto";
  const DEFAULT_SKILL_STATUS: SkillRegistrationStatus = {
    overall: "failed",
    agents: [],
    last_checked_at: 0,
    last_error_message: null,
  };

  type AIModelInfo = {
    id: string;
  };
  let aiModels: string[] = $state([]);
  let aiModelsLoading: boolean = $state(false);
  let aiModelsError: string | null = $state(null);
  let aiModelsLoadedKey: string = $state("");
  let aiModelsRequestSeq: number = 0;

  function getCurrentProfile(cfg: ProfilesConfig | null, key: string): Profile | null {
    if (!cfg) return null;
    if (!key) return null;
    const p = cfg.profiles?.[key];
    return p ?? null;
  }

  let currentProfile = $derived(getCurrentProfile(profiles, selectedProfileKey));
  let currentAiRequestKey = $derived.by(() => {
    const profileKey = selectedProfileKey.trim();
    const ai = currentProfile?.ai;
    const endpoint = ai?.endpoint?.trim() ?? "";
    if (!profileKey || !ai || !endpoint) return "";
    const apiKey = ai?.api_key?.trim() ?? "";
    return `${profileKey}::${endpoint}::${apiKey}`;
  });
  let aiModelOptions = $derived.by(() => {
    const current =
      aiModelsLoadedKey === currentAiRequestKey ? (currentProfile?.ai?.model?.trim() ?? "") : "";
    const options = [...aiModels];
    if (current && !options.includes(current)) {
      options.unshift(current);
    }
    return options;
  });
  let currentModelMissing = $derived.by(() => {
    if (aiModelsLoadedKey !== currentAiRequestKey) return false;
    const current = currentProfile?.ai?.model?.trim() ?? "";
    return current.length > 0 && !aiModels.includes(current);
  });

  function resetAiModelsState() {
    aiModelsRequestSeq += 1;
    aiModels = [];
    aiModelsLoading = false;
    aiModelsError = null;
    aiModelsLoadedKey = "";
  }

  $effect(() => {
    loadAll();
  });

  $effect(() => {
    if (!settings) return;
    const uiSize = settings.ui_font_size ?? 13;
    const terminalSize = settings.terminal_font_size ?? 13;
    if (uiSize >= 8 && uiSize <= 24) {
      applyUiFontSize(uiSize);
    }
    if (terminalSize >= 8 && terminalSize <= 24) {
      applyTerminalFontSize(terminalSize);
    }
    applyUiFontFamily(settings.ui_font_family);
    applyTerminalFontFamily(settings.terminal_font_family);
  });

  function isAiEnabled(profile: Profile | null): boolean {
    if (!profile) return false;
    return !!(profile.ai?.endpoint?.trim());
  }

  $effect(() => {
    const profileKey = selectedProfileKey.trim();
    const ai = currentProfile?.ai;
    const endpoint = ai?.endpoint?.trim() ?? "";

    if (!profileKey || !ai || !isAiEnabled(currentProfile)) {
      if (aiModelsLoadedKey || aiModels.length > 0 || aiModelsError) {
        resetAiModelsState();
      }
      return;
    }
    if (!endpoint) {
      if (aiModelsLoadedKey || aiModels.length > 0 || aiModelsError) {
        resetAiModelsState();
      }
      return;
    }

    const requestKey = currentAiRequestKey;
    if (
      requestKey !== aiModelsLoadedKey &&
      (aiModelsLoadedKey || aiModels.length > 0 || aiModelsError)
    ) {
      resetAiModelsState();
    }
  });

  onMount(() => {
    const rootStyle = getComputedStyle(document.documentElement);
    const computed = rootStyle.getPropertyValue("--ui-font-base");
    const parsedUi = Number.parseInt(computed.trim(), 10);
    savedUiFontSize = Number.isNaN(parsedUi) ? 13 : parsedUi;
    const storedTerminal = (window as any).__gwtTerminalFontSize;
    savedTerminalFontSize = typeof storedTerminal === "number" ? storedTerminal : 13;
    const computedUiFamily = rootStyle.getPropertyValue("--ui-font-family");
    savedUiFontFamily = normalizeUiFontFamily(computedUiFamily);
    const storedTerminalFamily = (window as any).__gwtTerminalFontFamily;
    if (typeof storedTerminalFamily === "string") {
      savedTerminalFontFamily = normalizeTerminalFontFamily(storedTerminalFamily);
    } else {
      savedTerminalFontFamily = normalizeTerminalFontFamily(
        rootStyle.getPropertyValue("--terminal-font-family")
      );
    }
  });

  onDestroy(() => {
    applyUiFontSize(savedUiFontSize);
    applyTerminalFontSize(savedTerminalFontSize);
    applyUiFontFamily(savedUiFontFamily);
    applyTerminalFontFamily(savedTerminalFontFamily);
  });

  function toErrorMessage(err: unknown): string {
    if (typeof err === "string") return err;
    if (err && typeof err === "object" && "message" in err) {
      const msg = (err as { message?: unknown }).message;
      if (typeof msg === "string") return msg;
    }
    return String(err);
  }

  function normalizeVoiceInputSettings(
    value: Partial<VoiceInputSettings> | null | undefined
  ): VoiceInputSettings {
    const hotkey = (value?.hotkey ?? "").trim();
    const language = (value?.language ?? "").trim().toLowerCase();
    const model = (value?.model ?? "").trim();

    return {
      enabled: !!value?.enabled,
      hotkey: hotkey.length > 0 ? hotkey : DEFAULT_VOICE_INPUT.hotkey,
      language:
        language === "ja" || language === "en" || language === "auto"
          ? (language as VoiceInputSettings["language"])
          : DEFAULT_VOICE_INPUT.language,
      model: model.length > 0 ? model : DEFAULT_VOICE_INPUT.model,
    };
  }

  function normalizeAppLanguage(
    value: string | null | undefined
  ): SettingsData["app_language"] {
    const language = (value ?? "").trim().toLowerCase();
    if (language === "ja" || language === "en" || language === "auto") {
      return language as SettingsData["app_language"];
    }
    return DEFAULT_APP_LANGUAGE;
  }

  function normalizeUiFontFamily(value: string | null | undefined): string {
    const family = (value ?? "").trim();
    if (family.length === 0) return DEFAULT_UI_FONT_FAMILY;
    const match = UI_FONT_PRESETS.find((preset) => preset.value === family);
    return match ? match.value : family;
  }

  function normalizeTerminalFontFamily(value: string | null | undefined): string {
    const family = (value ?? "").trim();
    if (family.length === 0) return DEFAULT_TERMINAL_FONT_FAMILY;
    const match = TERMINAL_FONT_PRESETS.find((preset) => preset.value === family);
    return match ? match.value : family;
  }

  function normalizeSkillScope(
    value: string | null | undefined
  ): SkillRegistrationScope | null {
    const normalized = (value ?? "").trim().toLowerCase();
    if (normalized === "user" || normalized === "project" || normalized === "local") {
      return normalized as SkillRegistrationScope;
    }
    return null;
  }

  function normalizeSkillStatus(
    value: Partial<SkillRegistrationStatus> | null | undefined
  ): SkillRegistrationStatus {
    const agents = Array.isArray(value?.agents)
      ? value.agents.map((agent) => ({
          agent_id: agent.agent_id ?? "unknown",
          label: agent.label ?? "Unknown",
          skills_path: agent.skills_path ?? null,
          registered: !!agent.registered,
          missing_skills: Array.isArray(agent.missing_skills)
            ? agent.missing_skills.filter((skill) => typeof skill === "string")
            : [],
          error_code: agent.error_code ?? null,
          error_message: agent.error_message ?? null,
        }))
      : [];

    return {
      overall: value?.overall ?? DEFAULT_SKILL_STATUS.overall,
      agents,
      last_checked_at:
        typeof value?.last_checked_at === "number" ? value.last_checked_at : Date.now(),
      last_error_message: value?.last_error_message ?? null,
    };
  }

  function skillStatusClass(status: string): "status-ok" | "status-degraded" | "status-failed" {
    if (status === "ok") return "status-ok";
    if (status === "degraded") return "status-degraded";
    return "status-failed";
  }

  function skillStatusText(status: string | null | undefined): string {
    return (status ?? "unknown").toUpperCase();
  }

  function formatRegistrationCheckedAt(millis: number | null | undefined): string {
    if (typeof millis !== "number" || millis <= 0) {
      return "-";
    }
    try {
      return new Date(millis).toLocaleString();
    } catch {
      return "-";
    }
  }

  async function loadSkillStatus(showRefreshMessage: boolean) {
    skillStatusLoading = true;
    if (showRefreshMessage) {
      skillStatusMessage = "";
    }
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      const status = await invoke<SkillRegistrationStatus>("get_skill_registration_status_cmd");
      skillStatus = normalizeSkillStatus(status);
      if (showRefreshMessage) {
        skillStatusMessage = "Skill status refreshed.";
      }
    } catch (err) {
      skillStatus = skillStatus ?? normalizeSkillStatus(null);
      const message = toErrorMessage(err);
      skillStatusMessage = showRefreshMessage
        ? `Failed to refresh skill status: ${message}`
        : `Failed to load skill status: ${message}`;
    } finally {
      skillStatusLoading = false;
    }
  }

  async function repairSkillStatus() {
    skillStatusRepairing = true;
    skillStatusMessage = "";
    try {
      const { invoke } = await import("$lib/tauriInvoke");
      const status = await invoke<SkillRegistrationStatus>("repair_skill_registration_cmd");
      skillStatus = normalizeSkillStatus(status);
      skillStatusMessage =
        status.overall === "ok"
          ? "Skill registration repaired."
          : "Skill registration remains degraded. Check details below.";
    } catch (err) {
      skillStatusMessage = `Failed to repair skill registration: ${toErrorMessage(err)}`;
    } finally {
      skillStatusRepairing = false;
    }
  }

  async function fetchAiModels(
    endpoint: string,
    apiKey: string,
    requestKey: string,
    force: boolean
  ) {
    if (!force && requestKey === aiModelsLoadedKey) return;

    const requestSeq = ++aiModelsRequestSeq;
    aiModelsLoading = true;
    aiModelsError = null;

    try {
      const { invoke } = await import("$lib/tauriInvoke");
      const models = await invoke<AIModelInfo[]>("list_ai_models", {
        endpoint,
        apiKey,
      });
      if (requestSeq !== aiModelsRequestSeq) return;

      const nextModels = Array.from(
        new Set((models ?? []).map((m) => (m.id ?? "").trim()).filter((id) => id.length > 0))
      ).sort((a, b) => a.localeCompare(b));

      aiModels = nextModels;
      aiModelsLoadedKey = requestKey;
      aiModelsError = null;
    } catch (err) {
      if (requestSeq !== aiModelsRequestSeq) return;
      aiModels = [];
      aiModelsLoadedKey = requestKey;
      aiModelsError = `Failed to load models: ${toErrorMessage(err)}`;
    } finally {
      if (requestSeq === aiModelsRequestSeq) {
        aiModelsLoading = false;
      }
    }
  }

  function refreshAiModels() {
    const profileKey = selectedProfileKey.trim();
    const ai = currentProfile?.ai;
    const endpoint = ai?.endpoint?.trim() ?? "";
    const apiKey = ai?.api_key?.trim() ?? "";
    if (!profileKey || !ai || !endpoint || !isAiEnabled(currentProfile)) {
      aiModelsError = "Endpoint is required.";
      return;
    }
    const requestKey = `${profileKey}::${endpoint}::${apiKey}`;
    void fetchAiModels(endpoint, apiKey, requestKey, true);
  }

  async function loadAll() {
    errorMessage = null;
    loadingSettings = true;
    loadingProfiles = true;

    try {
      const { invoke } = await import("$lib/tauriInvoke");
      const [loadedSettings, loadedProfiles] = await Promise.all([
        invoke<SettingsData>("get_settings"),
        invoke<ProfilesConfig>("get_profiles"),
      ]);
      loadedSettings.voice_input = normalizeVoiceInputSettings(loadedSettings.voice_input);
      loadedSettings.app_language = normalizeAppLanguage(loadedSettings.app_language);
      loadedSettings.ui_font_family = normalizeUiFontFamily(loadedSettings.ui_font_family);
      loadedSettings.terminal_font_family = normalizeTerminalFontFamily(
        loadedSettings.terminal_font_family
      );
      loadedSettings.agent_skill_registration_default_scope = normalizeSkillScope(
        loadedSettings.agent_skill_registration_default_scope
      );
      loadedSettings.agent_skill_registration_codex_scope = normalizeSkillScope(
        loadedSettings.agent_skill_registration_codex_scope
      );
      loadedSettings.agent_skill_registration_claude_scope = normalizeSkillScope(
        loadedSettings.agent_skill_registration_claude_scope
      );
      loadedSettings.agent_skill_registration_gemini_scope = normalizeSkillScope(
        loadedSettings.agent_skill_registration_gemini_scope
      );
      settings = loadedSettings;
      savedUiFontSize = loadedSettings.ui_font_size ?? 13;
      savedTerminalFontSize = loadedSettings.terminal_font_size ?? 13;
      savedUiFontFamily = loadedSettings.ui_font_family;
      savedTerminalFontFamily = loadedSettings.terminal_font_family;
      profiles = loadedProfiles;

      const keys = Object.keys(loadedProfiles.profiles ?? {});
      const nextSelected = loadedProfiles.active ?? keys[0] ?? "";
      selectedProfileKey = nextSelected;
      await loadSkillStatus(false);
      try {
        const result = await invoke<ShellInfo[]>("get_available_shells");
        availableShells = Array.isArray(result) ? result : [];
      } catch {
        availableShells = [];
      }
    } catch (err) {
      console.error("Failed to load settings/profiles:", err);
      errorMessage = `Failed to load settings: ${toErrorMessage(err)}`;
    }

    loadingSettings = false;
    loadingProfiles = false;
  }

  async function saveAll() {
    if (!settings) return;
    saving = true;
    saveMessage = "";
    try {
      const normalizedDefaultScope = normalizeSkillScope(
        settings.agent_skill_registration_default_scope
      );
      const normalizedCodexScope = normalizeSkillScope(
        settings.agent_skill_registration_codex_scope
      );
      const normalizedClaudeScope = normalizeSkillScope(
        settings.agent_skill_registration_claude_scope
      );
      const normalizedGeminiScope = normalizeSkillScope(
        settings.agent_skill_registration_gemini_scope
      );
      if (
        !normalizedDefaultScope &&
        (normalizedCodexScope || normalizedClaudeScope || normalizedGeminiScope)
      ) {
        saveMessage = "Choose default skill registration scope before setting agent overrides.";
        saving = false;
        return;
      }
      settings = {
        ...settings,
        ui_font_family: normalizeUiFontFamily(settings.ui_font_family),
        terminal_font_family: normalizeTerminalFontFamily(settings.terminal_font_family),
        agent_skill_registration_default_scope: normalizedDefaultScope,
        agent_skill_registration_codex_scope: normalizedCodexScope,
        agent_skill_registration_claude_scope: normalizedClaudeScope,
        agent_skill_registration_gemini_scope: normalizedGeminiScope,
      };

      const { invoke } = await import("$lib/tauriInvoke");
      await invoke("save_settings", { settings });
      if (profiles) {
        await invoke("save_profiles", { config: profiles });
      }
      settings.app_language = normalizeAppLanguage(settings.app_language);
      saveMessage = "Settings saved.";
      savedUiFontSize = settings.ui_font_size ?? 13;
      savedTerminalFontSize = settings.terminal_font_size ?? 13;
      savedUiFontFamily = normalizeUiFontFamily(settings.ui_font_family);
      savedTerminalFontFamily = normalizeTerminalFontFamily(
        settings.terminal_font_family
      );
      settings.voice_input = normalizeVoiceInputSettings(settings.voice_input);
      window.dispatchEvent(
        new CustomEvent("gwt-settings-updated", {
          detail: {
            uiFontSize: savedUiFontSize,
            terminalFontSize: savedTerminalFontSize,
            uiFontFamily: savedUiFontFamily,
            terminalFontFamily: savedTerminalFontFamily,
            appLanguage: settings.app_language,
            voiceInput: settings.voice_input,
          },
        })
      );
    } catch (err) {
      console.error("Failed to save settings/profiles:", err);
      saveMessage = `Failed to save settings: ${toErrorMessage(err)}`;
    }
    saving = false;
    setTimeout(() => {
      saveMessage = "";
    }, 2000);
  }

  function addBranch() {
    const trimmed = newBranch.trim();
    if (settings && trimmed && !settings.protected_branches.includes(trimmed)) {
      settings.protected_branches = [...settings.protected_branches, trimmed];
      newBranch = "";
    }
  }

  function removeBranch(branch: string) {
    if (!settings) return;
    settings.protected_branches = settings.protected_branches.filter((b) => b !== branch);
  }

  function handleBranchKeydown(e: KeyboardEvent) {
    if (e.key === "Enter") {
      e.preventDefault();
      addBranch();
    }
  }

  function sortedProfileKeys(cfg: ProfilesConfig): string[] {
    const keys = Object.keys(cfg.profiles ?? {});
    keys.sort((a, b) => a.localeCompare(b));
    return keys;
  }

  function clampFontSize(v: number): number {
    return Math.max(8, Math.min(24, Math.round(v)));
  }

  function applyUiFontSize(size: number) {
    document.documentElement.style.setProperty("--ui-font-base", size + "px");
  }

  function applyUiFontFamily(family: string | null | undefined) {
    document.documentElement.style.setProperty(
      "--ui-font-family",
      normalizeUiFontFamily(family)
    );
  }

  function applyTerminalFontSize(size: number) {
    (window as any).__gwtTerminalFontSize = size;
    window.dispatchEvent(new CustomEvent("gwt-terminal-font-size", { detail: size }));
  }

  function applyTerminalFontFamily(family: string | null | undefined) {
    const normalized = normalizeTerminalFontFamily(family);
    document.documentElement.style.setProperty("--terminal-font-family", normalized);
    (window as any).__gwtTerminalFontFamily = normalized;
    window.dispatchEvent(new CustomEvent("gwt-terminal-font-family", { detail: normalized }));
  }

  function adjustFontSize(field: "ui_font_size" | "terminal_font_size", delta: number) {
    if (!settings) return;
    const current = settings[field] ?? 13;
    const next = clampFontSize(current + delta);
    settings = { ...settings, [field]: next };
  }

  function handleClose() {
    applyUiFontSize(savedUiFontSize);
    applyTerminalFontSize(savedTerminalFontSize);
    applyUiFontFamily(savedUiFontFamily);
    applyTerminalFontFamily(savedTerminalFontFamily);
    onClose();
  }

  function setActiveProfile(name: string | null) {
    if (!profiles) return;
    profiles = { ...profiles, active: name };
    selectedProfileKey = name ?? "";
  }

  function createProfile() {
    if (!profiles) return;
    const name = newProfileName.trim();
    if (!name) return;
    if (!/^[a-z0-9-]+$/.test(name)) {
      saveMessage = "Profile name must be lowercase letters, numbers, or hyphens.";
      return;
    }
    if (profiles.profiles?.[name]) {
      saveMessage = "Profile already exists.";
      return;
    }

    const next: Profile = {
      name,
      description: "",
      env: {},
      disabled_env: [],
      ai: null,
    };

    profiles = {
      ...profiles,
      profiles: { ...profiles.profiles, [name]: next },
      active: profiles.active ?? name,
    };
    selectedProfileKey = name;
    newProfileName = "";
  }

  function deleteSelectedProfile() {
    if (!profiles) return;
    if (!selectedProfileKey) return;
    const copy = { ...(profiles.profiles ?? {}) };
    if (!copy[selectedProfileKey]) return;
    delete copy[selectedProfileKey];
    const nextKeys = Object.keys(copy).sort((a, b) => a.localeCompare(b));
    const nextActive =
      profiles.active === selectedProfileKey ? (nextKeys[0] ?? null) : profiles.active ?? null;
    profiles = { ...profiles, profiles: copy, active: nextActive };
    selectedProfileKey = nextActive ?? "";
  }

  function upsertEnvVar(key: string, value: string) {
    if (!profiles) return;
    const p = currentProfile;
    if (!p) return;
    if (!selectedProfileKey) return;
    const nextEnv = { ...(p.env ?? {}), [key]: value };
    const nextProfile: Profile = { ...p, env: nextEnv };
    profiles = {
      ...profiles,
      profiles: { ...(profiles.profiles ?? {}), [selectedProfileKey]: nextProfile },
    };
  }

  function removeEnvVar(key: string) {
    if (!profiles) return;
    const p = currentProfile;
    if (!p) return;
    if (!selectedProfileKey) return;
    const nextEnv = { ...(p.env ?? {}) };
    delete nextEnv[key];
    const nextProfile: Profile = { ...p, env: nextEnv };
    profiles = {
      ...profiles,
      profiles: { ...(profiles.profiles ?? {}), [selectedProfileKey]: nextProfile },
    };
  }

  function addEnvVar() {
    const key = newEnvKey.trim();
    if (!key) return;
    upsertEnvVar(key, newEnvValue);
    newEnvKey = "";
    newEnvValue = "";
  }

  function updateAiField(
    field: "endpoint" | "api_key" | "model" | "language" | "summary_enabled",
    value: string | boolean
  ) {
    if (!profiles) return;
    const p = currentProfile;
    if (!p) return;
    if (!selectedProfileKey) return;
    const nextAi = {
      endpoint: "",
      api_key: "",
      model: "",
      language: "en",
      summary_enabled: true,
      ...(p.ai ?? {}),
      [field]: value,
    } as Profile["ai"];
    const nextProfile: Profile = { ...p, ai: nextAi };
    profiles = {
      ...profiles,
      profiles: { ...(profiles.profiles ?? {}), [selectedProfileKey]: nextProfile },
    };
  }

  function updateVoiceInputField(
    field: keyof VoiceInputSettings,
    value: VoiceInputSettings[keyof VoiceInputSettings]
  ) {
    if (!settings) return;
    const current = normalizeVoiceInputSettings(settings.voice_input);
    const next = { ...current, [field]: value } as VoiceInputSettings;
    settings = { ...settings, voice_input: normalizeVoiceInputSettings(next) };
  }

  function updateSkillScopeField(
    field:
      | "agent_skill_registration_default_scope"
      | "agent_skill_registration_codex_scope"
      | "agent_skill_registration_claude_scope"
      | "agent_skill_registration_gemini_scope",
    value: string
  ) {
    if (!settings) return;
    settings = { ...settings, [field]: normalizeSkillScope(value) };
  }
</script>

<div class="settings-panel">
  <div class="settings-header">
    <h2>Settings</h2>
    <button class="close-btn" onclick={handleClose}>[x]</button>
  </div>

  {#if loadingSettings || loadingProfiles}
    <div class="loading">Loading settings...</div>
  {:else if errorMessage || !settings}
    <div class="loading">{errorMessage ?? "Failed to load settings."}</div>
  {:else}
    <div class="settings-body">
      <div class="settings-tabs">
        <button
          class="settings-tab-btn"
          class:active={activeSettingsTab === "appearance"}
          onclick={() => (activeSettingsTab = "appearance")}
        >Appearance</button>
        <button
          class="settings-tab-btn"
          class:active={activeSettingsTab === "voiceInput"}
          onclick={() => (activeSettingsTab = "voiceInput")}
        >Voice Input</button>
        <button
          class="settings-tab-btn"
          class:active={activeSettingsTab === "githubIntegration"}
          onclick={() => (activeSettingsTab = "githubIntegration")}
        >GitHub Integration</button>
        <button
          class="settings-tab-btn"
          class:active={activeSettingsTab === "profiles"}
          onclick={() => (activeSettingsTab = "profiles")}
        >Profiles</button>
        {#if availableShells.length > 0}
          <button
            class="settings-tab-btn"
            class:active={activeSettingsTab === "terminal"}
            onclick={() => (activeSettingsTab = "terminal")}
          >Terminal</button>
        {/if}
      </div>

      <div class="settings-tab-content">
        {#if activeSettingsTab === "appearance"}
          <div class="section-content">
            <div class="field">
              <!-- svelte-ignore a11y_label_has_associated_control -->
              <label>Terminal Font Size</label>
              <div class="font-size-control">
                <button
                  class="font-size-btn"
                  onclick={() => adjustFontSize("terminal_font_size", -1)}
                  disabled={!settings || (settings.terminal_font_size ?? 13) <= 8}
                >-</button>
                <input
                  type="number"
                  min="8"
                  max="24"
                  step="1"
                  value={settings.terminal_font_size ?? 13}
                  oninput={(e) => {
                    if (!settings) return;
                    const raw = (e.target as HTMLInputElement).value;
                    if (raw === "") return;
                    const parsed = Number(raw);
                    if (Number.isNaN(parsed)) return;
                    settings = { ...settings, terminal_font_size: parsed };
                  }}
                  onchange={() => {
                    if (!settings) return;
                    settings = { ...settings, terminal_font_size: clampFontSize(settings.terminal_font_size ?? 13) };
                  }}
                />
                <button
                  class="font-size-btn"
                  onclick={() => adjustFontSize("terminal_font_size", 1)}
                  disabled={!settings || (settings.terminal_font_size ?? 13) >= 24}
                >+</button>
                <span class="font-size-unit">px</span>
              </div>
            </div>

            <div class="field">
              <!-- svelte-ignore a11y_label_has_associated_control -->
              <label>UI Font Size</label>
              <div class="font-size-control">
                <button
                  class="font-size-btn"
                  onclick={() => adjustFontSize("ui_font_size", -1)}
                  disabled={!settings || (settings.ui_font_size ?? 13) <= 8}
                >-</button>
                <input
                  type="number"
                  min="8"
                  max="24"
                  step="1"
                  value={settings.ui_font_size ?? 13}
                  oninput={(e) => {
                    if (!settings) return;
                    const raw = (e.target as HTMLInputElement).value;
                    if (raw === "") return;
                    const parsed = Number(raw);
                    if (Number.isNaN(parsed)) return;
                    settings = { ...settings, ui_font_size: parsed };
                  }}
                  onchange={() => {
                    if (!settings) return;
                    settings = { ...settings, ui_font_size: clampFontSize(settings.ui_font_size ?? 13) };
                  }}
                />
                <button
                  class="font-size-btn"
                  onclick={() => adjustFontSize("ui_font_size", 1)}
                  disabled={!settings || (settings.ui_font_size ?? 13) >= 24}
                >+</button>
                <span class="font-size-unit">px</span>
              </div>
            </div>

            <div class="field">
              <label for="terminal-font-family">Terminal Font Family</label>
              <select
                id="terminal-font-family"
                class="select"
                value={settings.terminal_font_family}
                onchange={(e) => {
                  if (!settings) return;
                  const next = normalizeTerminalFontFamily(
                    (e.target as HTMLSelectElement).value
                  );
                  settings = { ...settings, terminal_font_family: next };
                  applyTerminalFontFamily(next);
                }}
              >
                {#each TERMINAL_FONT_PRESETS as preset}
                  <option value={preset.value}>{preset.label}</option>
                {/each}
              </select>
            </div>

            <div class="field">
              <label for="ui-font-family">UI Font Family</label>
              <select
                id="ui-font-family"
                class="select"
                value={settings.ui_font_family}
                onchange={(e) => {
                  if (!settings) return;
                  const next = normalizeUiFontFamily(
                    (e.target as HTMLSelectElement).value
                  );
                  settings = { ...settings, ui_font_family: next };
                  applyUiFontFamily(next);
                }}
              >
                {#each UI_FONT_PRESETS as preset}
                  <option value={preset.value}>{preset.label}</option>
                {/each}
              </select>
            </div>

            <div class="divider"></div>

            <div class="field">
              <label for="app-language">Summary Language (global)</label>
              <select
                id="app-language"
                class="select"
                value={settings.app_language}
                onchange={(e) => {
                  if (!settings) return;
                  settings = {
                    ...settings,
                    app_language: normalizeAppLanguage(
                      (e.target as HTMLSelectElement).value
                    ),
                  };
                }}
              >
                <option value="auto">Auto</option>
                <option value="ja">Japanese</option>
                <option value="en">English</option>
              </select>
              <span class="field-hint">
                Used as the app-wide language when rebuilding all branch summaries.
              </span>
            </div>

            <div class="divider"></div>

            <div class="field">
              <label for="log-retention">Log Retention (days)</label>
              <input
                id="log-retention"
                type="number"
                min="1"
                max="365"
                bind:value={settings.log_retention_days}
              />
              <span class="field-hint">
                Logs older than this will be cleaned up automatically.
              </span>
            </div>

            <div class="field">
              <!-- svelte-ignore a11y_label_has_associated_control -->
              <label>Protected Branches</label>
              <div class="branch-tags">
                {#each settings.protected_branches as branch}
                  <span class="branch-tag">
                    {branch}
                    <button class="tag-remove" onclick={() => removeBranch(branch)}>
                      x
                    </button>
                  </span>
                {/each}
              </div>
              <div class="branch-input-row">
                <input
                  type="text"
                  autocapitalize="off"
                  autocorrect="off"
                  autocomplete="off"
                  spellcheck="false"
                  bind:value={newBranch}
                  placeholder="Add branch..."
                  onkeydown={handleBranchKeydown}
                />
                <button class="btn btn-add" onclick={addBranch}>Add</button>
              </div>
              <span class="field-hint">
                Branches that cannot be deleted or force-pushed.
              </span>
            </div>

          </div>

        {:else if activeSettingsTab === "voiceInput"}
          <div class="section-content">
            <div class="field">
              <div class="ai-toggle">
                <input
                  id="voice-input-enabled"
                  type="checkbox"
                  checked={settings.voice_input.enabled}
                  onchange={(e) =>
                    updateVoiceInputField(
                      "enabled",
                      (e.target as HTMLInputElement).checked
                    )}
                />
                <label for="voice-input-enabled" class="ai-enabled-label">
                  Enable Voice Input
                </label>
              </div>
              <span class="field-hint">
                Hotkey toggles start/stop and inserts transcript into the focused input.
              </span>
            </div>

            <div class="field">
              <label for="voice-hotkey">Hotkey</label>
              <input
                id="voice-hotkey"
                type="text"
                value={settings.voice_input.hotkey}
                oninput={(e) =>
                  updateVoiceInputField(
                    "hotkey",
                    (e.target as HTMLInputElement).value
                  )}
                placeholder="Mod+Shift+M"
              />
              <span class="field-hint">Example: Mod+Shift+M</span>
            </div>

            <div class="field">
              <label for="voice-language">Language</label>
              <select
                id="voice-language"
                class="select"
                value={settings.voice_input.language}
                onchange={(e) =>
                  updateVoiceInputField(
                    "language",
                    (e.target as HTMLSelectElement).value as VoiceInputSettings["language"]
                  )}
              >
                <option value="auto">Auto</option>
                <option value="ja">Japanese</option>
                <option value="en">English</option>
              </select>
            </div>

            <div class="field">
              <label for="voice-model">Model</label>
              <input
                id="voice-model"
                type="text"
                value={settings.voice_input.model}
                oninput={(e) =>
                  updateVoiceInputField(
                    "model",
                    (e.target as HTMLInputElement).value
                  )}
                placeholder="base"
              />
              <span class="field-hint">Bundled STT model tier label.</span>
            </div>
          </div>

        {:else if activeSettingsTab === "githubIntegration"}
          <div class="section-content">
            <div class="field">
              <label for="skill-scope-default">Skill Registration Scope (Default)</label>
              <select
                id="skill-scope-default"
                class="select"
                value={settings.agent_skill_registration_default_scope ?? ""}
                onchange={(e) =>
                  updateSkillScopeField(
                    "agent_skill_registration_default_scope",
                    (e.target as HTMLSelectElement).value
                  )}
              >
                <option value="">Not selected (prompt on startup)</option>
                <option value="user">User (~/.xxx)</option>
                <option value="project">Project (&lt;repo&gt;/.xxx)</option>
                <option value="local">Local (&lt;repo&gt;/.xxx.local)</option>
              </select>
              <span class="field-hint">
                Controls where managed skills/plugins are auto-registered.
              </span>
            </div>

            <div class="field">
              <!-- svelte-ignore a11y_label_has_associated_control -->
              <label>Agent Scope Overrides</label>
              <div class="row">
                <div class="scope-select">
                  <label for="skill-scope-codex">Codex</label>
                  <select
                    id="skill-scope-codex"
                    class="select"
                    value={settings.agent_skill_registration_codex_scope ?? ""}
                    onchange={(e) =>
                      updateSkillScopeField(
                        "agent_skill_registration_codex_scope",
                        (e.target as HTMLSelectElement).value
                      )}
                  >
                    <option value="">Use default</option>
                    <option value="user">User</option>
                    <option value="project">Project</option>
                    <option value="local">Local</option>
                  </select>
                </div>
                <div class="scope-select">
                  <label for="skill-scope-claude">Claude Code</label>
                  <select
                    id="skill-scope-claude"
                    class="select"
                    value={settings.agent_skill_registration_claude_scope ?? ""}
                    onchange={(e) =>
                      updateSkillScopeField(
                        "agent_skill_registration_claude_scope",
                        (e.target as HTMLSelectElement).value
                      )}
                  >
                    <option value="">Use default</option>
                    <option value="user">User</option>
                    <option value="project">Project</option>
                    <option value="local">Local</option>
                  </select>
                </div>
                <div class="scope-select">
                  <label for="skill-scope-gemini">Gemini</label>
                  <select
                    id="skill-scope-gemini"
                    class="select"
                    value={settings.agent_skill_registration_gemini_scope ?? ""}
                    onchange={(e) =>
                      updateSkillScopeField(
                        "agent_skill_registration_gemini_scope",
                        (e.target as HTMLSelectElement).value
                      )}
                  >
                    <option value="">Use default</option>
                    <option value="user">User</option>
                    <option value="project">Project</option>
                    <option value="local">Local</option>
                  </select>
                </div>
              </div>
            </div>

            <div class="divider"></div>

            <div class="skill-overview">
              <span class={`skill-badge ${skillStatusClass(skillStatus?.overall ?? "failed")}`}>
                Overall: {skillStatusText(skillStatus?.overall)}
              </span>
              <span class="field-hint">
                Last checked: {formatRegistrationCheckedAt(skillStatus?.last_checked_at)}
              </span>
            </div>

            <div class="skill-agent-list">
              {#each skillStatus?.agents ?? [] as agent (agent.agent_id)}
                <div class="skill-agent-row">
                  <div class="skill-agent-meta">
                    <span class="skill-agent-label">{agent.label}</span>
                    {#if agent.skills_path}
                      <span class="skill-agent-path mono">{agent.skills_path}</span>
                    {/if}
                    {#if agent.missing_skills.length > 0}
                      <span class="field-hint">
                        Missing: {agent.missing_skills.join(", ")}
                      </span>
                    {/if}
                    {#if agent.error_message}
                      <span class="field-hint">{agent.error_message}</span>
                    {/if}
                  </div>
                  <span class={`skill-mini-badge ${agent.registered ? "status-ok" : "status-failed"}`}>
                    {agent.registered ? "REGISTERED" : "MISSING"}
                  </span>
                </div>
              {/each}
            </div>

            {#if skillStatus?.last_error_message}
              <span class="field-hint">{skillStatus.last_error_message}</span>
            {/if}
            {#if skillStatusMessage}
              <span class="field-hint">{skillStatusMessage}</span>
            {/if}

            <div class="row">
              <button
                class="btn btn-ghost"
                onclick={() => void loadSkillStatus(true)}
                disabled={skillStatusLoading || skillStatusRepairing}
              >
                {skillStatusLoading ? "Refreshing..." : "Refresh Skill Status"}
              </button>
              <button
                class="btn btn-add"
                onclick={() => void repairSkillStatus()}
                disabled={skillStatusRepairing}
              >
                {skillStatusRepairing ? "Repairing..." : "Repair Skill Registration"}
              </button>
            </div>
          </div>

        {:else if activeSettingsTab === "terminal"}
          <div class="section-content">
            <div class="field">
              <label for="default-shell">Default Shell</label>
              <select
                id="default-shell"
                class="select"
                value={settings.default_shell ?? ""}
                onchange={(e) => {
                  if (!settings) return;
                  const value = (e.target as HTMLSelectElement).value;
                  settings = { ...settings, default_shell: value || null };
                }}
              >
                <option value="">System Default</option>
                {#each availableShells as shell (shell.id)}
                  <option value={shell.id}>
                    {shell.name}{shell.version ? ` (${shell.version})` : ""}
                  </option>
                {/each}
              </select>
              <span class="field-hint">
                Shell used for new terminal sessions on Windows.
              </span>
            </div>
          </div>

        {:else if activeSettingsTab === "profiles"}
          <div class="section-content">
            <div class="field">
              <label for="active-profile">Active Profile</label>
              <select
                id="active-profile"
                class="select"
                value={profiles?.active ?? ""}
                onchange={(e) => setActiveProfile((e.target as HTMLSelectElement).value || null)}
              >
                <option value="">(none)</option>
                {#if profiles}
                  {#each sortedProfileKeys(profiles) as key}
                    <option value={key}>{key}</option>
                  {/each}
                {/if}
              </select>
              <span class="field-hint">Saved in ~/.gwt/profiles.toml</span>
            </div>

            <div class="field">
              <label for="profile-edit">Edit Profile</label>
              <div class="row">
                <select
                  id="profile-edit"
                  class="select"
                  bind:value={selectedProfileKey}
                  disabled={!profiles}
                >
                  {#if profiles}
                    {#each sortedProfileKeys(profiles) as key}
                      <option value={key}>{key}</option>
                    {/each}
                  {/if}
                </select>
                <button class="btn btn-danger" onclick={deleteSelectedProfile} disabled={!profiles || !selectedProfileKey}>
                  Delete
                </button>
              </div>
            </div>

            <div class="field">
              <label for="new-profile">New Profile</label>
              <div class="row">
                <input
                  id="new-profile"
                  type="text"
                  autocapitalize="off"
                  autocorrect="off"
                  autocomplete="off"
                  spellcheck="false"
                  bind:value={newProfileName}
                  placeholder="e.g. development"
                />
                <button class="btn btn-add" onclick={createProfile} disabled={!profiles || !newProfileName.trim()}>
                  Create
                </button>
              </div>
              <span class="field-hint">Name must be lowercase letters, numbers, or hyphens.</span>
            </div>

            <div class="field">
              <!-- svelte-ignore a11y_label_has_associated_control -->
              <label>Environment Variables</label>
              {#if profiles && selectedProfileKey && currentProfile}
                <div class="env-table">
                  {#if Object.keys(currentProfile.env ?? {}).length === 0}
                    <div class="env-empty">No environment variables</div>
                  {:else}
                    {#each Object.keys(currentProfile.env ?? {}).sort((a, b) => a.localeCompare(b)) as key (key)}
                      <div class="env-row">
                        <span class="env-key mono">{key}</span>
                        <input
                          class="env-value"
                          type="text"
                          autocapitalize="off"
                          autocorrect="off"
                          autocomplete="off"
                          spellcheck="false"
                          value={currentProfile.env[key]}
                          oninput={(e) => upsertEnvVar(key, (e.target as HTMLInputElement).value)}
                        />
                        <button class="btn btn-ghost" onclick={() => removeEnvVar(key)}>Remove</button>
                      </div>
                    {/each}
                  {/if}
                </div>

                <div class="env-add-row">
                  <input
                    class="env-key-input"
                    type="text"
                    autocapitalize="off"
                    autocorrect="off"
                    autocomplete="off"
                    spellcheck="false"
                    bind:value={newEnvKey}
                    placeholder="KEY"
                  />
                  <input
                    class="env-value-input"
                    type="text"
                    autocapitalize="off"
                    autocorrect="off"
                    autocomplete="off"
                    spellcheck="false"
                    bind:value={newEnvValue}
                    placeholder="value"
                  />
                  <button class="btn btn-add" onclick={addEnvVar} disabled={!newEnvKey.trim()}>
                    Add
                  </button>
                </div>
              {:else}
                <div class="field-hint">Create a profile to edit environment variables.</div>
              {/if}
            </div>

            <div class="field">
              <!-- svelte-ignore a11y_label_has_associated_control -->
              <label>AI Settings (per profile)</label>
              {#if profiles && selectedProfileKey && currentProfile}
                {@const currentAi = currentProfile.ai ?? {
                  endpoint: "",
                  api_key: "",
                  model: "",
                  language: "en",
                  summary_enabled: true,
                }}
                {@const currentEndpoint = currentAi?.endpoint?.trim() ?? ""}
                <div class="ai-grid">
                  <div class="ai-field">
                    <span class="ai-label">Endpoint</span>
                    <input
                      type="text"
                      autocapitalize="off"
                      autocorrect="off"
                      autocomplete="off"
                      spellcheck="false"
                      value={currentAi?.endpoint ?? ""}
                      oninput={(e) => updateAiField("endpoint", (e.target as HTMLInputElement).value)}
                    />
                  </div>
                  <div class="ai-field">
                    <span class="ai-label">API Key</span>
                    <input
                      type="password"
                      autocapitalize="off"
                      autocorrect="off"
                      autocomplete="off"
                      spellcheck="false"
                      value={currentAi?.api_key ?? ""}
                      oninput={(e) => updateAiField("api_key", (e.target as HTMLInputElement).value)}
                    />
                  </div>
                  <div class="ai-field">
                    <span class="ai-label">Model</span>
                    <div class="row ai-model-row">
                      <select
                        class="select ai-model-select"
                        value={currentAi?.model ?? ""}
                        disabled={aiModelsLoading || !currentEndpoint}
                        onchange={(e) => updateAiField("model", (e.target as HTMLSelectElement).value)}
                      >
                        <option value="">Select model...</option>
                        {#each aiModelOptions as modelId (modelId)}
                          <option value={modelId}>{modelId}</option>
                        {/each}
                      </select>
                      <button
                        class="btn btn-ghost"
                        onclick={refreshAiModels}
                        disabled={aiModelsLoading || !currentEndpoint}
                      >
                        {aiModelsLoading ? "Loading..." : "Refresh"}
                      </button>
                    </div>
                    {#if aiModelsError}
                      <span class="field-hint">{aiModelsError}</span>
                    {:else if currentModelMissing}
                      <span class="field-hint">
                        Current model is not listed in /v1/models.
                      </span>
                    {:else if
                      !aiModelsLoading &&
                      currentEndpoint &&
                      aiModelsLoadedKey !== currentAiRequestKey}
                      <span class="field-hint">Click Refresh to load models from /v1/models.</span>
                    {:else if
                      !aiModelsLoading &&
                      aiModels.length === 0 &&
                      currentEndpoint &&
                      aiModelsLoadedKey === currentAiRequestKey}
                      <span class="field-hint">No models returned from /v1/models.</span>
                    {/if}
                  </div>
                  <div class="ai-field">
                    <span class="ai-label">Profile Language</span>
                    <select
                      class="select"
                      value={currentAi?.language ?? "en"}
                      onchange={(e) => updateAiField("language", (e.target as HTMLSelectElement).value)}
                    >
                      <option value="en">English</option>
                      <option value="ja">Japanese</option>
                      <option value="auto">Auto</option>
                    </select>
                  </div>
                  <div class="ai-field">
                    <span class="ai-label">Session Summary</span>
                    <div class="ai-checkbox">
                      <input
                        id="ai-summary"
                        type="checkbox"
                        checked={currentAi?.summary_enabled ?? false}
                        onchange={(e) => updateAiField("summary_enabled", (e.target as HTMLInputElement).checked)}
                      />
                      <label for="ai-summary">Enabled</label>
                    </div>
                  </div>
                </div>
              {:else}
                <div class="field-hint">Create a profile to configure AI settings.</div>
              {/if}
            </div>
          </div>
        {/if}
      </div>
    </div>

    <div class="settings-footer">
      {#if saveMessage}
        <span class="save-message">{saveMessage}</span>
      {/if}
      <button class="btn btn-cancel" onclick={handleClose}>Close</button>
      <button
        class="btn btn-save"
        disabled={saving || !settings}
        onclick={saveAll}
      >
        {saving ? "Saving..." : "Save"}
      </button>
    </div>
  {/if}
</div>

<style>
  .settings-panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    background: var(--bg-primary);
  }

  .settings-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 16px 24px;
    border-bottom: 1px solid var(--border-color);
  }

  .settings-header h2 {
    font-size: var(--ui-font-xl);
    font-weight: 600;
    color: var(--text-primary);
  }

  .close-btn {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: var(--ui-font-lg);
    font-family: monospace;
    padding: 2px 4px;
  }

  .close-btn:hover {
    color: var(--text-primary);
  }

  .loading {
    padding: 40px;
    text-align: center;
    color: var(--text-muted);
  }

  .settings-body {
    flex: 1;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .settings-tabs {
    display: flex;
    gap: 2px;
    border-bottom: 1px solid var(--border-color);
    padding: 0 24px;
    flex-shrink: 0;
    min-width: 0;
    overflow-x: auto;
    overflow-y: hidden;
    -webkit-overflow-scrolling: touch;
  }

  .settings-tab-btn {
    padding: 10px 16px;
    background: none;
    border: none;
    border-bottom: 2px solid transparent;
    color: var(--text-muted);
    font-size: var(--ui-font-md);
    font-family: inherit;
    cursor: pointer;
    white-space: nowrap;
  }

  .settings-tab-btn:hover {
    color: var(--text-secondary);
  }

  .settings-tab-btn.active {
    color: var(--text-primary);
    border-bottom-color: var(--accent);
  }

  .settings-tab-content {
    flex: 1;
    overflow-y: auto;
    padding: 24px;
  }

  .divider {
    height: 1px;
    background: var(--border-color);
    opacity: 0.7;
  }

  .section-content {
    display: flex;
    flex-direction: column;
    gap: 24px;
    padding-top: 24px;
  }

  .field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .field label {
    font-size: var(--ui-font-md);
    font-weight: 500;
    color: var(--text-secondary);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .field input[type="number"],
  .field input[type="text"] {
    padding: 8px 12px;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-primary);
    font-size: var(--ui-font-base);
    font-family: monospace;
    outline: none;
    max-width: 200px;
  }

  .select {
    padding: 8px 12px;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-primary);
    font-size: var(--ui-font-base);
    font-family: monospace;
    outline: none;
    max-width: 320px;
  }

  .row {
    display: flex;
    gap: 8px;
    align-items: center;
    flex-wrap: wrap;
  }

  .scope-select {
    display: flex;
    flex-direction: column;
    gap: 6px;
    min-width: 180px;
  }

  .scope-select label {
    font-size: var(--ui-font-sm);
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.4px;
  }

  .scope-select .select {
    max-width: none;
  }

  .env-table {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 8px;
    min-height: 96px;
  }

  .env-row {
    display: grid;
    grid-template-columns: 1fr 2fr auto;
    gap: 8px;
    align-items: center;
  }

  .env-empty {
    color: var(--text-muted);
    font-size: var(--ui-font-sm);
    padding: 16px 0;
  }

  .env-key {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .env-value,
  .env-key-input,
  .env-value-input {
    padding: 6px 10px;
    background: var(--bg-primary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-primary);
    font-size: var(--ui-font-md);
    font-family: monospace;
    outline: none;
    width: 100%;
  }

  .field input[type="text"].env-value,
  .field input[type="text"].env-key-input,
  .field input[type="text"].env-value-input {
    max-width: none;
  }

  .env-add-row {
    display: grid;
    grid-template-columns: 1fr 2fr auto;
    gap: 8px;
    align-items: center;
    max-width: 760px;
  }

  .ai-toggle {
    display: flex;
    gap: 8px;
    align-items: center;
    padding: 8px 10px;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 8px;
    width: fit-content;
  }

  .ai-enabled-label {
    font-size: var(--ui-font-md);
    color: var(--text-secondary);
  }

  .ai-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 12px;
    margin-top: 10px;
    max-width: 760px;
  }

  .ai-field {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .ai-label {
    font-size: var(--ui-font-sm);
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  .ai-field input[type="text"],
  .ai-field input[type="password"],
  .ai-field select {
    padding: 8px 12px;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-primary);
    font-size: var(--ui-font-md);
    font-family: monospace;
    outline: none;
    max-width: none;
  }

  .ai-model-row {
    align-items: center;
  }

  .ai-model-select {
    flex: 1;
    max-width: none;
    min-width: 220px;
  }

  .ai-checkbox {
    display: flex;
    gap: 8px;
    align-items: center;
    padding: 8px 12px;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    width: fit-content;
  }

  .skill-overview {
    display: flex;
    align-items: center;
    gap: 10px;
    flex-wrap: wrap;
  }

  .skill-badge,
  .skill-mini-badge {
    padding: 4px 10px;
    border-radius: 999px;
    border: 1px solid var(--border-color);
    font-family: monospace;
    font-size: var(--ui-font-sm);
    letter-spacing: 0.2px;
  }

  .status-ok {
    border-color: var(--green);
    color: var(--green);
  }

  .status-degraded {
    border-color: var(--yellow);
    color: var(--yellow);
  }

  .status-failed {
    border-color: var(--red);
    color: var(--red);
  }

  .skill-agent-list {
    display: flex;
    flex-direction: column;
    gap: 8px;
    max-width: 960px;
  }

  .skill-agent-row {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: 12px;
    padding: 10px 12px;
    border: 1px solid var(--border-color);
    border-radius: 8px;
    background: var(--bg-secondary);
  }

  .skill-agent-meta {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
  }

  .skill-agent-label {
    color: var(--text-primary);
    font-size: var(--ui-font-md);
    font-weight: 500;
  }

  .skill-agent-path {
    color: var(--text-muted);
    font-size: var(--ui-font-sm);
    word-break: break-all;
  }

  .field input:focus {
    border-color: var(--accent);
  }

  .field-hint {
    font-size: var(--ui-font-sm);
    color: var(--text-muted);
  }

  .branch-tags {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
  }

  .branch-tag {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 4px 8px;
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
    border-radius: 4px;
    font-size: var(--ui-font-md);
    font-family: monospace;
    color: var(--text-primary);
  }

  .tag-remove {
    background: none;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-size: var(--ui-font-sm);
    font-family: monospace;
    padding: 0 2px;
    line-height: 1;
  }

  .tag-remove:hover {
    color: var(--red);
  }

  .branch-input-row {
    display: flex;
    gap: 6px;
    max-width: 300px;
  }

  .branch-input-row input {
    flex: 1;
    padding: 6px 10px;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-primary);
    font-size: var(--ui-font-md);
    font-family: monospace;
    outline: none;
  }

  .branch-input-row input:focus {
    border-color: var(--accent);
  }

  .settings-footer {
    display: flex;
    align-items: center;
    justify-content: flex-end;
    gap: 8px;
    padding: 16px 24px;
    border-top: 1px solid var(--border-color);
  }

  .save-message {
    font-size: var(--ui-font-md);
    color: var(--green);
    margin-right: auto;
  }

  .btn {
    padding: 8px 16px;
    border: none;
    border-radius: 6px;
    font-size: var(--ui-font-base);
    font-weight: 500;
    cursor: pointer;
    font-family: inherit;
    transition: background-color 0.15s;
  }

  .btn-add {
    padding: 6px 12px;
    background: var(--bg-surface);
    color: var(--text-secondary);
    font-size: var(--ui-font-md);
  }

  .btn-add:hover:not(:disabled) {
    background: var(--bg-hover);
  }

  .btn-add:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-cancel {
    background: var(--bg-surface);
    color: var(--text-secondary);
  }

  .btn-cancel:hover {
    background: var(--bg-hover);
  }

  .btn-save {
    background: var(--accent);
    color: var(--bg-primary);
  }

  .btn-save:hover:not(:disabled) {
    background: var(--accent-hover);
  }

  .btn-save:disabled {
    opacity: 0.5;
    cursor: not-allowed;
  }

  .btn-danger {
    background: var(--red);
    color: var(--bg-primary);
  }

  .btn-danger:hover:not(:disabled) {
    filter: brightness(1.05);
  }

  .btn-ghost {
    background: none;
    border: 1px solid var(--border-color);
    color: var(--text-secondary);
    padding: 6px 10px;
    font-size: var(--ui-font-md);
  }

  .btn-ghost:hover {
    background: var(--bg-hover);
    color: var(--text-primary);
  }

  .mono {
    font-family: monospace;
  }

  .font-size-control {
    display: flex;
    align-items: center;
    gap: 6px;
    max-width: 200px;
  }

  .font-size-control input[type="number"] {
    width: 60px;
    text-align: center;
    padding: 6px 4px;
    background: var(--bg-secondary);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-primary);
    font-size: var(--ui-font-base);
    font-family: monospace;
    outline: none;
    appearance: textfield;
    -moz-appearance: textfield;
  }

  .font-size-control input[type="number"]::-webkit-inner-spin-button,
  .font-size-control input[type="number"]::-webkit-outer-spin-button {
    -webkit-appearance: none;
    margin: 0;
  }

  .font-size-control input[type="number"]:focus {
    border-color: var(--accent);
  }

  .font-size-btn {
    width: 28px;
    height: 28px;
    display: flex;
    align-items: center;
    justify-content: center;
    background: var(--bg-surface);
    border: 1px solid var(--border-color);
    border-radius: 6px;
    color: var(--text-primary);
    font-size: var(--ui-font-lg);
    font-family: monospace;
    cursor: pointer;
    flex-shrink: 0;
  }

  .font-size-btn:hover:not(:disabled) {
    background: var(--bg-hover);
    border-color: var(--accent);
  }

  .font-size-btn:disabled {
    opacity: 0.4;
    cursor: not-allowed;
  }

  .font-size-unit {
    font-size: var(--ui-font-sm);
    color: var(--text-muted);
  }

  @media (max-width: 800px) {
    .env-row,
    .env-add-row {
      grid-template-columns: 1fr;
    }
    .ai-grid {
      grid-template-columns: 1fr;
    }
    .skill-agent-row {
      flex-direction: column;
      align-items: flex-start;
    }
  }
</style>
