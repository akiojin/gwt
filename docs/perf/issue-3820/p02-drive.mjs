// Issue #3820 T-006 / P02: open N fresh repositories one at a time through the
// production Hub event `reopen_recent_project` and wait for each
// `route:project.open` sample in the isolated perf log.
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";

const [url, perfDir, reposDir, nArg] = process.argv.slice(2);
const n = Number(nArg);
const OPEN_TIMEOUT_MS = 60_000;

function routeSamples() {
  let files = [];
  try {
    files = readdirSync(perfDir).filter((f) => f.startsWith("perf-") && f.endsWith(".jsonl"));
  } catch {
    return [];
  }
  const out = [];
  for (const f of files) {
    for (const line of readFileSync(join(perfDir, f), "utf8").split("\n")) {
      if (!line.includes('"route:project.open"')) continue;
      try {
        out.push(JSON.parse(line));
      } catch {}
    }
  }
  return out;
}

const wsUrl = new URL("/ws", url);
wsUrl.protocol = wsUrl.protocol === "https:" ? "wss:" : "ws:";
wsUrl.search = "";
const ws = new WebSocket(wsUrl);
const errors = [];
ws.onmessage = (event) => {
  const text = String(event.data);
  if (/"kind":"[a-z_]*error[a-z_]*"/.test(text) || /project_open_failed/.test(text)) {
    errors.push(text.slice(0, 500));
  }
};
await new Promise((resolve, reject) => {
  ws.onopen = resolve;
  ws.onerror = () => reject(new Error("websocket error"));
});
ws.send(JSON.stringify({ kind: "frontend_ready" }));
await new Promise((r) => setTimeout(r, 2000));

const results = [];
for (let i = 1; i <= n; i++) {
  const path = join(reposDir, `repo-${i}`);
  const before = routeSamples().length;
  const started = performance.now();
  ws.send(JSON.stringify({ kind: "reopen_recent_project", path, request_id: `p02-${i}` }));
  let observed = null;
  while (performance.now() - started < OPEN_TIMEOUT_MS) {
    const samples = routeSamples();
    if (samples.length > before) {
      observed = samples[samples.length - 1];
      break;
    }
    await new Promise((r) => setTimeout(r, 100));
  }
  results.push({
    index: i,
    route_ms: observed ? observed.value : null,
    send_to_log_observed_ms: observed ? Math.round(performance.now() - started) : null,
    timed_out: observed === null,
  });
  await new Promise((r) => setTimeout(r, 1000));
}
ws.close();
console.log(JSON.stringify({ n, results, hub_error_messages: errors }, null, 2));
