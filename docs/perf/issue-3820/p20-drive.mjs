// Open one fixture project via the production Hub event, then hold the session idle.
const [url, fixDir, secs, nArg] = process.argv.slice(2);
const wsUrl = new URL("/ws", url); wsUrl.protocol = "ws:"; wsUrl.search = "";
const ws = new WebSocket(wsUrl);
const errors = [];
ws.onmessage = (e) => { const t = String(e.data); if (/"kind":"[a-z_]*error[a-z_]*"/.test(t) || /project_open_failed/.test(t)) errors.push(t.slice(0, 300)); };
await new Promise((res, rej) => { ws.onopen = res; ws.onerror = () => rej(new Error("ws error")); });
ws.send(JSON.stringify({ kind: "frontend_ready" }));
await new Promise((r) => setTimeout(r, 2000));
for (let i = 1; i <= Number(nArg || 1); i++) {
  ws.send(JSON.stringify({ kind: "reopen_recent_project", path: fixDir + "/repo-" + i, request_id: "p20-open-" + i }));
  console.log("opened", i, new Date().toISOString());
  await new Promise((r) => setTimeout(r, 5000));
}
await new Promise((r) => setTimeout(r, Number(secs) * 1000));
console.log("done", new Date().toISOString(), "errors", errors.length);
for (const e of errors) console.log(e);
ws.close();
process.exit(0);
