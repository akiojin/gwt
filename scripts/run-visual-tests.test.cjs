const assert = require("node:assert/strict");
const { spawnSync } = require("node:child_process");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const test = require("node:test");

const bash = process.platform === "win32"
  ? path.join(process.env.ProgramFiles, "Git/bin/bash.exe") : "bash";
const shellPath = value => process.platform === "win32"
  ? value.replaceAll("\\", "/").replace(/^([A-Za-z]):/, (_, drive) => `/${drive.toLowerCase()}`) : value;
const nativePath = value => process.platform === "win32"
  ? value.replace(/^\/([A-Za-z])\//, (_, drive) => `${drive}:/`) : value;

for (const exitCode of [0, 1]) {
  test(`runner keeps PNG/trace and cleans temporary files after exit ${exitCode}`, () => {
    const fixture = fs.mkdtempSync(path.join(os.tmpdir(), "gwt-visual-runner-"));
    try {
      for (const directory of ["scripts", "crates/gwt/playwright/test-results", "crates/gwt/web", "deps/node_modules/.bin", "scratch"]) {
        fs.mkdirSync(path.join(fixture, directory), { recursive: true });
      }
      fs.copyFileSync(path.join(__dirname, "run-visual-tests.sh"), path.join(fixture, "scripts/run-visual-tests.sh"));
      fs.writeFileSync(path.join(fixture, "scripts/playwright-version.txt"), "1.49.1\n");
      fs.writeFileSync(path.join(fixture, "crates/gwt/playwright/playwright.config.ts"), "");
      fs.writeFileSync(path.join(fixture, "crates/gwt/web/index.html"), "fixture");
      fs.writeFileSync(path.join(fixture, "deps/node_modules/.bin/playwright"), `#!/usr/bin/env bash
set -euo pipefail
output="$PWD/crates/gwt/playwright/test-results"
printf '%s\\n' "$@" > "$GWT_RUNNER_TEST_LOG.args"
while [[ $# -gt 0 ]]; do
  case "$1" in
    --output) output="$2"; shift ;;
  esac
  shift
done
test -f crates/gwt/web/index.html
# Playwright replaces its output directory, including any pre-existing symlink.
case "$output" in
  "$GWT_RUNNER_TEST_ROOT"/*) rm -rf "$output" ;;
  *) exit 64 ;;
esac
mkdir -p "$output"
printf 'PNG attachment' > "$output/screenshot.png"
printf 'trace attachment' > "$output/trace.zip"
printf '%s\\n' "$PWD" "$output" > "$GWT_RUNNER_TEST_LOG"
exit "$GWT_RUNNER_TEST_EXIT"
`, { mode: 0o755 });
      const log = path.join(fixture, "run.log");
      const result = spawnSync(bash, [shellPath(path.join(fixture, "scripts/run-visual-tests.sh")), "--headed", "--trace=on"], {
        encoding: "utf8",
        env: {
          ...process.env,
          TMPDIR: shellPath(path.join(fixture, "scratch")),
          GWT_PLAYWRIGHT_DEPS_DIR: shellPath(path.join(fixture, "deps")),
          GWT_RUNNER_TEST_LOG: shellPath(log),
          GWT_RUNNER_TEST_ROOT: shellPath(fixture),
          GWT_RUNNER_TEST_EXIT: String(exitCode),
        },
      });
      assert.equal(result.status, exitCode, result.stderr);
      const [runner, output] = fs.readFileSync(log, "utf8").trim().split("\n");
      const args = fs.readFileSync(log + ".args", "utf8").trim().split("\n");
      assert.equal(fs.existsSync(nativePath(runner)), false, "temporary runner and web symlink must be removed");
      assert.equal(fs.readFileSync(path.join(nativePath(output), "screenshot.png"), "utf8"), "PNG attachment");
      assert.equal(fs.readFileSync(path.join(nativePath(output), "trace.zip"), "utf8"), "trace attachment");
      assert.match(path.relative(path.join(fixture, "crates/gwt/playwright/test-results"), nativePath(output)), /^run\.[\w]+$/);
      assert.ok(args.includes("--headed") && args.includes("--trace=on"), "verification arguments remain forwarded");
    } finally {
      fs.rmSync(fixture, { recursive: true, force: true });
    }
  });
}
