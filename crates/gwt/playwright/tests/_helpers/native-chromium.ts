import { readFile } from "node:fs/promises";
import { isAbsolute, join } from "node:path";
import { expect, type BrowserType, type TestInfo } from "@playwright/test";

export interface NativePage {
  addInitScript<A>(fn: (arg: A) => unknown, arg?: A): Promise<void>;
  evaluate<T, A = undefined>(fn: (arg: A) => T | Promise<T>, arg?: A): Promise<T>;
  goto(url: string): Promise<void>;
}

/** The measured target never receives Playwright's focus emulation. */
export async function withNativeHiddenPage<T, A>(
  browserType: BrowserType, info: TestInfo, prepare: (page: NativePage) => Promise<void>,
  observe: (arg: A) => T | Promise<T>, arg: A,
): Promise<T> {
  const server = await browserType.launchServer({ headless: false,
    args: ["--remote-debugging-port=0"], ignoreDefaultArgs: [
      "--disable-background-timer-throttling", "--disable-renderer-backgrounding",
      "--disable-backgrounding-occluded-windows",
    ] });
  const owned = server.process();
  const errors: string[] = [], failures: unknown[] = [];
  let native: NativeCdp | undefined, result: T | undefined;
  try {
    await info.attach("native-chromium-launch.json", { contentType: "application/json",
      body: JSON.stringify({ pid: owned.pid, executable: owned.spawnfile, args: owned.spawnargs }) });
    const profiles = owned.spawnargs.filter(arg => arg.startsWith("--user-data-dir="));
    if (profiles.length !== 1) throw new Error("owned Chromium has no unique profile");
    const profile = profiles[0].slice("--user-data-dir=".length);
    if (!isAbsolute(profile)) throw new Error("owned Chromium profile is not absolute");
    let endpoint = "";
    await expect.poll(async () => {
      if (owned.exitCode !== null || owned.signalCode !== null) throw new Error("owned Chromium exited before CDP startup");
      let text: string;
      try { text = await readFile(join(profile, "DevToolsActivePort"), "utf8"); }
      catch (error) { if ((error as NodeJS.ErrnoException).code === "ENOENT") return false; throw error; }
      const match = /^(\d{1,5})\n(\/devtools\/browser\/[\w-]+)\n?$/.exec(text);
      if (!match || Number(match[1]) < 1 || Number(match[1]) > 65535) return false;
      endpoint = `ws://127.0.0.1:${match[1]}${match[2]}`;
      return true;
    }, { timeout: 10_000, message: "owned Chromium publishes its loopback CDP endpoint" }).toBe(true);
    native = await NativeCdp.connect(endpoint);
    const main = await native.send("Target.createTarget", { url: "about:blank", newWindow: true, background: false });
    const { sessionId } = await native.send("Target.attachToTarget", { targetId: main.targetId, flatten: true });
    const send = (method: string, params = {}) => native!.send(method, params, sessionId);
    native.onEvent = event => {
      if (event.sessionId !== sessionId) return;
      if (event.method === "Runtime.exceptionThrown") errors.push(JSON.stringify(event.params.exceptionDetails));
      if (event.method === "Runtime.consoleAPICalled" && event.params.type === "error") errors.push(JSON.stringify(event.params.args));
      if (event.method === "Log.entryAdded" && event.params.entry.level === "error") errors.push(event.params.entry.text);
    };
    await send("Page.enable");
    await send("Runtime.enable");
    await send("Log.enable");
    const evaluate = async (expression: string) => {
      const response = await send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
      if (response.exceptionDetails) throw new Error(JSON.stringify(response.exceptionDetails));
      return response.result.value;
    };
    const source = (fn: (arg: any) => unknown, value: unknown) => `(${fn.toString()})(${JSON.stringify(value) ?? "undefined"})`;
    const page: NativePage = {
      addInitScript: async (fn, value) => { await send("Page.addScriptToEvaluateOnNewDocument", { source: source(fn, value) }); },
      evaluate: (fn, value) => evaluate(source(fn, value)),
      goto: async url => {
        const requested = new URL(url).href;
        const navigation = await send("Page.navigate", { url: requested });
        if (navigation.errorText) throw new Error(`native navigation failed: ${navigation.errorText}`);
        await expect.poll(() => evaluate(`document.readyState === "complete" && location.href === ${JSON.stringify(requested)}`),
          { timeout: 10_000, message: "native target finishes the requested navigation" }).toBe(true);
      },
    };
    await prepare(page);
    const peer = await native.send("Target.createTarget", { url: "about:blank", newWindow: false, background: false });
    await native.send("Target.activateTarget", { targetId: peer.targetId });
    const attention = () => evaluate("({ hidden: document.hidden, focused: document.hasFocus() })");
    try {
      await expect.poll(attention, { message: "a real foreground peer hides and unfocuses the measured page" })
        .toEqual({ hidden: true, focused: false });
    } finally {
      await info.attach("native-chromium-attention.json", { contentType: "application/json",
        body: JSON.stringify({ attention: await attention(), targets: await native.send("Target.getTargets") }) });
    }
    result = await evaluate(source(observe, arg));
    await send("Page.bringToFront");
    const screenshot = await send("Page.captureScreenshot", { format: "png" });
    await info.attach("native-background-after-measurement-foreground.png", {
      body: Buffer.from(screenshot.data, "base64"), contentType: "image/png",
    });
  } catch (error) { failures.push(error); }
  finally {
    native?.close();
    try { await server.close(); } catch (error) { failures.push(error); }
    try {
      await info.attach("native-chromium-owned-cleanup.json", { contentType: "application/json",
        body: JSON.stringify({ pid: owned.pid, exitCode: owned.exitCode, signalCode: owned.signalCode }) });
      expect(owned.exitCode !== null || owned.signalCode !== null, "owned Chromium exited").toBe(true);
      expect(errors, "zero startup/native console and page errors").toEqual([]);
    } catch (error) { failures.push(error); }
  }
  if (failures.length) throw new AggregateError(failures, failures.map(String).join("\n"));
  return result as T;
}

class NativeCdp {
  private nextId = 0;
  private pending = new Map<number, { resolve: (value: any) => void; reject: (error: Error) => void }>();
  onEvent: (event: any) => void = () => {};
  constructor(private socket: WebSocket) {
    socket.addEventListener("message", event => {
      const message = JSON.parse(String(event.data)), request = this.pending.get(message.id);
      if (!request) { this.onEvent(message); return; }
      this.pending.delete(message.id);
      if (message.error) request.reject(new Error(JSON.stringify(message.error)));
      else request.resolve(message.result);
    });
    const fail = () => { for (const request of this.pending.values()) request.reject(new Error("native CDP closed or errored")); this.pending.clear(); };
    socket.addEventListener("close", fail);
    socket.addEventListener("error", fail);
  }
  static async connect(endpoint: string) {
    const socket = new WebSocket(endpoint);
    try {
      await new Promise<void>((resolve, reject) => {
        const timer = setTimeout(() => reject(new Error("native CDP connection deadline")), 10_000);
        socket.addEventListener("open", () => { clearTimeout(timer); resolve(); }, { once: true });
        socket.addEventListener("error", () => { clearTimeout(timer); reject(new Error("native CDP connection failed")); }, { once: true });
      });
      return new NativeCdp(socket);
    } catch (error) { socket.close(); throw error; }
  }
  send(method: string, params = {}, sessionId?: string): Promise<any> {
    return new Promise((resolve, reject) => {
      const id = ++this.nextId;
      const timer = setTimeout(() => { this.pending.delete(id); reject(new Error(`native CDP deadline: ${method}`)); }, 10_000);
      const fail = (error: Error) => { clearTimeout(timer); reject(error); };
      this.pending.set(id, { resolve: value => { clearTimeout(timer); resolve(value); }, reject: fail });
      try { this.socket.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) })); }
      catch (error) { this.pending.delete(id); fail(error as Error); }
    });
  }
  close() { this.socket.close(); }
}
