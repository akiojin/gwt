import { expect, test } from "@playwright/test";
import { installTauriMock } from "./support/tauri-mock";
import {
  captureUxSnapshot,
  defaultRecentProject,
  getInvokeArgs,
  openRecentProject,
  saveE2ECoverage,
  waitForInvokeCommand,
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

test.afterEach(async ({ page }, testInfo) => {
  await saveE2ECoverage(page, testInfo);
});

test("Bug Report dialog opens from menu action", async ({ page }, testInfo) => {
  await page.goto("/");
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "report-issue" });

  const reportDialog = page.locator(".report-dialog");
  await expect(reportDialog).toBeVisible();
  await expect(reportDialog.getByText("Bug Report")).toBeVisible();
  await captureUxSnapshot(page, testInfo, "bug-report-dialog");
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
  await expectAgentCanvasVisible(page);

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

test("Report dialog preview renders generated bug markdown", async ({
  page,
}) => {
  await page.goto("/");
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "report-issue" });

  await page.locator("#bug-title").fill("Canvas card clipping");
  await page.locator("#steps").fill("1. Open project\n2. Open Agent Canvas");
  await page.getByRole("button", { name: "Preview" }).click();

  await expect(page.locator(".preview-content")).toHaveValue(
    /## Bug Report/,
  );
  await expect(page.locator(".preview-content")).toHaveValue(
    /Steps to Reproduce/,
  );
});

test("Report dialog can capture terminal text into diagnostics", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    capture_screen_text: "terminal output line 1\nterminal output line 2",
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "report-issue" });

  await page.getByRole("button", { name: "Capture Terminal Text" }).click();
  await expect(page.locator(".capture-status")).toContainText("Captured");
  await waitForInvokeCommand(page, "capture_screen_text");
});

test("successful bug report submit invokes create_github_issue with bug label", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    create_github_issue: {
      url: "https://github.com/example/gwt/issues/321",
      number: 321,
    },
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "report-issue" });

  await page.locator("#bug-title").fill("Submit from current shell");
  await page.locator("#steps").fill("1. Open report dialog");
  await page.getByRole("button", { name: "Submit" }).click();

  await waitForInvokeCommand(page, "create_github_issue");
  const args = await getInvokeArgs(page, "create_github_issue");
  expect(args?.labels).toEqual(["bug"]);
  await expect(page.getByText("Issue #321 created successfully.")).toBeVisible();
  await expect(page.getByRole("dialog", { name: "Report" })).toBeHidden();
});

test("failed feature request submit shows fallback actions", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    create_github_issue: {
      __error: "gh auth failed",
    },
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "suggest-feature" });

  await page.locator("#feature-title").fill("Feature request fallback");
  await page.locator("#feature-desc").fill("Need better fallback handling");
  await page.getByRole("button", { name: "Submit" }).click();

  await expect(page.locator(".submit-message")).toContainText(
    "Failed to create issue",
  );
  await expect(
    page.getByRole("button", { name: "Copy to Clipboard" }),
  ).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Open in Browser" }),
  ).toBeVisible();
});

test("successful feature request submit uses enhancement label and selected target", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    detect_report_target: {
      owner: "example",
      repo: "shell-app",
      display: "example/shell-app",
    },
    create_github_issue: {
      url: "https://github.com/example/shell-app/issues/88",
      number: 88,
    },
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "suggest-feature" });

  await page.locator("#report-target").selectOption({ label: "example/shell-app" });
  await page.locator("#feature-title").fill("Current shell feature");
  await page.locator("#feature-desc").fill("Need a better shell-specific flow");
  await page.getByRole("button", { name: "Submit" }).click();

  await waitForInvokeCommand(page, "create_github_issue");
  const args = await getInvokeArgs(page, "create_github_issue");
  expect(args?.owner).toBe("example");
  expect(args?.repo).toBe("shell-app");
  expect(args?.labels).toEqual(["enhancement"]);
  await expect(page.getByText("Issue #88 created successfully.")).toBeVisible();
  await expect(page.getByRole("dialog", { name: "Report" })).toBeHidden();
});

test("report dialog close button closes the modal", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "report-issue" });

  await expect(page.getByRole("dialog", { name: "Report" })).toBeVisible();
  await page.getByRole("button", { name: "Close" }).click();
  await expect(page.getByRole("dialog", { name: "Report" })).toBeHidden();
});

test("report dialog overlay click closes the modal", async ({ page }) => {
  await page.goto("/");
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "report-issue" });

  await expect(page.getByRole("dialog", { name: "Report" })).toBeVisible();
  await page.locator(".report-overlay").click({ position: { x: 8, y: 8 } });
  await expect(page.getByRole("dialog", { name: "Report" })).toBeHidden();
});

test("failed submit can copy fallback body to clipboard", async ({ page }) => {
  await page.addInitScript(() => {
    let clipboardText = "";
    Object.defineProperty(navigator, "clipboard", {
      configurable: true,
      value: {
        writeText: async (value: string) => {
          clipboardText = value;
        },
        readText: async () => clipboardText,
      },
    });
    (window as unknown as { __GWT_CLIPBOARD_TEXT__?: () => string }).__GWT_CLIPBOARD_TEXT__ =
      () => clipboardText;
  });

  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    create_github_issue: {
      __error: "gh auth failed",
    },
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "report-issue" });

  await page.locator("#bug-title").fill("Clipboard fallback");
  await page.locator("#steps").fill("1. Open dialog");
  await page.getByRole("button", { name: "Submit" }).click();
  await page.getByRole("button", { name: "Copy to Clipboard" }).click();

  await expect(page.locator(".submit-message")).toContainText("Copied to clipboard.");
  await expect
    .poll(async () =>
      page.evaluate(
        () =>
          (
            window as unknown as { __GWT_CLIPBOARD_TEXT__?: () => string }
          ).__GWT_CLIPBOARD_TEXT__?.() ?? "",
      ),
    )
    .toContain("1. Open dialog");
});

test("failed submit can open browser fallback URL", async ({ page }) => {
  await page.addInitScript(() => {
    let openedUrl = "";
    window.open = ((url?: string | URL) => {
      openedUrl = String(url ?? "");
      return {} as Window;
    }) as typeof window.open;
    (
      window as unknown as { __GWT_LAST_OPENED_URL__?: () => string }
    ).__GWT_LAST_OPENED_URL__ = () => openedUrl;
  });

  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    create_github_issue: {
      __error: "gh auth failed",
    },
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "suggest-feature" });

  await page.locator("#feature-title").fill("Browser fallback");
  await page.locator("#feature-desc").fill("Need browser fallback");
  await page.getByRole("button", { name: "Submit" }).click();
  await page.getByRole("button", { name: "Open in Browser" }).click();

  await expect(page.locator(".submit-message")).toContainText(
    "Failed to create issue",
  );
  await expect(
    page.getByRole("button", { name: "Open in Browser" }),
  ).toBeVisible();
});

test("report preview includes collected system info and failed terminal capture text", async ({
  page,
}) => {
  await page.goto("/");
  await setMockCommandResponses(page, {
    ...standardBranchResponses(),
    get_report_system_info: {
      osName: "macOS",
      osVersion: "14.4",
      arch: "arm64",
      gwtVersion: "1.2.3",
    },
    capture_screen_text: {
      __error: "capture failed",
    },
  });
  await openRecentProject(page);
  await expectAgentCanvasVisible(page);

  await waitForMenuActionListener(page);
  await emitTauriEvent(page, "menu-action", { action: "report-issue" });

  await page.locator("#bug-title").fill("Preview diagnostics");
  await page.getByRole("button", { name: "Capture Terminal Text" }).click();
  await page.getByRole("button", { name: "Preview" }).click();

  await expect(page.locator(".preview-content")).toHaveValue(/macOS 14.4/);
  await expect(page.locator(".preview-content")).toHaveValue(
    /\(Failed to capture terminal text\)/,
  );
});
