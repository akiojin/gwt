import { expect, test } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";
import {
  defaultRecentProject,
  branchMain,
  branchDevelop,
  branchFeature,
  openRecentProject,
  setMockCommandResponses,
  detectedAgents,
  standardBranchResponses,
  waitForInvokeCommand,
  waitForMenuActionListener,
  emitTauriEvent,
  getInvokeLog,
} from "./support/helpers";

test.beforeEach(async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
    },
  });
});

test("Launch Agent dialog opens from branch detail", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    detect_agents: detectedAgents,
    list_agent_versions: {
      agentId: "codex",
      package: "codex",
      tags: ["latest"],
      versions: ["0.99.0"],
      source: "cache",
    },
  });
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();

  await page
    .locator(".worktree-summary-panel")
    .getByRole("button", { name: "Launch Agent..." })
    .click();

  await expect(
    page.getByRole("dialog", { name: "Launch Agent" }),
  ).toBeVisible();
});

test("Launch Agent dialog shows detected agent in selector", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    detect_agents: detectedAgents,
    list_agent_versions: {
      agentId: "codex",
      package: "codex",
      tags: ["latest"],
      versions: ["0.99.0"],
      source: "cache",
    },
  });
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();
  await page
    .locator(".worktree-summary-panel")
    .getByRole("button", { name: "Launch Agent..." })
    .click();

  await expect(
    page.locator("select#agent-select"),
  ).toHaveValue("codex");
});

test("Launch Agent invokes start_launch_job", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    detect_agents: detectedAgents,
    list_agent_versions: {
      agentId: "codex",
      package: "codex",
      tags: ["latest"],
      versions: ["0.99.0"],
      source: "cache",
    },
  });
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();
  await page
    .locator(".worktree-summary-panel")
    .getByRole("button", { name: "Launch Agent..." })
    .click();

  await page
    .getByRole("dialog", { name: "Launch Agent" })
    .getByRole("button", { name: "Launch", exact: true })
    .click();

  await waitForInvokeCommand(page, "start_launch_job");
});

test("agent terminal tab appears after launch", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    detect_agents: detectedAgents,
    list_agent_versions: {
      agentId: "codex",
      package: "codex",
      tags: ["latest"],
      versions: ["0.99.0"],
      source: "cache",
    },
  });
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();
  await page
    .locator(".worktree-summary-panel")
    .getByRole("button", { name: "Launch Agent..." })
    .click();
  await page
    .getByRole("dialog", { name: "Launch Agent" })
    .getByRole("button", { name: "Launch", exact: true })
    .click();

  await expect(page.locator(".tab.active .tab-label")).toHaveText(
    branchFeature.name,
  );
});

test("terminal container is visible with xterm after launch", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    detect_agents: detectedAgents,
    list_agent_versions: {
      agentId: "codex",
      package: "codex",
      tags: ["latest"],
      versions: ["0.99.0"],
      source: "cache",
    },
  });
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();
  await page
    .locator(".worktree-summary-panel")
    .getByRole("button", { name: "Launch Agent..." })
    .click();
  await page
    .getByRole("dialog", { name: "Launch Agent" })
    .getByRole("button", { name: "Launch", exact: true })
    .click();

  const termContainer = page.locator(
    ".terminal-wrapper.active .terminal-container",
  );
  await expect(termContainer).toBeVisible();
  await expect(termContainer.locator(".xterm")).toBeVisible();
});

test("new terminal opens from menu action", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  await expect(
    page.getByPlaceholder("Type a task and press Enter..."),
  ).toBeVisible();

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });

  await waitForInvokeCommand(page, "spawn_shell");
});

test("multiple terminal tabs can be opened", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  await expect(
    page.getByPlaceholder("Type a task and press Enter..."),
  ).toBeVisible();

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });

  await expect
    .poll(async () => page.locator(".tab .tab-dot.terminal").count())
    .toBe(2);
});

test("terminal output event updates terminal buffer", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    detect_agents: detectedAgents,
    list_agent_versions: {
      agentId: "codex",
      package: "codex",
      tags: ["latest"],
      versions: ["0.99.0"],
      source: "cache",
    },
  });
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();
  await page
    .locator(".worktree-summary-panel")
    .getByRole("button", { name: "Launch Agent..." })
    .click();
  await page
    .getByRole("dialog", { name: "Launch Agent" })
    .getByRole("button", { name: "Launch", exact: true })
    .click();

  // Wait for terminal to be ready
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
            entry.args?.event === "terminal-output",
        );
      });
    })
    .toBe(true);

  const termContainer = page.locator(
    ".terminal-wrapper.active .terminal-container",
  );
  const paneId = await termContainer.getAttribute("data-pane-id");
  expect(paneId).toBeTruthy();

  // Emit terminal output
  const bytes = [72, 101, 108, 108, 111, 13, 10]; // "Hello\r\n"
  await emitTauriEvent(page, "terminal-output", {
    pane_id: paneId,
    data: bytes,
  });

  // Buffer should update
  await expect
    .poll(async () => {
      return page.evaluate(
        ({ targetPaneId }) => {
          const container = document.querySelector(
            `.terminal-wrapper.active .terminal-container[data-pane-id="${targetPaneId}"]`,
          ) as
            | (HTMLElement & {
                __gwtTerminal?: {
                  buffer?: {
                    active?: { length?: number };
                  };
                };
              })
            | null;
          return container?.__gwtTerminal?.buffer?.active?.length ?? 0;
        },
        { targetPaneId: paneId },
      );
    })
    .toBeGreaterThan(0);
});

test("terminal stream error shows error message", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  await expect(
    page.getByPlaceholder("Type a task and press Enter..."),
  ).toBeVisible();

  await waitForMenuActionListener(page);

  // Set next spawn to error
  await page.evaluate(() => {
    const globalWindow = window as unknown as {
      __GWT_MOCK_SET_NEXT_SPAWN_ERROR__?: (enabled: boolean) => void;
      __GWT_MOCK_EMIT_EVENT__?: (event: string, payload: unknown) => void;
    };
    globalWindow.__GWT_MOCK_SET_NEXT_SPAWN_ERROR__?.(true);
    globalWindow.__GWT_MOCK_EMIT_EVENT__?.("menu-action", {
      action: "new-terminal",
    });
  });

  await waitForInvokeCommand(page, "spawn_shell");

  const terminalState = await page.evaluate(async () => {
    const globalWindow = window as unknown as {
      __TAURI_INTERNALS__?: {
        invoke: (cmd: string, args?: unknown) => Promise<unknown>;
      };
      __GWT_MOCK_LAST_SPAWNED_PANE_ID__?: () => string | null;
    };

    const paneId = globalWindow.__GWT_MOCK_LAST_SPAWNED_PANE_ID__?.() ?? "";
    if (!paneId || !globalWindow.__TAURI_INTERNALS__) {
      return { paneId, scrollback: "" };
    }

    const scrollbackBytes = (await globalWindow.__TAURI_INTERNALS__.invoke(
      "terminal_ready",
      { paneId, maxBytes: 64 * 1024 },
    )) as number[];
    const scrollback = new TextDecoder().decode(
      new Uint8Array(scrollbackBytes),
    );

    return { paneId, scrollback };
  });

  expect(terminalState.scrollback).toContain(
    "PTY stream error: mocked read failure",
  );
  expect(terminalState.scrollback).toContain("Press Enter to close this tab.");
});

test("error terminal closes on Enter key", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  await expect(
    page.getByPlaceholder("Type a task and press Enter..."),
  ).toBeVisible();

  await waitForMenuActionListener(page);

  await page.evaluate(() => {
    const globalWindow = window as unknown as {
      __GWT_MOCK_SET_NEXT_SPAWN_ERROR__?: (enabled: boolean) => void;
      __GWT_MOCK_EMIT_EVENT__?: (event: string, payload: unknown) => void;
    };
    globalWindow.__GWT_MOCK_SET_NEXT_SPAWN_ERROR__?.(true);
    globalWindow.__GWT_MOCK_EMIT_EVENT__?.("menu-action", {
      action: "new-terminal",
    });
  });

  await waitForInvokeCommand(page, "spawn_shell");

  const terminalsAfterClose = await page.evaluate(async () => {
    const globalWindow = window as unknown as {
      __TAURI_INTERNALS__?: {
        invoke: (cmd: string, args?: unknown) => Promise<unknown>;
      };
      __GWT_MOCK_LAST_SPAWNED_PANE_ID__?: () => string | null;
    };

    const paneId = globalWindow.__GWT_MOCK_LAST_SPAWNED_PANE_ID__?.() ?? "";
    if (!paneId || !globalWindow.__TAURI_INTERNALS__) return [];

    // Trigger terminal_ready to get scrollback
    await globalWindow.__TAURI_INTERNALS__.invoke("terminal_ready", {
      paneId,
      maxBytes: 64 * 1024,
    });

    // Send Enter to close
    await globalWindow.__TAURI_INTERNALS__.invoke("write_terminal", {
      paneId,
      data: [13],
    });

    return (await globalWindow.__TAURI_INTERNALS__.invoke(
      "list_terminals",
      {},
    )) as Array<{ pane_id: string }>;
  });

  // Terminal should have been removed
  expect(terminalsAfterClose.length).toBe(0);
});

test("terminal-output listener is registered after agent launch", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    detect_agents: detectedAgents,
    list_agent_versions: {
      agentId: "codex",
      package: "codex",
      tags: ["latest"],
      versions: ["0.99.0"],
      source: "cache",
    },
  });
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();
  await page
    .locator(".worktree-summary-panel")
    .getByRole("button", { name: "Launch Agent..." })
    .click();
  await page
    .getByRole("dialog", { name: "Launch Agent" })
    .getByRole("button", { name: "Launch", exact: true })
    .click();

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
            entry.args?.event === "terminal-output",
        );
      });
    })
    .toBe(true);
});

test("Launch Agent dialog has Advanced section", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    detect_agents: detectedAgents,
    list_agent_versions: {
      agentId: "codex",
      package: "codex",
      tags: ["latest"],
      versions: ["0.99.0"],
      source: "cache",
    },
  });
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();
  await page
    .locator(".worktree-summary-panel")
    .getByRole("button", { name: "Launch Agent..." })
    .click();

  const dialog = page.getByRole("dialog", { name: "Launch Agent" });
  await expect(
    dialog.getByRole("button", { name: "Advanced" }),
  ).toBeVisible();
});

test("Launch Agent dialog close button dismisses dialog", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    detect_agents: detectedAgents,
    list_agent_versions: {
      agentId: "codex",
      package: "codex",
      tags: ["latest"],
      versions: ["0.99.0"],
      source: "cache",
    },
  });
  await openRecentProject(page);

  await page
    .locator(".branch-item")
    .filter({ hasText: branchFeature.name })
    .click();
  await page
    .locator(".worktree-summary-panel")
    .getByRole("button", { name: "Launch Agent..." })
    .click();

  const dialog = page.getByRole("dialog", { name: "Launch Agent" });
  await expect(dialog).toBeVisible();

  await dialog.getByRole("button", { name: "Cancel" }).click();
  await expect(dialog).toBeHidden();
});

test("tab switch between terminal and project mode", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  await expect(
    page.getByPlaceholder("Type a task and press Enter..."),
  ).toBeVisible();

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "new-terminal" });

  await waitForInvokeCommand(page, "spawn_shell");

  // Should now have a terminal tab
  await expect(page.locator(".tab .tab-dot.terminal")).toBeVisible();

  // Click project mode tab to switch back
  const projectModeTab = page.locator(".tab", { hasText: "Project Mode" });
  if (await projectModeTab.isVisible()) {
    await projectModeTab.click();
    await expect(
      page.getByPlaceholder("Type a task and press Enter..."),
    ).toBeVisible();
  }
});
