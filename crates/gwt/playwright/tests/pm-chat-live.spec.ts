/* SPEC #4930: real native transcript -> backend -> common PM view. */
import { expect, test } from "@playwright/test";
import { appendFileSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { gotoLiveGwt, sendLiveGwtEvent, withLiveGwtBackendLock } from "./_helpers/live-gwt";

const BASE = process.env.GWT_PLAYWRIGHT_BASE_URL ?? "http://127.0.0.1:0/";
// A browser-check fresh HOME containing the deliberately stopped fixture PM.
// Never point this suite at a user's live provider store.
const HOME = process.env.GWT_PM_CHAT_CHECK_HOME ?? "";
const ROOT = process.env.GWT_PLAYWRIGHT_PROJECT_ROOT ?? "";
const SESSION = "pm-chat-fixture";

test.describe("PM conversation", () => {
  test.skip(!process.env.GWT_PLAYWRIGHT_BASE_URL || !HOME, "requires isolated PM fixture");
  test.use({ viewport: { width: 1440, height: 1000 } });
  test.setTimeout(90_000);

  test("native text, silent cycles, log toggle, reload and provider switch", async ({ page }, info) => {
    await withLiveGwtBackendLock(BASE, info, async () => {
      const errors: string[] = [];
      page.on("pageerror", error => errors.push(error.message));
      page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
      const sessionPath = join(HOME, ".gwt/sessions", `${SESSION}.toml`);
      const originalSession = readFileSync(sessionPath, "utf8");
      expect(originalSession).toContain(`id = "${SESSION}"`);
      const nativeSession = (provider: string, conversation: string) => originalSession
        .replace(/agent_id = \{[^\n]+\}/, `agent_id = { type = "${provider}" }`)
        .replace(/(\[agent_id\]\s*\ntype = ")[^"]+/, `$1${provider}`)
        .replace(/agent_session_id = "[^"]+"/, `agent_session_id = "${conversation}"`);
      const claudePath = join(HOME, ".claude/projects/fixture/pm-chat-claude.jsonl");
      const claude = (uuid: string, role: string, content: unknown, extra = {}) => ({
        uuid, sessionId: "pm-chat-claude", cwd: ROOT, type: role,
        message: { role, content }, ...extra,
      });
      const lines = (records: unknown[]) => records.map(record => JSON.stringify(record)).join("\n") + "\n";
      writeFileSync(sessionPath, nativeSession("ClaudeCode", "pm-chat-claude"));
      writeFileSync(claudePath, lines([
        claude("bootstrap", "user", "$gwt-pm"),
        claude("question", "user", "What should we do next?"),
        claude("tool", "assistant", [{ type: "tool_use", name: "Bash", input: { command: "hidden-tool-command" } }]),
        claude("answer", "assistant", [{ type: "text", text: "Review the proposed change, then run the checks." }]),
        claude("stop", "user", "Stop hook feedback: hidden-stop-contract", { isMeta: true }),
      ]));
      try {
        await gotoLiveGwt(page, BASE, { enableTestBridge: true });
        const pane = page.locator('.workspace-window').filter({ has: page.locator('.pm-chat') });
        const chat = pane.getByRole("log", { name: "PM conversation" });
        await expect(chat).toContainText("Review the proposed change");
        await expect(chat.locator("article")).toHaveCount(2);
        await expect(chat).not.toContainText("hidden-");
        await expect(chat).not.toContainText("$gwt-pm");
        const id = await pane.getAttribute("data-id");
        expect(id).toBeTruthy();
        const refresh = () => sendLiveGwtEvent(page, { kind: "load_pm_conversation", id });
        // Parsed conversation and RAW PTY are distinct data in one window version.
        // Hide the pane and hold animation frames: neither subscription may need focus.
        const revisions = await page.evaluate(async ({ id, session }) => {
          const pane = document.querySelector<HTMLElement>(`.workspace-window[data-id="${id}"]`)!;
          const chat = pane.querySelector<HTMLElement>('.pm-chat')!;
          const log = pane.querySelector<HTMLElement>('.pm-conversation-log')!;
          const socket = (window as any).__gwtPlaywrightSockets.find((socket: WebSocket) =>
            socket.readyState === WebSocket.OPEN && new URL(socket.url).searchParams.has('repo_hash')) as WebSocket;
          if (!socket) throw new Error('Active project socket missing');
          const originalFrame = window.requestAnimationFrame;
          const heldFrames: FrameRequestCallback[] = [];
          const before = Number(chat.dataset.stateVersion);
          const versions: string[][] = [];
          const start = performance.now();
          pane.hidden = true;
          window.requestAnimationFrame = callback => -heldFrames.push(callback);
          try {
            for (const event of [
              { kind: 'terminal_snapshot', id, data_base64: btoa('RAW snapshot\r\n') },
              { kind: 'pm_conversation', id, session_id: session, snapshot: { availability: 'ready', conversation_id: 'pm-chat-claude', messages: [{ id: 'shared', role: 'assistant', text: 'Parsed shared-state answer' }] } },
              { kind: 'terminal_output', id, data_base64: btoa('RAW appended\r\n') },
            ]) {
              await new Promise<void>((resolve, reject) => {
                const observer = new MutationObserver(() => {
                  observer.disconnect();
                  clearTimeout(deadline);
                  resolve();
                });
                const deadline = setTimeout(() => { observer.disconnect(); reject(new Error('Hidden window did not update')); }, 1000);
                observer.observe(chat, { attributes: true, attributeFilter: ['data-state-version'] });
                socket.dispatchEvent(new MessageEvent('message', { data: JSON.stringify(event) }));
              });
              versions.push([chat.dataset.stateVersion!, log.dataset.stateVersion!]);
            }
            socket.dispatchEvent(new MessageEvent('message', { data: JSON.stringify({
              kind: 'pm_conversation', id, session_id: 'stale-session',
              snapshot: { availability: 'ready', messages: [{ id: 'stale', role: 'assistant', text: 'Stale answer' }] },
            }) }));
            await new Promise(resolve => setTimeout(resolve, 150));
            return { before, versions, after: Number(chat.dataset.stateVersion), elapsed: performance.now() - start };
          } finally {
            pane.hidden = false;
            window.requestAnimationFrame = originalFrame;
            heldFrames.forEach(callback => originalFrame(callback));
          }
        }, { id, session: SESSION });
        expect(revisions.versions).toEqual([1, 2, 3].map(offset => [String(revisions.before + offset), String(revisions.before + offset)]));
        expect(revisions.after).toBe(revisions.before + 3);
        expect(revisions.elapsed).toBeLessThan(1000);
        await expect(chat).toContainText('Parsed shared-state answer');
        await expect(chat).not.toContainText('RAW');
        await pane.getByRole("button", { name: "Execution log", exact: true }).click();
        await expect(pane.locator('.xterm')).toContainText('RAW snapshot');
        await expect(pane.locator('.xterm')).toContainText('RAW appended');
        await pane.getByRole("button", { name: "Chat", exact: true }).click();
        await refresh();
        await expect(chat).toContainText('Review the proposed change');
        appendFileSync(claudePath, lines([
          claude("silent", "assistant", [{ type: "thinking", thinking: "hidden-thinking" }]),
          claude("silent-stop", "user", "hidden-stop-contract", { isMeta: true }),
        ]));
        await refresh();
        await expect(chat.locator("article")).toHaveCount(2);
        await pane.getByRole("button", { name: "Execution log", exact: true }).click();
        await expect(pane.locator(".terminal-root")).toBeVisible();
        await expect(chat).toBeHidden();
        await pane.getByRole("button", { name: "Chat", exact: true }).click();
        await expect(chat).toBeVisible();
        await expect(pane.locator(".terminal-root")).toBeHidden();

        // This fixture has no agent process: failure must preserve the draft.
        const input = pane.getByRole("textbox", { name: "Message PM" });
        await input.fill("Keep this draft after a failed send");
        await pane.getByRole("button", { name: "Send", exact: true }).click();
        await expect(pane.locator(".pm-chat__error")).toBeVisible();
        await expect(input).toBeEnabled();
        await expect(input).toHaveValue("Keep this draft after a failed send");

        // Reproduce a pane disappearing between the last workspace frame and
        // submit. The real backend rejects this stale session without a window ID.
        await page.evaluate(() => {
          const originalSend = WebSocket.prototype.send;
          WebSocket.prototype.send = function(data) {
            const payload = typeof data === "string" ? JSON.parse(data) : null;
            if (payload?.kind === "pane_send_input") {
              WebSocket.prototype.send = originalSend;
              data = JSON.stringify({ ...payload, session_id: "removed-pm-session" });
            }
            return originalSend.call(this, data);
          };
        });
        await input.fill("Keep the draft if the PM disappears");
        await pane.getByRole("button", { name: "Send", exact: true }).click();
        await expect(pane.locator(".pm-chat__error")).toBeVisible();
        await expect(input).toBeEnabled();
        await expect(input).toHaveValue("Keep the draft if the PM disappears");

        await page.reload();
        await expect(chat).toContainText("Review the proposed change");
        await expect(chat.locator("article")).toHaveCount(2);
        const codexPath = join(HOME, ".codex/sessions/2026/10/03/rollout-pm-chat-codex.jsonl");
        writeFileSync(codexPath, lines([
          { type: "session_meta", payload: { id: "pm-chat-codex", cwd: ROOT } },
          { type: "response_item", payload: { type: "message", id: "u", role: "user", content: [{ type: "input_text", text: "What should we do next?" }] } },
          { type: "response_item", payload: { type: "function_call_output", output: "hidden-tool-output" } },
          { type: "response_item", payload: { type: "message", id: "a", role: "assistant", content: [{ type: "output_text", text: "Review the proposed change, then run the checks." }] } },
          { type: "response_item", payload: { type: "message", role: "developer", content: [{ type: "input_text", text: "hidden-stop-contract" }] } },
        ]));
        writeFileSync(sessionPath, nativeSession("Codex", "pm-chat-codex"));
        await refresh();
        await page.waitForFunction(() => (window as any).__gwtPlaywrightMessages?.some((entry: any) =>
          entry.payload?.kind === "pm_conversation" && entry.payload.snapshot?.conversation_id === "pm-chat-codex" && entry.payload.snapshot?.availability === "ready"));
        await expect(chat.locator("article")).toHaveCount(2);
        await expect(chat).toContainText("Review the proposed change");
        await expect(chat).not.toContainText("hidden-");
        await expect(input).toBeEnabled();
        await expect(pane.locator(".pm-chat__notice")).toBeHidden();
        const theme = info.project.name.endsWith("light") ? "light" : "dark";
        await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
        await info.attach(`pm-chat-${theme}`, { body: await pane.screenshot(), contentType: "image/png" });
        if (process.env.GWT_PM_CHAT_SCREENSHOT_DIR) await pane.screenshot({ path: join(process.env.GWT_PM_CHAT_SCREENSHOT_DIR, `pm-chat-${theme}.png`) });
        writeFileSync(sessionPath, nativeSession("OpenCode", "pm-chat-unsupported"));
        await refresh();
        await expect(pane.locator(".pm-chat")).toBeHidden();
        await expect(pane.locator(".terminal-root")).toBeVisible();
        expect(errors).toEqual([]);
      } finally {
        writeFileSync(sessionPath, originalSession);
      }
    });
  });
});
