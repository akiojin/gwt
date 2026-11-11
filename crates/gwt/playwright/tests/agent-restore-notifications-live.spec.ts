import { expect, test } from "@playwright/test";
import { execFileSync } from "node:child_process";
import { randomUUID } from "node:crypto";
import { mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { startExactRelaunchFixture } from "./_helpers/exact-relaunch";
import {
  gotoLiveGwt,
  sendLiveGwtEvent,
  withLiveGwtBackendLock,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";

// Issue #3730: real backend candidate filtering; Rust tests inject metadata
// errors. A newer receipt-backed history must not hide the eligible history.
test("recovery receipt excludes saved history from Launch Wizard QuickStart", async ({ page }, info) => {
  test.skip(process.env.GWT_PLAYWRIGHT_RECOVERY_RECEIPTS !== "1",
    "requires freshly built checkout binaries and an isolated saved-session fixture");
  test.setTimeout(180_000);
  const theme = info.project.name.includes("light") ? "light" : "dark";
  const histories = [
    { id: "receipt-ready", native: randomUUID(), timestamp: "2026-01-01T00:00:00Z" },
    { id: "receipt-pending", native: randomUUID(), timestamp: "2026-01-02T00:00:00Z" },
  ];
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
  const fixture = await startExactRelaunchFixture(info, {
    async beforeStart({ home, project, preferences }) {
      const sessions = join(home, ".gwt/sessions");
      const recovery = join(sessions, "execution-launch-recovery");
      await mkdir(recovery, { recursive: true });
      const hash = dirname(preferences).split(/[\\/]/).at(-1)!;
      for (const history of histories) {
        await writeFile(join(sessions, `${history.id}.toml`), [
          `id = ${JSON.stringify(history.id)}`, `worktree_path = ${JSON.stringify(project)}`,
          `project_state_root = ${JSON.stringify(project)}`, `repo_hash = ${JSON.stringify(hash)}`,
          'branch = "main"', 'agent_id = { type = "Codex" }', 'display_name = "Codex"',
          `agent_session_id = ${JSON.stringify(history.native)}`, 'status = "Stopped"',
          'session_mode = "Resume"', 'restore_window_on_startup = false', 'schema_version = 5',
          `created_at = ${JSON.stringify(history.timestamp)}`, `updated_at = ${JSON.stringify(history.timestamp)}`,
          `last_activity_at = ${JSON.stringify(history.timestamp)}`, "",
        ].join("\n"));
      }
      // Presence is the candidate fence; no recovery transaction is fabricated.
      await writeFile(join(recovery, `${histories[1].id}.json`), "{}");
    },
  });
  try {
    await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const after = await page.evaluate(() => (window as any).__gwtPlaywrightMessageSequence ?? 0);
    await sendLiveGwtEvent(page, { kind: "open_active_work_launch_wizard", branch_name: fixture.branch });
    const entries = await page.waitForFunction(after => {
      const wizard = (window as any).__gwtPlaywrightMessages?.findLast((entry: any) =>
        entry.sequence > after && entry.payload.kind === "launch_wizard_state"
        && entry.payload.wizard && !entry.payload.wizard.is_hydrating)?.payload.wizard;
      return wizard?.show_start_methods ? wizard.quick_start_entries : null;
    }, after, { timeout: 60_000 }).then(handle => handle.jsonValue());
    expect(entries.map((entry: any) => entry.resume_session_id)).toEqual([histories[0].native]);
    const wizard = page.locator("#wizard-modal");
    await expect(wizard).toBeVisible();
    const resume = wizard.getByRole("button", { name: /Continue last session/ });
    await expect(resume).toBeEnabled();
    await expect(resume).toContainText(histories[0].native);
    await expect(wizard).not.toContainText(histories[1].native);
    expect(await fixture.launches(), "opening saved history never spawns a provider").toEqual([]);
    expect(errors, "console and page errors").toEqual([]);
    await info.attach(`recovery-receipt-${theme}`, {
      body: await page.screenshot({ fullPage: true }), contentType: "image/png",
    });
    await info.attach(`recovery-receipt-evidence-${theme}`, {
      body: JSON.stringify({ histories, entries, errors }), contentType: "application/json",
    });
  } finally {
    try { await page.close(); } finally { await fixture.stop(); }
  }
  expect(errors, "console and page errors including owned cleanup").toEqual([]);
});

// Issue #6725: this runs two actual checkout GUI processes over the same
// isolated HOME. Only the saved history and local landed ref are fixtures;
// restore admission, source retirement, and notifications are not injected.
test.describe("retired automatic restore sources", () => {
  test("twelve landed windows stay diagnostic and are not retried on the next startup", async ({ page }, info) => {
    test.skip(process.env.GWT_PLAYWRIGHT_RETIRED_RESTORE !== "1",
      "requires freshly built checkout binaries and an isolated saved-session fixture");
    test.setTimeout(180_000);
    const theme = info.project.name.includes("light") ? "light" : "dark";
    const sources = Array.from({ length: 12 }, (_, index) => `landed-restore-${index}`);
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    await page.addInitScript(theme => {
      try { localStorage.setItem("gwt:ui:theme", theme); } catch { /* about:blank during cleanup */ }
      (window as any).__gwtAgentErrorToasts = [];
      // Observe transient toasts too: checking only the final DOM could miss
      // an error that disappeared while the twelve restores were settling.
      new MutationObserver(() => {
        for (const title of document.querySelectorAll(".toast-alerts__title")) {
          if (title.textContent === "Agent error")
            (window as any).__gwtAgentErrorToasts.push(title.textContent);
        }
      }).observe(document, { childList: true, subtree: true, characterData: true });
    }, theme);
    let logDirectories: string[] = [];
    const fixture = await startExactRelaunchFixture(info, {
      async beforeStart({ home, project, env, preferences }) {
        env.RUST_LOG = "info,gwt.restore.admission=debug";
        execFileSync("git", ["update-ref", "refs/remotes/origin/develop", "HEAD"], { cwd: project, env });
        const sessions = join(home, ".gwt/sessions");
        await mkdir(sessions, { recursive: true });
        const timestamp = new Date().toISOString();
        const hash = dirname(preferences).split(/[\\/]/).at(-1)!;
        for (const id of sources) {
          await writeFile(join(sessions, `${id}.toml`), [
            `id = ${JSON.stringify(id)}`, `worktree_path = ${JSON.stringify(project)}`,
            `project_state_root = ${JSON.stringify(project)}`, `repo_hash = ${JSON.stringify(hash)}`,
            'branch = "main"', 'agent_id = { type = "Codex" }',
            `agent_session_id = ${JSON.stringify(randomUUID())}`, 'status = "Idle"',
            'session_mode = "Resume"', 'restore_window_on_startup = true', 'schema_version = 5',
            `created_at = ${JSON.stringify(timestamp)}`, `updated_at = ${JSON.stringify(timestamp)}`,
            `last_activity_at = ${JSON.stringify(timestamp)}`, 'last_hook_event = "Stop"',
            `last_hook_event_at = ${JSON.stringify(timestamp)}`, `last_completed_stop_at = ${JSON.stringify(timestamp)}`,
            'display_name = "Codex"', "",
          ].join("\n"));
        }
        await writeFile(join(dirname(preferences), "workspace.json"), JSON.stringify({
          windows: sources.map((id, index) => ({ id: `diagnostic-${index}`, title: `Saved Agent ${index}`,
            preset: "codex", status: "stopped", z_index: index + 1, agent_id: "codex", session_id: id,
            // A meaningful diagnostic is retained, unlike an empty landed pane.
            dynamic_title_detail: `Retained history for ${id}` })),
          next_z_index: sources.length + 1,
        }));
        logDirectories = [join(home, ".gwt/logs"), join(dirname(preferences), "logs")];
      },
    });
    async function logs(directory: string): Promise<any[]> {
      const rows: any[] = [];
      for (const entry of await readdir(directory, { withFileTypes: true }).catch(() => [])) {
        const path = join(directory, entry.name);
        if (entry.isDirectory()) rows.push(...await logs(path));
        else if (/\.log(?:\.|$)|\.jsonl$/.test(entry.name)) {
          for (const line of (await readFile(path, "utf8")).split("\n")) {
            try { rows.push(JSON.parse(line)); } catch { /* An in-flight final line is retried. */ }
          }
        }
      }
      return rows;
    }
    const admissionRows = async () => (await Promise.all(logDirectories.map(logs))).flat()
      .filter(row => row.target === "gwt.restore.admission");
    const summaries = (rows: any[]) => rows.filter(row =>
      row.fields?.message === "session restore admission summary" && row.fields.scope === "startup restore");
    const landedRows = (rows: any[]) => rows.filter(row =>
      row.fields?.message === "session restore refused" && row.fields.reason === "landed_worktree");
    try {
      let summaryCount = 0;
      for (const generation of [1, 2]) {
        await gotoLiveGwt(page, fixture.url, { enableTestBridge: true });
        await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
        await expect.poll(async () => summaries(await admissionRows()).length,
          { timeout: 60_000, message: `startup ${generation} restore drain settles` }).toBe(summaryCount + 1);
        const rows = await admissionRows();
        summaryCount = summaries(rows).length;
        if (generation === 1) {
          expect(landedRows(rows)).toHaveLength(sources.length);
          expect(new Set(landedRows(rows).map(row => row.fields.session_id))).toEqual(new Set(sources));
        } else {
          expect(landedRows(rows), "the second process must not retry the retired sources").toHaveLength(sources.length);
          expect(summaries(rows).at(-1).fields.reasons).toContain("window_not_open=12");
        }
        for (const id of sources) {
          const saved = await readFile(join(fixture.home, ".gwt/sessions", `${id}.toml`), "utf8");
          expect(saved).toMatch(/^restore_window_on_startup\s*=\s*false$/m);
          expect(saved).toMatch(/^status\s*=\s*"Stopped"$/m);
        }
        await page.locator("#op-notifications-button").click();
        const drawer = page.locator("#notification-center");
        await expect(drawer).toBeVisible();
        const skipped = drawer.locator(".notification-center__item")
          .filter({ hasText: "Automatic restore skipped: landed_worktree" });
        await expect(skipped).toHaveCount(generation === 1 ? sources.length : 0);
        if (generation === 1) {
          expect(await skipped.evaluateAll(nodes => nodes.map(node => (node as HTMLElement).dataset.level)))
            .toEqual(sources.map(() => "info"));
        }
        await expect(drawer.locator('.notification-center__item[data-level="error"]')).toHaveCount(0);
        await expect(page.locator("#wizard-modal")).toBeHidden();
        const observations = await page.evaluate(() => ({
          agentErrorToasts: (window as any).__gwtAgentErrorToasts,
          errors: ((window as any).__gwtPlaywrightMessages ?? []).filter((entry: any) =>
            entry.payload.kind === "terminal_status" && entry.payload.status === "error"),
          windows: (((window as any).__gwtPlaywrightMessages ?? [])
            .findLast((entry: any) => entry.payload.kind === "workspace_state")?.payload.workspace.tabs ?? [])
            .flatMap((tab: any) => tab.workspace.windows),
        }));
        expect(observations.agentErrorToasts, "Agent error toast count").toEqual([]);
        expect(observations.errors, "refusals must not become terminal errors").toEqual([]);
        expect(observations.windows.filter((window: any) => sources.includes(window.session_id)))
          .toHaveLength(sources.length);
        expect(observations.windows.some((window: any) => window.status === "error")).toBe(false);
        expect(await fixture.launches(), "landed history never spawns a provider").toEqual([]);
        expect(errors, "console and page errors").toEqual([]);
        await info.attach(`retired-restore-${generation}-${theme}`, {
          body: await page.screenshot({ fullPage: true }), contentType: "image/png",
        });
        await info.attach(`retired-restore-evidence-${generation}-${theme}`, {
          body: JSON.stringify({ generation, sources, admission: rows, observations, errors }),
          contentType: "application/json",
        });
        // Stop reconnects before terminating only this helper's recorded PID.
        await page.goto("about:blank");
        if (generation === 1) await fixture.restart();
      }
    } finally {
      try { await page.close(); } finally { await fixture.stop(); }
    }
    expect(errors, "console and page errors including owned cleanup").toEqual([]);
  });
});

// Issue #4153: receive-contract coverage for the configuration diagnostic.
// Rust process fixtures prove detection, fallback, and attempt preservation.
test("Codex config failures retain cause, file, and recovery in launch error surfaces", async ({ page }, testInfo) => {
  test.skip(!BASE || !process.env.GWT_PLAYWRIGHT_CHECK_HOME,
    "requires browser-check checkout binaries and a fresh HOME");
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => {
    if (message.type() === "error") errors.push(message.text());
  });
  const theme = testInfo.project.name.includes("light") ? "light" : "dark";
  await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);
  const diagnostic = "codex_config_preflight: Codex CLI 0.148.0 could not load configuration (~/.codex/config.toml). invalid type: map, expected a boolean in features\nNext: run `codex doctor` with the same CODEX_HOME and fix the reported config error, or update Codex if the setting requires a newer CLI; relaunch after repair.";
  await withLiveGwtBackendLock(BASE, testInfo, async () => {
    await gotoLiveGwt(page, BASE);
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const drawer = page.locator("#notification-center");
    await page.evaluate(message => window.dispatchEvent(new CustomEvent("__gwt_test_inject", {
      detail: { kind: "issue_monitor_toast", level: "error", message,
        issue_number: 4153, notification_transition: null },
    })), diagnostic);
    await page.locator("#op-notifications-button").click();
    const row = drawer.locator(".notification-center__item").filter({ hasText: "codex_config_preflight:" });
    await expect(row).toHaveCount(1);
    await expect(row).toContainText("~/.codex/config.toml");
    await expect(row).toContainText("invalid type: map, expected a boolean in features");
    await expect(row).toContainText("codex doctor");
    await drawer.getByRole("button", { name: "Close notifications", exact: true }).click();
    await page.evaluate(message => window.dispatchEvent(new CustomEvent("__gwt_test_inject", {
      detail: { kind: "launch_wizard_open_error", title: "Codex configuration", message },
    })), diagnostic);
    const wizard = page.locator("#wizard-modal");
    await expect(wizard).toBeVisible();
    await expect(wizard.locator("#wizard-error")).toHaveText(diagnostic);
    const screenshot = testInfo.outputPath(`codex-config-error-${theme}.png`);
    await page.screenshot({ path: screenshot, fullPage: true });
    await testInfo.attach(`codex-config-error-${theme}`, {
      path: screenshot, contentType: "image/png",
    });
    await wizard.getByRole("button", { name: "Close", exact: true }).click();
    expect(errors, "console and page errors").toEqual([]);
  });
});

// Issue #5866: verify how the checkout-served UI presents the real event
// contracts. Runtime restore selection, initial reassignment, and continuing
// the remaining launches are covered in restore_launch_tests.rs; injecting
// receive events here does not prove those runtime behaviors.
test("automatic restore errors stay in Notifications while manual errors open a dialog", async ({ page }, testInfo) => {
  test.skip(!BASE || !process.env.GWT_PLAYWRIGHT_CHECK_HOME,
    "requires browser-check checkout binaries and a fresh HOME");
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => {
    if (message.type() === "error") errors.push(message.text());
  });
  const theme = testInfo.project.name.includes("light") ? "light" : "dark";
  await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);

  await withLiveGwtBackendLock(BASE, testInfo, async () => {
    // The permanent receive-injection hook is sufficient here. Enabling the
    // optional bridge installs a second listener for the same injected event.
    await gotoLiveGwt(page, BASE);
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);

    const bell = page.locator("#op-notifications-button");
    const drawer = page.locator("#notification-center");
    const rows = drawer.locator(".notification-center__item");
    const wizard = page.locator("#wizard-modal");
    await bell.click();
    await drawer.getByRole("button", { name: "Clear all", exact: true }).click();
    await drawer.getByRole("button", { name: "Close notifications", exact: true }).click();

    const restoreError = "Unable to restore saved Agent: configured executable is unavailable.";
    await page.evaluate(message => {
      window.dispatchEvent(new CustomEvent("__gwt_test_inject", {
        detail: {
          kind: "issue_monitor_toast", level: "error", message,
          issue_number: null, notification_transition: null,
        },
      }));
    }, restoreError);
    await expect(bell.locator(".op-rail__badge")).toHaveText("1");
    await expect(bell.locator(".op-rail__badge")).toHaveAttribute("data-has-error", "true");
    await expect(wizard).toBeHidden();
    await expect(page.locator(".modal-backdrop:visible")).toHaveCount(0);

    // Clicking the ordinary rail control also verifies the failure did not
    // install a blocking overlay over the workspace.
    await bell.click();
    await expect(drawer).toBeVisible();
    await expect(rows).toHaveCount(1);
    await expect(rows.first()).toHaveAttribute("data-level", "error");
    await expect(rows.first()).toContainText(restoreError);
    const notificationShot = testInfo.outputPath(`automatic-restore-notification-${theme}.png`);
    await page.screenshot({ path: notificationShot, fullPage: true });
    await testInfo.attach(`automatic-restore-notification-${theme}`, {
      path: notificationShot, contentType: "image/png",
    });
    await drawer.getByRole("button", { name: "Close notifications", exact: true }).click();

    const manualError = "Agent restore failed: configured executable is unavailable.";
    await page.evaluate(message => {
      window.dispatchEvent(new CustomEvent("__gwt_test_inject", {
        detail: { kind: "launch_wizard_open_error", title: "Agent restore", message },
      }));
    }, manualError);
    await expect(wizard).toBeVisible();
    await expect(wizard.getByRole("dialog", { name: /agent restore/i }))
      .toHaveAttribute("aria-modal", "true");
    await expect(wizard.locator("#wizard-error")).toHaveText(manualError);
    const manualShot = testInfo.outputPath(`manual-launch-error-${theme}.png`);
    await page.screenshot({ path: manualShot, fullPage: true });
    await testInfo.attach(`manual-launch-error-${theme}`, {
      path: manualShot, contentType: "image/png",
    });
    await wizard.getByRole("button", { name: "Close", exact: true }).click();
    await expect(wizard).toBeHidden();
    expect(errors, "console and page errors").toEqual([]);
  });
});

// Issue #4222: this checks presentation of backend fallback reasons; native
// conversation ownership and resume selection are exercised by Rust tests.
test("Issues fresh-session notifications retain each native conversation fallback reason", async ({ page }, testInfo) => {
  test.skip(!BASE || !process.env.GWT_PLAYWRIGHT_CHECK_HOME,
    "requires browser-check checkout binaries and a fresh HOME");
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => {
    if (message.type() === "error") errors.push(message.text());
  });
  const theme = testInfo.project.name.includes("light") ? "light" : "dark";
  await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);

  await withLiveGwtBackendLock(BASE, testInfo, async () => {
    await gotoLiveGwt(page, BASE);
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);

    const bell = page.locator("#op-notifications-button");
    const drawer = page.locator("#notification-center");
    const rows = drawer.locator(".notification-center__item");
    await bell.click();
    await drawer.getByRole("button", { name: "Clear all", exact: true }).click();
    await drawer.getByRole("button", { name: "Close notifications", exact: true }).click();

    const messages = [
      "Issues started a fresh session because native conversation is missing",
      "Issues started a fresh session because native conversation ownership does not match this worktree",
      "Issues started a fresh session because native conversation could not be verified",
      "Issues started a fresh session because native conversation is already held by window holder-window",
    ];
    for (const [index, message] of messages.entries()) {
      await page.evaluate(message => {
        window.dispatchEvent(new CustomEvent("__gwt_test_inject", {
          detail: {
            kind: "issue_monitor_toast", level: "warn", message,
            issue_number: 4222, notification_transition: null,
          },
        }));
      }, message);
      await expect(bell.locator(".op-rail__badge")).toHaveText(String(index + 1));
    }

    await bell.click();
    await expect(drawer).toBeVisible();
    await expect(rows).toHaveCount(messages.length);
    for (const message of messages) {
      const row = rows.filter({ hasText: message });
      await expect(row).toHaveCount(1);
      await expect(row).toHaveAttribute("data-level", "warn");
      await expect(row).toBeVisible();
    }
    await expect(page.locator(".modal-backdrop:visible")).toHaveCount(0);
    const screenshot = testInfo.outputPath(`native-conversation-fallback-reasons-${theme}.png`);
    await page.screenshot({ path: screenshot, fullPage: true });
    await testInfo.attach(`native-conversation-fallback-reasons-${theme}`, {
      path: screenshot, contentType: "image/png",
    });
    expect(errors, "console and page errors").toEqual([]);
  });
});

// Issue #3626: this receive contract covers warning presentation only. The
// Rust launch regression injects installer failure and proves PTY/input reachability;
// injecting a toast here does not prove a full degraded launch.
test("degraded resume warnings retain the reason and Start Work recovery without blocking workspace controls", async ({ page }, testInfo) => {
  test.skip(!BASE || !process.env.GWT_PLAYWRIGHT_CHECK_HOME,
    "requires browser-check checkout binaries and a fresh HOME");
  const errors: string[] = [];
  page.on("pageerror", error => errors.push(error.message));
  page.on("console", message => {
    if (message.type() === "error") errors.push(message.text());
  });
  const theme = testInfo.project.name.includes("light") ? "light" : "dark";
  await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);

  await withLiveGwtBackendLock(BASE, testInfo, async () => {
    await gotoLiveGwt(page, BASE);
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    const bell = page.locator("#op-notifications-button");
    const drawer = page.locator("#notification-center");
    const rows = drawer.locator('.notification-center__item[data-kind="issue-monitor"]');
    await bell.click();
    await drawer.getByRole("button", { name: "Clear all", exact: true }).click();
    await drawer.getByRole("button", { name: "Close notifications", exact: true }).click();

    const reason = "injected resume capability installation failure";
    const warning = `Resume degraded to unbound (no execution authority). Reason: ${reason}. Input remains available. Use Start Work / Launch to obtain execution authority before delivering changes.`;
    await page.evaluate(message => {
      window.dispatchEvent(new CustomEvent("__gwt_test_inject", {
        detail: {
          kind: "issue_monitor_toast", level: "warn", message,
          issue_number: 3626, notification_transition: null,
        },
      }));
    }, warning);
    await expect(bell.locator(".op-rail__badge")).toHaveText("1");
    await expect(page.locator(".modal-backdrop:visible")).toHaveCount(0);
    await expect(page.locator("#wizard-modal")).toBeHidden();

    // Ordinary notification controls remain usable after the warning. Closing
    // and reopening the drawer must retain its Issue, reason, and recovery path.
    await bell.click();
    await expect(drawer).toBeVisible();
    await expect(rows).toHaveCount(1);
    await expect(rows.first()).toHaveAttribute("data-level", "warn");
    await expect(rows.first().locator(".notification-center__title")).toHaveText("Issues #3626");
    await expect(rows.first().locator(".notification-center__message")).toHaveText(warning);
    await drawer.getByRole("button", { name: "Close notifications", exact: true }).click();
    await expect(drawer).toBeHidden();
    await bell.click();
    await expect(rows).toHaveCount(1);
    await expect(rows.first()).toContainText(reason);
    await expect(rows.first()).toContainText("Start Work / Launch");
    const screenshot = testInfo.outputPath(`degraded-resume-notification-${theme}.png`);
    await page.screenshot({ path: screenshot, fullPage: true });
    await testInfo.attach(`degraded-resume-notification-${theme}`, {
      path: screenshot, contentType: "image/png",
    });
    await drawer.getByRole("button", { name: "Close notifications", exact: true }).click();
    await expect(page.locator(".modal-backdrop:visible")).toHaveCount(0);
    expect(errors, "console and page errors").toEqual([]);
  });
});
