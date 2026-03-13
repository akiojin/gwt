import { expect, test, type Page } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";

const defaultRecentProject = {
  path: "/tmp/gwt-playwright",
  lastOpened: "2026-02-13T00:00:00.000Z",
};

const branchFeature = {
  name: "feature/windows-shell-selection",
  commit: "abc1234",
  is_current: false,
  ahead: 1,
  behind: 0,
  divergence_status: "Ahead",
  last_tool_usage: null,
  is_agent_running: false,
  commit_timestamp: 1_700_000_200,
};

const branchMain = {
  ...branchFeature,
  name: "main",
  is_current: true,
  divergence_status: "UpToDate",
  ahead: 0,
  commit_timestamp: 1_700_000_250,
};

const branchDevelop = {
  ...branchFeature,
  name: "develop",
  is_current: false,
  divergence_status: "UpToDate",
  ahead: 0,
  commit_timestamp: 1_700_000_230,
};

const availableShells = [
  { id: "powershell", name: "PowerShell", version: "7.4.1" },
  { id: "cmd", name: "Command Prompt" },
  { id: "wsl", name: "WSL" },
];

const DEFAULT_UI_FONT_FAMILY =
  'system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif';
const DEFAULT_TERMINAL_FONT_FAMILY =
  '"JetBrains Mono", "Fira Code", "SF Mono", Menlo, Consolas, monospace';
const UI_FONT_FAMILY_INTER =
  '"Inter", system-ui, -apple-system, "Segoe UI", Roboto, Ubuntu, sans-serif';
const TERMINAL_FONT_FAMILY_CASCADIA =
  '"Cascadia Mono", "Cascadia Code", Consolas, monospace';

const detectedAgents = [
  {
    id: "codex",
    name: "Codex",
    version: "0.0.1",
    authenticated: true,
    available: true,
  },
];

const settingsFixture = {
  protected_branches: ["main", "develop"],
  default_base_branch: "main",
  worktree_root: "/tmp/worktrees",
  debug: false,
  log_retention_days: 30,
  agent_default: "codex",
  agent_auto_install_deps: true,
  docker_force_host: true,
  ui_font_size: 13,
  terminal_font_size: 13,
  ui_font_family: DEFAULT_UI_FONT_FAMILY,
  terminal_font_family: DEFAULT_TERMINAL_FONT_FAMILY,
  app_language: "auto",
  voice_input: {
    enabled: false,
    hotkey: "Mod+Shift+M",
    language: "auto",
    model: "base",
  },
  default_shell: null,
};

const profilesFixture = {
  version: 1,
  active: "default",
  profiles: {
    default: {
      name: "default",
      description: "",
      env: {},
      disabled_env: [],
      ai_enabled: false,
      ai: null,
    },
  },
};

async function setMockCommandResponses(
  page: Page,
  commandResponses: Record<string, unknown>,
) {
  await page.evaluate((responses) => {
    (window as unknown as {
      __GWT_MOCK_COMMAND_RESPONSES__?: Record<string, unknown>;
    }).__GWT_MOCK_COMMAND_RESPONSES__ = responses;
  }, commandResponses);
}

async function dismissSkillRegistrationScopeDialogIfPresent(_page: Page) {
  // No-op: scope dialog removed after scope simplification.
}

async function openProjectAndSelectBranch(
  page: Page,
  commandResponses: Record<string, unknown>,
) {
  await page.goto("/");
  await setMockCommandResponses(page, commandResponses);

  await expect(
    page.getByRole("button", { name: "Open Project..." }),
  ).toBeVisible();
  await dismissSkillRegistrationScopeDialogIfPresent(page);
  await page.locator("button.recent-item").first().click();
  await expect(
    page.getByPlaceholder("Type a task and press Enter..."),
  ).toBeVisible();

  const branchButton = page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name });
  await expect(branchButton).toBeVisible();
  await branchButton.click();

  await expect(page.locator(".branch-detail h2")).toContainText(branchFeature.name);
}

async function waitForInvokeCommand(page: Page, cmd: string) {
  await expect
    .poll(async () => {
      return page.evaluate((targetCmd) => {
        const globalWindow = window as unknown as {
          __GWT_TAURI_INVOKE_LOG__?: Array<{ cmd: string }>;
        };
        return (globalWindow.__GWT_TAURI_INVOKE_LOG__ ?? []).some(
          (entry) => entry.cmd === targetCmd,
        );
      }, cmd);
    })
    .toBe(true);
}

async function waitForMenuActionListener(page: Page) {
  await expect
    .poll(async () => {
      return page.evaluate(() => {
        const globalWindow = window as unknown as {
          __GWT_TAURI_INVOKE_LOG__?: Array<{
            cmd: string;
            args?: { event?: string };
          }>;
        };
        return (globalWindow.__GWT_TAURI_INVOKE_LOG__ ?? []).some(
          (entry) =>
            entry.cmd === "plugin:event|listen" &&
            entry.args?.event === "menu-action",
        );
      });
    })
    .toBe(true);
}

async function readFontFamilyPreview(page: Page) {
  return page.evaluate(() => ({
    ui: getComputedStyle(document.documentElement)
      .getPropertyValue("--ui-font-family")
      .trim(),
    terminal: getComputedStyle(document.documentElement)
      .getPropertyValue("--terminal-font-family")
      .trim(),
    terminalWindow: (window as any).__gwtTerminalFontFamily ?? "",
  }));
}

async function openSettingsFromMenu(page: Page) {
  await waitForMenuActionListener(page);
  await page.evaluate(() => {
    const globalWindow = window as unknown as {
      __GWT_MOCK_EMIT_EVENT__?: (event: string, payload: unknown) => void;
    };
    globalWindow.__GWT_MOCK_EMIT_EVENT__?.("menu-action", {
      action: "open-settings",
    });
  });

  await expect(page.getByRole("heading", { name: "Settings" })).toBeVisible();
}

test.beforeEach(async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
    },
  });
});

test("launches with selected Windows shell from Launch Agent form", async ({
  page,
}) => {
  await openProjectAndSelectBranch(page, {
    list_worktree_branches: [branchMain, branchDevelop, branchFeature],
    list_remote_branches: [],
    list_worktrees: [],
    detect_agents: detectedAgents,
    get_available_shells: availableShells,
  });

  await page.getByRole("button", { name: "Launch Agent..." }).click();
  await expect(page.getByRole("dialog", { name: "Launch Agent" })).toBeVisible();

  await page.getByRole("button", { name: "Advanced" }).click();
  await page.getByLabel("Shell").selectOption("wsl");
  await page.getByRole("button", { name: "Launch", exact: true }).click();

  await waitForInvokeCommand(page, "start_launch_job");

  const request = await page.evaluate(() => {
    const globalWindow = window as unknown as {
      __GWT_TAURI_INVOKE_LOG__?: Array<{
        cmd: string;
        args?: { request?: Record<string, unknown> };
      }>;
    };
    const log = globalWindow.__GWT_TAURI_INVOKE_LOG__ ?? [];
    const entry = [...log].reverse().find((item) => item.cmd === "start_launch_job");
    return entry?.args?.request ?? null;
  });

  expect(request).not.toBeNull();
  expect(request?.terminalShell).toBe("wsl");
});

test("disables shell selection in Docker mode and does not send terminalShell", async ({
  page,
}) => {
  await openProjectAndSelectBranch(page, {
    list_worktree_branches: [branchMain, branchDevelop, branchFeature],
    list_remote_branches: [],
    list_worktrees: [],
    detect_agents: detectedAgents,
    get_available_shells: availableShells,
    detect_docker_context: {
      file_type: "compose",
      compose_services: ["app"],
      docker_available: true,
      compose_available: true,
      daemon_running: true,
      force_host: false,
      container_status: "running",
      images_exist: true,
      worktree_path: "/tmp/gwt-playwright",
    },
  });

  await page.getByRole("button", { name: "Launch Agent..." }).click();
  await expect(page.getByRole("dialog", { name: "Launch Agent" })).toBeVisible();

  await page.getByRole("button", { name: "Advanced" }).click();
  await expect(page.getByLabel("Shell")).toBeDisabled();
  await expect(page.getByText("Container default")).toBeVisible();

  await page.getByRole("button", { name: "Launch", exact: true }).click();
  await waitForInvokeCommand(page, "start_launch_job");

  const request = await page.evaluate(() => {
    const globalWindow = window as unknown as {
      __GWT_TAURI_INVOKE_LOG__?: Array<{
        cmd: string;
        args?: { request?: Record<string, unknown> };
      }>;
    };
    const log = globalWindow.__GWT_TAURI_INVOKE_LOG__ ?? [];
    const entry = [...log].reverse().find((item) => item.cmd === "start_launch_job");
    return entry?.args?.request ?? null;
  });

  expect(request).not.toBeNull();
  expect(request?.terminalShell).toBeUndefined();
});

test("saves Settings Terminal default shell via Terminal tab", async ({ page }) => {
  await openProjectAndSelectBranch(page, {
    list_worktree_branches: [branchMain, branchDevelop, branchFeature],
    list_remote_branches: [],
    list_worktrees: [],
    get_settings: settingsFixture,
    get_profiles: profilesFixture,
    get_available_shells: availableShells,
  });

  await waitForMenuActionListener(page);
  await page.evaluate(() => {
    const globalWindow = window as unknown as {
      __GWT_MOCK_EMIT_EVENT__?: (event: string, payload: unknown) => void;
    };
    globalWindow.__GWT_MOCK_EMIT_EVENT__?.("menu-action", {
      action: "open-settings",
    });
  });

  await expect(page.getByRole("heading", { name: "Settings" })).toBeVisible();
  await page.getByRole("button", { name: "Terminal", exact: true }).click();

  await expect(page.getByLabel("Default Shell")).toBeVisible();
  await expect(page.getByLabel("Default Shell")).toContainText("PowerShell (7.4.1)");

  await page.getByLabel("Default Shell").selectOption("wsl");
  await page.getByRole("button", { name: "Save" }).click();

  await waitForInvokeCommand(page, "save_settings");

  const savedSettings = await page.evaluate(() => {
    const globalWindow = window as unknown as {
      __GWT_TAURI_INVOKE_LOG__?: Array<{
        cmd: string;
        args?: { settings?: Record<string, unknown> };
      }>;
    };
    const log = globalWindow.__GWT_TAURI_INVOKE_LOG__ ?? [];
    const entry = [...log].reverse().find((item) => item.cmd === "save_settings");
    return entry?.args?.settings ?? null;
  });

  expect(savedSettings).not.toBeNull();
  expect(savedSettings?.default_shell).toBe("wsl");
});

test("saves UI and terminal font families from General and Terminal tabs", async ({
  page,
}) => {
  await openProjectAndSelectBranch(page, {
    list_worktree_branches: [branchMain, branchDevelop, branchFeature],
    list_remote_branches: [],
    list_worktrees: [],
    get_settings: settingsFixture,
    get_profiles: profilesFixture,
    get_available_shells: availableShells,
  });

  await openSettingsFromMenu(page);
  await expect(page.getByRole("button", { name: "General", exact: true })).toHaveClass(/active/);

  await page.getByLabel("UI font family").selectOption(UI_FONT_FAMILY_INTER);
  await page.getByRole("button", { name: "Terminal", exact: true }).click();
  await page
    .getByLabel("Terminal font family")
    .selectOption(TERMINAL_FONT_FAMILY_CASCADIA);

  const preview = await readFontFamilyPreview(page);
  expect(preview.ui).toBe(UI_FONT_FAMILY_INTER);
  expect(preview.terminal).toBe(TERMINAL_FONT_FAMILY_CASCADIA);
  expect(preview.terminalWindow).toBe(TERMINAL_FONT_FAMILY_CASCADIA);

  await page.getByRole("button", { name: "Save" }).click();
  await waitForInvokeCommand(page, "save_settings");

  const savedSettings = await page.evaluate(() => {
    const globalWindow = window as unknown as {
      __GWT_TAURI_INVOKE_LOG__?: Array<{
        cmd: string;
        args?: { settings?: Record<string, unknown> };
      }>;
    };
    const log = globalWindow.__GWT_TAURI_INVOKE_LOG__ ?? [];
    const entry = [...log].reverse().find((item) => item.cmd === "save_settings");
    return entry?.args?.settings ?? null;
  });

  expect(savedSettings).not.toBeNull();
  expect(savedSettings?.ui_font_family).toBe(UI_FONT_FAMILY_INTER);
  expect(savedSettings?.terminal_font_family).toBe(TERMINAL_FONT_FAMILY_CASCADIA);
});

test("restores font family preview on Close without saving", async ({ page }) => {
  await openProjectAndSelectBranch(page, {
    list_worktree_branches: [branchMain, branchDevelop, branchFeature],
    list_remote_branches: [],
    list_worktrees: [],
    get_settings: settingsFixture,
    get_profiles: profilesFixture,
    get_available_shells: availableShells,
  });

  await openSettingsFromMenu(page);

  await page.getByLabel("UI font family").selectOption(UI_FONT_FAMILY_INTER);
  await page.getByRole("button", { name: "Terminal", exact: true }).click();
  await page
    .getByLabel("Terminal font family")
    .selectOption(TERMINAL_FONT_FAMILY_CASCADIA);

  await page.locator(".settings-footer .btn-cancel", { hasText: "Close" }).click();
  await expect(page.getByRole("heading", { name: "Settings" })).toBeHidden();

  const preview = await readFontFamilyPreview(page);
  expect(preview.ui).toBe(DEFAULT_UI_FONT_FAMILY);
  expect(preview.terminal).toBe(DEFAULT_TERMINAL_FONT_FAMILY);
  expect(preview.terminalWindow).toBe(DEFAULT_TERMINAL_FONT_FAMILY);
});

test("opens a terminal from WorktreeSummaryPanel New Terminal button", async ({
  page,
}) => {
  await openProjectAndSelectBranch(page, {
    list_worktree_branches: [branchMain, branchDevelop, branchFeature],
    list_remote_branches: [],
    list_worktrees: [],
  });

  await page.getByTitle("New Terminal").click();
  await waitForInvokeCommand(page, "spawn_shell");

  const spawnArgs = await page.evaluate(() => {
    const globalWindow = window as unknown as {
      __GWT_TAURI_INVOKE_LOG__?: Array<{
        cmd: string;
        args?: Record<string, unknown>;
      }>;
    };
    const log = globalWindow.__GWT_TAURI_INVOKE_LOG__ ?? [];
    const entry = [...log].reverse().find((item) => item.cmd === "spawn_shell");
    return entry?.args ?? null;
  });

  expect(spawnArgs).not.toBeNull();
});
