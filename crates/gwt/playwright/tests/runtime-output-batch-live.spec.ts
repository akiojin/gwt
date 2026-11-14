/** #5379: fixed five-agent PTY workload and native GUI dispatch measurements. */
import { expect, test, type Page } from "@playwright/test";
import { readFile, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { execFileSync } from "node:child_process";
import { gotoLiveGwt, sendLiveGwtEvent } from "./_helpers/live-gwt";

const ROOT = process.env.GWT_PLAYWRIGHT_CHECKOUT_ROOT!;
const callRunner = (name: string, args: unknown[]) => JSON.parse(execFileSync(process.execPath, ["--input-type=module", "-e",
  `import {pathToFileURL} from 'node:url'; const api = await import(pathToFileURL(process.argv[2]).href); console.log(JSON.stringify(api[process.argv[3]](...JSON.parse(process.argv[4]))));`,
  "contract", join(ROOT, "scripts/verify-issue-5379.mjs"), name, JSON.stringify(args)], { encoding: "utf8", stdio: ["ignore", "pipe", "pipe"] }));

test("measurement CLI keeps its duration explicit and forwards Playwright arguments", async () => {
  const parseOptions = (args: string[]) => callRunner("parseOptions", [args]);
  expect(parseOptions(["--seconds", "1800", "--label=before", "--out", "/tmp/before.json", "--headed", "--reporter=/tmp/reporter.cjs"]))
    .toMatchObject({ seconds: 1800, label: "before", out: "/tmp/before.json",
      playwrightArgs: ["--headed", "--reporter=/tmp/reporter.cjs"] });
  expect(parseOptions([]).seconds).toBe(30);
  expect(() => parseOptions(["--seconds", "0"])).toThrow();
});

test("native timing counts dispatches separately from background hook health", async () => {
  const record = (at: number, event: string, elapsed_ms: number, message: string) =>
    ({ timestamp: new Date(at).toISOString(), target: "gwt.frontend.timing", fields: { event, elapsed_ms, message } });
  const records = [
    record(1000, "RuntimeOutput", 40, "RuntimeOutput blocked the GUI event loop for 40ms"),
    record(2000, "RuntimeOutput", 120, "RuntimeOutput blocked the GUI event loop for 120ms"),
    record(2500, "RuntimeHook", 4, "GUI event loop dispatch handled"),
    record(3000, "Other", 500, "Other blocked the GUI event loop for 500ms"),
    { timestamp: new Date(1500).toISOString(), target: "gwt.frontend.timing",
      fields: { stage: "work_rows_hook_health_excluding_project", elapsed_ms: 700, work_count: 5,
        message: "Work row hook health aggregation exceeded budget (project health excluded)" } },
  ];
  expect(callRunner("summarizeTiming", [records, 1000, 3000])).toEqual({
    all_dispatch: { count: 3, stall_count: 2, at_least_100ms: 1, max_ms: 120 },
    runtime_output: { count: 2, stall_count: 2, at_least_100ms: 1, max_ms: 120 },
    hook_health_worker: { count: 1, at_least_100ms: 1, max_ms: 700 },
  });
});

test("five custom agents deliver fixed ordered PTY output", async ({ page }, info) => {
  test.skip(!process.env.GWT_LOOP_MANIFEST, "opt in through scripts/verify-issue-5379.mjs; CI never starts a 30-minute run");
  const theme = info.project.name.includes("light") ? "light" : "dark";
  const manifests = JSON.parse(await readFile(process.env.GWT_LOOP_MANIFEST!, "utf8"));
  const manifest = manifests[theme];
  expect(manifest, `fresh ${theme} backend manifest`).toBeTruthy();
  test.setTimeout((manifest.seconds + 300) * 1000);
  const errors: string[] = [];
  const outputs = Array.from({ length: 5 }, () => ({ tail: "", sequence: 0, markers: 0 }));
  const windowIds: string[] = [];
  let started = 0, ended = 0, passed = false;
  page.on("console", message => { if (message.type() === "error") errors.push(message.text()); });
  page.on("pageerror", error => errors.push(error.message));
  page.on("websocket", socket => {
    socket.on("socketerror", error => errors.push(`socket: ${error}`));
    socket.on("framereceived", ({ payload }) => {
      let event;
      try { event = JSON.parse(String(payload)); } catch { return; }
      if (event.kind !== "terminal_output") return;
      const index = windowIds.indexOf(event.id);
      if (index < 0) return;
      const output = outputs[index];
      const text = output.tail + Buffer.from(event.data_base64, "base64").toString("ascii");
      let lastEnd = 0;
      for (const match of text.matchAll(/GWT_LOOP_5379_(\d)_(\d{4})/g)) {
        const sequence = Number(match[2]);
        if (Number(match[1]) !== index || sequence !== output.sequence + 1) errors.push(`pane ${index}: expected ${output.sequence + 1}, received ${sequence}`);
        output.sequence = sequence; output.markers++;
        lastEnd = match.index! + match[0].length;
      }
      output.tail = text.slice(Math.max(lastEnd, text.length - 24));
    });
  });
  await page.setViewportSize({ width: 1440, height: 1000 });
  await page.addInitScript(value => localStorage.setItem("gwt:ui:theme", value), theme);
  try {
    await gotoLiveGwt(page, manifest.url, { enableTestBridge: true });
    await expect(page.locator("html")).toHaveAttribute("data-theme", theme);
    await sendLiveGwtEvent(page, { kind: "create_window", preset: "work", bounds: { x: 10, y: 10, width: 600, height: 500 } });
    const work = page.locator('.workspace-window[data-preset="work"]');
    await expect(work).toHaveCount(1);
    const workId = await work.getAttribute("data-id");
    for (let index = 0; index < 5; index++) {
      await wizardRequest(page, { kind: "open_launch_wizard", id: workId, branch_name: "develop" }, true);
      const action = (action: unknown) => wizardRequest(page, { kind: "launch_wizard_action", action,
        bounds: { x: 30, y: 50, width: 440, height: 350 } });
      await action({ kind: "use_start_method", method: "configure_and_start" });
      await action({ kind: "set_branch_mode", create_new: true });
      await action({ kind: "set_branch_name", value: `work/loop-load-${index}` });
      let state = await action({ kind: "set_agent", agent_id: `loop-load-${index}` });
      for (let step = 0; step < 12 && state; step++) {
        expect(state.error).toBeFalsy();
        expect(state.primary_action_enabled, state.primary_action_disabled_reason).toBe(true);
        state = state.selected_runtime_target !== "host" && state.runtime_target_options?.some((option: any) => option.value === "host")
          ? await action({ kind: "set_runtime_target", target: "Host" }) : await action({ kind: "submit" });
      }
      expect(state).toBeNull();
      // PTY creation follows Wizard closure; serialize materialization before the next launch.
      await expect.poll(async () => {
        try { return JSON.parse(await readFile(join(manifest.agentDir, `${index}.json`), "utf8")).pid > 0; } catch { return false; }
      }, { timeout: 90_000 }).toBe(true);
      const windows = await projectReply(page, { kind: "list_windows" }, "window_list");
      const entry = windows.windows.find((window: any) => window.agent_id === `loop-load-${index}`);
      expect(entry, `custom agent ${index} window`).toBeTruthy();
      windowIds.push(entry.id);
      const geometry = { x: 30 + (index % 3) * 470, y: 50 + Math.floor(index / 3) * 400, width: 440, height: 350 };
      await sendLiveGwtEvent(page, { kind: "undock_agent_window", id: entry.id, geometry });
      await sendLiveGwtEvent(page, { kind: "update_window_geometry", id: entry.id, geometry, cols: 50, rows: 16 });
    }
    expect(new Set(windowIds).size).toBe(5);
    started = Date.now() + 1000;
    ended = started + manifest.seconds * 1000;
    await writeFile(manifest.barrier, JSON.stringify({ start: started, seconds: manifest.seconds }));
    console.log(JSON.stringify({ phase: "measuring", theme, start: started, end: ended, agents: 5, seconds: manifest.seconds }));
    while (Date.now() < ended) await new Promise(resolve => setTimeout(resolve, Math.min(1000, ended - Date.now())));
    await expect.poll(() => outputs.map(output => output.sequence), { timeout: 30_000 }).toEqual(Array(5).fill(manifest.seconds));
    for (let index = 0; index < 5; index++) {
      await expect.poll(async () => JSON.parse(await readFile(join(manifest.agentDir, `${index}.json`), "utf8")), { timeout: 30_000 })
        .toMatchObject({ index, sequence: manifest.seconds, bytes: manifest.seconds * 8192, complete: true });
      expect(outputs[index].markers).toBe(manifest.seconds);
      await expect.poll(() => page.evaluate(id => String((window as any).__gwtTerminalTestApi?.bufferText?.(id) ?? ""), windowIds[index]), { timeout: 30_000 })
        .toContain(`GWT_LOOP_5379_${index}_${String(manifest.seconds).padStart(4, "0")}`);
    }
    expect(errors).toEqual([]);
    await page.screenshot({ path: join(manifest.home, "canvas.png") });
    passed = true;
  } finally {
    const receipts = await Promise.all(Array.from({ length: 5 }, (_, index) => readFile(join(manifest.agentDir, `${index}.json`), "utf8").then(JSON.parse).catch(() => null)));
    await writeFile(manifest.result, JSON.stringify({ theme, started_at_ms: started, ended_at_ms: ended,
      seconds: manifest.seconds, outputs: outputs.map(({ sequence, markers }) => ({ sequence, markers })), receipts, errors,
      AgentVisualCheck: passed ? "pass" : "fail", UserVerificationResult: "n/a (autonomous)" }, null, 2));
    for (const id of windowIds) await sendLiveGwtEvent(page, { kind: "close_window", id }).catch(() => undefined);
  }
});

async function projectReply(page: Page, detail: unknown, kind: string, requireOpen = false): Promise<any> {
  return page.evaluate(({ detail, kind, requireOpen }) => {
    const socket = [...((window as any).__gwtPlaywrightSockets as WebSocket[])].reverse().find(socket =>
      socket.readyState === WebSocket.OPEN && new URL(socket.url).pathname === "/ws" && new URL(socket.url).searchParams.has("repo_hash"));
    if (!socket) throw new Error("no project socket");
    return new Promise((resolve, reject) => {
      const cleanup = () => { clearTimeout(timer); socket.removeEventListener("message", reply); };
      const reply = (event: MessageEvent) => {
        let payload; try { payload = JSON.parse(String(event.data)); } catch { return; }
        if (payload.kind !== kind || (requireOpen && !payload.wizard) || (payload.wizard &&
          (payload.wizard.is_hydrating || payload.wizard.runtime_resolution_pending || payload.wizard.launch_materialization_pending))) return;
        cleanup(); resolve(payload);
      };
      const timer = setTimeout(() => { cleanup(); reject(new Error(`${kind} reply timed out`)); }, 90_000);
      socket.addEventListener("message", reply);
      window.dispatchEvent(new CustomEvent("__gwt_test_send", { detail }));
    });
  }, { detail, kind, requireOpen });
}

async function wizardRequest(page: Page, detail: unknown, requireOpen = false): Promise<any> {
  return (await projectReply(page, detail, "launch_wizard_state", requireOpen)).wizard;
}
