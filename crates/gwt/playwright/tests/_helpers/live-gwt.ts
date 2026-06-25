import type { Page } from "@playwright/test";

type LiveGwtOptions = {
  enableTestBridge?: boolean;
  keepPresetModal?: boolean;
  suppressUpdateApplyStart?: boolean;
};

export async function gotoLiveGwt(
  page: Page,
  base: string,
  options: LiveGwtOptions = {},
): Promise<void> {
  await page.addInitScript(({ enableTestBridge, suppressUpdateApplyStart }) => {
    try {
      window.sessionStorage.setItem("gwt:ui:briefing", "1");
    } catch {
      /* no-op */
    }
    if (enableTestBridge) {
      (window as any).__gwtPlaywrightTestBridge = true;
    }
    if (suppressUpdateApplyStart && !(window as any).__gwtSuppressUpdateApplyStart) {
      (window as any).__gwtSuppressUpdateApplyStart = true;
      const originalSend = WebSocket.prototype.send;
      WebSocket.prototype.send = function (data: string | ArrayBufferLike | Blob | ArrayBufferView) {
        try {
          const payload = typeof data === "string" ? JSON.parse(data) : null;
          if (payload && payload.kind === "apply_update_start") {
            return;
          }
        } catch {
          /* no-op */
        }
        return originalSend.call(this, data);
      };
    }
  }, {
    enableTestBridge: Boolean(options.enableTestBridge),
    suppressUpdateApplyStart: Boolean(options.suppressUpdateApplyStart),
  });

  await page.goto(base);

  const hiddenStartupSelectors = [
    "#op-briefing",
    "#project-picker",
    "#project-onboarding",
  ];
  if (!options.keepPresetModal) {
    hiddenStartupSelectors.push("#preset-modal");
  }
  await page.addStyleTag({
    content: `
      ${hiddenStartupSelectors.join(",\n      ")} {
        display: none !important;
        pointer-events: none !important;
      }
    `,
  });

  await page.evaluate(() => {
    for (const id of ["op-briefing", "project-picker", "project-onboarding"]) {
      const element = document.getElementById(id);
      if (element) element.hidden = true;
    }
  });

  if (options.enableTestBridge) {
    await page.waitForFunction(
      () => (window as any).__gwtPlaywrightTestBridgeInstalled === true,
    );
  }
}

export async function sendLiveGwtEvent(page: Page, payload: unknown): Promise<void> {
  await page.evaluate((detail) => {
    window.dispatchEvent(new CustomEvent("__gwt_test_send", { detail }));
  }, payload);
}

export async function suppressInitialFrontendReady(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const originalSend = WebSocket.prototype.send;
    WebSocket.prototype.send = function sendWithInitialReadySuppressed(data) {
      try {
        const payload = typeof data === "string" ? JSON.parse(data) : null;
        if (
          payload?.kind === "frontend_ready" &&
          (window as any).__gwtDropInitialFrontendReady !== false
        ) {
          (window as any).__gwtDropInitialFrontendReady = false;
          return;
        }
      } catch {
        /* no-op */
      }
      return originalSend.call(this, data);
    };
  });
}

export async function openLiveGwtProject(
  page: Page,
  projectRoot = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT ?? process.cwd(),
): Promise<void> {
  await sendLiveGwtEvent(page, {
    kind: "reopen_recent_project",
    path: projectRoot,
  });
  await page.waitForSelector(".project-tab", { state: "visible" });
}
