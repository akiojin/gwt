#!/usr/bin/env node
// Issue #3845: exercise the repository's real timeout, without a faster test profile.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { mkdtemp, mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const config = fileURLToPath(new URL("../.config/nextest.toml", import.meta.url));
const fixture = await mkdtemp(join(tmpdir(), "gwt-nextest-timeout-"));
const pidFile = join(fixture, "hanging-test.pid");
const passFile = join(fixture, "following-test.passed");
let output = "";
let expired = false;

async function killHangingTest() {
  try {
    const pid = Number(await readFile(pidFile, "utf8"));
    if (Number.isInteger(pid) && pid > 0) process.kill(pid, "SIGKILL");
  } catch (error) {
    if (!["ENOENT", "ESRCH"].includes(error.code)) throw error;
  }
}

try {
  await mkdir(join(fixture, "src"));
  await writeFile(join(fixture, "Cargo.toml"), `
[package]
name = "gwt-timeout-fixture"
version = "0.0.0"
edition = "2021"
[workspace]
`);
  await writeFile(join(fixture, "src/lib.rs"), `
#[cfg(test)]
mod tests {
    #[test]
    fn a_hangs() {
        std::fs::write(std::env::var("GWT_TIMEOUT_PID_FILE").unwrap(),
            std::process::id().to_string()).unwrap();
        loop { std::thread::park(); }
    }

    #[test]
    fn z_following_test_passes() {
        std::fs::write(std::env::var("GWT_TIMEOUT_PASS_FILE").unwrap(), "passed").unwrap();
    }
}
`);
  const child = spawn("cargo", [
    "nextest", "run", "--config-file", config, "--test-threads=1", "--color=never",
  ], {
    cwd: fixture,
    env: {
      ...process.env,
      CARGO_TARGET_DIR: join(fixture, "target"),
      GWT_TIMEOUT_PID_FILE: pidFile,
      GWT_TIMEOUT_PASS_FILE: passFile,
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  for (const stream of [child.stdout, child.stderr]) {
    stream.on("data", (data) => {
      output += data;
      process.stdout.write(data);
    });
  }
  // Includes compilation and nextest's termination grace period. If the
  // configured timeout regresses, also reap the isolated fixture process.
  const deadline = setTimeout(() => {
    expired = true;
    void killHangingTest()
      .catch((error) => console.error("fixture cleanup failed:", error))
      .finally(() => child.kill("SIGKILL"));
  }, 180_000);
  let result;
  try {
    result = await new Promise((resolve, reject) => {
      child.once("error", reject);
      child.once("close", (code, signal) => resolve({ code, signal }));
    });
  } finally {
    clearTimeout(deadline);
  }
  assert.equal(expired, false, "outer deadline fired: repository per-test timeout did not finish");
  assert.equal(result.signal, null, "runner must finish normally with a failing exit status");
  assert.ok(result.code > 0, "the timeout must fail the overall run");
  const timeout = output.search(/TIMEOUT[^\n]*gwt-timeout-fixture\s+tests::a_hangs\b/);
  const following = output.search(/PASS[^\n]*gwt-timeout-fixture\s+tests::z_following_test_passes\b/);
  assert.ok(timeout >= 0, "timeout must identify the binary and full test name");
  assert.ok(following > timeout, "the following test must pass after the timeout");
  assert.equal(await readFile(passFile, "utf8"), "passed");
  console.log("PASS: per-test timeout failed the named test and continued the suite");
} finally {
  // The deadline already killed the fixture if necessary; do not signal
  // the recorded PID again after nextest has reaped it (PID reuse).
  await rm(fixture, { recursive: true, force: true });
}
