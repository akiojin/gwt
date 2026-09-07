/* SPEC-3671 — Issue window as the primary surface, amended by Issue #3884.
 *
 * An Issue Monitor auto-launch must not add a window to the canvas. It becomes an
 * `issue_preview` placement that the Issue window mirrors read-only in its right
 * pane, and only an explicit Windowize puts it back on the canvas. Since Issue
 * #3884 that placement is also visible without selection as a read-only status
 * row on the Issue row, is not drawn on the Fleet Minimap, and is broken out of
 * the Status Strip RUNNING cell as "N inline".
 *
 * The fixture serves the embedded frontend through Playwright routes and replaces
 * WebSocket with a deterministic backend, matching `tests/kanban.spec.ts`.
 */
import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

test.describe("Issue preview placement", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1600, height: 1000 },
  });

  // 受け入れシナリオ 1 / FR-004.
  test("an auto-launched agent does not add a window to the canvas", async ({ page }) => {
    const consoleErrors: string[] = [];
    const pageErrors: string[] = [];
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });
    page.on("pageerror", (error) => pageErrors.push(String(error)));

    await installEmbeddedRoutes(page);
    await installIssuePreviewBackend(page);

    await page.goto(APP_URL);

    await expect(page.locator(".workspace-window.surface-knowledge")).toBeVisible();
    await expect(page.locator(".workspace-window:visible")).toHaveCount(1);
    await expect(page.locator(".workspace-window.surface-terminal:visible")).toHaveCount(0);

    expect(consoleErrors).toEqual([]);
    expect(pageErrors).toEqual([]);
  });

  // 受け入れシナリオ 2 / FR-007 / FR-008.
  test("the Issue window mirrors the agent output read-only", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installIssuePreviewBackend(page);

    await page.goto(APP_URL);

    const preview = page.locator(".surface-knowledge .issue-preview");
    await expect(preview).toHaveCount(1);
    await expect(preview).toHaveAttribute("data-window-id", "tab-issue::agent-preview");
    await expect(preview.locator(".issue-preview-title")).toHaveText("Issue #3671 agent");

    const terminal = preview.locator(".issue-preview-terminal .terminal-root");
    await expect(terminal).toBeVisible();

    await page.evaluate(() => window.__emitAgentOutput("SPEC-3671 preview line"));
    await expect(terminal).toContainText("SPEC-3671 preview line");

    await terminal.click();
    await page.keyboard.type("rm -rf /");
    const inputMessages = await page.evaluate(() =>
      window.__knowledgeLoadMessages.filter((message) => message.kind === "terminal_input"),
    );
    expect(inputMessages).toEqual([]);
  });

  // 受け入れシナリオ 3 / FR-009.
  test("only the selected Issue's agent is mirrored", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installIssuePreviewBackend(page);

    await page.goto(APP_URL);

    await expect(page.locator(".surface-knowledge .issue-preview")).toHaveAttribute(
      "data-window-id",
      "tab-issue::agent-preview",
    );

    await page
      .locator(".surface-knowledge .knowledge-row[data-issue-number='3672'] .knowledge-row-select")
      .click();

    await expect(page.locator(".surface-knowledge .issue-preview")).toHaveCount(1);
    await expect(page.locator(".surface-knowledge .issue-preview")).toHaveAttribute(
      "data-window-id",
      "tab-issue::agent-preview-2",
    );
  });

  // 受け入れシナリオ 4 / FR-010.
  test("Windowize puts the mirrored agent on the canvas", async ({ page }) => {
    await installEmbeddedRoutes(page);
    await installIssuePreviewBackend(page);

    await page.goto(APP_URL);

    await expect(page.locator(".workspace-window:visible")).toHaveCount(1);
    await page
      .locator(".surface-knowledge .issue-preview [data-action='windowize-issue-preview']")
      .click();

    const undocked = await page.evaluate(() =>
      window.__knowledgeLoadMessages.filter(
        (message) => message.kind === "undock_agent_window",
      ),
    );
    expect(undocked).toHaveLength(1);
    expect(undocked[0].id).toBe("tab-issue::agent-preview");

    await expect(page.locator(".workspace-window.surface-terminal:visible")).toHaveCount(1);
    await expect(page.locator(".surface-knowledge .issue-preview")).toHaveCount(0);
  });

  // SPEC #3885 AC-11 / AC-12 / AC-13 — the Windowized agent is an Issue window.
  test("Windowize produces an Issue window that can fold back into the list", async ({
    page,
  }) => {
    const consoleErrors: string[] = [];
    const pageErrors: string[] = [];
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });
    page.on("pageerror", (error) => pageErrors.push(String(error)));

    await installEmbeddedRoutes(page);
    await installIssuePreviewBackend(page);

    await page.goto(APP_URL);

    await page
      .locator(".surface-knowledge .issue-preview [data-action='windowize-issue-preview']")
      .click();

    // AC-11: what lands on the canvas is an Issue window, not a bare terminal.
    const agentWindow = page.locator(
      ".workspace-window.surface-terminal[data-id='tab-issue::agent-preview']",
    );
    await expect(agentWindow).toBeVisible();
    const header = agentWindow.locator(".issue-window-header");
    await expect(header).toHaveCount(1);
    await expect(header).toHaveAttribute("data-issue-number", "3671");
    await expect(header.locator(".issue-window-header-number")).toHaveText("#3671");
    await expect(header.locator(".issue-window-header-title")).toHaveText("Issue #3671");
    await expect(header.locator(".issue-window-header-badge")).toHaveText("Running");
    await expect(header.locator(".issue-window-header-badge")).toHaveCount(1);
    expect(
      await header.locator("button[data-action]").count(),
    ).toBeLessThanOrEqual(2);
    // The terminal is still the window's own, and it is still interactive.
    await expect(agentWindow.locator(".window-body .terminal-root")).toBeVisible();

    // AC-12: the return control folds the window back into its Issue row.
    await header.locator("[data-action='return-to-list']").dispatchEvent("click");
    await expect
      .poll(() =>
        page.evaluate(() =>
          window.__knowledgeLoadMessages
            .filter((message) => message.kind === "dock_agent_window_to_issue")
            .map((message) => message.id),
        ),
      )
      .toEqual(["tab-issue::agent-preview"]);
    await expect(
      page.locator(".workspace-window.surface-terminal[data-id='tab-issue::agent-preview']:visible"),
    ).toHaveCount(0);
    await expect(page.locator(".surface-knowledge .issue-preview")).toHaveAttribute(
      "data-window-id",
      "tab-issue::agent-preview",
    );

    expect(consoleErrors).toEqual([]);
    expect(pageErrors).toEqual([]);
  });

  // 受け入れシナリオ 5 / FR-011.
  test("an errored agent is badged in the Issue row, not opened on the canvas", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installIssuePreviewBackend(page, { agentStatus: "error" });

    await page.goto(APP_URL);

    await expect(
      page.locator(".surface-knowledge .issue-preview .knowledge-monitor-chip"),
    ).toHaveText("Error");
    const badge = page.locator(".surface-knowledge [data-issue-number='3671'] .knowledge-row-badge");
    await expect(badge).toHaveText("Error");
    await expect(badge).toHaveAttribute("data-tone", "blocked");
    await expect(page.locator(".workspace-window:visible")).toHaveCount(1);
    await expect(page.locator(".workspace-window.surface-terminal:visible")).toHaveCount(0);
  });

  // 受け入れシナリオ 8 / FR-012 / FR-013 / T-025: the Work information and the
  // Work actions are reachable without ever opening the Work window.
  test("the Issue row carries Work state and Work actions without a Work window", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installIssuePreviewBackend(page);

    await page.goto(APP_URL);

    await expect(page.locator(".workspace-window.surface-work")).toHaveCount(0);

    // SPEC #3885 T-004: the Work state is folded into the row — the attention
    // reason and the PR are the two secondary items under the primary badge, and
    // the Work actions sit in the row's overflow menu while the agent is live.
    // `.issue-preview` in the detail pane also carries data-issue-number, so
    // scope every row assertion to `.knowledge-row`.
    const row = page.locator(".surface-knowledge .knowledge-row[data-issue-number='3671']");
    await expect(row.locator(".knowledge-row-work")).toHaveCount(0);
    await expect(row.locator(".knowledge-row-badge")).toHaveText("Running");
    const secondary = row.locator(".knowledge-row-secondary-item");
    await expect(secondary).toHaveCount(2);
    await expect(secondary.nth(0)).toHaveAttribute("data-kind", "reason");
    await expect(secondary.nth(0)).toHaveText("Waiting on review");
    await expect(secondary.nth(1)).toHaveText("PR #3699 · open");
    await expect(
      row.locator("button[data-action]:not(.knowledge-row-menu-list button)"),
    ).toHaveText(["Windowize"]);
    const menu = row.locator(".knowledge-row-menu");
    await expect(menu).toHaveCount(1);
    await menu.locator("summary").click();
    await expect(menu).toHaveAttribute("open", "");
    await expect(menu.locator('[data-action="continue-work"]')).toBeVisible();
    await expect(menu.locator('[data-action="continue-work"]')).toBeEnabled();
    await expect(menu.locator('[data-action="resume-work"]')).toBeEnabled();
    // The backend owns cleanup eligibility; a live agent keeps the action off.
    await expect(menu.locator('[data-action="cleanup-work"]')).toBeDisabled();

    // An Issue with no correlated Work row has no PR chip and no Work actions.
    const other = page.locator(".surface-knowledge .knowledge-row[data-issue-number='3672']");
    await expect(other.locator('.knowledge-row-secondary-item[data-key="pr"]')).toHaveCount(0);
    await expect(other.locator('[data-action="continue-work"]')).toHaveCount(0);

    // SPEC #3885 AC-5: one primary badge, at most two secondary items, at most
    // two visible actions on every row.
    for (const issue of ["3671", "3672", "3673", "3674"]) {
      const each = page.locator(`.surface-knowledge .knowledge-row[data-issue-number='${issue}']`);
      await expect(each.locator(".knowledge-row-badge")).toHaveCount(1);
      expect(await each.locator(".knowledge-row-secondary-item").count()).toBeLessThanOrEqual(2);
      const visibleActions = await each
        .locator("button[data-action]:not(.knowledge-row-menu-list button)")
        .count();
      expect(visibleActions).toBeLessThanOrEqual(2);
      await expect(each.locator(".knowledge-chip")).toHaveCount(0);
      await expect(each.locator(".knowledge-state-chip")).toHaveCount(0);
      await expect(each.locator(".knowledge-monitor-chip")).toHaveCount(0);
    }
  });

  // Issue #3884 AC-1 / AC-3 / AC-5: with three auto-launched agents and an
  // otherwise empty canvas, nothing suggests a vanished window — the minimap
  // shows only the Issue window, and RUNNING says where the agents are.
  test("Issue #3884: no minimap cell for auto-launched agents, and RUNNING explains itself", async ({
    page,
  }) => {
    const consoleErrors: string[] = [];
    const pageErrors: string[] = [];
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });
    page.on("pageerror", (error) => pageErrors.push(String(error)));
    await installEmbeddedRoutes(page);
    await installIssuePreviewBackend(page);

    await page.goto(APP_URL);

    await expect(page.locator(".workspace-window:visible")).toHaveCount(1);
    const cells = page.locator("#fleet-minimap .fleet-minimap__cell");
    await expect(cells).toHaveCount(1);
    await expect(cells.first()).toHaveAttribute("data-window-id", "tab-issue::issue-1");

    await expect(page.locator("#op-strip-running")).toHaveText("3");
    const inline = page.locator("#op-strip-running-inline");
    await expect(inline).toBeVisible();
    await expect(inline).toHaveText("3 inline");
    await expect(page.locator(".op-status-strip__cell--running")).toHaveAttribute(
      "title",
      "3 of 3 running agents are inline terminals in the Issue window",
    );

    // Windowize one: it gains a minimap cell and the breakdown drops.
    await page
      .locator(
        ".surface-knowledge [data-issue-number='3671'] .issue-agent-status [data-action='windowize-issue-preview']",
      )
      .click();
    await expect(page.locator(".workspace-window.surface-terminal:visible")).toHaveCount(1);
    await expect(page.locator("#fleet-minimap .fleet-minimap__cell")).toHaveCount(2);
    await expect(
      page.locator("#fleet-minimap .fleet-minimap__cell[data-window-id='tab-issue::agent-preview']"),
    ).toHaveCount(1);
    await expect(page.locator("#op-strip-running")).toHaveText("3");
    await expect(inline).toHaveText("2 inline");

    // SPEC #3885 T-005 / FR-012: the row keeps the Issue ↔ agent link as a
    // "Shown on canvas" face with no second input face for the PTY.
    const face = page.locator(
      ".surface-knowledge [data-issue-number='3671'] .issue-agent-status",
    );
    await expect(face).toHaveCount(1);
    await expect(face).toHaveClass(/is-on-canvas/);
    await expect(face).toContainText("Shown on canvas");
    await expect(face.locator(".terminal-root")).toHaveCount(0);
    await expect(face.locator("[data-action='windowize-issue-preview']")).toHaveCount(0);
    await expect(
      page.locator(".surface-knowledge [data-issue-number='3671'] .knowledge-row-badge"),
    ).toHaveText("Running");
    // The Windowized canvas window overlaps the Issue window in the fixture, so
    // dispatch the click instead of relying on hit-testing through it.
    await face.locator("[data-action='focus-canvas-window']").dispatchEvent("click");
    await expect
      .poll(() =>
        page.evaluate(() =>
          window.__knowledgeLoadMessages
            .filter((message) => message.kind === "focus_window")
            .map((message) => message.id)
            .at(-1),
        ),
      )
      .toBe("tab-issue::agent-preview");

    expect(consoleErrors).toEqual([]);
    expect(pageErrors).toEqual([]);
  });

  // Issue #3884 AC-6 (PM ruling): every launched Issue row carries a read-only
  // status row — name, state, last activity line, elapsed — without selection.
  test("Issue #3884: each launched Issue row shows a read-only agent status row", async ({
    page,
  }) => {
    const consoleErrors: string[] = [];
    const pageErrors: string[] = [];
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });
    page.on("pageerror", (error) => pageErrors.push(String(error)));
    await installEmbeddedRoutes(page);
    await installIssuePreviewBackend(page);

    await page.goto(APP_URL);

    const statusRows = page.locator(".surface-knowledge .issue-agent-status");
    await expect(statusRows).toHaveCount(3);
    for (const [issue, id] of [
      ["3671", "tab-issue::agent-preview"],
      ["3672", "tab-issue::agent-preview-2"],
      ["3673", "tab-issue::agent-preview-3"],
    ]) {
      const row = page.locator(
        `.surface-knowledge [data-issue-number='${issue}'] .issue-agent-status`,
      );
      await expect(row).toHaveAttribute("data-window-id", id);
      // SPEC #3885 AC-5: the agent state is the Issue row's single primary badge.
      await expect(row.locator(".knowledge-monitor-chip")).toHaveCount(0);
      await expect(
        page.locator(`.surface-knowledge [data-issue-number='${issue}'] .knowledge-row-badge`),
      ).toHaveText("Running");
      await expect(row.locator(".terminal-root")).toHaveCount(0);
    }
    await expect(
      page.locator(".surface-knowledge [data-issue-number='3674'] .issue-agent-status"),
    ).toHaveCount(0);
    // Unselected rows carry the status row too (3672 / 3673 are not selected).
    await expect(page.locator(".surface-knowledge .knowledge-row.selected")).toHaveCount(1);

    const first = page.locator(
      ".surface-knowledge [data-issue-number='3671'] .issue-agent-status",
    );
    await expect(first.locator(".issue-agent-status-title")).toHaveText("Issue #3671 agent");
    await expect(first.locator(".issue-agent-status-output")).toHaveText("Running cargo test");
    await expect(first.locator(".issue-agent-status-elapsed")).toHaveText("<1m");
    await expect(page.locator(".surface-knowledge .knowledge-list")).not.toContainText(
      /preview/i,
    );

    // Clicking the status row is not a selection gesture.
    await first.locator(".issue-agent-status-output").click();
    await expect(page.locator(".surface-knowledge .knowledge-row.selected")).toHaveAttribute(
      "data-issue-number",
      "3671",
    );

    expect(consoleErrors).toEqual([]);
    expect(pageErrors).toEqual([]);
  });

  // SPEC #3885 AC-14 / US-7 (Issue #4082 T-018): list ⇄ split without losing the PTY.
  test("Issue #4082: the Issue window switches between list and split views on the same PTY", async ({
    page,
  }) => {
    const consoleErrors: string[] = [];
    const pageErrors: string[] = [];
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });
    page.on("pageerror", (error) => pageErrors.push(String(error)));
    await installEmbeddedRoutes(page);
    await installIssuePreviewBackend(page);

    await page.goto(APP_URL);

    const root = page.locator(".surface-knowledge .issue-bridge-root");
    await expect(root).toHaveAttribute("data-view-mode", "list");
    await expect(page.locator(".surface-knowledge .issue-agent-status")).toHaveCount(3);
    await page.evaluate(() => window.__emitAgentOutput("split-view keeps this line"));
    await expect(page.locator(".surface-knowledge .issue-preview .terminal-root")).toContainText(
      "split-view keeps this line",
    );

    await page.locator(".surface-knowledge [data-issue-view='split']").click();
    await expect(root).toHaveAttribute("data-view-mode", "split");
    const pairs = page.locator(".surface-knowledge .issue-split-pair");
    await expect(pairs).toHaveCount(3);
    await expect(pairs.nth(0)).toHaveAttribute("data-issue-number", "3671");
    await expect(pairs.nth(0)).toHaveAttribute("data-window-id", "tab-issue::agent-preview");
    await expect(pairs.nth(0)).toHaveClass(/selected/);
    await expect(page.locator(".surface-knowledge .issue-agent-status")).toHaveCount(0);
    await expect(page.locator(".surface-knowledge .issue-preview")).toHaveCount(0);
    for (let index = 0; index < 3; index += 1) {
      await expect(pairs.nth(index).locator(".issue-split-terminal .terminal-root")).toBeVisible();
      await expect(pairs.nth(index).locator(".knowledge-row-badge")).toHaveText("Running");
    }
    // The scrollback moved with the runtime.
    const pairTerminal = pairs.nth(0).locator(".issue-split-terminal .terminal-root");
    await expect(pairTerminal).toContainText("split-view keeps this line");
    // The pair's terminal is interactive: keystrokes reach the PTY.
    await expect
      .poll(
        () =>
          page.evaluate(
            () => window.__gwtTerminalTestApi.metrics("tab-issue::agent-preview").isReady,
          ),
        { timeout: 15_000 },
      )
      .toBe(true);
    await pairTerminal.click();
    await page.keyboard.type("ls");
    await expect
      .poll(() =>
        page.evaluate(() =>
          window.__knowledgeLoadMessages
            .filter((message) => message.kind === "terminal_input")
            .map((message) => message.id),
        ),
      )
      .toContain("tab-issue::agent-preview");

    // Expand / shrink a pair in place (T-005).
    const sizeToggle = pairs.nth(0).locator("[data-action='toggle-pair-size']");
    await sizeToggle.click();
    await expect(pairs.nth(0)).toHaveAttribute("data-size", "expanded");
    await expect(pairs.nth(1)).toHaveAttribute("data-size", "normal");
    await pairs.nth(0).locator("[data-action='toggle-pair-size']").click();
    await expect(pairs.nth(0)).toHaveAttribute("data-size", "normal");

    // Back to the list: status rows, the read-only mirror, the selection.
    await page.locator(".surface-knowledge [data-issue-view='list']").click();
    await expect(root).toHaveAttribute("data-view-mode", "list");
    await expect(page.locator(".surface-knowledge .issue-split-pair")).toHaveCount(0);
    await expect(page.locator(".surface-knowledge .issue-agent-status")).toHaveCount(3);
    await expect(page.locator(".surface-knowledge .knowledge-row.selected")).toHaveAttribute(
      "data-issue-number",
      "3671",
    );
    const mirror = page.locator(".surface-knowledge .issue-preview");
    await expect(mirror).toHaveAttribute("data-window-id", "tab-issue::agent-preview");
    await expect(mirror.locator(".terminal-root")).toContainText("split-view keeps this line");
    const lifecycle = await page.evaluate(() =>
      window.__knowledgeLoadMessages
        .filter((message) => ["close_window", "stop_window", "restart_window"].includes(message.kind))
        .map((message) => message.kind),
    );
    expect(lifecycle).toEqual([]);

    expect(consoleErrors).toEqual([]);
    expect(pageErrors).toEqual([]);
  });

  // SPEC #3885 AC-15 / FR-015 (Issue #4082 T-019).
  test("Issue #4082: the agent window titlebar minimizes and opens the Issue; stop lives in the row menu", async ({
    page,
  }) => {
    const consoleErrors: string[] = [];
    const pageErrors: string[] = [];
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });
    page.on("pageerror", (error) => pageErrors.push(String(error)));
    await installEmbeddedRoutes(page);
    await installIssuePreviewBackend(page);

    await page.goto(APP_URL);

    await page
      .locator(".surface-knowledge .issue-preview [data-action='windowize-issue-preview']")
      .click();
    const agentWindow = page.locator(
      ".workspace-window.surface-terminal[data-id='tab-issue::agent-preview']",
    );
    await expect(agentWindow).toBeVisible();
    const titlebar = agentWindow.locator(".titlebar");
    await expect(titlebar.locator("[data-action='stop']")).toHaveCount(0);
    const minimize = titlebar.locator("[data-action='minimize-to-issue']");
    const openIssue = titlebar.locator("[data-action='open-issue']");
    await expect(minimize).toBeVisible();
    await expect(openIssue).toBeVisible();
    await expect(titlebar.locator("[data-action='close']")).toBeVisible();

    // The Issue popup opens the owning Issue's detail.
    const detailRequestsBefore = await page.evaluate(
      () =>
        window.__knowledgeLoadMessages.filter(
          (message) => message.kind === "select_knowledge_bridge_entry",
        ).length,
    );
    await openIssue.click();
    await expect
      .poll(() =>
        page.evaluate(() =>
          window.__knowledgeLoadMessages
            .filter((message) => message.kind === "select_knowledge_bridge_entry")
            .map((message) => message.number),
        ),
      )
      .toHaveLength(detailRequestsBefore + 1);
    const lastDetail = await page.evaluate(
      () =>
        window.__knowledgeLoadMessages
          .filter((message) => message.kind === "select_knowledge_bridge_entry")
          .at(-1)?.number,
    );
    expect(lastDetail).toBe(3671);

    // Minimize folds the window back into its Issue row (AC-12 path).
    await minimize.click();
    await expect
      .poll(() =>
        page.evaluate(() =>
          window.__knowledgeLoadMessages
            .filter((message) => message.kind === "dock_agent_window_to_issue")
            .map((message) => message.id),
        ),
      )
      .toEqual(["tab-issue::agent-preview"]);
    await expect(
      page.locator(".workspace-window.surface-terminal[data-id='tab-issue::agent-preview']:visible"),
    ).toHaveCount(0);
    await expect(page.locator(".surface-knowledge .issue-preview")).toHaveAttribute(
      "data-window-id",
      "tab-issue::agent-preview",
    );

    // Stop is offered only from the row's ⋯ menu.
    const row = page.locator(".surface-knowledge .knowledge-row[data-issue-number='3671']");
    await expect(row.locator(".issue-agent-status [data-action='stop-agent']")).toHaveCount(0);
    await row.locator(".knowledge-row-menu-trigger").click();
    await row.locator(".knowledge-row-menu [data-action='stop-agent']").click();
    await expect
      .poll(() =>
        page.evaluate(() =>
          window.__knowledgeLoadMessages
            .filter((message) => message.kind === "stop_window")
            .map((message) => message.id),
        ),
      )
      .toEqual(["tab-issue::agent-preview"]);

    expect(consoleErrors).toEqual([]);
    expect(pageErrors).toEqual([]);
  });

  // SPEC #3885 US-4 (Issue #4082 T-005): Windowize from the split view.
  test("Issue #4082: Windowize from the split view leaves a canvas face in the pair", async ({
    page,
  }) => {
    const consoleErrors: string[] = [];
    const pageErrors: string[] = [];
    page.on("console", (message) => {
      if (message.type() === "error") consoleErrors.push(message.text());
    });
    page.on("pageerror", (error) => pageErrors.push(String(error)));
    await installEmbeddedRoutes(page);
    await installIssuePreviewBackend(page);

    await page.goto(APP_URL);
    await page.locator(".surface-knowledge [data-issue-view='split']").click();
    const pair = page.locator(".surface-knowledge .issue-split-pair[data-issue-number='3671']");
    await expect(pair.locator(".issue-split-terminal .terminal-root")).toBeVisible();

    await pair.locator("[data-action='windowize-issue-preview']").click();
    const agentWindow = page.locator(
      ".workspace-window.surface-terminal[data-id='tab-issue::agent-preview']",
    );
    await expect(agentWindow).toBeVisible();
    await expect(agentWindow.locator(".window-body .terminal-root")).toBeVisible();
    await expect(pair).toHaveClass(/is-on-canvas/);
    await expect(pair.locator(".issue-split-placeholder")).toContainText("Shown on canvas");
    await expect(pair.locator(".terminal-root")).toHaveCount(0);
    await expect(pair.locator("[data-action='focus-canvas-window']")).toBeVisible();
    await expect(page.locator(".surface-knowledge .issue-split-pair")).toHaveCount(3);

    // Minimizing from the titlebar brings the terminal back into the pair.
    await agentWindow.locator(".titlebar [data-action='minimize-to-issue']").click();
    await expect(pair).not.toHaveClass(/is-on-canvas/);
    await expect(pair.locator(".issue-split-terminal .terminal-root")).toBeVisible();

    expect(consoleErrors).toEqual([]);
    expect(pageErrors).toEqual([]);
  });

  // SPEC #3885 T-020 (Issue #4082): the backend's agent start time drives the elapsed label.
  test("Issue #4082: the status row elapsed time follows the backend agent start time", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installIssuePreviewBackend(page);

    await page.goto(APP_URL);
    const elapsed = page.locator(
      ".surface-knowledge [data-issue-number='3671'] .issue-agent-status-elapsed",
    );
    await expect(elapsed).toHaveText("<1m");

    await page.evaluate(() =>
      window.__patchWindow("tab-issue::agent-preview", {
        runtime_started_at_ms: Date.now() - 125 * 60_000,
      }),
    );
    await expect(elapsed).toHaveText("2h 05m");
    await expect(
      page.locator(".surface-knowledge [data-issue-number='3672'] .issue-agent-status-elapsed"),
    ).toHaveText("<1m");
  });
});

async function installIssuePreviewBackend(page, { agentStatus = "running" } = {}) {
  await page.addInitScript(
    ({ agentStatus: status }) => {
      window.__knowledgeLoadMessages = [];

      const issueWindow = {
        id: "tab-issue::issue-1",
        title: "Issue",
        preset: "issue",
        geometry: { x: 40, y: 60, width: 1400, height: 860 },
        z_index: 1,
        status: "running",
        persist: true,
        purpose_title: null,
        dynamic_title: null,
        dynamic_title_detail: null,
        agent_id: null,
        agent_color: null,
        tab_group_id: null,
        tab_group_active: false,
      };

      const agentWindow = (id, issueNumber, title) => ({
        id,
        title,
        preset: "agent",
        geometry: { x: 120, y: 120, width: 1280, height: 800 },
        z_index: 2,
        status,
        persist: true,
        purpose_title: null,
        dynamic_title: null,
        dynamic_title_detail: null,
        agent_id: "codex",
        agent_color: null,
        tab_group_id: null,
        tab_group_active: false,
        // SPEC #3885 FR-011: the Issue a window belongs to is durable and survives
        // Windowize, so the canvas face can carry the Issue header.
        linked_issue_number: issueNumber,
        placement: {
          kind: "issue_preview",
          issue_window_id: "tab-issue::issue-1",
          issue_number: issueNumber,
        },
      });

      let windows = [
        issueWindow,
        {
          ...agentWindow("tab-issue::agent-preview", 3671, "Issue #3671 agent"),
          dynamic_title_detail: "Running cargo test",
        },
        agentWindow("tab-issue::agent-preview-2", 3672, "Issue #3672 agent"),
        agentWindow("tab-issue::agent-preview-3", 3673, "Issue #3673 agent"),
      ];

      const workspaceState = () => ({
        kind: "workspace_state",
        workspace: {
          app_version: "playwright",
          tabs: [
            {
              id: "tab-issue",
              title: "Fixture Project",
              project_root: "/fixture",
              kind: "git",
              workspace: {
                viewport: { x: 0, y: 0, zoom: 1 },
                windows: windows.map((entry) => ({ ...entry })),
              },
            },
          ],
          active_tab_id: "tab-issue",
          recent_projects: [],
        },
      });

      const entries = [3671, 3672, 3673, 3674].map((number) => ({
        number,
        title: `Issue #${number}`,
        state: "open",
        meta: "Auto-launch fixture",
        labels: ["bug"],
        linked_branch_count: 0,
        match_score: 100,
        phase: null,
        has_unknown_phase: false,
        is_spec: false,
        monitor_state: "launched",
        queue_position: null,
        exclusion_reason: null,
        // SPEC-3671 FR-012: the backend-computed Issue -> Work correlation.
        related_work_refs: [
          { id: `work-${number}`, branch: `work/issue-${number}`, updated_at: "" },
        ],
      }));

      // SPEC-3671 FR-012: the active Work projection the frontend already
      // receives. The Issue row joins it; it is never re-derived.
      const activeWorkProjection = {
        id: "projection-1",
        title: "Fixture Project",
        status_category: "active",
        status_text: "",
        board_refs: [],
        journal_entries: [],
        works: [],
        agents: [],
        unassigned_agents: [],
        active_works: [
          {
            id: "work-3671",
            title: "Issue window as the primary surface",
            status_category: "blocked",
            status_text: "Implementing P4",
            blocked_reason: "Waiting on review",
            lifecycle_state: "active",
            active_agents: 1,
            blocked_agents: 1,
            branch: "work/issue-3671",
            worktree_path: "/fixture/work/issue-3671",
            pr_number: 3699,
            pr_url: "https://example.invalid/pull/3699",
            pr_state: "open",
            board_refs: [],
            agents: [],
            works: [],
            cleanup_candidate: null,
            cleanup_blocked_reason: "live_agent",
            updated_at: "2026-08-19T00:00:00Z",
          },
        ],
      };

      class FixtureWebSocket extends EventTarget {
        static CONNECTING = 0;
        static OPEN = 1;
        static CLOSING = 2;
        static CLOSED = 3;

        constructor(url) {
          super();
          this.url = url;
          this.readyState = FixtureWebSocket.CONNECTING;
          window.__fixtureSocket = this;
          setTimeout(() => {
            this.readyState = FixtureWebSocket.OPEN;
            this.dispatchEvent(new Event("open"));
          }, 0);
        }

        send(raw) {
          const message = JSON.parse(raw);
          window.__knowledgeLoadMessages.push(message);
          if (message.kind === "frontend_ready") {
            this.emit(workspaceState());
            this.emit({
              kind: "active_work_projection",
              projection: activeWorkProjection,
            });
            return;
          }
          if (message.kind === "load_knowledge_bridge") {
            this.emit({
              kind: "knowledge_entries",
              id: message.id,
              knowledge_kind: message.knowledge_kind,
              request_id: message.request_id,
              entries,
              selected_number: 3671,
              empty_message: null,
              refresh_enabled: true,
            });
            return;
          }
          if (message.kind === "select_knowledge_bridge_entry") {
            this.emit({
              kind: "knowledge_detail",
              id: message.id,
              knowledge_kind: message.knowledge_kind,
              request_id: message.request_id,
              detail: {
                number: message.number,
                title: `Issue #${message.number}`,
                state: "open",
                subtitle: "Cached Issue detail",
                labels: ["bug"],
                launch_issue_number: message.number,
                sections: [
                  {
                    title: "Description",
                    body: "Issue preview fixture body",
                    body_html: "<p>Issue preview fixture body</p>",
                  },
                ],
              },
            });
            return;
          }
          if (message.kind === "undock_agent_window") {
            // The backend answers a Windowize by moving the window to the canvas.
            windows = windows.map((entry) =>
              entry.id === message.id
                ? { ...entry, placement: { kind: "canvas" } }
                : entry,
            );
            this.emit(workspaceState());
            return;
          }
          if (message.kind === "dock_agent_window_to_issue") {
            // SPEC #3885 FR-012: the inverse transition. The window already knows
            // its Issue, so the backend resolves the host Issue window itself.
            windows = windows.map((entry) =>
              entry.id === message.id
                ? {
                    ...entry,
                    placement: {
                      kind: "issue_preview",
                      issue_window_id: "tab-issue::issue-1",
                      issue_number: entry.linked_issue_number,
                    },
                  }
                : entry,
            );
            this.emit(workspaceState());
          }
        }

        close() {
          this.readyState = FixtureWebSocket.CLOSED;
          this.dispatchEvent(new CloseEvent("close"));
        }

        emit(payload) {
          setTimeout(() => {
            this.dispatchEvent(
              new MessageEvent("message", { data: JSON.stringify(payload) }),
            );
          }, 0);
        }
      }

      window.__emitAgentOutput = (text) => {
        window.__fixtureSocket?.emit({
          kind: "terminal_output",
          id: "tab-issue::agent-preview",
          data_base64: btoa(`${text}\r\n`),
        });
      };
      // Issue #4082 (SPEC #3885 T-020): let a spec change one window's wire fields
      // the way a backend broadcast would.
      window.__patchWindow = (id, patch) => {
        windows = windows.map((entry) => (entry.id === id ? { ...entry, ...patch } : entry));
        window.__fixtureSocket?.emit(workspaceState());
      };
      // Expose the terminal test bridge (readiness / buffer probes).
      window.__gwtPlaywrightTestBridge = true;

      Object.defineProperty(window, "WebSocket", {
        configurable: true,
        value: FixtureWebSocket,
      });
    },
    { agentStatus },
  );
}
