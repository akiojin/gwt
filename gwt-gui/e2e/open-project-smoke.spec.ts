import { expect, test } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";

test.beforeEach(async ({ page }) => {
  await installTauriMock(page);
});

test("launches and completes open-project -> agent send smoke flow", async ({
  page,
}) => {
  await page.goto("/");

  await expect(
    page.getByRole("button", { name: "Open Project..." }),
  ).toBeVisible();
  const recentItem = page.locator("button.recent-item").first();
  await expect(recentItem).toBeVisible();

  await recentItem.click();

  const prompt = page.getByPlaceholder("Type a task and press Enter...");
  await expect(prompt).toBeVisible();

  const message = "smoke message";
  await prompt.fill(message);
  await page.getByRole("button", { name: "Send" }).click();

  await expect(page.getByText(`Echo: ${message}`)).toBeVisible();

  const invokeCommands = await page.evaluate(() => {
    const globalWindow = window as unknown as {
      __GWT_TAURI_INVOKE_LOG__?: Array<{ cmd: string }>;
    };
    return (globalWindow.__GWT_TAURI_INVOKE_LOG__ ?? []).map(
      (entry) => entry.cmd,
    );
  });

  expect(invokeCommands).toContain("open_project");
  expect(invokeCommands).toContain("send_agent_mode_message");
});

test("shows terminal stream error and closes errored terminal tab on Enter", async ({
  page,
}) => {
  await page.goto("/");

  await expect(
    page.getByRole("button", { name: "Open Project..." }),
  ).toBeVisible();
  await page.locator("button.recent-item").first().click();
  await expect(
    page.getByPlaceholder("Type a task and press Enter..."),
  ).toBeVisible();

  await expect
    .poll(async () => {
      return page.evaluate(() => {
        const raw = window.localStorage.getItem("gwt.projectTabs.v2");
        if (!raw) return false;
        try {
          const parsed = JSON.parse(raw) as {
            byProjectPath?: Record<string, { activeTabId?: string | null }>;
          };
          return (
            parsed.byProjectPath?.["/tmp/gwt-playwright"]?.activeTabId ===
            "agentMode"
          );
        } catch {
          return false;
        }
      });
    })
    .toBe(true);

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

  await expect
    .poll(async () => {
      return page.evaluate(() => {
        const globalWindow = window as unknown as {
          __GWT_TAURI_INVOKE_LOG__?: Array<{ cmd: string }>;
        };
        return (globalWindow.__GWT_TAURI_INVOKE_LOG__ ?? []).some(
          (entry) => entry.cmd === "spawn_shell",
        );
      });
    })
    .toBe(true);

  const terminalState = await page.evaluate(async () => {
    const globalWindow = window as unknown as {
      __TAURI_INTERNALS__?: {
        invoke: (
          cmd: string,
          args?: unknown,
          options?: unknown,
        ) => Promise<unknown>;
      };
      __GWT_MOCK_LAST_SPAWNED_PANE_ID__?: () => string | null;
    };

    const paneId = globalWindow.__GWT_MOCK_LAST_SPAWNED_PANE_ID__?.() ?? "";
    if (!paneId || !globalWindow.__TAURI_INTERNALS__) {
      return {
        paneId,
        scrollback: "",
        terminalsAfterClose: [] as Array<{ pane_id: string }>,
      };
    }

    const scrollback = (await globalWindow.__TAURI_INTERNALS__.invoke(
      "capture_scrollback_tail",
      {
        paneId,
        maxBytes: 64 * 1024,
      },
    )) as string;

    await globalWindow.__TAURI_INTERNALS__.invoke("write_terminal", {
      paneId,
      data: [13],
    });

    const terminalsAfterClose = (await globalWindow.__TAURI_INTERNALS__.invoke(
      "list_terminals",
      {},
    )) as Array<{ pane_id: string }>;

    return { paneId, scrollback, terminalsAfterClose };
  });

  expect(terminalState.paneId.length).toBeGreaterThan(0);
  expect(terminalState.scrollback).toContain(
    "PTY stream error: mocked read failure",
  );
  expect(terminalState.scrollback).toContain("Press Enter to close this tab.");
  expect(
    terminalState.terminalsAfterClose.some(
      (term) => term.pane_id === terminalState.paneId,
    ),
  ).toBe(false);

  const invokeCommands = await page.evaluate(() => {
    const globalWindow = window as unknown as {
      __GWT_TAURI_INVOKE_LOG__?: Array<{ cmd: string }>;
    };
    return (globalWindow.__GWT_TAURI_INVOKE_LOG__ ?? []).map(
      (entry) => entry.cmd,
    );
  });

  expect(invokeCommands).toContain("spawn_shell");
  expect(invokeCommands).toContain("write_terminal");
  expect(invokeCommands).toContain("capture_scrollback_tail");
});
