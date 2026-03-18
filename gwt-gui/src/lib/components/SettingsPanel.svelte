<script lang="ts">
  import { onDestroy, onMount } from "svelte";
  import type {
    ProfilesConfig,
    Profile,
    SettingsData,
    ShellInfo,
    VoiceInputSettings,
  } from "../types";
  import {
    UI_FONT_PRESETS,
    TERMINAL_FONT_PRESETS,
    getCurrentProfile,
    isDefaultProfileKey,
    toErrorMessage,
    detectGpuAvailability,
    normalizeVoiceInputSettings,
    normalizeAppLanguage,
    normalizeUiFontFamily,
    normalizeTerminalFontFamily,
    clampFontSize,
    PROFILE_NAME_PATTERN,
  } from "./settingsPanelHelpers";
  import ConfirmDialog from "./ConfirmDialog.svelte";
  import CreateProfileDialog from "./CreateProfileDialog.svelte";

  let { onClose }: { onClose: () => void } = $props();

  type SettingsTabId =
    | "general"
    | "profiles"
    | "terminal"
    | "voiceInput"
    | "agent";
  let activeSettingsTab: SettingsTabId = $state("general");

  let showCreateProfileDialog: boolean = $state(false);
  let showDeleteProfileDialog: boolean = $state(false);

  let availableShells: ShellInfo[] = $state([]);

  let settings: SettingsData | null = $state(null);
  let profiles: ProfilesConfig | null = $state(null);

  let loadingSettings: boolean = $state(true);
  let loadingProfiles: boolean = $state(true);
  let saving: boolean = $state(false);
  let errorMessage: string | null = $state(null);
  let newBranch: string = $state("");
  let saveMessage: string = $state("");
  let voiceCapabilityLoading: boolean = $state(false);
  let voiceAvailable: boolean = $state(true);
  let voiceUnavailableReason: string | null = $state(null);
  let voiceRuntimeSettingUp: boolean = $state(false);
  let voiceSetupMessage: string | null = $state(null);

  let selectedProfileKey: string = $state("");
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
  let apiKeyDraft: string = $state("");
  let apiKeyDraftProfileKey: string = $state("");
  let apiKeyDraftSourceValue: string = $state("");
  let peekingApiKey: boolean = $state(false);
  let apiKeyCopied: boolean = $state(false);
  let copyTimer: ReturnType<typeof setTimeout> | null = null;

  type AIModelInfo = {
    id: string;
  };
  let aiModels: string[] = $state([]);
  let aiModelsLoading: boolean = $state(false);
  let aiModelsError: string | null = $state(null);
  let aiModelsLoadedKey: string = $state("");
  let aiModelsRequestSeq: number = 0;

  let currentProfile = $derived(getCurrentProfile(profiles, selectedProfileKey));
  let currentAiRequestKey = $derived.by(() => {
    const profileKey = selectedProfileKey.trim();
    const ai = currentProfile?.ai;
    const endpoint = ai?.endpoint?.trim() ?? "";
    if (!profileKey || !ai || !endpoint) return "";
    const apiKey = apiKeyDraft.trim();
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
  let defaultProfileSelected = $derived(isDefaultProfileKey(selectedProfileKey));

  function toPlainData<T>(value: T): T {
    return JSON.parse(JSON.stringify(value)) as T;
  }

  function resetAiModelsState() {
    aiModelsRequestSeq += 1;
    aiModels = [];
    aiModelsLoading = false;
    aiModelsError = null;
    aiModelsLoadedKey = "";
  }

  function resetApiKeyUiState() {
    peekingApiKey = false;
    apiKeyCopied = false;
    if (copyTimer !== null) {
      clearTimeout(copyTimer);
      copyTimer = null;
    }
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

  $effect(() => {
    if (!settings) return;
    const quality = (settings.voice_input?.quality ?? "balanced").trim().toLowerCase();
    const gpuAvailable = detectGpuAvailability();
    let cancelled = false;

    (async () => {
      voiceCapabilityLoading = true;
      try {
        const { invoke } = await import("@tauri-apps/api/core");
        const capability = await invoke<{
          available: boolean;
          reason?: string | null;
        }>("get_voice_capability", {
          gpuAvailable,
          quality,
        });
        if (cancelled) return;
        voiceAvailable = !!capability.available;
        voiceUnavailableReason = capability.reason ?? null;
      } catch {
        if (cancelled) return;
        // In web preview, keep voice fields editable for config compatibility.
        voiceAvailable = true;
        voiceUnavailableReason = null;
      } finally {
        if (!cancelled) {
          voiceCapabilityLoading = false;
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  });

  $effect(() => {
    const profileKey = selectedProfileKey.trim();
    const nextValue = currentProfile?.ai?.api_key ?? "";
    if (profileKey === apiKeyDraftProfileKey && nextValue === apiKeyDraftSourceValue) {
      return;
    }
    apiKeyDraftProfileKey = profileKey;
    apiKeyDraftSourceValue = nextValue;
    apiKeyDraft = nextValue;
    resetApiKeyUiState();
  });

  async function handleSetupVoiceRuntime() {
    voiceRuntimeSettingUp = true;
    voiceSetupMessage = null;
    try {
      const { invoke } = await import("@tauri-apps/api/core");
      await invoke("ensure_voice_runtime");
      voiceSetupMessage = "Voice runtime setup completed.";
      // Refresh capability status
      voiceCapabilityLoading = true;
      try {
        const cap = await invoke<{
          available: boolean;
          reason?: string | null;
          modelReady: boolean;
        }>("get_voice_capability", {
          gpuAvailable: detectGpuAvailability(),
          quality: settings?.voice_input?.quality ?? "balanced",
        });
        voiceAvailable = cap.available;
        voiceUnavailableReason = cap.reason ?? null;
      } finally {
        voiceCapabilityLoading = false;
      }
    } catch (err) {
      voiceSetupMessage = `Setup failed: ${toErrorMessage(err)}`;
    } finally {
      voiceRuntimeSettingUp = false;
      setTimeout(() => {
        voiceSetupMessage = null;
      }, 5000);
    }
  }

  $effect(() => {
    const profileKey = selectedProfileKey.trim();
    const ai = currentProfile?.ai;

    if (!profileKey || !ai || !ai?.endpoint?.trim()) {
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
    if (copyTimer !== null) clearTimeout(copyTimer);
    applyUiFontSize(savedUiFontSize);
    applyTerminalFontSize(savedTerminalFontSize);
    applyUiFontFamily(savedUiFontFamily);
    applyTerminalFontFamily(savedTerminalFontFamily);
  });

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
      // The Tauri command accepts a camelCase request payload.
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
    const apiKey = apiKeyDraft.trim();
    if (!profileKey || !ai || !endpoint) {
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
      settings = loadedSettings;
      savedUiFontSize = loadedSettings.ui_font_size ?? 13;
      savedTerminalFontSize = loadedSettings.terminal_font_size ?? 13;
      savedUiFontFamily = loadedSettings.ui_font_family;
      savedTerminalFontFamily = loadedSettings.terminal_font_family;
      profiles = loadedProfiles;

      const keys = Object.keys(loadedProfiles.profiles ?? {});
      const nextSelected = loadedProfiles.active ?? keys[0] ?? "";
      selectedProfileKey = nextSelected;
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
      const normalizedSettings = {
        ...settings,
        ui_font_family: normalizeUiFontFamily(settings.ui_font_family),
        terminal_font_family: normalizeTerminalFontFamily(settings.terminal_font_family),
      };
      const plainSettings = toPlainData(normalizedSettings);
      settings = plainSettings;

      const { invoke } = await import("$lib/tauriInvoke");
      await invoke("save_settings", { settings: plainSettings });
      if (profiles) {
        const plainProfiles = toPlainData(buildProfilesConfigWithApiKeyDraft());
        const aiInPayload = plainProfiles.profiles?.[selectedProfileKey]?.ai;
        console.debug("[gwt] save_profiles AI payload:", selectedProfileKey, aiInPayload);
        profiles = plainProfiles;
        await invoke("save_profiles", { config: plainProfiles });
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

  function setActiveProfile(name: string) {
    if (!profiles) return;
    if (!name || !profiles.profiles?.[name]) return;
    profiles = { ...profiles, active: name };
    selectedProfileKey = name;
  }

  function handleCreateProfile(name: string) {
    if (!profiles) return;
    const trimmed = name.trim().toLowerCase();
    if (!trimmed) return;
    if (!PROFILE_NAME_PATTERN.test(trimmed)) {
      saveMessage = "Profile name must be lowercase letters, numbers, or hyphens.";
      return;
    }
    if (profiles.profiles?.[trimmed]) {
      saveMessage = "Profile already exists.";
      return;
    }

    const next: Profile = {
      name: trimmed,
      description: "",
      env: {},
      disabled_env: [],
      ai: null,
    };

    profiles = {
      ...profiles,
      profiles: { ...profiles.profiles, [trimmed]: next },
      active: trimmed,
    };
    selectedProfileKey = trimmed;
    showCreateProfileDialog = false;
  }

  function confirmDeleteProfile() {
    if (!profiles) return;
    if (!selectedProfileKey) return;
    if (isDefaultProfileKey(selectedProfileKey)) return;
    const copy = { ...(profiles.profiles ?? {}) };
    if (!copy[selectedProfileKey]) return;
    delete copy[selectedProfileKey];
    const nextKeys = Object.keys(copy).sort((a, b) => a.localeCompare(b));
    const nextActive =
      profiles.active === selectedProfileKey ? (nextKeys[0] ?? null) : profiles.active ?? null;
    profiles = { ...profiles, profiles: copy, active: nextActive };
    selectedProfileKey = nextActive ?? "";
    showDeleteProfileDialog = false;
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
    field: "endpoint" | "api_key" | "model" | "language",
    value: string
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
      ...(p.ai ?? {}),
      [field]: value,
    } as Profile["ai"];
    const nextProfile: Profile = { ...p, ai: nextAi };
    profiles = {
      ...profiles,
      profiles: { ...(profiles.profiles ?? {}), [selectedProfileKey]: nextProfile },
    };
  }

  function buildProfilesConfigWithApiKeyDraft(): ProfilesConfig {
    if (!profiles) {
      throw new Error("Profiles are not loaded");
    }
    const p = currentProfile;
    if (!p || !selectedProfileKey) {
      return profiles;
    }
    if (!p.ai && apiKeyDraft.length === 0) {
      return profiles;
    }

    const nextAi = {
      endpoint: p.ai?.endpoint ?? "",
      api_key: apiKeyDraft,
      model: p.ai?.model ?? "",
      language: p.ai?.language ?? "en",
    } as NonNullable<Profile["ai"]>;
    const nextProfile: Profile = { ...p, ai: nextAi };
    const nextProfiles = {
      ...profiles,
      profiles: { ...(profiles.profiles ?? {}), [selectedProfileKey]: nextProfile },
    };
    return nextProfiles;
  }

  async function handleCopyApiKey() {
    const key = apiKeyDraft;
    if (!key) return;
    try {
      await navigator.clipboard.writeText(key);
      apiKeyCopied = true;
      if (copyTimer !== null) clearTimeout(copyTimer);
      copyTimer = setTimeout(() => { apiKeyCopied = false; copyTimer = null; }, 1500);
    } catch (e) { console.warn("Failed to copy API key:", e); }
  }

  function startApiKeyPeek() {
    peekingApiKey = true;
  }

  function stopApiKeyPeek() {
    peekingApiKey = false;
  }

  function applyApiKeyDraft(nextValue: string) {
    const profileKey = selectedProfileKey.trim();
    apiKeyCopied = false;
    apiKeyDraft = nextValue;
    apiKeyDraftProfileKey = profileKey;
    apiKeyDraftSourceValue = nextValue;
    updateAiField("api_key", nextValue);
  }

  function syncApiKeyDraftToProfile() {
    applyApiKeyDraft(apiKeyDraft);
  }

  function syncApiKeyDraftFromDom(input: HTMLInputElement | null) {
    if (!input) return;
    setTimeout(() => {
      applyApiKeyDraft(input.value);
    }, 0);
  }

  function handleApiKeyPaste(event: ClipboardEvent) {
    const input = event.currentTarget as HTMLInputElement | null;
    if (!input) return;

    const pastedText = event.clipboardData?.getData("text/plain");
    if (typeof pastedText === "string" && pastedText.length > 0) {
      event.preventDefault();
      const selectionStart = input.selectionStart ?? input.value.length;
      const selectionEnd = input.selectionEnd ?? input.value.length;
      const nextValue =
        input.value.slice(0, selectionStart) +
        pastedText +
        input.value.slice(selectionEnd);
      applyApiKeyDraft(nextValue);
      setTimeout(() => {
        const nextCursor = selectionStart + pastedText.length;
        input.setSelectionRange(nextCursor, nextCursor);
      }, 0);
      return;
    }

    syncApiKeyDraftFromDom(input);
  }

  function toggleApiKeyPeekFromNonPointerClick(event: MouseEvent) {
    // Keyboard and assistive activation can dispatch click without pointer down/up.
    if (event.detail !== 0) return;
    peekingApiKey = !peekingApiKey;
  }

  function updateVoiceInputField(
    field: keyof VoiceInputSettings,
    value: VoiceInputSettings[keyof VoiceInputSettings]
  ) {
    if (!settings) return;
    const current = normalizeVoiceInputSettings(settings.voice_input);
    const next = { ...current, [field]: value } as VoiceInputSettings;
    if (field === "quality") {
      const quality = String(value).trim().toLowerCase();
      next.model =
        quality === "fast" ? "Qwen/Qwen3-ASR-0.6B" : "Qwen/Qwen3-ASR-1.7B";
    }
    if (!next.engine || next.engine.trim().toLowerCase() !== "qwen3-asr") {
      next.engine = "qwen3-asr";
    }
    settings = { ...settings, voice_input: normalizeVoiceInputSettings(next) };
  }

</script>

<div class="settings-panel">
  <div class="settings-header">
    <h2>Settings</h2>
    <button class="close-btn" onclick={handleClose} aria-label="Close">&times;</button>
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
          class:active={activeSettingsTab === "general"}
          onclick={() => (activeSettingsTab = "general")}
        >General</button>
        <button
          class="settings-tab-btn"
          class:active={activeSettingsTab === "profiles"}
          onclick={() => (activeSettingsTab = "profiles")}
        >Profiles</button>
        <button
          class="settings-tab-btn"
          class:active={activeSettingsTab === "terminal"}
          onclick={() => (activeSettingsTab = "terminal")}
        >Terminal</button>
        <button
          class="settings-tab-btn"
          class:active={activeSettingsTab === "voiceInput"}
          onclick={() => (activeSettingsTab = "voiceInput")}
        >Voice Input</button>
        <button
          class="settings-tab-btn"
          class:active={activeSettingsTab === "agent"}
          onclick={() => (activeSettingsTab = "agent")}
        >Agent</button>
      </div>

      <div class="settings-tab-content">
        <!-- ═══ General ═══ -->
        {#if activeSettingsTab === "general"}
          <div class="section-content">
            <div class="settings-section">
              <h3 class="settings-section-title">Display</h3>
              <div class="settings-section-body">
                <div class="field">
                  <!-- svelte-ignore a11y_label_has_associated_control -->
                  <label>UI font size</label>
                  <div class="font-size-control">
                    <button
                      class="font-size-btn"
                      onclick={() => adjustFontSize("ui_font_size", -1)}
                      disabled={(settings.ui_font_size ?? 13) <= 8}
                    >-</button>
                    <input
                      type="number"
                      min="8"
                      max="24"
                      step="1"
                      value={settings.ui_font_size ?? 13}
                      oninput={(e) => {
                        const raw = (e.target as HTMLInputElement).value;
                        if (raw === "") return;
                        const parsed = Number(raw);
                        if (Number.isNaN(parsed)) return;
                        const current = settings as SettingsData;
                        settings = { ...current, ui_font_size: parsed };
                      }}
                      onchange={() => {
                        const current = settings as SettingsData;
                        settings = {
                          ...current,
                          ui_font_size: clampFontSize(current.ui_font_size ?? 13),
                        };
                      }}
                    />
                    <button
                      class="font-size-btn"
                      onclick={() => adjustFontSize("ui_font_size", 1)}
                      disabled={(settings.ui_font_size ?? 13) >= 24}
                    >+</button>
                    <span class="font-size-unit">px</span>
                  </div>
                </div>

                <div class="field">
                  <label for="ui-font-family">UI font family</label>
                  <select
                    id="ui-font-family"
                    class="select"
                    value={settings.ui_font_family}
                    onchange={(e) => {
                      const current = settings as SettingsData;
                      const next = normalizeUiFontFamily(
                        (e.target as HTMLSelectElement).value
                      );
                      settings = { ...current, ui_font_family: next };
                      applyUiFontFamily(next);
                    }}
                  >
                    {#each UI_FONT_PRESETS as preset}
                      <option value={preset.value}>{preset.label}</option>
                    {/each}
                  </select>
                </div>
              </div>
            </div>

            <div class="settings-section">
              <h3 class="settings-section-title">Language</h3>
              <div class="settings-section-body">
                <div class="field">
                  <label for="app-language">AI summary language</label>
                  <select
                    id="app-language"
                    class="select"
                    value={settings.app_language}
                    onchange={(e) => {
                      const current = settings as SettingsData;
                      settings = {
                        ...current,
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
                    Default language for AI-generated branch summaries.
                  </span>
                </div>
              </div>
            </div>

            <div class="settings-section">
              <h3 class="settings-section-title">Git</h3>
              <div class="settings-section-body">
                <div class="field">
                  <!-- svelte-ignore a11y_label_has_associated_control -->
                  <label>Protected branches</label>
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
            </div>

            <div class="settings-section">
              <h3 class="settings-section-title">Maintenance</h3>
              <div class="settings-section-body">
                <div class="field">
                  <label for="log-retention">Log retention</label>
                  <input
                    id="log-retention"
                    type="number"
                    min="1"
                    max="365"
                    bind:value={settings.log_retention_days}
                  />
                  <span class="field-hint">
                    Days before logs are automatically deleted (1–365).
                  </span>
                </div>
              </div>
            </div>
          </div>

        <!-- ═══ Profiles ═══ -->
        {:else if activeSettingsTab === "profiles"}
          <div class="section-content">
            <div class="profile-header">
              <select
                class="select profile-select"
                value={profiles?.active ?? ""}
                onchange={(e) => setActiveProfile((e.target as HTMLSelectElement).value)}
              >
                {#if profiles}
                  {#each sortedProfileKeys(profiles) as key}
                    <option value={key}>{key}</option>
                  {/each}
                {/if}
              </select>
              <button
                class="btn btn-ghost btn-danger-text"
                onclick={() => (showDeleteProfileDialog = true)}
                disabled={!profiles || !selectedProfileKey || defaultProfileSelected}
                title={defaultProfileSelected ? "The default profile cannot be deleted" : "Delete profile"}
              >Delete</button>
              <button
                class="btn btn-add"
                onclick={() => (showCreateProfileDialog = true)}
                disabled={!profiles}
              >+ New</button>
            </div>

            {#if profiles && selectedProfileKey && currentProfile}
              <div class="settings-section">
                <h3 class="settings-section-title">Environment Variables</h3>
                <div class="settings-section-body">
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
                </div>
              </div>

              {@const currentAi = currentProfile.ai ?? {
                endpoint: "",
                api_key: "",
                model: "",
                language: "en",
              }}
              {@const currentEndpoint = currentAi?.endpoint?.trim() ?? ""}
              {@const hasApiKey = apiKeyDraft.length > 0}
              <div class="settings-section">
                <h3 class="settings-section-title">AI Configuration</h3>
                <div class="settings-section-body">
                  <div class="field">
                    <label for="ai-endpoint">Endpoint</label>
                    <input
                      id="ai-endpoint"
                      type="text"
                      autocapitalize="off"
                      autocorrect="off"
                      autocomplete="off"
                      spellcheck="false"
                      value={currentAi?.endpoint ?? ""}
                      oninput={(e) => updateAiField("endpoint", (e.target as HTMLInputElement).value)}
                    />
                    <span class="field-hint">OpenAI-compatible API base URL (e.g. https://api.openai.com/v1)</span>
                  </div>

                  <div class="field">
                    <!-- svelte-ignore a11y_label_has_associated_control -->
                    <label>API key</label>
                    <div class="row ai-apikey-row">
                      <input
                        type="text"
                        class:api-key-masked={!peekingApiKey}
                        autocapitalize="off"
                        autocorrect="off"
                        autocomplete="off"
                        spellcheck="false"
                        bind:value={apiKeyDraft}
                        oninput={syncApiKeyDraftToProfile}
                        onchange={syncApiKeyDraftToProfile}
                        onpaste={handleApiKeyPaste}
                      />
                      <div class="ai-apikey-actions" class:hidden={!hasApiKey}>
                        <button
                          type="button"
                          class="btn btn-ghost btn-icon btn-peek-apikey"
                          class:peeking={peekingApiKey}
                          disabled={!hasApiKey}
                          onmousedown={startApiKeyPeek}
                          onmouseup={stopApiKeyPeek}
                          onmouseleave={stopApiKeyPeek}
                          onblur={stopApiKeyPeek}
                          onclick={toggleApiKeyPeekFromNonPointerClick}
                          title="Peek API Key"
                          aria-label="Peek API Key"
                        >
                          <svg viewBox="0 0 24 24" aria-hidden="true" focusable="false">
                            <path class="eye-outline" d="M2 12C4.5 8 7.5 6 12 6s7.5 2 10 6c-2.5 4-5.5 6-10 6s-7.5-2-10-6Z" />
                            <circle class="eye-pupil" cx="12" cy="12" r="2.2"></circle>
                            {#if !peekingApiKey}
                              <path class="eye-slash" d="M4 20L20 4" />
                            {/if}
                          </svg>
                        </button>
                        <button
                          type="button"
                          class="btn btn-ghost btn-icon btn-copy-apikey"
                          class:copied={apiKeyCopied}
                          disabled={!hasApiKey}
                          onclick={handleCopyApiKey}
                          title={apiKeyCopied ? "Copied!" : "Copy API Key"}
                          aria-label={apiKeyCopied ? "Copied!" : "Copy API Key"}
                        >
                          <svg viewBox="0 0 24 24" aria-hidden="true" focusable="false">
                            <rect class="copy-back" x="6" y="4" width="10" height="12" rx="1.8"></rect>
                            <rect class="copy-front" x="9" y="8" width="10" height="12" rx="1.8"></rect>
                          </svg>
                        </button>
                      </div>
                    </div>
                    <span class="field-hint">Stored locally in ~/.gwt/config.toml</span>
                  </div>

                  <div class="field">
                    <!-- svelte-ignore a11y_label_has_associated_control -->
                    <label>Model</label>
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
                    {#if currentEndpoint && !(currentAi?.model?.trim())}
                      <span class="field-hint field-hint-warning">
                        Both endpoint and model are required to enable AI features.
                      </span>
                    {/if}
                  </div>

                  <div class="field">
                    <label for="ai-language">AI response language</label>
                    <select
                      id="ai-language"
                      class="select"
                      value={currentAi?.language ?? "en"}
                      onchange={(e) => updateAiField("language", (e.target as HTMLSelectElement).value)}
                    >
                      <option value="en">English</option>
                      <option value="ja">Japanese</option>
                      <option value="auto">Auto</option>
                    </select>
                    <span class="field-hint">Language used for AI-generated responses in this profile.</span>
                  </div>
                </div>
              </div>
            {:else}
              <div class="field-hint" style="padding: 16px 0;">Create a profile to configure settings.</div>
            {/if}
          </div>

        <!-- ═══ Terminal ═══ -->
        {:else if activeSettingsTab === "terminal"}
          <div class="section-content">
            <div class="settings-section">
              <h3 class="settings-section-title">Font</h3>
              <div class="settings-section-body">
                <div class="field">
                  <!-- svelte-ignore a11y_label_has_associated_control -->
                  <label>Terminal font size</label>
                  <div class="font-size-control">
                    <button
                      class="font-size-btn"
                      onclick={() => adjustFontSize("terminal_font_size", -1)}
                      disabled={(settings.terminal_font_size ?? 13) <= 8}
                    >-</button>
                    <input
                      type="number"
                      min="8"
                      max="24"
                      step="1"
                      value={settings.terminal_font_size ?? 13}
                      oninput={(e) => {
                        const raw = (e.target as HTMLInputElement).value;
                        if (raw === "") return;
                        const parsed = Number(raw);
                        if (Number.isNaN(parsed)) return;
                        const current = settings as SettingsData;
                        settings = { ...current, terminal_font_size: parsed };
                      }}
                      onchange={() => {
                        const current = settings as SettingsData;
                        settings = {
                          ...current,
                          terminal_font_size: clampFontSize(current.terminal_font_size ?? 13),
                        };
                      }}
                    />
                    <button
                      class="font-size-btn"
                      onclick={() => adjustFontSize("terminal_font_size", 1)}
                      disabled={(settings.terminal_font_size ?? 13) >= 24}
                    >+</button>
                    <span class="font-size-unit">px</span>
                  </div>
                </div>

                <div class="field">
                  <label for="terminal-font-family">Terminal font family</label>
                  <select
                    id="terminal-font-family"
                    class="select"
                    value={settings.terminal_font_family}
                    onchange={(e) => {
                      const current = settings as SettingsData;
                      const next = normalizeTerminalFontFamily(
                        (e.target as HTMLSelectElement).value
                      );
                      settings = { ...current, terminal_font_family: next };
                      applyTerminalFontFamily(next);
                    }}
                  >
                    {#each TERMINAL_FONT_PRESETS as preset}
                      <option value={preset.value}>{preset.label}</option>
                    {/each}
                  </select>
                </div>
              </div>
            </div>

            {#if availableShells.length > 0}
              <div class="settings-section">
                <h3 class="settings-section-title">Shell</h3>
                <div class="settings-section-body">
                  <div class="field">
                    <label for="default-shell">Default shell</label>
                    <select
                      id="default-shell"
                      class="select"
                      value={settings.default_shell ?? ""}
                      onchange={(e) => {
                        const current = settings as SettingsData;
                        const value = (e.target as HTMLSelectElement).value;
                        settings = { ...current, default_shell: value || null };
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
                      Shell used when launching new terminal panes.
                    </span>
                  </div>
                </div>
              </div>
            {/if}
          </div>

        <!-- ═══ Voice Input ═══ -->
        {:else if activeSettingsTab === "voiceInput"}
          <div class="section-content">
            <div class="field">
              <div class="ai-toggle">
                <input
                  id="voice-input-enabled"
                  type="checkbox"
                  checked={settings.voice_input.enabled}
                  disabled={voiceCapabilityLoading}
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
            </div>

            {#if settings.voice_input.enabled}
              <div class="settings-section">
                <h3 class="settings-section-title">Hotkeys</h3>
                <div class="settings-section-body">
                  <div class="field">
                    <label for="voice-hotkey">Toggle hotkey</label>
                    <input
                      id="voice-hotkey"
                      type="text"
                      value={settings.voice_input.hotkey}
                      disabled={voiceCapabilityLoading}
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
                    <label for="voice-ptt-hotkey">Push-to-talk hotkey</label>
                    <input
                      id="voice-ptt-hotkey"
                      type="text"
                      value={settings.voice_input.ptt_hotkey}
                      disabled={voiceCapabilityLoading}
                      oninput={(e) =>
                        updateVoiceInputField(
                          "ptt_hotkey",
                          (e.target as HTMLInputElement).value
                        )}
                      placeholder="Mod+Shift+Space"
                    />
                    <span class="field-hint">Press and hold to capture speech.</span>
                  </div>
                </div>
              </div>

              <div class="settings-section">
                <h3 class="settings-section-title">Recognition</h3>
                <div class="settings-section-body">
                  <div class="field">
                    <label for="voice-language">Language</label>
                    <select
                      id="voice-language"
                      class="select"
                      value={settings.voice_input.language}
                      disabled={voiceCapabilityLoading}
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
                    <label for="voice-quality">Quality</label>
                    <select
                      id="voice-quality"
                      class="select"
                      value={settings.voice_input.quality}
                      disabled={voiceCapabilityLoading}
                      onchange={(e) =>
                        updateVoiceInputField(
                          "quality",
                          (e.target as HTMLSelectElement).value as VoiceInputSettings["quality"]
                        )}
                    >
                      <option value="fast">Fast (Qwen3-ASR-0.6B)</option>
                      <option value="balanced">Balanced (Qwen3-ASR-1.7B)</option>
                      <option value="accurate">Accurate (Qwen3-ASR-1.7B)</option>
                    </select>
                    <span class="field-hint">
                      Voice runtime and Qwen model are prepared automatically on first use.
                    </span>
                  </div>

                  <div class="field">
                    <label for="voice-model">Model</label>
                    <input
                      id="voice-model"
                      type="text"
                      value={settings.voice_input.model}
                      readonly
                      disabled
                    />
                    <span class="field-hint">Automatically selected based on Quality setting.</span>
                  </div>
                </div>
              </div>

              {#if voiceCapabilityLoading}
                <div class="field">
                  <span class="field-hint">Checking voice runtime capability...</span>
                </div>
              {:else if !voiceAvailable}
                <div class="field">
                  <span class="field-hint" style="color: var(--warning-color, #e6a700);">
                    {voiceUnavailableReason ?? "GPU acceleration and Qwen runtime are required."}
                  </span>
                  {#if voiceUnavailableReason && (voiceUnavailableReason.toLowerCase().includes("runtime") || voiceUnavailableReason.toLowerCase().includes("python") || voiceUnavailableReason.toLowerCase().includes("package"))}
                    <button
                      class="btn btn-sm"
                      onclick={handleSetupVoiceRuntime}
                      disabled={voiceRuntimeSettingUp}
                    >
                      {voiceRuntimeSettingUp ? "Setting up..." : "Setup Voice Runtime"}
                    </button>
                  {/if}
                  {#if voiceSetupMessage}
                    <span class="field-hint">{voiceSetupMessage}</span>
                  {/if}
                  <span class="field-hint">
                    Settings can still be configured and will take effect once the runtime is available.
                  </span>
                </div>
              {/if}
            {/if}
          </div>

        <!-- ═══ Agent ═══ -->
        {:else if activeSettingsTab === "agent"}
          <div class="section-content">
            <div class="settings-section">
              <h3 class="settings-section-title">Skill Registration</h3>
              <div class="settings-section-body">
                <div class="field">
                  <div class="ai-toggle">
                    <input
                      id="agent-skill-registration"
                      type="checkbox"
                      checked={settings.agent_skill_registration_enabled ?? false}
                      onchange={(e) => {
                        const current = settings as SettingsData;
                        settings = { ...current, agent_skill_registration_enabled: (e.target as HTMLInputElement).checked };
                      }}
                    />
                    <label for="agent-skill-registration">Enable skill registration</label>
                  </div>
                </div>
              </div>
            </div>

            <div class="settings-section">
              <h3 class="settings-section-title">Docs Injection</h3>
              <div class="settings-section-body">
                <div class="field">
                  <div class="ai-toggle">
                    <input
                      id="agent-inject-claude-md"
                      type="checkbox"
                      checked={settings.agent_inject_claude_md ?? true}
                      onchange={(e) => {
                        const current = settings as SettingsData;
                        settings = { ...current, agent_inject_claude_md: (e.target as HTMLInputElement).checked };
                      }}
                    />
                    <label for="agent-inject-claude-md">CLAUDE.md</label>
                  </div>
                </div>
                <div class="field">
                  <div class="ai-toggle">
                    <input
                      id="agent-inject-agents-md"
                      type="checkbox"
                      checked={settings.agent_inject_agents_md ?? false}
                      onchange={(e) => {
                        const current = settings as SettingsData;
                        settings = { ...current, agent_inject_agents_md: (e.target as HTMLInputElement).checked };
                      }}
                    />
                    <label for="agent-inject-agents-md">AGENTS.md</label>
                  </div>
                </div>
                <div class="field">
                  <div class="ai-toggle">
                    <input
                      id="agent-inject-gemini-md"
                      type="checkbox"
                      checked={settings.agent_inject_gemini_md ?? false}
                      onchange={(e) => {
                        const current = settings as SettingsData;
                        settings = { ...current, agent_inject_gemini_md: (e.target as HTMLInputElement).checked };
                      }}
                    />
                    <label for="agent-inject-gemini-md">GEMINI.md</label>
                  </div>
                </div>
                <span class="field-hint">
                  Inject skills catalog on each agent launch.
                </span>
              </div>
            </div>

            <div class="settings-section">
              <h3 class="settings-section-title">Agent Paths</h3>
              <div class="settings-section-body">
                <div class="field">
                  <label for="agent-claude-path">Claude Code</label>
                  <input
                    id="agent-claude-path"
                    type="text"
                    autocapitalize="off"
                    autocorrect="off"
                    autocomplete="off"
                    spellcheck="false"
                    value={settings.agent_claude_path ?? ""}
                    oninput={(e) => {
                      const current = settings as SettingsData;
                      const v = (e.target as HTMLInputElement).value;
                      settings = { ...current, agent_claude_path: v || null };
                    }}
                    placeholder="claude"
                  />
                </div>
                <div class="field">
                  <label for="agent-codex-path">Codex CLI</label>
                  <input
                    id="agent-codex-path"
                    type="text"
                    autocapitalize="off"
                    autocorrect="off"
                    autocomplete="off"
                    spellcheck="false"
                    value={settings.agent_codex_path ?? ""}
                    oninput={(e) => {
                      const current = settings as SettingsData;
                      const v = (e.target as HTMLInputElement).value;
                      settings = { ...current, agent_codex_path: v || null };
                    }}
                    placeholder="codex"
                  />
                </div>
                <div class="field">
                  <label for="agent-gemini-path">Gemini CLI</label>
                  <input
                    id="agent-gemini-path"
                    type="text"
                    autocapitalize="off"
                    autocorrect="off"
                    autocomplete="off"
                    spellcheck="false"
                    value={settings.agent_gemini_path ?? ""}
                    oninput={(e) => {
                      const current = settings as SettingsData;
                      const v = (e.target as HTMLInputElement).value;
                      settings = { ...current, agent_gemini_path: v || null };
                    }}
                    placeholder="gemini"
                  />
                </div>
              </div>
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

<CreateProfileDialog
  open={showCreateProfileDialog}
  onClose={() => (showCreateProfileDialog = false)}
  onCreate={handleCreateProfile}
/>

<ConfirmDialog
  open={showDeleteProfileDialog}
  title="Delete Profile"
  message={`Delete profile "${selectedProfileKey}"? This will remove all its environment variables and AI configuration.`}
  confirmLabel="Delete"
  confirmDanger={true}
  onClose={() => (showDeleteProfileDialog = false)}
  onConfirm={confirmDeleteProfile}
/>

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
    font-size: 20px;
    padding: 4px 8px;
    border-radius: 4px;
    line-height: 1;
  }

  .close-btn:hover {
    color: var(--text-primary);
    background: var(--bg-hover);
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

  .section-content {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding-top: 8px;
  }

  .settings-section {
    margin-top: 8px;
  }

  .settings-section:first-child {
    margin-top: 0;
  }

  .settings-section-title {
    font-size: var(--ui-font-sm);
    font-weight: 600;
    color: var(--text-muted);
    text-transform: uppercase;
    letter-spacing: 0.5px;
    padding-bottom: 8px;
    border-bottom: 1px solid var(--border-color);
    margin-bottom: 16px;
  }

  .settings-section-body {
    display: flex;
    flex-direction: column;
    gap: 20px;
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
  }

  .profile-header {
    display: flex;
    align-items: center;
    gap: 8px;
    padding-bottom: 16px;
    border-bottom: 1px solid var(--border-color);
    margin-bottom: 8px;
  }

  .profile-select {
    flex: 1;
    min-width: 120px;
  }

  .btn-danger-text {
    color: var(--red);
    border-color: var(--red);
  }

  .btn-danger-text:hover:not(:disabled) {
    background: var(--red);
    color: var(--bg-primary);
  }

  .btn-danger-text:disabled {
    opacity: 0.4;
    color: var(--text-muted);
    border-color: var(--border-color);
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

  /* AI configuration fields need full width */
  .settings-section-body .ai-apikey-row input,
  #ai-endpoint,
  #ai-language {
    max-width: none;
  }

  .ai-apikey-row input { flex: 1; min-width: 0; max-width: none; }
  .ai-apikey-row input.api-key-masked { -webkit-text-security: disc; }
  .ai-apikey-actions {
    display: flex;
    gap: 8px;
    flex-shrink: 0;
  }
  .ai-apikey-actions.hidden {
    visibility: hidden;
    opacity: 0;
    pointer-events: none;
  }

  .btn-icon {
    width: 32px;
    height: 32px;
    padding: 0;
    display: flex;
    align-items: center;
    justify-content: center;
    position: relative;
    flex-shrink: 0;
    cursor: pointer;
    line-height: 0;
    appearance: none;
    -webkit-appearance: none;
  }

  .btn-icon svg {
    display: block;
    width: 18px;
    height: 18px;
  }

  /* Eye icon (peek) */
  .btn-peek-apikey .eye-outline,
  .btn-peek-apikey .eye-slash {
    fill: none;
    stroke: var(--text-secondary);
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .btn-peek-apikey .eye-pupil {
    fill: var(--text-secondary);
  }
  .btn-peek-apikey.peeking .eye-outline,
  .btn-peek-apikey.peeking .eye-slash { stroke: var(--accent); }
  .btn-peek-apikey.peeking .eye-pupil { fill: var(--accent); }

  /* Copy icon */
  .btn-copy-apikey .copy-front,
  .btn-copy-apikey .copy-back {
    fill: none;
    stroke: var(--text-secondary);
    stroke-width: 1.8;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .btn-copy-apikey.copied .copy-front,
  .btn-copy-apikey.copied .copy-back { stroke: var(--green); }

  /* Hover states */
  .btn-peek-apikey:hover .eye-outline,
  .btn-peek-apikey:hover .eye-slash { stroke: var(--text-primary); }
  .btn-peek-apikey:hover .eye-pupil { fill: var(--text-primary); }
  .btn-copy-apikey:hover .copy-front,
  .btn-copy-apikey:hover .copy-back { stroke: var(--text-primary); }

  .ai-model-row {
    align-items: center;
  }

  .ai-model-select {
    flex: 1;
    max-width: none;
    min-width: 220px;
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
  }
</style>
