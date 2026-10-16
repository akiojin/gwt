import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import { build, run, runCore, targets } from "./ci-windows-tests.mjs";

test("required Windows CI builds and runs the gate lifecycle regressions", () => {
  assert.ok(targets("default").some(([pkg, kind, name]) =>
    pkg === "gwt-terminal" && kind === "test" && name === "pty_start_gate_test"));
  const workflow = fs.readFileSync(new URL("../.github/workflows/test.yml", import.meta.url), "utf8")
    .replace(/\r\n/g, "\n");
  const windowsJob = workflow.split("  test-windows-rust:\n")[1]?.split(/\n  [\w-]+:/)[0];
  assert.ok(windowsJob?.includes(
    "node scripts/ci-windows-tests.mjs run gwt-terminal test pty_start_gate_test -- --test-threads=1"));
});

test("Windows CI runs the cross-process FIFO reservation contention regression", () => {
  const workflow = fs.readFileSync(new URL("../.github/workflows/test.yml", import.meta.url), "utf8")
    .replace(/\r\n/g, "\n");
  const windowsJob = workflow.split("  test-windows-rust:\n")[1]?.split(/\n  [\w-]+:/)[0];
  assert.ok(windowsJob?.includes(
    "node scripts/ci-windows-tests.mjs run gwt test cli_contracts verification_lease_cli_test::deferred_fifo_survives_status_read_contention_and_new_processes -- --exact --nocapture"));
});

function fixture(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "gwt-windows-tests-"));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  return root;
}

function artifacts(root, group) {
  return targets(group).map(([pkg, kind, name]) => {
    const executable = path.join(root, "target", "debug", "deps", `${kind}-${name}.exe`);
    fs.mkdirSync(path.dirname(executable), { recursive: true });
    fs.writeFileSync(executable, "");
    return {
      reason: "compiler-artifact", package_id: `path+file:///workspace/${pkg}#1.0.0`,
      manifest_path: path.join(root, "crates", pkg, "Cargo.toml"),
      target: { kind: [kind], name }, profile: { test: true }, executable,
    };
  });
}

test("default Windows integration targets use Cargo's consolidated registrations", () => {
  const manifest = fs.readFileSync(new URL("../crates/gwt/Cargo.toml", import.meta.url), "utf8");
  const declared = [...manifest.matchAll(/\[\[test\]\]\s+name = "([^"]+)"/g)].map((match) => match[1]);
  const selected = targets("default").filter(([pkg, kind]) => pkg === "gwt" && kind === "test");
  assert.equal(new Set(selected.map((target) => target.join("|"))).size, selected.length);
  for (const [, , name] of selected) assert.ok(declared.includes(name), `${name} must be a Cargo test target`);
});

test("each feature set builds once and runs the exact target with Cargo's cwd and failure status", (t) => {
  const root = fixture(t);
  for (const group of ["default", "warm"]) {
    let builds = 0;
    const messages = artifacts(root, group);
    assert.equal(build(group, root, (command, args) => {
      builds++;
      assert.equal(command, "cargo");
      assert.equal(args[0], "test");
      assert.ok(args.includes("--no-run"));
      assert.ok(args.includes("--message-format=json"));
      assert.equal(args.includes("--all-features"), group === "warm");
      // A normal binary and a different package's same-name harness must not win.
      const decoys = [
        { ...messages[0], profile: { test: false }, executable: "wrong.exe" },
        { ...messages[0], package_id: "registry+other#unrelated@1.0.0", manifest_path: "/other/Cargo.toml", executable: "wrong.exe" },
      ];
      return { status: 0, stdout: [...decoys, ...messages].map(JSON.stringify).join("\n") };
    }), 0);
    assert.equal(builds, 1);
    assert.equal(run(group, ["gwt", "lib", "gwt", "the_filter", "--", "--exact", "--test-threads=1"], root,
      (command, args, options) => {
        assert.equal(command, messages.find((m) => m.target.kind[0] === "lib" && m.target.name === "gwt").executable);
        assert.deepEqual(args, ["the_filter", "--exact", "--test-threads=1"]);
        assert.equal(options.cwd, path.join(root, "crates", "gwt"));
        const pathKey = Object.keys(options.env).find((key) => key.toLowerCase() === "path");
        assert.ok(options.env[pathKey].split(path.delimiter).includes(path.dirname(command)));
        return { status: 37 };
      }), 37);
  }
});

test("failed builds invalidate old manifests and missing artifacts cannot silently pass", (t) => {
  const root = fixture(t);
  const manifest = path.join(root, "target", "ci-windows-tests-default.json");
  fs.mkdirSync(path.dirname(manifest), { recursive: true });
  fs.writeFileSync(manifest, "stale");
  assert.equal(build("default", root, () => ({ status: 19, stdout: "" })), 19);
  assert.equal(fs.existsSync(manifest), false);
  assert.throws(() => build("default", root, () => ({ status: 0, stdout: "" })), /missing.*gwt/i);
  assert.throws(() => run("default", ["gwt", "lib", "gwt"], root), /ENOENT/);
});

test("core artifacts include every Cargo harness and run without rebuilding at default parallelism", (t) => {
  const root = fixture(t);
  const messages = artifacts(root, "warm").map((message) => ({
    ...message, manifest_path: path.join(root, "crates", "gwt-core", "Cargo.toml"),
    target: { kind: ["lib"], name: "gwt_core" },
  }));
  messages.push({ ...messages[0], target: { kind: ["test"], name: "index_coordinator" },
    executable: path.join(root, "target", "debug", "deps", "index_coordinator.exe") });
  messages.push({ ...messages[0], target: { kind: ["test"], name: "windows_process_resolver" },
    executable: path.join(root, "target", "debug", "deps", "windows_process_resolver.exe") });
  assert.equal(build("core", root, (command, args) => {
    assert.equal(command, "cargo");
    assert.deepEqual(args, ["test", "-p", "gwt-core", "--all-features", "--no-run", "--message-format=json"]);
    return { status: 0, stdout: messages.map(JSON.stringify).join("\n") };
  }), 0);
  const manifest = JSON.parse(fs.readFileSync(path.join(root, "target", "ci-windows-tests-core.json")));
  assert.deepEqual(Object.keys(manifest), ["gwt-core|lib|gwt_core", "gwt-core|test|index_coordinator",
    "gwt-core|test|windows_process_resolver"]);
  assert.equal(run("core", ["gwt-core", "test", "index_coordinator"], root, (command, args) => {
    assert.equal(command, messages[1].executable);
    assert.deepEqual(args, []);
    return { status: 0 };
  }), 0);
  const executed = [];
  assert.equal(runCore(root, (command, args) => {
    executed.push(command);
    assert.deepEqual(args, []);
    return { status: executed.length === 2 ? 23 : 0 };
  }), 23);
  assert.deepEqual(executed, messages.slice(0, 2).map((message) => message.executable));
});
