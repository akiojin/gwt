import { test } from "node:test";
import assert from "node:assert/strict";
import { parseHTML } from "linkedom";
import { renderHookCenterPanel } from "../hook-center-panel.js";

test("Hook Center renders observations, exact trust gaps and unexecuted repair steps as text", () => {
  const { document } = parseHTML("<html><body><section></section></body></html>");
  const panel = document.querySelector("section");
  const report = {
    health: { status: "needs_attention", issues: ["Binary skew"], last_event: "Stop" },
    managed_hook_bin: "/current/gwtd",
    configuration_audit: {
      asset_root: "/shared/project", gwt_bin_path: "/caller/gwtd", gwt_hook_bin: "/pin/gwtd", profile_path: null,
      configs: [{ provider: "claude", path: "/shared/.claude/settings.local.json", state: "present", error: null,
        event_binaries: [{ event: "Stop", fallback_binary: null }] },
      { provider: "codex", path: "/shared/.codex/hooks.json", state: "invalid_json", error: "Invalid JSON <script>bad()</script>", event_binaries: [] }],
    },
    codex_hook_audit: { discovery_mode: "both", expected_hook_paths: ["/linked/.codex/hooks.json", "/shared/.codex/hooks.json"],
      trust: { config_path: "/home/.codex/config.toml", trusted_entries: [{ key: "known:Stop:0:0", trusted_hash: "abc123" }],
        missing_managed_events: ["shared:SessionStart"], modified_gwt_hooks: ["linked:Stop"],
        untrusted_gwt_hooks: ["linked:Stop:0:0"] }, error: null },
    repair_plan: { automatic: ["regenerate_managed_provider_configs"], user_actions: ["verify_codex_hook_trust"], new_session_actions: ["start_new_agent_session"] },
    repair: null,
  };
  const saved = JSON.stringify(report);
  renderHookCenterPanel(panel, { projectRoot: "/project", state: { report } });
  assert.equal(panel.dataset.hookAuditStatus, "loaded");
  assert.equal(panel.querySelector('[data-role="hook-health"]').dataset.status, "needs_attention");
  assert.equal(panel.querySelector('[data-action="repair-managed-hooks"]').disabled, true);
  assert.equal(panel.querySelector('[data-action="refresh-hook-audit"]').disabled, false);
  assert.match(panel.textContent, /Fix the listed invalid or unreadable configuration, then refresh before regenerating\./);
  assert.equal(JSON.stringify(report), saved);
  for (const value of ["Needs attention", "Binary skew", "/current/gwtd", "/caller/gwtd", "/pin/gwtd", "/shared/project",
    "/shared/.claude/settings.local.json", "Readable JSON", "Invalid JSON", "Stop · Unknown", "Not configured",
    "both", "/linked/.codex/hooks.json", "known:Stop:0:0", "abc123", "shared:SessionStart", "linked:Stop", "linked:Stop:0:0",
    "Regenerate managed provider configs", "Verify Codex hook trust", "Start a new agent session"]) assert.ok(panel.textContent.includes(value), value);
  assert.match(panel.textContent, /does not imply managed or trusted/);
  assert.match(panel.textContent, /GUI caller.*agent session/s);
  assert.match(panel.textContent, /No repair requested/);
  assert.match(panel.textContent, /Latest audit: Read-only/);
  assert.equal(panel.querySelectorAll("script").length, 0);
  assert.equal(panel.querySelectorAll('input[type="checkbox"]').length, 0, "profile inspection does not opt in collection");
});

test("Hook Center lists the registered hooks from the audit and registers one from a timing and a script location", () => {
  const { document, Event } = parseHTML("<html><body><section></section></body></html>");
  const panel = document.querySelector("section");
  const changes = [];
  const render = userHooks => renderHookCenterPanel(panel, {
    projectRoot: "/project", state: { report: { health: { status: "ready" }, user_hooks: userHooks } },
    onChangeUserHook: change => changes.push(change),
  });
  const rows = () => [...panel.querySelectorAll('[data-role="user-hook"]')];

  render({ hooks: [], error: null, change_error: null });
  assert.equal(rows().length, 0);
  assert.match(panel.textContent, /No hooks are registered\./);
  assert.match(panel.textContent, /Registered hooks are included in this audit\./);
  const timing = panel.querySelector('select[data-role="user-hook-timing"]');
  const script = panel.querySelector('input[data-role="user-hook-script"]');
  assert.deepEqual([...timing.options].map(option => [option.value, option.textContent]),
    [["session_start", "Session start"], ["stop", "Stop"]]);
  for (const option of timing.options) option.selected = option.value === "stop";
  timing.dispatchEvent(new Event("change"));
  script.value = ".gwt/hooks/notify.sh";
  script.dispatchEvent(new Event("input"));
  panel.querySelector('[data-action="register-user-hook"]').click();
  assert.deepEqual(changes, [{ action: "register", timing: "stop", script: ".gwt/hooks/notify.sh" }]);

  // A refused registration keeps what was typed and says why.
  render({ hooks: [], error: null, change_error: "No file exists at /project/.gwt/hooks/notify.sh." });
  assert.equal(panel.querySelector('[data-role="user-hook-change-error"]').getAttribute("role"), "alert");
  assert.match(panel.textContent, /No file exists at \/project\/\.gwt\/hooks\/notify\.sh\./);
  assert.equal(panel.querySelector('input[data-role="user-hook-script"]').value, ".gwt/hooks/notify.sh");
  assert.equal(panel.querySelector('select[data-role="user-hook-timing"]').value, "stop");
  panel.querySelector('[data-action="register-user-hook"]').click();

  render({ hooks: [
    { timing: "stop", script: "/project/.gwt/hooks/notify.sh", state: "ready" },
    { timing: "session_start", script: "/gone/setup.sh", state: "missing" },
  ], error: null, change_error: null });
  assert.equal(panel.querySelector('input[data-role="user-hook-script"]').value, "", "an accepted registration clears the form");
  assert.deepEqual(rows().map(row => [row.dataset.state, row.textContent.includes("Timing: Stop"), row.textContent.includes("Timing: Session start")]),
    [["ready", true, false], ["missing", false, true]]);
  for (const value of ["notify.sh", "Location: /project/.gwt/hooks/notify.sh", "Registered",
    "setup.sh", "Location: /gone/setup.sh", "Script missing"]) assert.ok(panel.textContent.includes(value), value);
  rows()[1].querySelector('[data-action="remove-user-hook"]').click();
  assert.deepEqual(changes.at(-1), { action: "remove", timing: "session_start", script: "/gone/setup.sh" });

  render({ hooks: [], error: "Registered hooks could not be read: invalid data", change_error: null });
  assert.match(panel.textContent, /Registered hooks could not be read: invalid data/);
});
