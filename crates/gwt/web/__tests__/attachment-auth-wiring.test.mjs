import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import test from "node:test";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const appSource = readFileSync(resolve(here, "../app.js"), "utf8");
const attachmentsSource = readFileSync(
  resolve(here, "../terminal-attachments.js"),
  "utf8",
);

test("attachment upload capability waits for the frontend session exchange", () => {
  assert.match(
    appSource,
    /createTerminalAttachments\s*\(\s*\{[\s\S]*?ensureFrontendSession,[\s\S]*?\}\s*\)/,
  );

  const loaderStart = attachmentsSource.indexOf("async function attachmentUploadToken()");
  const sessionExchange = attachmentsSource.indexOf(
    "ensureFrontendSession()",
    loaderStart,
  );
  const tokenFetch = attachmentsSource.indexOf(
    'fetch("/internal/attachment-upload-token"',
    loaderStart,
  );
  assert.notEqual(loaderStart, -1);
  assert.notEqual(sessionExchange, -1);
  assert.notEqual(tokenFetch, -1);
  assert.ok(sessionExchange < tokenFetch, "session exchange must precede token fetch");
  assert.match(
    attachmentsSource.slice(tokenFetch, tokenFetch + 180),
    /method:\s*"POST"/,
  );
  assert.match(attachmentsSource, /xhr\.withCredentials\s*=\s*true/);
});
