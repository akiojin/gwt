/** Issue #5594 / SPEC #1921: compact version columns in real headed Chromium. */
import { expect, test } from "@playwright/test";
import {
  gotoLiveGwt,
  openLiveGwtProject,
  withLiveGwtBackendLock,
} from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "";
const PROJECT = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT ?? process.cwd();
const metadataError = "Fixture update metadata unavailable: distribution could not be verified.";
const installed = [
  { id: "claude", name: "Claude Code", raw: "2.1.301 (Claude Code)", version: "2.1.301" },
  { id: "codex", name: "Codex", raw: "codex-cli 0.162.1", version: "0.162.1" },
  { id: "grok", name: "Grok Build", raw: "grok 1.0.46 (2765805b9442) [stable]", version: "1.0.46" },
  { id: "openclaw", name: "OpenClaw", raw: "OpenClaw 2026.3.23-2 (7ffe7e4)", version: "2026.3.23-2" },
  { id: "hermes", name: "Hermes Agent", raw: "Unknown (version unavailable)", version: "Unknown" },
];
const snapshot = {
  kind: "supported_agent_list",
  auto_update: false,
  maintenance_pending: false,
  agents: [
    ...installed.map(agent => ({
      id: agent.id,
      name: agent.name,
      installed: true,
      installed_version: agent.raw,
      available_version: agent.id === "hermes" ? null : "2099.1.0",
      update_check_error: agent.id === "hermes" ? metadataError : null,
      update_available: agent.id !== "hermes",
      up_to_date: false,
      install_supported: true,
    })),
    {
      id: "agy", name: "Antigravity CLI", installed: false, installed_version: null,
      available_version: "1.20.0", update_check_error: null,
      update_available: false, up_to_date: false, install_supported: true,
    },
  ],
};

test.describe("Supported Agents version layout (isolated live backend)", () => {
  test.skip(!BASE, "requires browser-check checkout and fresh HOME");
  test.use({ viewport: { width: 1440, height: 1000 } });
  test.setTimeout(120_000);

  test("keeps banners in tooltips and aligns short versions and actions", async ({ page }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      const theme = info.project.name.endsWith("light") ? "light" : "dark";
      await page.addInitScript(theme => localStorage.setItem("gwt:ui:theme", theme), theme);

      // Only the catalog is deterministic. Settings creation, project routing,
      // assets and every other protocol frame use the checkout's real backend.
      // Replace late server snapshots too, so detection cannot overwrite it.
      let catalogRequests = 0;
      const requests: Record<string, unknown>[] = [];
      await page.routeWebSocket(/\/ws(?:\?|$)/, socket => {
        const server = socket.connectToServer();
        socket.onMessage(message => {
          const payload = typeof message === "string" ? JSON.parse(message) : null;
          if (payload?.kind === "list_supported_agents") {
            catalogRequests += 1;
            socket.send(JSON.stringify(snapshot));
          } else if (["maintain_supported_agent", "set_agent_auto_update"].includes(payload?.kind)) {
            requests.push(payload);
          } else server.send(message);
        });
        server.onMessage(message => {
          const payload = typeof message === "string" ? JSON.parse(message) : null;
          if (!["supported_agent_list", "supported_agent_maintenance"].includes(payload?.kind)) socket.send(message);
        });
      });
      await gotoLiveGwt(page, BASE, { enableTestBridge: true });
      await openLiveGwtProject(page, PROJECT);
      await page.bringToFront();
      await page.evaluate(() => document.dispatchEvent(
        new CustomEvent("settings:open", { detail: { target: "supported-agents" } }),
      ));
      const settings = page.locator('.workspace-window[data-preset="settings"]');
      await expect(settings).toBeVisible();
      await expect(settings.getByRole("tab", { name: "Supported Agents", exact: true }))
        .toHaveAttribute("aria-selected", "true");
      const panel = settings.locator('[data-settings-panel="supported-agents"]');
      await expect(panel).toBeVisible();
      await expect.poll(() => catalogRequests).toBeGreaterThan(0);
      const table = panel.getByRole("table", { name: "Supported agents and installed versions" });
      await expect(table.locator("tbody tr")).toHaveCount(snapshot.agents.length);
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);

      for (const agent of installed) {
        const row = table.locator(`[data-agent-id="${agent.id}"]`);
        await expect(row.locator("td").nth(2)).toHaveText(agent.version);
        await expect(row.locator("td").nth(2)).toHaveAttribute("title", agent.raw);
        if (agent.id !== "hermes") {
          const update = row.getByRole("button", { name: `Update ${agent.name}`, exact: true });
          await expect(update).toHaveText("Update");
          await expect(update).toBeEnabled();
        }
      }
      const unknown = table.locator('[data-agent-id="hermes"]');
      await expect(unknown.locator("td").nth(3)).toHaveText("Unknown");
      await expect(unknown.locator("td").nth(3)).toHaveAttribute("title", metadataError);
      await expect(unknown).not.toContainText(metadataError);
      await expect(unknown.getByRole("button")).toHaveCount(0);
      const missing = table.locator('[data-agent-id="agy"]');
      await expect(missing.locator("td").nth(2)).toHaveText("Not installed");
      const install = missing.getByRole("button", { name: "Install Antigravity CLI", exact: true });
      await expect(install).toHaveText("Install");
      await expect(install).toBeEnabled();

      const layout = await table.evaluate(element => {
        const bounds = (node: Element) => {
          const rect = node.getBoundingClientRect();
          return { left: rect.left, right: rect.right, width: rect.width, height: rect.height };
        };
        const headers = Array.from(element.querySelectorAll("th"), bounds);
        const rows = Array.from(element.querySelectorAll("tbody tr"), row => ({
          ...bounds(row),
          cells: Array.from(row.querySelectorAll("td"), cell => ({
            ...bounds(cell), nowrap: getComputedStyle(cell).whiteSpace,
          })),
        }));
        const buttons = Array.from(element.querySelectorAll("button"), button => ({
          ...bounds(button), nowrap: getComputedStyle(button).whiteSpace,
          clientWidth: button.clientWidth, scrollWidth: button.scrollWidth,
        }));
        return { table: bounds(element), headers, rows, buttons };
      });
      for (const row of layout.rows) {
        expect(Math.abs(row.height - layout.rows[0].height), "all catalog rows have the same height")
          .toBeLessThanOrEqual(1);
        row.cells.forEach((cell, index) => {
          expect(cell.nowrap).toBe("nowrap");
          expect(Math.abs(cell.left - layout.headers[index].left)).toBeLessThanOrEqual(1);
          expect(Math.abs(cell.right - layout.headers[index].right)).toBeLessThanOrEqual(1);
          expect(cell.left).toBeGreaterThanOrEqual(layout.table.left - 1);
          expect(cell.right).toBeLessThanOrEqual(layout.table.right + 1);
        });
      }
      expect(layout.buttons).toHaveLength(5);
      for (const button of layout.buttons) {
        expect(Math.abs(button.width - layout.buttons[0].width), "Install and Update use equal widths")
          .toBeLessThanOrEqual(1);
        expect(button.nowrap).toBe("nowrap");
        expect(button.scrollWidth).toBeLessThanOrEqual(button.clientWidth);
        expect(button.left).toBeGreaterThanOrEqual(layout.headers[4].left);
        expect(button.right).toBeLessThanOrEqual(layout.headers[4].right);
      }
      await info.attach(`supported-agents-layout-${theme}`, {
        body: JSON.stringify(layout, null, 2), contentType: "application/json",
      });
      const automatic = panel.getByRole("checkbox", { name: "Automatically update agents" });
      await automatic.focus();
      await panel.hover();
      for (const suspendRaf of [false, true]) {
        const result = await panel.evaluate(async (panel, { snapshot, suspendRaf }) => {
          const before = panel.innerHTML;
          const automatic = panel.querySelector<HTMLInputElement>('[aria-label="Automatically update agents"]')!;
          const originalRaf = window.requestAnimationFrame;
          if (suspendRaf) window.requestAnimationFrame = () => 0;
          let renders = 0;
          let observer: MutationObserver | undefined;
          let timer: ReturnType<typeof setTimeout>;
          const started = performance.now();
          try {
            const rendered = new Promise<void>((resolve, reject) => {
              observer = new MutationObserver(records => {
                renders += records.flatMap(record => [...record.addedNodes])
                  .filter(node => node instanceof HTMLTableElement).length;
                if (panel.querySelector('[data-agent-id="codex"] td:nth-child(3)')?.textContent === "0.162.50") resolve();
              });
              observer.observe(panel, { childList: true });
              timer = setTimeout(() => reject(new Error("Supported Agents frame exceeded one second")), 1000);
            });
            for (let index = 1; index <= 50; index++) {
              window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: {
                ...snapshot, agents: snapshot.agents.map(agent => agent.id === "codex"
                  ? { ...agent, installed_version: `codex-cli 0.162.${index}` } : agent),
              } }));
            }
            const unchangedBeforeFrame = panel.innerHTML === before;
            automatic.checked = true;
            automatic.dispatchEvent(new Event("change", { bubbles: true }));
            await rendered;
            return { unchangedBeforeFrame, renders, elapsed: performance.now() - started,
              checked: automatic.checked, sameCheckbox: panel.querySelector('[aria-label="Automatically update agents"]') === automatic,
              focused: document.activeElement === automatic, hovered: panel.matches(":hover") };
          } finally {
            clearTimeout(timer!);
            observer?.disconnect();
            window.requestAnimationFrame = originalRaf;
          }
        }, { snapshot, suspendRaf });
        expect(result).toMatchObject({ unchangedBeforeFrame: true, renders: 1, checked: true,
          sameCheckbox: true, focused: true, hovered: true });
        expect(result.elapsed).toBeLessThan(1000);
        await info.attach(suspendRaf ? "supported-agents-fallback" : "supported-agents-frame", {
          body: JSON.stringify(result), contentType: "application/json",
        });
        await page.evaluate(() => window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: {
          kind: "supported_agent_maintenance", agent_id: "", pending: false, success: false, message: "Fixture save denied",
        } })));
        await expect(automatic).not.toBeChecked();
        await expect(panel.getByRole("alert")).toContainText("Fixture save denied");
      }
      await expect.poll(() => requests.filter(request => request.kind === "set_agent_auto_update"))
        .toEqual([{ kind: "set_agent_auto_update", enabled: true }, { kind: "set_agent_auto_update", enabled: true }]);
      await install.click();
      await expect(install).toBeDisabled();
      await expect(automatic).toBeDisabled();
      await expect.poll(() => requests.filter(request => request.kind === "maintain_supported_agent"))
        .toEqual([{ kind: "maintain_supported_agent", agent_id: "agy", action: "install" }]);
      await page.evaluate(snapshot => window.dispatchEvent(new CustomEvent("__gwt_test_inject", { detail: snapshot })), snapshot);
      await expect(install).toBeEnabled();
      await expect(automatic).not.toBeChecked();
      const screenshot = info.outputPath(`supported-agents-versions-${theme}.png`);
      await settings.screenshot({ path: screenshot });
      await info.attach(`supported-agents-versions-${theme}`, { path: screenshot, contentType: "image/png" });
      expect(errors, "zero console and page errors").toEqual([]);
    });
  });
});
