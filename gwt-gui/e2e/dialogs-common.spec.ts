import { expect, test } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";
import {
  defaultRecentProject,
  openRecentProject,
  waitForMenuActionListener,
  emitTauriEvent,
  standardBranchResponses,
  setMockCommandResponses,
  expectAgentCanvasVisible,
} from "./support/helpers";

test.beforeEach(async ({ page }) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
    },
  });
});

test("Bug Report dialog opens from menu action", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "report-issue" });

  const reportDialog = page.locator(".report-dialog");
  await expect(reportDialog).toBeVisible();
  await expect(reportDialog.getByText("Bug Report")).toBeVisible();
});

test("Bug Report dialog has title input", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "report-issue" });

  await expect(page.locator("#bug-title")).toBeVisible();
});

test("Bug Report dialog has steps field", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "report-issue" });

  await expect(page.locator("#steps")).toBeVisible();
});

test("Feature Request dialog opens from menu action", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "suggest-feature" });

  const reportDialog = page.locator(".report-dialog");
  await expect(reportDialog).toBeVisible();
  await expect(reportDialog.getByText("Feature Request")).toBeVisible();
});

test("Feature Request dialog has description field", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "suggest-feature" });

  await expect(page.locator("#feature-desc")).toBeVisible();
});

test("Bug Report dialog covers most of viewport height", async ({
  page,
}) => {
  await page.goto("/");
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "report-issue" });

  const reportDialog = page.locator(".report-dialog");
  await expect(reportDialog).toBeVisible();

  const viewport = await reportDialog.evaluate((dialog) => {
    const rect = dialog.getBoundingClientRect();
    return {
      heightPx: rect.height,
      viewportHeightPx: window.innerHeight,
    };
  });

  expect(viewport.heightPx).toBeGreaterThanOrEqual(
    viewport.viewportHeightPx * 0.88,
  );
});

test("Bug Report form text has readable font size", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "report-issue" });

  const reportDialog = page.locator(".report-dialog");
  const typography = await reportDialog.evaluate((dialog) => {
    const titleLabel = dialog.querySelector<HTMLLabelElement>(
      "label[for='bug-title']",
    );
    const titleInput = dialog.querySelector<HTMLInputElement>("#bug-title");
    if (!titleLabel || !titleInput) return null;
    return {
      labelFontSizePx: parseFloat(getComputedStyle(titleLabel).fontSize),
      inputFontSizePx: parseFloat(getComputedStyle(titleInput).fontSize),
    };
  });

  expect(typography).not.toBeNull();
  expect(typography?.labelFontSizePx ?? 0).toBeGreaterThanOrEqual(13);
  expect(typography?.inputFontSizePx ?? 0).toBeGreaterThanOrEqual(14);
});

test("About dialog opens from menu action", async ({ page }) => {
  await page.goto("/");
  await setMockCommandResponses(page, standardBranchResponses());
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "about" });

  // About dialog should show
  await expect(page.getByRole("heading", { name: "gwt" })).toBeVisible();
});

test("switching between Bug Report and Feature Request tabs", async ({
  page,
}) => {
  await page.goto("/");
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);

  // Open Bug Report first
  await emitTauriEvent(page, "menu-action", { action: "report-issue" });
  const reportDialog = page.locator(".report-dialog");
  await expect(reportDialog.getByText("Bug Report")).toBeVisible();

  // Switch to Feature Request
  await emitTauriEvent(page, "menu-action", {
    action: "suggest-feature",
  });
  await expect(reportDialog.getByText("Feature Request")).toBeVisible();
});

test("Feature Request dialog form has readable font sizes", async ({
  page,
}) => {
  await page.goto("/");
  await openRecentProject(page);
  await expect(
    page.getByPlaceholder("Type a task and press Enter..."),
  ).toBeVisible();

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "suggest-feature" });

  const reportDialog = page.locator(".report-dialog");
  const typography = await reportDialog.evaluate((dialog) => {
    const descLabel = dialog.querySelector<HTMLLabelElement>(
      "label[for='feature-desc']",
    );
    const descTextArea =
      dialog.querySelector<HTMLTextAreaElement>("#feature-desc");
    if (!descLabel || !descTextArea) return null;
    return {
      labelFontSizePx: parseFloat(getComputedStyle(descLabel).fontSize),
      textareaFontSizePx: parseFloat(
        getComputedStyle(descTextArea).fontSize,
      ),
    };
  });

  expect(typography).not.toBeNull();
  expect(typography?.labelFontSizePx ?? 0).toBeGreaterThanOrEqual(13);
  expect(typography?.textareaFontSizePx ?? 0).toBeGreaterThanOrEqual(14);
});

test("Report dialog stays above migration modal when both are open", async ({
  page,
}) => {
  await installTauriMock(page, {
    commandResponses: {
      get_recent_projects: [defaultRecentProject],
      probe_path: {
        kind: "migrationRequired",
        migrationSourceRoot: "/tmp/gwt-playwright",
      },
    },
  });

  await page.goto("/");
  await openRecentProject(page);
  await expect(
    page.getByRole("dialog", { name: "Migration Required" }),
  ).toBeVisible();

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "report-issue" });

  const reportDialog = page.getByRole("dialog", { name: "Report" });
  const migrationDialog = page.getByRole("dialog", {
    name: "Migration Required",
  });
  await expect(reportDialog).toBeVisible();
  await expect(migrationDialog).toBeVisible();

  const layering = await page.evaluate(() => {
    const report = document.querySelector<HTMLElement>(
      "[role='dialog'][aria-label='Report']",
    );
    const migration = document.querySelector<HTMLElement>(
      "[role='dialog'][aria-label='Migration Required']",
    );
    if (!report || !migration) {
      return { reportZ: -1, migrationZ: -1, topLayerDialog: null };
    }

    const reportRect = report.getBoundingClientRect();
    const probeX = reportRect.left + reportRect.width / 2;
    const probeY = reportRect.top + 20;
    const topAtPoint = document.elementFromPoint(probeX, probeY);
    const topLayerDialog = topAtPoint
      ?.closest("[role='dialog']")
      ?.getAttribute("aria-label");

    return {
      reportZ: Number.parseInt(getComputedStyle(report).zIndex || "0", 10),
      migrationZ: Number.parseInt(
        getComputedStyle(migration).zIndex || "0",
        10,
      ),
      topLayerDialog: topLayerDialog ?? null,
    };
  });

  expect(layering.reportZ).toBeGreaterThan(layering.migrationZ);
  expect(layering.topLayerDialog).toBe("Report");

  const title = page.locator("#bug-title");
  await title.click();
  await expect(title).toBeFocused();
});
