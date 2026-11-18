import { mkdir, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { expect, test } from "@playwright/test";
import { startExactRelaunchFixture } from "./_helpers/exact-relaunch";
import { gotoLiveGwt } from "./_helpers/live-gwt";

const codex = process.env.GWT_PLAYWRIGHT_CODEX_BINARY ?? "";
test.skip(!codex || process.platform === "win32", "requires a local Codex executable on Unix");

test("Codex paints its composer before the browser attaches", async ({ page }, info) => {
  test.setTimeout(180_000);
  const server = await startExactRelaunchFixture(info);
  try {
    const base = server.url;
    const theme = info.project.use.colorScheme === "light" ? "light" : "dark";
    const errors: string[] = [];
    page.on("pageerror", error => errors.push(error.message));
    page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
    await page.addInitScript(({ theme, origin }) => {
      if (location.origin === origin) localStorage.setItem("gwt:ui:theme", theme);
    }, { theme, origin: new URL(base).origin });

    const fixture = join(server.home, "codex-composer");
    await mkdir(join(fixture, ".codex"), { recursive: true });
    const script = join(fixture, "run-codex");
    // Local provider configuration lets the real TUI mount with an empty
    // CODEX_HOME. The test never submits a prompt.
    await writeFile(script, [
      "#!/bin/sh",
      'cd "$(dirname "$0")" || exit 1',
      'exec env -i PATH="$PATH" HOME="$PWD" CODEX_HOME="$PWD/.codex" TERM=xterm-256color COLORTERM=truecolor LANG=en_US.UTF-8 '
        + shellQuote(codex) + ' --no-daemon --no-alt-screen -m gpt-5.4 \\',
      "  -c 'check_for_update_on_startup=false' -c 'analytics.enabled=false' -c 'otel.exporter=\"none\"' \\",
      "  -c 'model_provider=\"gwt_protocol_test\"' -c 'model_providers.gwt_protocol_test.name=\"Protocol test\"' \\",
      "  -c 'model_providers.gwt_protocol_test.base_url=\"http://127.0.0.1:9/v1\"' \\",
      "  -c 'model_providers.gwt_protocol_test.wire_api=\"responses\"' -c 'model_providers.gwt_protocol_test.requires_openai_auth=false'",
      "",
    ].join("\n"), { mode: 0o700 });

    const sockets: WebSocket[] = [];
    let id: string | undefined;
    let project: Awaited<ReturnType<typeof connect>> | undefined;
    async function connect(path: string) {
      const url = new URL(path, base);
      url.protocol = "ws:";
      const socket = new WebSocket(url);
      sockets.push(socket);
      const messages: any[] = [];
      socket.addEventListener("message", event => {
        if (typeof event.data === "string") messages.push(JSON.parse(event.data));
      });
      await new Promise<void>((resolve, reject) => {
        socket.addEventListener("open", () => resolve(), { once: true });
        socket.addEventListener("error", () => reject(new Error("gwt control socket failed")), { once: true });
      });
      const send = (event: object) => socket.send(JSON.stringify(event));
      send({ kind: "frontend_ready" });
      return { messages, send };
    }
    try {
      const hub = await connect("/ws");
      const projectKey = () => hub.messages.findLast(message => message.kind === "hub_state")?.hub.projects[0]?.project_key;
      await expect.poll(projectKey).toMatch(/^[0-9a-f]{16}$/);
      const key = projectKey();
      const control = project = await connect("/ws?repo_hash=" + key);
      // Render the isolated Project once to satisfy shell launch admission.
      await gotoLiveGwt(page, base, { projectKey: key, enableTestBridge: true });
      const projectUrl = page.url();
      await expect.poll(() => {
        control.send({ kind: "get_project_execution_status" });
        return control.messages.findLast(message => message.kind === "project_execution_status")?.status;
      }).toMatchObject({ view_state: "open", degraded: false });
      const workspace = () => control.messages.findLast(message => message.kind === "workspace_state")?.workspace;
      const windows = () => (workspace()?.tabs ?? [])
        .flatMap((tab: any) => tab.workspace.windows);
      await expect.poll(workspace).toBeTruthy();
      const existing = new Set(windows().map((window: any) => window.id));
      control.send({ kind: "create_window", preset: "shell" });
      const shell = () => windows().find((window: any) => window.preset === "shell" && !existing.has(window.id));
      await expect.poll(shell, { timeout: 30_000 }).toBeTruthy();
      id = shell().id;
      const status = () => control.messages.findLast(message => message.kind === "terminal_status" && message.id === id);
      await expect.poll(() => status()?.status, { timeout: 30_000 }).toMatch(/^(running|error)$/);
      expect(status()?.status, status()?.detail).toBe("running");
      control.send({ kind: "focus_window", id });
      // Detach the browser before Codex starts. Only the Node control
      // socket, which has no terminal emulator, can observe the queries.
      await page.goto("about:blank");
      control.send({ kind: "terminal_input", id, data: shellQuote(script) + "\r" });
      const startupOutput = () => control.messages
        .filter(message => message.kind === "terminal_output" && message.id === id)
        .map(message => Buffer.from(message.data_base64, "base64").toString()).join("");
      await expect.poll(startupOutput, { timeout: 30_000 }).toContain("Ask Codex to do anything");
      // Codex 0.169.0 blends the two Operator colors into this background.
      expect(startupOutput()).toContain("\x1b[48;2;39;42;46m");

      await page.goto(projectUrl);
      await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
      await expect.poll(() => page.evaluate(paneId => (window as any).__gwtPlaywrightMessages.some((entry: any) =>
        entry.payload.kind === "terminal_snapshot" && entry.payload.id === paneId
        && atob(entry.payload.data_base64).includes("Ask Codex to do anything")), id),
      { timeout: 30_000 }).toBe(true);
      const rows = page.locator('.workspace-window[data-id="' + id + '"] .xterm-rows');
      await expect.poll(() => rows.locator("div").evaluateAll(lines => lines.some(line =>
        line.textContent?.includes("Ask Codex to do anything")
        && [...line.querySelectorAll("span")].some(span => span.style.backgroundColor === "rgb(39, 42, 46)"))),
      { timeout: 30_000 }).toBe(true);
      expect(errors, "console and page errors").toEqual([]);
      await info.attach("codex-composer", { body: await page.screenshot(), contentType: "image/png" });
    } finally {
      if (id && project) project.send({ kind: "close_window", id });
      for (const socket of sockets) socket.close();
    }
  } finally {
    await server.stop();
  }
});

function shellQuote(path: string): string {
  return "'" + path.replaceAll("'", "'\\''") + "'";
}
