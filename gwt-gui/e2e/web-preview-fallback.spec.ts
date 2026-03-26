import { expect, test } from "@playwright/test";
import { saveE2ECoverage } from "./support/helpers";

test.afterEach(async ({ page }, testInfo) => {
  await saveE2ECoverage(page, testInfo);
});

test("web preview without Tauri renders the open-project screen", async ({
  page,
}) => {
  await page.goto("/");

  await expect(page.getByRole("button", { name: "Open Project..." })).toBeVisible();
  await expect(page.getByRole("button", { name: "New Project" })).toBeVisible();
  await expect(page.getByText("Recent Projects")).toHaveCount(0);
});

test("web preview keyboard shortcut can open Settings without Tauri runtime", async ({
  page,
}) => {
  await page.goto("/");

  await page.evaluate(() => {
    (
      window as unknown as {
        __GWT_E2E_APP__?: { openSettingsTab: () => void };
      }
    ).__GWT_E2E_APP__?.openSettingsTab();
  });

  const state = await page.evaluate(() => {
    const app = (
      window as unknown as {
        __GWT_E2E_APP__?: {
          getActiveTabId: () => string;
          getTabs: () => Array<{ id: string; type: string; label: string }>;
        };
      }
    ).__GWT_E2E_APP__;
    return {
      activeTabId: app?.getActiveTabId() ?? null,
      tabs: app?.getTabs() ?? [],
    };
  });
  expect(state.activeTabId).toBe("settings");
  expect(state.tabs).toEqual(
    expect.arrayContaining([
      expect.objectContaining({ id: "settings", type: "settings" }),
    ]),
  );
});

test("web preview project-mode spec event can open an Issue Spec tab", async ({
  page,
}) => {
  await page.goto("/");

  await page.evaluate(() => {
    (
      window as unknown as {
        __GWT_E2E_APP__?: { openIssueSpecTab: (issueNumber: number) => void };
      }
    ).__GWT_E2E_APP__?.openIssueSpecTab(901);
  });

  const state = await page.evaluate(() => {
    const app = (
      window as unknown as {
        __GWT_E2E_APP__?: {
          getActiveTabId: () => string;
          getTabs: () => Array<{
            id: string;
            type: string;
            label: string;
            issueNumber?: number;
          }>;
        };
      }
    ).__GWT_E2E_APP__;
    return {
      activeTabId: app?.getActiveTabId() ?? null,
      tabs: app?.getTabs() ?? [],
    };
  });
  expect(state.activeTabId).toBe("issueSpec");
  expect(state.tabs).toEqual(
    expect.arrayContaining([
      expect.objectContaining({
        id: "issueSpec",
        type: "issueSpec",
        label: "Issue #901",
        issueNumber: 901,
      }),
    ]),
  );
});

test("web preview hook can open shell tabs without Tauri runtime", async ({
  page,
}) => {
  await page.goto("/");

  const state = await page.evaluate(() => {
    const app = (
      window as unknown as {
        __GWT_E2E_APP__?: {
          isTauriRuntimeAvailable: () => boolean;
          openIssuesTab: () => void;
          setIssueCount: (count: number) => void;
          openPullRequestsTab: () => void;
          openProjectIndexTab: () => void;
          openVersionHistoryTab: () => void;
          getTabs: () => Array<{ id: string; label: string; type: string }>;
          getActiveTabId: () => string;
        };
      }
    ).__GWT_E2E_APP__;
    if (!app) return null;
    app.openIssuesTab();
    app.setIssueCount(2);
    app.openPullRequestsTab();
    app.openProjectIndexTab();
    app.openVersionHistoryTab();
    return {
      tauri: app.isTauriRuntimeAvailable(),
      activeTabId: app.getActiveTabId(),
      tabs: app.getTabs(),
    };
  });

  expect(state).not.toBeNull();
  expect((state as any).tauri).toBe(false);
  expect((state as any).activeTabId).toBe("versionHistory");
  expect((state as any).tabs).toEqual(
    expect.arrayContaining([
      expect.objectContaining({ id: "issues", label: "Issues (2)" }),
      expect.objectContaining({ id: "prs", label: "Pull Requests" }),
      expect.objectContaining({ id: "projectIndex", label: "Project Index" }),
      expect.objectContaining({ id: "versionHistory", label: "Version History" }),
    ]),
  );
});
