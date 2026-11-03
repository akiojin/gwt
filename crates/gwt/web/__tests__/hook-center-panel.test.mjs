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
