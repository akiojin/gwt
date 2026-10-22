#!/usr/bin/env node
// Issue #4821: build each Windows CI harness once, then run its filters.
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const defaultTargets = [
  ["gwt", "lib", "gwt"],
  ["gwt", "bin", "gwt"],
  ...["hook_contracts", "ci_contracts", "runtime_tests", "coordination_tests",
    "cli_contracts", "startup_tray_performance"]
    .map((name) => ["gwt", "test", name]),
  ["gwt-core", "lib", "gwt_core"],
  ...["process_priority_roundtrip", "windows_process_resolver", "process_adapter_parity", "windows_claude_user_agent"]
    .map((name) => ["gwt-core", "test", name]),
  ["gwt-agent", "lib", "gwt_agent"],
  ["gwt-terminal", "lib", "gwt_terminal"],
  ["gwt-terminal", "test", "pty_start_gate_test"],
];

export function targets(group) {
  if (group === "default") return defaultTargets;
  if (group === "default-lib") return defaultTargets.filter(([, kind]) => kind === "lib");
  if (group === "default-runtime") return defaultTargets.filter(([, kind]) => kind !== "lib");
  if (group === "warm") return [["gwt", "lib", "gwt"]];
  // Cargo discovers all integration targets; do not maintain a second list.
  if (group === "core") return [["gwt-core", "lib", "gwt_core"]];
  throw new Error(`Unknown Windows test feature set: ${group}`);
}

const manifestFile = (repo, group) => path.join(repo, "target", `ci-windows-tests-${group}.json`);
const key = (target) => target.join("|");

export function build(group, repo = root, spawn = spawnSync) {
  const selected = targets(group);
  const manifest = manifestFile(repo, group);
  fs.rmSync(manifest, { force: true });
  // Keep the same roots in both default slices: agent dev-dependencies enable
  // gwt-docker/test-support even when only runtime harnesses are selected.
  const packages = [...new Set((group.startsWith("default") ? defaultTargets : selected).map(([pkg]) => pkg))];
  const selectors = [...new Map(selected.map(([, kind, name]) =>
    [kind === "lib" ? kind : `${kind}|${name}`, kind === "lib" ? ["--lib"] : [`--${kind}`, name]])).values()].flat();
  const args = ["test", ...packages.flatMap((pkg) => ["-p", pkg]), ...(group === "core" ? [] : selectors),
    ...(group === "warm" || group === "core" ? ["--all-features"] : []), "--no-run", "--message-format=json"];
  console.log(`cargo ${args.join(" ")}`);
  const result = spawn("cargo", args, { cwd: repo, encoding: "utf8", stdio: ["ignore", "pipe", "inherit"], maxBuffer: 64 * 1024 * 1024 });
  if (result.error) throw result.error;
  const messages = result.stdout.split(/\r?\n/).filter(Boolean).map((line) => JSON.parse(line));
  for (const message of messages) {
    if (message.reason === "compiler-message" && message.message?.rendered) process.stderr.write(message.message.rendered);
  }
  if (result.status !== 0) return result.status ?? 1;

  const artifacts = {};
  for (const message of messages) {
    if (message.reason !== "compiler-artifact" || !message.profile.test || !message.executable) continue;
    const pkg = packages.find((candidate) => path.resolve(message.manifest_path) === path.join(repo, "crates", candidate, "Cargo.toml"));
    if (!pkg) continue;
    const target = group === "core" ? [pkg, message.target.kind[0], message.target.name]
      : selected.find(([p, kind, name]) => p === pkg && message.target.kind.includes(kind) && message.target.name === name);
    if (!target) continue;
    if (artifacts[key(target)]) throw new Error(`Duplicate test artifact: ${key(target)}`);
    artifacts[key(target)] = { package_id: message.package_id, executable: message.executable, cwd: path.dirname(message.manifest_path) };
  }
  for (const target of selected) {
    if (!artifacts[key(target)]) throw new Error(`Missing test artifact: ${key(target)}`);
  }
  fs.mkdirSync(path.dirname(manifest), { recursive: true });
  fs.writeFileSync(manifest, JSON.stringify(artifacts, null, 2));
  return 0;
}

export function run(group, argv, repo = root, spawn = spawnSync) {
  targets(group);
  const [pkg, kind, name, ...testArgs] = argv;
  const artifact = JSON.parse(fs.readFileSync(manifestFile(repo, group), "utf8"))[key([pkg, kind, name])];
  if (!artifact) throw new Error(`No ${group} test artifact for ${pkg}/${kind}/${name}`);
  const separator = testArgs.indexOf("--");
  if (separator !== -1) testArgs.splice(separator, 1);
  const env = { ...process.env };
  const pathKey = Object.keys(env).find((name) => name.toLowerCase() === "path") ?? "PATH";
  const deps = path.dirname(artifact.executable);
  env[pathKey] = [deps, path.dirname(deps), env[pathKey] ?? ""].join(path.delimiter);
  const result = spawn(artifact.executable, testArgs, { cwd: artifact.cwd, env, stdio: "inherit" });
  if (result.error) throw result.error;
  return result.status ?? 1;
}

export function runCore(repo = root, spawn = spawnSync, args = []) {
  const artifacts = JSON.parse(fs.readFileSync(manifestFile(repo, "core"), "utf8"));
  if (!artifacts["gwt-core|lib|gwt_core"]) throw new Error("Missing complete core suite");
  for (const target of Object.keys(artifacts)) {
    const status = run("core", [...target.split("|"), ...args], repo, spawn);
    if (status !== 0) return status;
  }
  return 0;
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try {
    const [operation, ...args] = process.argv.slice(2);
    if (operation === "build") process.exitCode = build(args[0]);
    else if (operation === "run-core") process.exitCode = runCore(root, spawnSync, args);
    else if (operation === "run-core-index") process.exitCode = run("core", ["gwt-core", "test", "index_coordinator"]);
    else if (["run", "run-lib", "run-runtime", "run-warm"].includes(operation)) {
      const group = { run: "default", "run-lib": "default-lib", "run-runtime": "default-runtime", "run-warm": "warm" }[operation];
      process.exitCode = run(group, args);
    } else throw new Error("Usage: ci-windows-tests.mjs build <default|default-lib|default-runtime|warm|core> | <run|run-lib|run-runtime|run-warm> <package> <kind> <target> [filter] -- [libtest args] | run-core [libtest args] | run-core-index");
  } catch (error) {
    console.error(error.message);
    process.exitCode = 1;
  }
}
