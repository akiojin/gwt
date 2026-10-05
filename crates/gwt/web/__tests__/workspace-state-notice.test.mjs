import assert from "node:assert/strict";
import test from "node:test";
import { readFileSync } from "node:fs";
import { parseHTML } from "linkedom";
import { createWorkspaceStateNotice } from "../workspace-state-notice.js";

test("load errors preserve file and cause as text and Retry requests a real reload", () => {
  const { document } = parseHTML('<html><body><div id="app"></div></body></html>');
  const sent = [];
  const surface = createWorkspaceStateNotice({ document, send: message => sent.push(message) });
  const notice = { kind: "load_error", path: '/state/<img src=x>/current.json', message: '<script>bad JSON</script>' };
  surface.receive(notice);
  surface.receive(notice);
  assert.equal(document.querySelectorAll(".workspace-state-notice").length, 1);
  const banner = document.querySelector('[role="alert"]');
  assert.ok(banner.textContent.includes(notice.path));
  assert.ok(banner.textContent.includes(notice.message));
  assert.equal(banner.querySelector("img, script"), null);
  banner.querySelector("button").click();
  assert.deepEqual(sent, [{ kind: "retry_workspace_state_load" }]);
  assert.ok(document.querySelector('[role="alert"]'), "retry must not hide an unresolved failure");
  surface.receive(null);
  assert.equal(document.querySelector(".workspace-state-notice"), null);
});

test("notice is embedded, dispatched, and styled with Operator tokens in the existing shell", () => {
  const app = readFileSync(new URL('../app.js', import.meta.url), 'utf8');
  const manifest = readFileSync(new URL('../../src/embedded_web.rs', import.meta.url), 'utf8');
  const styles = readFileSync(new URL('../styles/components.css', import.meta.url), 'utf8');
  assert.match(app, /case "workspace_state_notice":/);
  assert.match(app, /workspaceStateNotice\.receive\(event\.notice\)/);
  assert.match(manifest, /"workspace-state-notice.js" => "createWorkspaceStateNotice"/);
  const rule = styles.match(/\.workspace-state-notice\s*\{([^}]+)\}/)?.[1];
  assert.ok(rule);
  assert.match(rule, /var\(--color-/);
  assert.doesNotMatch(rule, /#[0-9a-f]{3,8}\b|rgba?\(|position:\s*(fixed|absolute)/i);
});
