import { mkdir, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import type { Page, TestInfo } from "@playwright/test";

type LiveGwtOptions = {
  enableTestBridge?: boolean;
  /** Issue #4538: stay on the Hub (`/`) instead of a Project route. */
  hub?: boolean;
  /** Issue #4538: open this Project route (`/p/<key>`) explicitly. */
  projectKey?: string;
  keepPresetModal?: boolean;
  suppressProjectSurfaces?: boolean;
  suppressUpdateApplyStart?: boolean;
};

const LIVE_BACKEND_LOCK_STALE_MS = 5 * 60 * 1000;

function liveBackendLockPath(base: string): string {
  const key = base.replace(/[^a-zA-Z0-9._-]+/g, "_").slice(0, 96) || "default";
  return join(tmpdir(), `gwt-live-playwright-${key}.lock`);
}

function sleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

async function removeStaleLiveBackendLock(path: string): Promise<boolean> {
  try {
    const content = await readFile(join(path, "owner.json"), "utf8");
    const owner = JSON.parse(content) as { createdAt?: number };
    if (owner.createdAt && Date.now() - owner.createdAt < LIVE_BACKEND_LOCK_STALE_MS) {
      return false;
    }
  } catch {
    // mkdir and owner.json cannot be created atomically. If the owner dies in
    // that narrow window, fall back to the directory timestamp so later live
    // tests can reclaim the orphan instead of waiting the full lock timeout.
    try {
      const metadata = await stat(path);
      if (Date.now() - metadata.mtimeMs < LIVE_BACKEND_LOCK_STALE_MS) {
        return false;
      }
    } catch {
      return false;
    }
  }
  await rm(path, { recursive: true, force: true });
  return true;
}

export async function acquireLiveGwtBackendLock(
  base: string,
  testInfo: TestInfo,
): Promise<() => Promise<void>> {
  const lockPath = liveBackendLockPath(base);
  const deadline = Date.now() + LIVE_BACKEND_LOCK_STALE_MS;
  while (Date.now() < deadline) {
    try {
      await mkdir(lockPath);
      try {
        await writeFile(
          join(lockPath, "owner.json"),
          JSON.stringify({
            createdAt: Date.now(),
            titlePath: testInfo.titlePath,
            workerIndex: testInfo.workerIndex,
          }),
        );
      } catch (error) {
        await rm(lockPath, { recursive: true, force: true });
        throw error;
      }
      return async () => {
        await rm(lockPath, { recursive: true, force: true });
      };
    } catch (error) {
      if ((error as { code?: string }).code !== "EEXIST") {
        throw error;
      }
      await removeStaleLiveBackendLock(lockPath);
      await sleep(250);
    }
  }
  throw new Error(`Timed out waiting for live gwt backend lock: ${lockPath}`);
}

export async function withLiveGwtBackendLock<T>(
  base: string,
  testInfo: TestInfo,
  run: () => Promise<T>,
): Promise<T> {
  const release = await acquireLiveGwtBackendLock(base, testInfo);
  try {
    return await run();
  } finally {
    await release();
  }
}

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
      (window as any).__gwtPlaywrightMessages = [];
      (window as any).__gwtPlaywrightMessageSequence = 0;
      const NativeWebSocket = window.WebSocket;
      window.WebSocket = new Proxy(NativeWebSocket, {
        construct(Target, args) {
          const socket = new Target(...args as ConstructorParameters<typeof WebSocket>);
          ((window as any).__gwtPlaywrightSockets ??= []).push(socket);
          socket.addEventListener("message", (event) => {
            try {
              const payload = JSON.parse(String(event.data));
              const sequence = ((window as any).__gwtPlaywrightMessageSequence as number) + 1;
              (window as any).__gwtPlaywrightMessageSequence = sequence;
              const messages = (window as any).__gwtPlaywrightMessages as Array<{
                sequence: number;
                payload: unknown;
              }>;
              messages.push({ sequence, payload });
              if (messages.length > 256) messages.splice(0, messages.length - 256);
            } catch {
              /* non-JSON backend frames are irrelevant to the test bridge */
            }
          });
          return socket;
        },
      });
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

  liveGwtPageOptions.set(page, options);
  await page.goto(await liveGwtRouteUrl(page, base, options));
  await prepareLiveGwtPage(page, options);
}

// Options of the last gotoLiveGwt per page, re-applied when a helper has to
// move the page to another Project route (Issue #4538).
const liveGwtPageOptions = new WeakMap<Page, LiveGwtOptions>();

async function prepareLiveGwtPage(page: Page, options: LiveGwtOptions): Promise<void> {
  const hiddenStartupSelectors = ["#op-briefing"];
  if (options.suppressProjectSurfaces) {
    hiddenStartupSelectors.push("#project-picker", "#project-onboarding");
  }
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

  await page.evaluate(({ suppressProjectSurfaces }) => {
    const hiddenStartupIds = ["op-briefing"];
    if (suppressProjectSurfaces) {
      hiddenStartupIds.push("project-picker", "project-onboarding");
    }
    for (const id of hiddenStartupIds) {
      const element = document.getElementById(id);
      if (element) element.hidden = true;
    }
  }, {
    suppressProjectSurfaces: Boolean(options.suppressProjectSurfaces),
  });

  if (options.enableTestBridge) {
    await page.waitForFunction(
      () => (window as any).__gwtPlaywrightTestBridgeInstalled === true,
    );
  }
}

export type LiveHubCatalog = {
  projects: Array<{ id: string; project_key: string; title: string }>;
  recent_projects: Array<{ path: string; title: string; project_key: string | null }>;
};

const PROJECT_ROUTE = /\/p\/[0-9a-f]{16}$/;

export function liveGwtProjectUrl(base: string, projectKey: string): string {
  return new URL(`/p/${projectKey}`, base).toString();
}

/**
 * Issue #4538: `/` is the Hub and the workspace lives at `/p/<key>`. A root
 * `base` resolves to the requested Project, else the first open Project,
 * else the first Recent Project; a Project URL `base` is used as-is.
 */
export async function liveGwtRouteUrl(
  page: Page,
  base: string,
  options: LiveGwtOptions = {},
): Promise<string> {
  if (options.hub || PROJECT_ROUTE.test(new URL(base).pathname)) return base;
  if (options.projectKey) return liveGwtProjectUrl(base, options.projectKey);
  const catalog = await readLiveHubCatalog(page, base);
  const key = catalog.projects[0]?.project_key
    ?? catalog.recent_projects.find((entry) => entry.project_key)?.project_key;
  if (!key) throw new Error("live gwt has no open or recent Project to route to");
  return liveGwtProjectUrl(base, key);
}

/**
 * Read the Hub catalog through a same-origin WebSocket from the Hub page.
 * Waits until every Recent entry carries its resolved ProjectKey.
 * `send` is an optional Hub navigation message sent after hydration.
 */
export async function readLiveHubCatalog(
  page: Page,
  base: string,
  options: {
    send?: unknown;
    /** Runs in the page: it may only use `catalog` and the serialisable `arg`. */
    until?: (catalog: LiveHubCatalog, arg: any) => boolean;
    arg?: unknown;
  } = {},
): Promise<LiveHubCatalog> {
  // A separate Hub page keeps the caller's page, its init scripts, and its
  // WebSocket instrumentation untouched.
  const hubUrl = new URL("/", base).toString();
  const reader = await page.context().newPage();
  try {
    await reader.goto(hubUrl);
    return await reader.evaluate(({ url, send, until, arg }) => new Promise<any>((resolve, reject) => {
      const accept = until ? new Function(`return (${until})`)() : null;
      const socket = new WebSocket(url);
      const timer = window.setTimeout(() => {
        socket.close();
        reject(new Error("timed out waiting for the live Hub catalog"));
      }, 30_000);
      let sent = false;
      socket.addEventListener("open", () => socket.send(JSON.stringify({ kind: "frontend_ready" })));
      socket.addEventListener("message", (event) => {
        const message = JSON.parse(String(event.data));
        if (message.kind !== "hub_state") return;
        if (send && !sent) {
          sent = true;
          socket.send(JSON.stringify(send));
          return;
        }
        const hub = message.hub;
        const resolved = hub.recent_projects.every((entry: any) => entry.project_key);
        if (resolved && (!accept || accept(hub, arg))) {
          window.clearTimeout(timer);
          socket.close();
          resolve(hub);
        }
      });
    }), {
      url: hubUrl.replace(/^http/, "ws") + "ws",
      send: options.send ?? null,
      until: options.until?.toString() ?? null,
      arg: options.arg ?? null,
    });
  } finally {
    await reader.close();
  }
}

export async function sendLiveGwtEvent(page: Page, payload: unknown): Promise<void> {
  await page.evaluate((detail) => {
    window.dispatchEvent(new CustomEvent("__gwt_test_send", { detail }));
  }, payload);
}

export type LiveLaunchWizardFixture = {
  windowId: string;
  createdWorkWindow: boolean;
  cleanup: () => Promise<void>;
};

const LIVE_WORK_SURFACE_SELECTOR = [
  '.workspace-window[data-preset="work"]',
  '.workspace-window[data-preset="branches"]',
].join(", ");

/**
 * Open the normal branch Launch Wizard through the active project's
 * visible Work/legacy Branches singleton. An existing user window is reused
 * and never owned by this helper. If none is visible, the helper requests a
 * persisted Work window; the backend may create one or reveal an existing
 * singleton. Cleanup closes the returned window only when its id did not exist
 * before that request. Neither path creates a branch, worktree, or agent
 * process.
 */
export async function openLiveLaunchWizardForBranch(
  page: Page,
  branchName = process.env.GWT_PLAYWRIGHT_BRANCH_NAME ?? "develop",
): Promise<LiveLaunchWizardFixture> {
  const preexistingWorkSurfaceIds = await liveWorkSurfaceIds(page);
  let id = await topmostLiveWorkSurfaceId(page);
  if (!id) {
    await sendLiveGwtEvent(page, {
      kind: "create_window",
      preset: "work",
      bounds: { x: 96, y: 96, width: 880, height: 520 },
    });
    id = await page
      .waitForFunction((selector) => {
        const nodes = Array.from(document.querySelectorAll(selector))
          .filter((node) => !(node as HTMLElement).hidden);
        const topmost = nodes.sort((left, right) => {
          const leftZ =
            Number.parseInt((left as HTMLElement).style.zIndex || "0", 10) || 0;
          const rightZ =
            Number.parseInt((right as HTMLElement).style.zIndex || "0", 10) || 0;
          return rightZ - leftZ;
        })[0] as HTMLElement | undefined;
        return topmost?.dataset.id || "";
      }, LIVE_WORK_SURFACE_SELECTOR)
      .then((handle) => handle.jsonValue());
  }
  if (!id) throw new Error("Work window fixture did not materialize");
  const createdWorkWindow = !preexistingWorkSurfaceIds.has(id);

  await sendLiveGwtEvent(page, {
    kind: "open_launch_wizard",
    id,
    branch_name: branchName,
  });

  let cleaned = false;
  return {
    windowId: id,
    createdWorkWindow,
    cleanup: async () => {
      if (!createdWorkWindow || cleaned) return;
      const stillExists = await page
        .locator(LIVE_WORK_SURFACE_SELECTOR)
        .evaluateAll(
          (nodes, windowId) =>
            nodes.some((node) => (node as HTMLElement).dataset.id === windowId),
          id,
        );
      if (!stillExists) {
        cleaned = true;
        return;
      }
      await sendLiveGwtEvent(page, { kind: "close_window", id });
      await page.waitForFunction(
        ({ selector, windowId }) =>
          !Array.from(document.querySelectorAll(selector))
            .some((node) => (node as HTMLElement).dataset.id === windowId),
        { selector: LIVE_WORK_SURFACE_SELECTOR, windowId: id },
      );
      cleaned = true;
    },
  };
}

async function liveWorkSurfaceIds(page: Page): Promise<Set<string>> {
  const ids = await page.locator(LIVE_WORK_SURFACE_SELECTOR).evaluateAll((nodes) =>
    nodes.flatMap((node) => {
      const id = (node as HTMLElement).dataset.id;
      return id ? [id] : [];
    }),
  );
  return new Set(ids);
}

async function topmostLiveWorkSurfaceId(page: Page): Promise<string | null> {
  return page.locator(LIVE_WORK_SURFACE_SELECTOR).evaluateAll((nodes) => {
    const topmost = [...nodes]
      .filter((node) => !(node as HTMLElement).hidden)
      .sort((left, right) => {
        const leftZ =
          Number.parseInt((left as HTMLElement).style.zIndex || "0", 10) || 0;
        const rightZ =
          Number.parseInt((right as HTMLElement).style.zIndex || "0", 10) || 0;
        return rightZ - leftZ;
      })[0] as HTMLElement | undefined;
    return topmost?.dataset.id || null;
  });
}

export async function clearLiveLaunchWizard(page: Page): Promise<void> {
  const wizard = page.locator("#wizard-modal");
  for (let attempt = 0; attempt < 3; attempt += 1) {
    // DOM visibility is only page-local and may still reflect the startup
    // default. FrontendReady always returns the backend-authoritative wizard
    // snapshot, including a null tombstone, so use that frame as the fence.
    const readyCursor = await liveMessageCursor(page);
    await sendLiveGwtEvent(page, { kind: "frontend_ready" });
    const authoritative = await waitForLaunchWizardState(page, readyCursor, null, 5_000)
      .catch(() => null);
    if (!authoritative) continue;
    if (authoritative.hasWizard === false) {
      await wizard.waitFor({ state: "hidden", timeout: 5_000 });
      return;
    }

    const cancelCursor = await liveMessageCursor(page);
    await sendLiveGwtEvent(page, {
      kind: "launch_wizard_action",
      action: { kind: "cancel" },
      bounds: null,
    });
    const cancelled = await waitForLaunchWizardState(page, cancelCursor, false, 5_000)
      .then(() => true)
      .catch(() => false);
    if (!cancelled) continue;
    await wizard.waitFor({ state: "hidden", timeout: 5_000 });
    return;
  }
  throw new Error("live Launch Wizard did not clear before the test");
}

async function liveMessageCursor(page: Page): Promise<number> {
  return page.evaluate(() =>
    Number((window as any).__gwtPlaywrightMessageSequence) || 0
  );
}

async function waitForLaunchWizardState(
  page: Page,
  cursor: number,
  expectedHasWizard: boolean | null,
  timeout: number,
): Promise<{ hasWizard: boolean }> {
  const handle = await page.waitForFunction(
    ({ cursor, expectedHasWizard }) => {
      const messages = (window as any).__gwtPlaywrightMessages;
      if (!Array.isArray(messages)) return null;
      const state = messages.find((entry: any) =>
          entry?.sequence > cursor
          && entry?.payload?.kind === "launch_wizard_state"
          && (
            expectedHasWizard === null
            || (entry.payload.wizard !== null) === expectedHasWizard
          )
        );
      return state ? { hasWizard: state.payload.wizard !== null } : null;
    },
    { cursor, expectedHasWizard },
    { timeout },
  );
  return handle.jsonValue();
}

export async function suppressInitialFrontendReady(page: Page): Promise<void> {
  await page.addInitScript(() => {
    const originalSend = WebSocket.prototype.send;
    // Issue #4538: a Project page hydrates through two sockets (Hub catalog
    // and its Project scope), so each socket's first ready is dropped until
    // the test sets `__gwtDropInitialFrontendReady = false`.
    const suppressed = new WeakSet<WebSocket>();
    WebSocket.prototype.send = function sendWithInitialReadySuppressed(data) {
      try {
        const payload = typeof data === "string" ? JSON.parse(data) : null;
        if (
          payload?.kind === "frontend_ready" &&
          !suppressed.has(this) &&
          (window as any).__gwtDropInitialFrontendReady !== false
        ) {
          suppressed.add(this);
          return;
        }
      } catch {
        /* no-op */
      }
      return originalSend.call(this, data);
    };
  });
}

/**
 * Issue #4538: bind this page to `projectRoot`'s Project route. The Hub opens
 * the root (it becomes a Recent entry with a ProjectKey); a page on another
 * route then navigates to `/p/<key>` and re-applies its gotoLiveGwt setup.
 */
export async function openLiveGwtProject(
  page: Page,
  projectRoot = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT ?? process.cwd(),
): Promise<void> {
  const base = new URL("/", page.url()).toString();
  const catalog = await readLiveHubCatalog(page, base, {
    send: { kind: "reopen_recent_project", path: projectRoot },
    until: (hub) => hub.recent_projects.length > 0,
  });
  const normalized = projectRoot.replace(/\/+$/, "");
  const entry = catalog.recent_projects.find((recent) =>
    recent.path.replace(/\/+$/, "") === normalized && recent.project_key)
    ?? catalog.recent_projects.find((recent) => recent.project_key);
  if (!entry?.project_key) throw new Error(`live gwt did not resolve ${projectRoot}`);
  const target = liveGwtProjectUrl(base, entry.project_key);
  if (page.url() !== target) {
    await page.goto(target);
    await prepareLiveGwtPage(page, liveGwtPageOptions.get(page) ?? {});
  }
  await page.waitForSelector(".project-tab", { state: "visible" });
}
