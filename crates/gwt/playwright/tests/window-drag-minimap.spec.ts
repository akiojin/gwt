import { expect, test } from "@playwright/test";
import { APP_URL, installEmbeddedRoutes } from "./_helpers/embedded-frontend";

// Issue #3364 — window drag & drop geometry sync.
//
// Root cause chain (v9.69.0 live investigation): the drag pointerup only sent
// the geometry with the client model's (possibly stale) base revision and,
// unlike the resize path, never armed the local edit guard nor updated the
// model. Under a backlogged `workspace_state` queue the server rejected the
// stale-base update and rebroadcast the OLD state, which the unguarded client
// re-applied: the window snapped back and the Fleet Minimap (which renders
// the MODEL, not the DOM) never showed the drop.
//
// Contract under test:
// 1. a drop commits an EXPLICIT user placement: the send omits
//    `base_geometry_revision` so the server applies it unconditionally;
// 2. the minimap cell reflects the drop IMMEDIATELY (before any echo);
// 3. backlogged stale `workspace_state` broadcasts (any revision) cannot snap
//    the window or the minimap back while the commit echo is in flight;
// 4. the commit's own echo releases the guard and the state converges.
//
// Stale/echo states carry a changed window TITLE as a render marker: titles
// are not geometry-guarded, so once the title landed in the DOM the state has
// provably flowed through the full render path.
test.describe("Window drag → geometry sync + Fleet Minimap", () => {
  test.use({
    deviceScaleFactor: 1,
    viewport: { width: 1440, height: 900 },
  });

  test("a drop survives stale broadcasts and reaches the minimap immediately", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installDragFixtureBackend(page);
    await page.goto(APP_URL);

    const windowFrame = page.locator(".workspace-window[data-id='agent-1']");
    const titlebar = windowFrame.locator(".titlebar");
    const titleText = windowFrame.locator(".title-text");
    await expect(windowFrame).toBeVisible();

    const minimap = page.locator("#fleet-minimap");
    await expect(minimap).toBeVisible({ timeout: 10_000 });
    const cell = minimap.locator('.fleet-minimap__cell[data-window-id="agent-1"]');
    await expect(cell).toHaveCount(1);
    const cellLeftBefore = await cell.evaluate((el) => parseFloat(el.style.left));
    const cellTopBefore = await cell.evaluate((el) => parseFloat(el.style.top));

    await dragTitlebarBy(page, titlebar, { pointerId: 41, dx: 200, dy: 150 });

    // The drop position lands in the DOM (fixture starts at 120,100; zoom 1).
    await expect
      .poll(() =>
        windowFrame.evaluate((element) => ({
          left: element.style.left,
          top: element.style.top,
        })),
      )
      .toEqual({ left: "320px", top: "250px" });

    // The commit is an explicit user placement: geometry matches the drop and
    // the base revision is OMITTED (the server must not discard the drop just
    // because this client's model lagged behind under load).
    const commit = await page.evaluate(() => {
      const sends = (window as any).__dragFixture.sends;
      return (
        sends.filter((msg) => msg.kind === "update_window_geometry").at(-1) ??
        null
      );
    });
    expect(commit).not.toBeNull();
    expect(commit.geometry).toMatchObject({ x: 320, y: 250 });
    expect("base_geometry_revision" in commit).toBe(false);

    // The minimap reflects the drop IMMEDIATELY — no server echo has been
    // emitted yet, so this proves the optimistic model sync.
    await expect
      .poll(() => cell.evaluate((el) => parseFloat(el.style.left)))
      .toBeGreaterThan(cellLeftBefore);
    await expect
      .poll(() => cell.evaluate((el) => parseFloat(el.style.top)))
      .toBeGreaterThan(cellTopBefore);
    const cellLeftAfterDrop = await cell.evaluate((el) => parseFloat(el.style.left));

    // A backlogged STALE broadcast (old geometry, advanced revision — the
    // high-load queue scenario) must not snap the window or the cell back.
    await page.evaluate(() => {
      (window as any).__dragFixture.emitState({
        x: 120,
        y: 100,
        revision: 7,
        title: "Agent (stale)",
      });
    });
    await expect(titleText).toHaveText("Agent (stale)");
    expect(
      await windowFrame.evaluate((element) => ({
        left: element.style.left,
        top: element.style.top,
      })),
    ).toEqual({ left: "320px", top: "250px" });
    expect(await cell.evaluate((el) => parseFloat(el.style.left))).toBe(
      cellLeftAfterDrop,
    );

    // The commit's own echo converges the state and releases the guard.
    await page.evaluate(() => {
      (window as any).__dragFixture.emitState({
        x: 320,
        y: 250,
        revision: 8,
        title: "Agent (echo)",
      });
    });
    await expect(titleText).toHaveText("Agent (echo)");
    expect(
      await windowFrame.evaluate((element) => ({
        left: element.style.left,
        top: element.style.top,
      })),
    ).toEqual({ left: "320px", top: "250px" });
    expect(await cell.evaluate((el) => parseFloat(el.style.left))).toBe(
      cellLeftAfterDrop,
    );
  });

  test("a server-driven window move reaches the minimap cell (echo path)", async ({
    page,
  }) => {
    await installEmbeddedRoutes(page);
    await installDragFixtureBackend(page);
    await page.goto(APP_URL);

    const minimap = page.locator("#fleet-minimap");
    await expect(minimap).toBeVisible({ timeout: 10_000 });
    const cell = minimap.locator('.fleet-minimap__cell[data-window-id="agent-1"]');
    await expect(cell).toHaveCount(1);
    const cellLeftBefore = await cell.evaluate((el) => parseFloat(el.style.left));

    // No local gesture: a plain server-side move must flow model → minimap.
    await page.evaluate(() => {
      (window as any).__dragFixture.emitState({ x: 520, y: 400, revision: 6 });
    });

    await expect
      .poll(() => cell.evaluate((el) => parseFloat(el.style.left)))
      .toBeGreaterThan(cellLeftBefore);
    await expect(
      page.locator(".workspace-window[data-id='agent-1']"),
    ).toHaveCSS("left", "520px");
  });
});

async function dragTitlebarBy(page, titlebar, { pointerId, dx, dy }) {
  const box = await titlebar.boundingBox();
  expect(box).not.toBeNull();
  const startX = Math.round(box!.x + box!.width / 2);
  const startY = Math.round(box!.y + box!.height / 2);

  await titlebar.dispatchEvent("pointerdown", {
    pointerId,
    pointerType: "mouse",
    button: 0,
    buttons: 1,
    clientX: startX,
    clientY: startY,
  });
  // Two moves: the first crosses the 2px moved-threshold, the second lands on
  // the drop point (mirrors the agent-resize gesture pattern).
  for (const [mx, my] of [
    [startX + Math.round(dx / 2), startY + Math.round(dy / 2)],
    [startX + dx, startY + dy],
  ]) {
    await page.evaluate(
      ({ x, y, id }) => {
        window.dispatchEvent(
          new PointerEvent("pointermove", {
            pointerId: id,
            pointerType: "mouse",
            buttons: 1,
            clientX: x,
            clientY: y,
            bubbles: true,
          }),
        );
      },
      { x: mx, y: my, id: pointerId },
    );
  }
  await page.evaluate(
    ({ x, y, id }) => {
      window.dispatchEvent(
        new PointerEvent("pointerup", {
          pointerId: id,
          pointerType: "mouse",
          button: 0,
          buttons: 0,
          clientX: x,
          clientY: y,
          bubbles: true,
        }),
      );
    },
    { x: startX + dx, y: startY + dy, id: pointerId },
  );
}

async function installDragFixtureBackend(page) {
  await page.addInitScript(() => {
    function stateMsg({ x, y, revision, title }) {
      return {
        kind: "workspace_state",
        workspace: {
          app_version: "playwright",
          tabs: [
            {
              id: "tab-1",
              title: "Drag Fixture",
              project_root: "/fixture",
              kind: "git",
              workspace: {
                viewport: { x: 0, y: 0, zoom: 1 },
                windows: [
                  {
                    id: "agent-1",
                    title: title || "Agent",
                    preset: "agent",
                    geometry: { x, y, width: 520, height: 300 },
                    geometry_revision: revision,
                    z_index: 1,
                    status: "running",
                    persist: true,
                    purpose_title: null,
                    dynamic_title: null,
                    dynamic_title_detail: null,
                    agent_id: "agent-1",
                    agent_color: "cyan",
                    tab_group_id: null,
                    tab_group_active: false,
                  },
                ],
              },
            },
          ],
          active_tab_id: "tab-1",
          recent_projects: [],
        },
      };
    }

    const fixture = {
      sends: [],
      socket: null,
      emitState(state) {
        fixture.socket?.emit(stateMsg(state));
      },
    };
    (window as any).__dragFixture = fixture;

    class FixtureWebSocket extends EventTarget {
      constructor(url) {
        super();
        this.url = url;
        this.readyState = FixtureWebSocket.CONNECTING;
        fixture.socket = this;
        setTimeout(() => {
          this.readyState = FixtureWebSocket.OPEN;
          this.dispatchEvent(new Event("open"));
          this.emit(stateMsg({ x: 120, y: 100, revision: 5 }));
        }, 0);
      }

      send(data) {
        try {
          fixture.sends.push(JSON.parse(data));
        } catch {
          // ignore non-JSON frames
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
    (FixtureWebSocket as any).CONNECTING = 0;
    (FixtureWebSocket as any).OPEN = 1;
    (FixtureWebSocket as any).CLOSING = 2;
    (FixtureWebSocket as any).CLOSED = 3;

    Object.defineProperty(window, "WebSocket", {
      configurable: true,
      value: FixtureWebSocket,
    });
  });
}
