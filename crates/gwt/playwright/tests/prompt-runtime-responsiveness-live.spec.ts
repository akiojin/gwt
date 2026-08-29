/**
 * Issue #3777 AC-5/AC-6/AC-7 — real-backend interaction liveness while a
 * UserPromptSubmit RuntimeHook and repository-scale Work refresh are active.
 *
 * This suite intentionally does not synthesize Board or WorkItems bytes. The
 * fixture owns formats and locks that production also consumes, so the
 * browser-check launcher must seed them in its isolated HOME and provide a
 * trigger which exercises the real hook/refresh routes. Missing or undersized
 * fixtures are setup errors, never skips.
 */
import {
  execFile,
  spawn,
  type ChildProcessWithoutNullStreams,
} from "node:child_process";
import { createHash } from "node:crypto";
import { constants as fsConstants } from "node:fs";
import {
  access,
  mkdtemp,
  mkdir,
  readFile,
  realpath,
  rm,
  stat,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, isAbsolute, join, relative } from "node:path";
import { promisify } from "node:util";
import { expect, test, type Page } from "@playwright/test";
import {
  acquireLiveGwtBackendLock,
  clearLiveLaunchWizard,
  gotoLiveGwt,
  openLiveGwtProject,
  sendLiveGwtEvent,
} from "./_helpers/live-gwt";

const execFileAsync = promisify(execFile);
const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const BOARD_MIN_BYTES = 8 * 1024 * 1024;
const WORK_ITEMS_MIN_BYTES = 155 * 1024 * 1024;
const INTERACTION_BUDGET_MS = 1_000;
const LONG_TASK_BUDGET_MS = 100;
const RAF_GAP_BUDGET_MS = 250;
const START_HOOK_MARKER = "GWT_RESPONSIVENESS_START_HOOK";
const HOOK_STARTED_MARKER = "GWT_RESPONSIVENESS_HOOK_STARTED";
const HOOK_COMPLETED_MARKER = "GWT_RESPONSIVENESS_HOOK_COMPLETED";
const LOAD_STARTED_MARKER = "GWT_RESPONSIVENESS_LOAD_STARTED";
const INTERACTIONS_COMPLETE_MARKER = "GWT_RESPONSIVENESS_INTERACTIONS_COMPLETE";
const DEFAULT_STRESS_WORK_ID = "work-issue-3777-stress-0000";

// The browser-check fixture trigger is a line-oriented rendezvous, not a
// generic command hook. It must start the real `gwtd hook event
// UserPromptSubmit` path and one real Work refresh against the isolated
// backend and emit HOOK_STARTED_MARKER from the actual hook process lifetime,
// followed by LOAD_STARTED_MARKER once the Work fixture is installed. The test
// then opens a second same-origin fixture project, which starts the real
// repository-scale Work ingest/refresh route, before measuring interactions.
// The hook subprocess records its exit directly in an independent rendezvous
// file. The trigger itself remains joined until this test writes
// INTERACTIONS_COMPLETE_MARKER to stdin; exit 0 means the hook completed and
// every other exit is test failure.

type StressFixture = {
  boardPath: string;
  capabilityAgentId: string;
  checkHome: string;
  controlIssueNumber: number;
  hookProfilePath: string;
  issueNumber: number;
  projectRoot: string;
  gwtdPath: string;
  stressWorkId: string;
  triggerPath: string;
  workItemsPath: string;
};

type TraceEntry = {
  kind?: string;
  duration_ms?: number;
  gap_ms?: number;
};

type RunningLoadTrigger = {
  child: ChildProcessWithoutNullStreams;
  finish: () => Promise<void>;
  startHook: () => Promise<{
    finish: () => Promise<void>;
    hasCompleted: () => Promise<boolean>;
  }>;
  stop: () => Promise<void>;
};

type RefreshProject = {
  root: string;
  storeRoot: string;
  workItemsTargetPath: string;
};

type ActiveWorkRendezvous = {
  hasCompleted: () => Promise<boolean>;
  stop: () => Promise<void>;
  waitForCompleted: () => Promise<void>;
  waitForStarted: () => Promise<void>;
};

test.describe.serial("Issue #3777 prompt/runtime responsiveness (live backend)", () => {
  test.skip(!BASE, "GWT_PLAYWRIGHT_BASE_URL is not set; live E2E skipped");
  // Fixture setup launches a real agent and lets fresh-home semantic indexes
  // finish; only the interaction and hook budgets below are acceptance gates.
  test.setTimeout(600_000);

  test("tab, Issue identity, and terminal roundtrip stay live under hook and Work refresh load", async ({
    page,
  }, testInfo) => {
    const fixture = await requireStressFixture();
    const releaseBackendLock = await acquireLiveGwtBackendLock(BASE, testInfo);
    const refreshProjects: RefreshProject[] = [];
    const refreshTabIds = new Set<string>();
    const consoleErrors: string[] = [];
    const pageErrors: string[] = [];
    let trigger: RunningLoadTrigger | undefined;
    let shellWindowId: string | undefined;
    let issueWindowId: string | undefined;
    let refreshTabId: string | undefined;
    let refreshRendezvous: ActiveWorkRendezvous | undefined;

    page.on("console", (message) => {
      if (message.type() === "error") {
        const location = message.location();
        consoleErrors.push(
          `${message.text()} @ ${location.url || "<unknown>"}:${location.lineNumber}`,
        );
      }
    });
    page.on("pageerror", (error) => pageErrors.push(String(error)));

    try {
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await clearLiveLaunchWizard(page);
      await clearLiveMigrationModal(page);
      await selectTheme(page, testInfo.project.name.includes("light") ? "light" : "dark");
      await openLiveGwtProject(page, fixture.projectRoot);
      const primaryTabId = await projectTabIdForRoot(page, fixture.projectRoot);
      await activateProjectTab(page, primaryTabId);
      await closeOtherProjectTabs(page, primaryTabId);

      issueWindowId = await createWindow(page, "issue");
      const issueSurface = page.locator(
        `.workspace-window[data-id="${issueWindowId}"][data-preset="issue"]`,
      );
      const issueRow = issueSurface.locator(
        `.knowledge-row[data-issue-number="${fixture.issueNumber}"]`,
      );
      const controlIssueRow = issueSurface.locator(
        `.knowledge-row[data-issue-number="${fixture.controlIssueNumber}"]`,
      );
      try {
        await expect(issueRow).toBeVisible({ timeout: 20_000 });
        await expect(controlIssueRow).toBeVisible({ timeout: 20_000 });
      } catch (error) {
        const snapshot = await issueSurface.evaluateAll((surfaces) =>
          surfaces.map((surface) => ({
            heading: surface.querySelector(".knowledge-heading")?.textContent ?? "",
            status: surface.querySelector(".knowledge-status")?.textContent ?? "",
            issues: Array.from(surface.querySelectorAll(".knowledge-row")).map(
              (row) => (row as HTMLElement).dataset.issueNumber ?? "",
            ),
          })),
        );
        throw new Error(`${String(error)}\nKnowledge snapshot: ${JSON.stringify(snapshot)}`);
      }
      await selectIssueAndWait(page, issueWindowId, fixture.controlIssueNumber);

      shellWindowId = await createWindow(page, "shell");
      const shellWindow = page.locator(
        `.workspace-window[data-id="${shellWindowId}"][data-preset="shell"]`,
      );
      await expect(shellWindow).toBeVisible({ timeout: 20_000 });
      await expect(shellWindow).toHaveAttribute("data-agent-state", "running", {
        timeout: 20_000,
      });
      await launchHookCapabilityAgent(
        page,
        await currentBranchName(fixture.projectRoot),
        fixture.capabilityAgentId,
        fixture.checkHome,
      );

      const hookProfileCursor = (await readUserPromptSubmitProfiles(
        fixture.hookProfilePath,
      )).length;
      const traceEntries: TraceEntry[] = [];
      let exactProjectionValidated = false;
      const runLoadedInteraction = async <T,>(
        preparePrimary: boolean,
        measure: () => Promise<T>,
        prepareInteraction?: () => Promise<void>,
      ): Promise<T> => {
        let refreshProject = refreshProjects[0];
        if (!refreshProject) {
          refreshProject = await createRefreshProject(
            fixture.checkHome,
            `${testInfo.project.name}-refresh`,
          );
          refreshProjects.push(refreshProject);
        }
        const existingRefreshTabId = refreshTabIds.values().next().value;
        if (existingRefreshTabId) {
          await activateProjectTab(page, primaryTabId);
        }
        trigger = await startLoadTrigger(
          fixture,
          fixture.projectRoot,
          refreshProject.workItemsTargetPath,
          { issueWindowId, shellWindowId },
        );
        refreshRendezvous = await armActiveWorkRendezvous(
          fixture.checkHome,
          refreshProject.workItemsTargetPath,
        );
        refreshTabId = existingRefreshTabId;
        if (refreshTabId) {
          await activateProjectTab(page, refreshTabId);
        } else {
          refreshTabId = await openAndWaitForNewProjectTab(page, refreshProject.root);
          refreshTabIds.add(refreshTabId);
        }
        expect(refreshTabId).not.toBe(primaryTabId);
        await skipPendingMigration(page, refreshTabId);
        await refreshRendezvous.waitForStarted();
        expect(await refreshRendezvous.hasCompleted()).toBe(false);
        expect(
          trigger.child.exitCode,
          "Work decode trigger must be active when interaction measurement begins",
        ).toBeNull();
        if (preparePrimary) {
          await activateProjectTab(page, primaryTabId);
        }
        await prepareInteraction?.();

        await runPaletteCommand(page, "Start UI Trace");
        const hook = await trigger.startHook();
        expect(
          await hook.hasCompleted(),
          "the measured interaction must begin inside its fresh real hook interval",
        ).toBe(false);
        const result = await measure();
        const interactionCompleteCursor = await liveMessageCursor(page);
        expect(
          await hook.hasCompleted(),
          "the measured interaction must finish before its fresh real hook process exits",
        ).toBe(false);
        expect(
          await refreshRendezvous.hasCompleted(),
          "the measured interaction must finish while its fresh real Work decode is still running",
        ).toBe(false);
        traceEntries.push(...await stopAndReadUiTrace(page, fixture.checkHome));

        await hook.finish();
        await trigger.finish();
        trigger = undefined;
        await refreshRendezvous.waitForCompleted();
        if (!exactProjectionValidated) {
          const projectionCursor = await liveMessageCursor(page);
          await activateProjectTab(page, refreshTabId);
          const workloadSignals = await waitForWorkloadSignals(
            page,
            Math.max(interactionCompleteCursor, projectionCursor),
            fixture.stressWorkId,
          );
          expect(workloadSignals.activeWorkProjectionSeen).toBe(true);
          expect(workloadSignals.sequence).toBeGreaterThan(interactionCompleteCursor);
          exactProjectionValidated = true;
        }
        await refreshRendezvous.stop();
        refreshRendezvous = undefined;
        refreshTabId = undefined;
        return result;
      };

      const tabLatencyMs = await runLoadedInteraction(
        false,
        () => measureTabSwitch(page, primaryTabId),
        () =>
          page.locator(
            `.project-tab[data-project-tab-id="${primaryTabId}"]`,
          ).evaluate((node) => (node as HTMLElement).focus()),
      );
      const issueResult = await runLoadedInteraction(
        true,
        () => measureIssueSelection(page, issueWindowId!, fixture.issueNumber),
        () =>
          page.locator(
            `.workspace-window[data-id="${issueWindowId}"] .knowledge-row[data-issue-number="${fixture.issueNumber}"] .knowledge-row-select`,
          ).focus({ timeout: INTERACTION_BUDGET_MS }),
      );
      const terminalLatencyMs = await runLoadedInteraction(
        true,
        () => measureTerminalRoundtrip(page, shellWindowId!),
        () =>
          page.locator(
            `.workspace-window[data-id="${shellWindowId}"] .xterm-helper-textarea`,
          ).focus({ timeout: INTERACTION_BUDGET_MS }),
      );

      const hookProfiles = (
        await readUserPromptSubmitProfiles(fixture.hookProfilePath)
      ).slice(hookProfileCursor);
      expect(hookProfiles).toHaveLength(3);
      for (const hookProfile of hookProfiles) {
        expect(Number.isFinite(hookProfile.duration_ms)).toBe(true);
        expect(hookProfile.duration_ms).toBeGreaterThan(0);
        expect(hookProfile.duration_ms).toBeLessThan(250);
        expect(hookProfile.provider_read_count).toBe(1);
        expect(hookProfile.history_materialization_count).toBe(1);
        expect(Number(hookProfile.projection_load_count)).toBeLessThanOrEqual(2);
      }
      const maxHookDurationMs = Math.max(
        ...hookProfiles.map((profile) => Number(profile.duration_ms)),
      );

      const overBudgetLongTasks = traceEntries.filter(
        (entry) =>
          entry.kind === "long_task" &&
          Number(entry.duration_ms ?? 0) >= LONG_TASK_BUDGET_MS,
      );
      const overBudgetRafGaps = traceEntries.filter(
        (entry) =>
          entry.kind === "raf_gap" &&
          Number(entry.gap_ms ?? 0) >= RAF_GAP_BUDGET_MS,
      );

      expect(traceEntries.some((entry) => entry.kind === "trace_start")).toBe(true);
      expect(tabLatencyMs).toBeLessThan(INTERACTION_BUDGET_MS);
      expect(issueResult.latencyMs).toBeLessThan(INTERACTION_BUDGET_MS);
      expect(issueResult.mismatchedFrames).toBe(0);
      expect(terminalLatencyMs).toBeLessThan(INTERACTION_BUDGET_MS);
      expect(overBudgetLongTasks).toEqual([]);
      expect(overBudgetRafGaps).toEqual([]);
      expect(consoleErrors).toEqual([]);
      expect(pageErrors).toEqual([]);

      testInfo.annotations.push({
        type: "measurement",
        description:
          `tab=${tabLatencyMs.toFixed(1)}ms ` +
          `issue=${issueResult.latencyMs.toFixed(1)}ms ` +
          `terminal=${terminalLatencyMs.toFixed(1)}ms ` +
          `hook_under_work_load_max=${maxHookDurationMs.toFixed(1)}ms ` +
          `long_tasks=${overBudgetLongTasks.length} ` +
          `raf_gaps=${overBudgetRafGaps.length}`,
      });
    } finally {
      await trigger?.stop();
      await refreshRendezvous?.stop();
      await clearLiveLaunchWizard(page).catch(() => undefined);
      // The passive fixture agent is shared by the serial dark/light runs.
      // Keeping it alive avoids a default-agent replacement between themes;
      // browser-check teardown owns the isolated process and capability file.
      if (shellWindowId) {
        await sendLiveGwtEvent(page, { kind: "close_window", id: shellWindowId })
          .catch(() => undefined);
      }
      if (issueWindowId) {
        await sendLiveGwtEvent(page, { kind: "close_window", id: issueWindowId })
          .catch(() => undefined);
      }
      if (refreshTabId) {
        await sendLiveGwtEvent(page, {
          kind: "close_project_tab",
          tab_id: refreshTabId,
        }).catch(() => undefined);
      }
      for (const tabId of refreshTabIds) {
        await sendLiveGwtEvent(page, {
          kind: "close_project_tab",
          tab_id: tabId,
        }).catch(() => undefined);
      }
      for (const refreshProject of refreshProjects) {
        await rm(refreshProject.root, { recursive: true, force: true });
        await rm(refreshProject.storeRoot, { recursive: true, force: true });
      }
      await releaseBackendLock();
    }
  });
});

async function requireStressFixture(): Promise<StressFixture> {
  const repoRoot = await requiredRealPath("GWT_PLAYWRIGHT_CHECKOUT_ROOT");
  const checkHome = await requiredRealPath("GWT_PLAYWRIGHT_CHECK_HOME");
  const projectRoot = await requiredRealPath("GWT_PLAYWRIGHT_PROJECT_ROOT");
  const boardPath = await requiredRealPath("GWT_PLAYWRIGHT_BOARD_FIXTURE_PATH");
  const workItemsPath = await requiredRealPath("GWT_PLAYWRIGHT_WORK_ITEMS_FIXTURE_PATH");
  const issueMetaPath = await requiredRealPath("GWT_PLAYWRIGHT_ISSUE_META_PATH");
  const hookProfilePath = await requiredRealPath("GWT_PLAYWRIGHT_HOOK_PROFILE_PATH");
  const controlIssueMetaPath = await requiredRealPath(
    "GWT_PLAYWRIGHT_CONTROL_ISSUE_META_PATH",
  );
  const triggerPath = await realpath(
    process.env.GWT_PLAYWRIGHT_LOAD_TRIGGER ??
      join(repoRoot, "scripts", "run-issue-3777-browser-load.sh"),
  );
  const gwtdPath = await realpath(
    process.env.GWT_PLAYWRIGHT_GWTD_PATH ?? join(repoRoot, "target", "debug", "gwtd"),
  );
  const capabilityAgentId = process.env.GWT_PLAYWRIGHT_CAPABILITY_AGENT_ID?.trim() ?? "";
  const stressWorkId =
    process.env.GWT_PLAYWRIGHT_STRESS_WORK_ID?.trim() || DEFAULT_STRESS_WORK_ID;
  const issueNumber = Number(process.env.GWT_PLAYWRIGHT_ISSUE_NUMBER ?? "");
  const controlIssueNumber = Number(
    process.env.GWT_PLAYWRIGHT_CONTROL_ISSUE_NUMBER ?? "",
  );

  if (!checkHome.includes("gwt-fresh-home.")) {
    throw new Error(
      `Issue #3777 fixture contract: GWT_PLAYWRIGHT_CHECK_HOME must be a browser-check fresh home; got ${checkHome}`,
    );
  }
  for (const [label, candidate] of [
    ["Board", boardPath],
    ["WorkItems", workItemsPath],
    ["Issue meta", issueMetaPath],
    ["control Issue meta", controlIssueMetaPath],
    ["hook profile", hookProfilePath],
  ] as const) {
    if (!isInside(checkHome, candidate)) {
      throw new Error(
        `Issue #3777 fixture contract: ${label} fixture must be inside isolated CHECK_HOME; got ${candidate}`,
      );
    }
  }
  if (!isInside(repoRoot, triggerPath)) {
    throw new Error(
      `Issue #3777 fixture contract: load trigger must be tracked inside the checkout; got ${triggerPath}`,
    );
  }

  await requireMinimumSize(boardPath, BOARD_MIN_BYTES, "Board");
  await requireMinimumSize(workItemsPath, WORK_ITEMS_MIN_BYTES, "WorkItems");
  if (workItemsPath.endsWith(`${process.platform === "win32" ? "\\" : "/"}works.json`)) {
    throw new Error(
      "Issue #3777 fixture contract: the large WorkItems source must not be the live works.json; the trigger materializes it only after Issue/Shell setup",
    );
  }
  await access(triggerPath, fsConstants.X_OK).catch((error) => {
    throw new Error(
      `Issue #3777 fixture contract: load trigger is not executable: ${triggerPath}: ${error}`,
    );
  });
  if (!Number.isSafeInteger(issueNumber) || issueNumber <= 0) {
    throw new Error(
      "Issue #3777 fixture contract: GWT_PLAYWRIGHT_ISSUE_NUMBER must be a positive integer",
    );
  }
  if (!capabilityAgentId) {
    throw new Error(
      "Issue #3777 fixture contract: GWT_PLAYWRIGHT_CAPABILITY_AGENT_ID is required",
    );
  }
  if (!stressWorkId) {
    throw new Error("Issue #3777 fixture contract: stress Work id is required");
  }
  if (
    !Number.isSafeInteger(controlIssueNumber) ||
    controlIssueNumber <= 0 ||
    controlIssueNumber === issueNumber
  ) {
    throw new Error(
      "Issue #3777 fixture contract: GWT_PLAYWRIGHT_CONTROL_ISSUE_NUMBER must be a distinct positive integer",
    );
  }
  await requireIssueMeta(issueMetaPath, issueNumber, "target");
  await requireIssueMeta(controlIssueMetaPath, controlIssueNumber, "control");

  return {
    boardPath,
    capabilityAgentId,
    checkHome,
    controlIssueNumber,
    hookProfilePath,
    issueNumber,
    projectRoot,
    gwtdPath,
    stressWorkId,
    triggerPath,
    workItemsPath,
  };
}

async function requireIssueMeta(
  path: string,
  expectedNumber: number,
  label: string,
): Promise<void> {
  const issueMeta = JSON.parse(await readFile(path, "utf8")) as {
    number?: number;
  };
  if (issueMeta.number !== expectedNumber) {
    throw new Error(
      `Issue #3777 fixture contract: ${label} Issue meta number ${String(issueMeta.number)} does not match ${expectedNumber}`,
    );
  }
}

async function requiredRealPath(name: string): Promise<string> {
  const value = process.env[name];
  if (!value || !isAbsolute(value)) {
    throw new Error(
      `Issue #3777 fixture contract: ${name} must be an absolute path; live stress fixtures are not optional`,
    );
  }
  return realpath(value).catch((error) => {
    throw new Error(`Issue #3777 fixture contract: cannot resolve ${name}=${value}: ${error}`);
  });
}

function isInside(parent: string, candidate: string): boolean {
  const rel = relative(parent, candidate);
  const separator = process.platform === "win32" ? "\\" : "/";
  return (
    rel !== "" &&
    rel !== ".." &&
    !rel.startsWith(`..${separator}`) &&
    !isAbsolute(rel)
  );
}

async function requireMinimumSize(
  path: string,
  minimum: number,
  label: string,
): Promise<void> {
  const metadata = await stat(path);
  if (!metadata.isFile() || metadata.size < minimum) {
    throw new Error(
      `Issue #3777 fixture contract: ${label} fixture must be a file >= ${minimum} bytes; got ${metadata.size} at ${path}`,
    );
  }
}

async function armActiveWorkRendezvous(
  checkHome: string,
  workItemsPath: string,
): Promise<ActiveWorkRendezvous> {
  const directory = join(
    checkHome,
    ".gwt",
    "issue-3777-active-work-rendezvous",
  );
  const startedPath = join(directory, "started");
  const completedPath = join(directory, "completed");
  await rm(directory, { recursive: true, force: true });
  await mkdir(directory, { recursive: true });
  await writeFile(join(directory, "work-items-path"), `${workItemsPath}\n`, "utf8");

  const exists = (path: string) =>
    access(path, fsConstants.F_OK)
      .then(() => true)
      .catch(() => false);
  return {
    hasCompleted: () => exists(completedPath),
    stop: () => rm(directory, { recursive: true, force: true }),
    waitForCompleted: () =>
      waitUntil(
        () => exists(completedPath),
        60_000,
        () => "Issue #3777 Active Work refresh did not complete after real Work decode",
      ),
    waitForStarted: () =>
      waitUntil(
        () => exists(startedPath),
        20_000,
        () =>
          "Issue #3777 Active Work refresh did not begin decoding the repository-scale Work fixture; start the browser-check backend with GWT_PLAYWRIGHT_ACTIVE_WORK_RENDEZVOUS=1",
      ),
  };
}

async function createRefreshProject(
  checkHome: string,
  projectName: string,
): Promise<RefreshProject> {
  const root = await mkdtemp(join(tmpdir(), "gwt-prompt-responsive-secondary-"));
  const branchName = `issue-3777-${projectName.replace(/[^a-z0-9-]/gi, "-")}`;
  const originPath = `gwt-browser-check/${projectName.toLowerCase()}-${basename(root).toLowerCase()}`;
  const originUrl = `https://github.com/${originPath}.git`;
  await execFileAsync("git", ["init", "--quiet", `--initial-branch=${branchName}`, root]);
  await execFileAsync("git", ["-C", root, "config", "user.name", "gwt browser-check"]);
  await execFileAsync("git", ["-C", root, "config", "user.email", "browser-check@gwt.local"]);
  await execFileAsync("git", ["-C", root, "commit", "--quiet", "--allow-empty", "-m", "fixture"]);
  await execFileAsync("git", ["-C", root, "remote", "add", "origin", originUrl]);
  const repoHash = createHash("sha256")
    .update(`github.com/${originPath}`)
    .digest("hex")
    .slice(0, 16);
  const storeRoot = join(checkHome, ".gwt", "projects", repoHash);
  const projectStateRoot = join(storeRoot, "project-state");
  await rm(storeRoot, { recursive: true, force: true });
  await mkdir(projectStateRoot, { recursive: true });
  await writeFile(
    join(projectStateRoot, "pm.json"),
    `${JSON.stringify({
      registration: null,
      settings: { auto_start: false, loop_interval_secs: 60 },
    })}\n`,
    "utf8",
  );
  return {
    root,
    storeRoot,
    workItemsTargetPath: join(projectStateRoot, "works.json"),
  };
}

async function currentBranchName(projectRoot: string): Promise<string> {
  const { stdout } = await execFileAsync("git", [
    "-C",
    projectRoot,
    "branch",
    "--show-current",
  ]);
  const branch = stdout.trim();
  if (!branch) throw new Error("Issue #3777 fixture contract: primary branch is detached");
  return branch;
}

async function selectTheme(page: Page, theme: "light" | "dark"): Promise<void> {
  await page.locator(`#op-theme-toggle [data-theme-value="${theme}"]`).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
}

async function clearLiveMigrationModal(page: Page): Promise<void> {
  const modal = page.locator("#migration-modal.open");
  if ((await modal.count()) === 0) return;
  await skipPendingMigration(page, await activeProjectTabId(page));
}

async function skipPendingMigration(page: Page, tabId: string): Promise<void> {
  const modal = page.locator("#migration-modal");
  if (!(await modal.evaluate((node) => node.classList.contains("open")))) return;
  await sendLiveGwtEvent(page, { kind: "skip_migration", tab_id: tabId });
  // SkipMigration intentionally has no backend broadcast because production
  // closes the legacy modal locally before sending it. The live test bridge
  // mirrors that half of the retired UI path after clearing migration_pending.
  await modal.evaluate((node) => {
    node.classList.remove("open");
    node.setAttribute("aria-hidden", "true");
  });
  await expect(modal).not.toHaveClass(/\bopen\b/, { timeout: 20_000 });
}

async function activeProjectTabId(page: Page): Promise<string> {
  const tab = page.locator(".project-tab[aria-current='page']");
  await expect(tab).toBeVisible({ timeout: 20_000 });
  const id = await tab.getAttribute("data-project-tab-id");
  if (!id) throw new Error("active project tab has no data-project-tab-id");
  return id;
}

async function projectTabIdForRoot(page: Page, projectRoot: string): Promise<string> {
  const id = await page
    .waitForFunction(
      (expectedRoot) => {
        const normalize = (value: string) => value.replace(/^\/private(?=\/var\/)/, "");
        const match = Array.from(document.querySelectorAll<HTMLElement>(".project-tab"))
          .find((tab) =>
            normalize(tab.dataset.projectRoot ?? "") === normalize(expectedRoot),
          );
        return match?.dataset.projectTabId ?? "";
      },
      projectRoot,
      { timeout: 20_000 },
    )
    .then((handle) => handle.jsonValue())
    .catch(async (error) => {
      const tabs = await page.locator(".project-tab").evaluateAll((nodes) =>
        nodes.map((node) => ({
          id: (node as HTMLElement).dataset.projectTabId ?? "",
          root: (node as HTMLElement).dataset.projectRoot ?? "",
        })),
      );
      throw new Error(
        `${String(error)}\nExpected project root: ${projectRoot}\nProject tabs: ${JSON.stringify(tabs)}`,
      );
    });
  if (!id) throw new Error(`live backend did not expose project tab for ${projectRoot}`);
  return id;
}

async function closeOtherProjectTabs(page: Page, keepTabId: string): Promise<void> {
  const staleTabIds = await page.locator(".project-tab").evaluateAll(
    (tabs, keepId) => tabs
      .map((tab) => (tab as HTMLElement).dataset.projectTabId ?? "")
      .filter((id) => id && id !== keepId),
    keepTabId,
  );
  for (const tabId of staleTabIds) {
    await sendLiveGwtEvent(page, { kind: "close_project_tab", tab_id: tabId });
  }
  await page.waitForFunction(
    (keepId) => Array.from(document.querySelectorAll<HTMLElement>(".project-tab"))
      .every((tab) => tab.dataset.projectTabId === keepId),
    keepTabId,
    { timeout: 20_000 },
  );
}

async function openAndWaitForNewProjectTab(page: Page, projectRoot: string): Promise<string> {
  const before = await page
    .locator(".project-tab")
    .evaluateAll((tabs) => tabs.map((tab) => (tab as HTMLElement).dataset.projectTabId ?? ""));
  await openLiveGwtProject(page, projectRoot);
  const id = await page
    .waitForFunction(
      (previousIds) => {
        const previous = new Set(previousIds);
        const active = document.querySelector<HTMLElement>(
          ".project-tab[aria-current='page']",
        );
        const candidate = active?.dataset.projectTabId ?? "";
        return candidate && !previous.has(candidate) ? candidate : "";
      },
      before,
      { timeout: 20_000 },
    )
    .then((handle) => handle.jsonValue());
  if (!id) throw new Error("live backend did not activate the refresh project tab");
  return id;
}

async function createWindow(
  page: Page,
  preset: "issue" | "shell" | "work",
): Promise<string> {
  const before = await page
    .locator(`.workspace-window[data-preset="${preset}"]`)
    .evaluateAll((nodes) => nodes.map((node) => (node as HTMLElement).dataset.id ?? ""));
  await sendLiveGwtEvent(page, {
    kind: "create_window",
    preset,
    bounds: {
      x: preset === "issue" ? 40 : 520,
      y: preset === "work" ? 180 : 80,
      width: 820,
      height: 560,
    },
  });
  const id = await page
    .waitForFunction(
      ({ preset, before }) => {
        const previous = new Set(before);
        const match = Array.from(
          document.querySelectorAll(`.workspace-window[data-preset="${preset}"]`),
        ).find((node) => !previous.has((node as HTMLElement).dataset.id ?? ""));
        return match ? (match as HTMLElement).dataset.id ?? "" : "";
      },
      { preset, before },
      { timeout: 20_000 },
    )
    .then((handle) => handle.jsonValue());
  if (!id) throw new Error(`live backend did not create ${preset} window`);
  return id;
}

async function startLoadTrigger(
  fixture: StressFixture,
  loadProjectRoot: string,
  workItemsTargetPath: string,
  windows: { issueWindowId: string; shellWindowId: string },
): Promise<RunningLoadTrigger> {
  const child = spawn(fixture.triggerPath, [], {
    cwd: loadProjectRoot,
    env: {
      ...process.env,
      GWT_PLAYWRIGHT_BOARD_FIXTURE_PATH: fixture.boardPath,
      GWT_PLAYWRIGHT_CHECK_HOME: fixture.checkHome,
      GWT_PLAYWRIGHT_ISSUE_WINDOW_ID: windows.issueWindowId,
      GWT_HOOK_PROFILE_PATH: fixture.hookProfilePath,
      GWT_PLAYWRIGHT_GWTD_PATH: fixture.gwtdPath,
      GWT_PLAYWRIGHT_PROJECT_ROOT: loadProjectRoot,
      GWT_PLAYWRIGHT_SHELL_WINDOW_ID: windows.shellWindowId,
      GWT_PLAYWRIGHT_WORK_ITEMS_FIXTURE_PATH: fixture.workItemsPath,
      GWT_PLAYWRIGHT_WORK_ITEMS_TARGET_PATH: workItemsTargetPath,
    },
    stdio: "pipe",
  });
  let stdout = "";
  let stderr = "";
  const hookCompletionPath = join(
    fixture.checkHome,
    ".gwt",
    "issue-3777-hook-completed",
  );
  const hookHasCompleted = () =>
    access(hookCompletionPath, fsConstants.F_OK)
      .then(() => true)
      .catch(() => false);
  child.stdout.setEncoding("utf8");
  child.stderr.setEncoding("utf8");
  child.stdout.on("data", (chunk: string) => {
    stdout += chunk;
  });
  child.stderr.on("data", (chunk: string) => {
    stderr += chunk;
  });

  await waitUntil(
    () => stdout.includes(LOAD_STARTED_MARKER),
    20_000,
    () =>
      `Issue #3777 load trigger did not emit ${LOAD_STARTED_MARKER}; exit=${String(child.exitCode)} stdout=${stdout} stderr=${stderr}`,
  );
  if (child.exitCode !== null) {
    throw new Error(
      `Issue #3777 load trigger exited before the interaction window; exit=${child.exitCode} stdout=${stdout} stderr=${stderr}`,
    );
  }

  const stop = async () => {
    if (child.exitCode === null) child.kill("SIGTERM");
    await waitForExit(child, 5_000).catch(() => undefined);
  };
  const startHook = async () => {
    child.stdin.write(`${START_HOOK_MARKER}\n`);
    await waitUntil(
      () => stdout.includes(HOOK_STARTED_MARKER),
      20_000,
      () =>
        `Issue #3777 real hook did not emit ${HOOK_STARTED_MARKER}; exit=${String(child.exitCode)} stdout=${stdout} stderr=${stderr}`,
    );
    if (await hookHasCompleted()) {
      throw new Error(
        `Issue #3777 real hook completed before its interaction began; stdout=${stdout}`,
      );
    }
    return {
      hasCompleted: hookHasCompleted,
      finish: async () => {
        child.stdin.write(`${INTERACTIONS_COMPLETE_MARKER}\n`);
        await waitUntil(
          () => stdout.includes(HOOK_COMPLETED_MARKER),
          20_000,
          () =>
            `Issue #3777 real hook did not emit ${HOOK_COMPLETED_MARKER}; exit=${String(child.exitCode)} stdout=${stdout} stderr=${stderr}`,
        );
      },
    };
  };
  const finish = async () => {
    child.stdin.end();
    const exitCode = await waitForExit(child, 60_000);
    if (exitCode !== 0) {
      throw new Error(
        `Issue #3777 load trigger failed; exit=${String(exitCode)} stdout=${stdout} stderr=${stderr}`,
      );
    }
  };
  return { child, finish, startHook, stop };
}

async function measureTabSwitch(page: Page, tabId: string): Promise<number> {
  const cursor = await liveMessageCursor(page);
  const start = await page.evaluate(() => performance.now());
  const tab = page.locator(`.project-tab[data-project-tab-id="${tabId}"]`);
  await tab.click({ force: true, timeout: INTERACTION_BUDGET_MS });
  await expect(tab).toHaveAttribute("aria-current", "page", {
    timeout: INTERACTION_BUDGET_MS,
  });
  await waitForLiveMessage(page, cursor, "workspace_state", (payload) =>
    payload?.workspace?.active_tab_id === tabId,
  );
  return page.evaluate((startedAt) => performance.now() - startedAt, start);
}

async function activateProjectTab(page: Page, tabId: string): Promise<void> {
  const tab = page.locator(`.project-tab[data-project-tab-id="${tabId}"]`);
  if (await tab.getAttribute("aria-current") === "page") return;
  const cursor = await liveMessageCursor(page);
  await tab.evaluate((node) => (node as HTMLElement).click());
  await expect(tab).toHaveAttribute("aria-current", "page", { timeout: 20_000 });
  await waitForLiveMessage(
    page,
    cursor,
    "workspace_state",
    (payload) => payload?.workspace?.active_tab_id === tabId,
    20_000,
  );
}

async function measureIssueSelection(
  page: Page,
  windowId: string,
  issueNumber: number,
): Promise<{ latencyMs: number; mismatchedFrames: number }> {
  const target = page.locator(
    `.workspace-window[data-id="${windowId}"] .knowledge-row[data-issue-number="${issueNumber}"] .knowledge-row-select`,
  );
  const cursor = await liveMessageCursor(page);
  const probe = page.evaluate(async ({ expectedNumber, windowId }) => {
    const surface = Array.from(document.querySelectorAll<HTMLElement>(".workspace-window"))
      .find((window) => window.dataset.id === windowId);
    if (!surface) throw new Error(`Issue window ${windowId} is unavailable`);
    const startedAt = performance.now();
    let mismatchedFrames = 0;
    let lastSelectedNumber: number | null = null;
    let lastDetailNumber = Number.NaN;
    while (performance.now() - startedAt < 5_000) {
      const selected = surface.querySelector(
        ".knowledge-row.selected .knowledge-row-select[aria-current='true']",
      );
      const selectedNumber = selected
        ? Number((selected.closest(".knowledge-row") as HTMLElement | null)?.dataset.issueNumber)
        : null;
      const subtitle =
        surface.querySelector(".knowledge-detail-subtitle")?.textContent ?? "";
      const detailNumber = Number(subtitle.match(/#(\d+)/)?.[1] ?? NaN);
      lastSelectedNumber = selectedNumber;
      lastDetailNumber = detailNumber;
      if (
        selectedNumber !== null &&
        Number.isFinite(detailNumber) &&
        selectedNumber !== detailNumber
      ) {
        mismatchedFrames += 1;
      }
      if (selectedNumber === expectedNumber && detailNumber === expectedNumber) {
        return { latencyMs: performance.now() - startedAt, mismatchedFrames };
      }
      await new Promise<void>((resolve) => requestAnimationFrame(() => resolve()));
    }
    throw new Error(
      `Issue #${expectedNumber} identity did not settle within 5000ms; selected=${String(lastSelectedNumber)} detail=${String(lastDetailNumber)} hidden=${String(surface.hidden)} ariaHidden=${String(surface.getAttribute("aria-hidden"))}`,
    );
  }, { expectedNumber: issueNumber, windowId });

  await target.press("Enter", { timeout: INTERACTION_BUDGET_MS });
  const [result] = await Promise.all([
    probe,
    waitForLiveMessage(
      page,
      cursor,
      "knowledge_detail",
      (payload) => payload?.id === windowId && payload?.detail?.number === issueNumber,
      5_000,
    ),
  ]);
  return result;
}

async function selectIssueAndWait(
  page: Page,
  windowId: string,
  issueNumber: number,
): Promise<void> {
  const cursor = await liveMessageCursor(page);
  const target = page.locator(
    `.workspace-window[data-id="${windowId}"] .knowledge-row[data-issue-number="${issueNumber}"] .knowledge-row-select`,
  );
  await target.focus();
  await target.press("Enter");
  await waitForLiveMessage(page, cursor, "knowledge_detail", (payload) =>
    payload?.id === windowId && payload?.detail?.number === issueNumber,
  );
  await expect(
    page.locator(
      `.workspace-window[data-id="${windowId}"] .knowledge-detail-subtitle`,
    ),
  ).toContainText(`#${issueNumber}`);
}

async function measureTerminalRoundtrip(page: Page, windowId: string): Promise<number> {
  const suffix = `${Date.now()}_${Math.random().toString(16).slice(2)}`;
  const sentinel = `__GWT_RESPONSIVE_${suffix}__`;
  const terminal = page.locator(
    `.workspace-window[data-id="${windowId}"] .xterm-helper-textarea`,
  );
  const cursor = await liveMessageCursor(page);
  const start = await page.evaluate(() => performance.now());
  await terminal.focus({ timeout: INTERACTION_BUDGET_MS });
  await page.keyboard.type(`printf '%s%s\\n' '__GWT_RESPONSIVE_' '${suffix}__'`);
  await page.keyboard.press("Enter");
  await page.waitForFunction(
    ({ cursor, sentinel, windowId }) => {
      const messages = (window as any).__gwtPlaywrightMessages;
      if (!Array.isArray(messages)) return false;
      const encoded = messages
        .filter(
          (entry: any) =>
            entry?.sequence > cursor &&
            entry?.payload?.kind === "terminal_output" &&
            entry.payload.id === windowId,
        )
        .map((entry: any) => String(entry.payload.data_base64 ?? ""));
      let output = "";
      for (const chunk of encoded) {
        try {
          output += atob(chunk);
        } catch {
          return false;
        }
      }
      return output.includes(sentinel);
    },
    { cursor, sentinel, windowId },
    { timeout: INTERACTION_BUDGET_MS },
  );
  return page.evaluate((startedAt) => performance.now() - startedAt, start);
}

async function launchHookCapabilityAgent(
  page: Page,
  branchName: string,
  agentId: string,
  checkHome: string,
): Promise<{ agentWindowId: string; setupWindowId: string }> {
  const capabilityPath = join(
    checkHome,
    ".gwt",
    "issue-3777-hook-capability",
  );
  const existingAgentWindowId = await liveAgentWindowId(page, agentId);
  if (existingAgentWindowId && await hookCapabilityReady(capabilityPath)) {
    return { agentWindowId: existingAgentWindowId, setupWindowId: "" };
  }
  const existingSetupWindowId = await page
    .locator('.workspace-window[data-preset="work"]')
    .last()
    .getAttribute("data-id")
    .catch(() => null);
  if (!existingSetupWindowId) {
    throw new Error(
      "Issue #3777 fixture contract: seeded Work launch surface is missing",
    );
  }
  const setupWindowId = existingSetupWindowId;
  let cursor = await liveMessageCursor(page);
  await sendLiveGwtEvent(page, {
    kind: "open_launch_wizard",
    id: setupWindowId,
    branch_name: branchName,
    linked_issue_number: null,
  });
  let wizardState = await waitForLiveMessage(
    page,
    cursor,
    "launch_wizard_state",
    (payload) => payload?.wizard != null,
    20_000,
  );

  cursor = await liveMessageCursor(page);
  await sendLiveGwtEvent(page, {
    kind: "launch_wizard_action",
    action: { kind: "set_launch_path", path: "manual_setup" },
    bounds: null,
  });
  wizardState = await waitForLiveMessage(
    page,
    cursor,
    "launch_wizard_state",
    (payload) => payload?.wizard?.selected_launch_path === "manual_setup",
    20_000,
  );
  const agentOptions = Array.isArray(wizardState.wizard?.agent_options)
    ? wizardState.wizard.agent_options
    : [];
  if (!agentOptions.some((option: any) => option?.value === agentId)) {
    const offeredIds = agentOptions
      .map((option: any) => String(option?.value ?? ""))
      .filter(Boolean);
    throw new Error(
      `Issue #3777 fixture contract: custom capability agent ${agentId} is not offered by Launch Wizard manual setup; offered=${offeredIds.join(",")}`,
    );
  }

  cursor = await liveMessageCursor(page);
  await sendLiveGwtEvent(page, {
    kind: "launch_wizard_action",
    action: { kind: "set_agent", agent_id: agentId },
    bounds: null,
  });
  wizardState = await waitForLiveMessage(
    page,
    cursor,
    "launch_wizard_state",
    (payload) =>
      payload?.wizard?.selected_agent_id === agentId &&
      launchWizardTransitionSettled(payload.wizard),
    30_000,
  );

  const runtimeTargetOptions = Array.isArray(
    wizardState.wizard?.runtime_target_options,
  )
    ? wizardState.wizard.runtime_target_options
    : [];
  if (
    wizardState.wizard?.selected_runtime_target !== "host" &&
    runtimeTargetOptions.some((option: any) => option?.value === "host")
  ) {
    cursor = await liveMessageCursor(page);
    await sendLiveGwtEvent(page, {
      kind: "launch_wizard_action",
      action: { kind: "set_runtime_target", target: "Host" },
      bounds: null,
    });
    wizardState = await waitForLiveMessage(
      page,
      cursor,
      "launch_wizard_state",
      (payload) =>
        payload?.wizard?.selected_runtime_target === "host" &&
        launchWizardTransitionSettled(payload.wizard),
      30_000,
    );
  }

  for (let step = 0; step < 12 && wizardState.wizard != null; step += 1) {
    const wizard = wizardState.wizard;
    const stepRuntimeTargetOptions = Array.isArray(wizard.runtime_target_options)
      ? wizard.runtime_target_options
      : [];
    if (
      wizard.selected_runtime_target !== "host" &&
      stepRuntimeTargetOptions.some((option: any) => option?.value === "host")
    ) {
      cursor = await liveMessageCursor(page);
      await sendLiveGwtEvent(page, {
        kind: "launch_wizard_action",
        action: { kind: "set_runtime_target", target: "Host" },
        bounds: null,
      });
      wizardState = await waitForLiveMessage(
        page,
        cursor,
        "launch_wizard_state",
        (payload) =>
          payload?.wizard?.selected_runtime_target === "host" &&
          launchWizardTransitionSettled(payload.wizard),
        30_000,
      );
      continue;
    }
    if (wizard.error) {
      throw new Error(`Issue #3777 capability agent Launch Wizard failed: ${wizard.error}`);
    }
    if (!wizard.primary_action_enabled) {
      throw new Error(
        `Issue #3777 capability agent Launch Wizard cannot advance: phase=${String(wizard.phase ?? "unknown")} disabled_reason=${String(wizard.primary_action_disabled_reason ?? "unknown")}`,
      );
    }
    cursor = await liveMessageCursor(page);
    await sendLiveGwtEvent(page, {
      kind: "launch_wizard_action",
      action: { kind: "submit" },
      bounds: { x: 24, y: 24, width: 1280, height: 800 },
    });
    wizardState = await waitForLiveMessage(
      page,
      cursor,
      "launch_wizard_state",
      (payload) =>
        payload?.wizard == null || launchWizardTransitionSettled(payload.wizard),
      30_000,
    );
  }
  if (wizardState.wizard != null) {
    throw new Error(
      "Issue #3777 capability agent Launch Wizard did not complete within 12 state transitions",
    );
  }

  const agentWindowId = await page
    .waitForFunction(
      (agentId) => {
        const messages = (window as any).__gwtPlaywrightMessages;
        if (!Array.isArray(messages)) return "";
        for (let index = messages.length - 1; index >= 0; index -= 1) {
          const payload = messages[index]?.payload;
          if (payload?.kind !== "workspace_state") continue;
          for (const tab of payload.workspace?.tabs ?? []) {
            const match = (tab?.workspace?.windows ?? []).find(
              (candidate: any) =>
                candidate?.agent_id === agentId &&
                (candidate?.status === "running" || candidate?.status === "idle"),
            );
            if (match?.id) return String(match.id);
          }
        }
        return "";
      },
      agentId,
      { timeout: 30_000 },
    )
    .then((handle) => handle.jsonValue());
  if (!agentWindowId) {
    throw new Error("Issue #3777 capability agent window was not created");
  }

  await waitUntil(
    () => hookCapabilityReady(capabilityPath),
    20_000,
    () =>
      `Issue #3777 capability agent did not publish a complete hook capability at ${capabilityPath}`,
  );
  return {
    agentWindowId,
    setupWindowId: agentWindowId === setupWindowId ? "" : setupWindowId,
  };
}

async function liveAgentWindowId(page: Page, agentId: string): Promise<string> {
  return page.evaluate((expectedAgentId) => {
    const messages = (window as any).__gwtPlaywrightMessages;
    if (!Array.isArray(messages)) return "";
    for (let index = messages.length - 1; index >= 0; index -= 1) {
      const payload = messages[index]?.payload;
      if (payload?.kind !== "workspace_state") continue;
      for (const tab of payload.workspace?.tabs ?? []) {
        const match = (tab?.workspace?.windows ?? []).find(
          (candidate: any) =>
            candidate?.agent_id === expectedAgentId &&
            (candidate?.status === "running" || candidate?.status === "idle"),
        );
        if (match?.id) return String(match.id);
      }
    }
    return "";
  }, agentId);
}

async function hookCapabilityReady(path: string): Promise<boolean> {
  const values = await readFile(path, "utf8")
    .then((contents) => contents.trimEnd().split("\n"))
    .catch(() => []);
  return values.length === 4 && values.every((value) => value.length > 0);
}

function launchWizardTransitionSettled(wizard: any): boolean {
  return Boolean(wizard?.error) || !(
    wizard?.is_hydrating ||
    wizard?.runtime_resolution_pending ||
    wizard?.launch_materialization_pending
  );
}

async function waitForWorkloadSignals(
  page: Page,
  cursor: number,
  stressWorkId: string,
): Promise<{
  activeWorkProjectionSeen: boolean;
  sequence: number;
}> {
  return page
    .waitForFunction(
      ({ cursor, stressWorkId }) => {
        const messages = (window as any).__gwtPlaywrightMessages;
        if (!Array.isArray(messages)) return null;
        const workload = messages.filter((entry: any) => entry?.sequence > cursor);
        const exactProjection = workload.find(
          (entry: any) =>
            entry?.payload?.kind === "active_work_projection" &&
            (entry.payload?.projection?.active_works ?? []).some(
              (work: any) => work?.id === stressWorkId,
            ),
        );
        return exactProjection
          ? {
              activeWorkProjectionSeen: true,
              sequence: Number(exactProjection.sequence),
            }
          : null;
      },
      { cursor, stressWorkId },
      { timeout: 20_000 },
    )
    .then((handle) => handle.jsonValue());
}

async function readUserPromptSubmitProfiles(
  path: string,
): Promise<Record<string, any>[]> {
  const records = (await readFile(path, "utf8"))
    .split("\n")
    .filter(Boolean)
    .map((line) => JSON.parse(line) as Record<string, any>);
  const totals = records.filter(
    (record) =>
      record.event === "UserPromptSubmit" && record.handler === "event-total",
  );
  for (const total of totals) {
    expect(Object.keys(total).sort()).toEqual(
      [
        "additional_context_bytes",
        "duration_ms",
        "event",
        "handler",
        "history_materialization_count",
        "occurred_at",
        "projection_load_count",
        "provider_read_count",
        "status",
      ].sort(),
    );
  }
  return totals;
}

async function runPaletteCommand(page: Page, query: string): Promise<void> {
  await page.locator("#op-palette-button").click();
  const input = page.locator("#op-palette-input");
  await expect(input).toBeVisible();
  await input.fill(query);
  await page.keyboard.press("Enter");
  await expect(page.locator("#op-palette-backdrop")).not.toHaveAttribute(
    "data-open",
    "true",
  );
}

async function stopAndReadUiTrace(
  page: Page,
  checkHome: string,
): Promise<TraceEntry[]> {
  const cursor = await liveMessageCursor(page);
  await runPaletteCommand(page, "Stop UI Trace");
  const saved = await waitForLiveMessage(page, cursor, "ui_trace_saved", () => true);
  const rawTracePath = String(saved.path ?? "");
  if (!rawTracePath || !isAbsolute(rawTracePath)) {
    throw new Error(`live backend returned an invalid UI trace path: ${rawTracePath}`);
  }
  const tracePath = await realpath(rawTracePath);
  if (!isInside(checkHome, tracePath)) {
    throw new Error(
      `live backend UI trace escaped isolated CHECK_HOME: ${tracePath}`,
    );
  }
  const contents = await readFile(tracePath, "utf8");
  return contents
    .split("\n")
    .filter(Boolean)
    .map((line) => JSON.parse(line) as TraceEntry);
}

async function liveMessageCursor(page: Page): Promise<number> {
  return page.evaluate(() => Number((window as any).__gwtPlaywrightMessageSequence) || 0);
}

async function waitForLiveMessage(
  page: Page,
  cursor: number,
  kind: string,
  predicate: (payload: any) => boolean,
  timeoutMs = INTERACTION_BUDGET_MS,
): Promise<any> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const messages = await page.evaluate(
      ({ cursor, kind }) => {
        const entries = (window as any).__gwtPlaywrightMessages;
        if (!Array.isArray(entries)) return [];
        return entries
          .filter(
            (entry: any) => entry?.sequence > cursor && entry?.payload?.kind === kind,
          )
          .map((entry: any) => entry.payload);
      },
      { cursor, kind },
    );
    const match = messages.find(predicate);
    if (match) return match;
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  throw new Error(`live backend did not emit matching ${kind} after cursor ${cursor}`);
}

async function waitUntil(
  predicate: () => boolean | Promise<boolean>,
  timeoutMs: number,
  errorMessage: () => string,
): Promise<void> {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    if (await predicate()) return;
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  throw new Error(errorMessage());
}

async function waitForExit(
  child: ChildProcessWithoutNullStreams,
  timeoutMs: number,
): Promise<number | null> {
  if (child.exitCode !== null) return child.exitCode;
  return new Promise((resolve, reject) => {
    const timeout = setTimeout(() => {
      cleanup();
      reject(new Error(`load trigger did not exit within ${timeoutMs}ms`));
    }, timeoutMs);
    const onExit = (code: number | null) => {
      cleanup();
      resolve(code);
    };
    const onError = (error: Error) => {
      cleanup();
      reject(error);
    };
    const cleanup = () => {
      clearTimeout(timeout);
      child.off("exit", onExit);
      child.off("error", onError);
    };
    child.once("exit", onExit);
    child.once("error", onError);
  });
}
