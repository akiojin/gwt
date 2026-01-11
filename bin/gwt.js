#!/usr/bin/env bun
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { buildBunReexecCommand } from "../dist/utils/bun-runtime.js";

const reexec = buildBunReexecCommand({
  hasBunGlobal: "Bun" in globalThis,
  bunExecPath: process.env.BUN_EXEC_PATH,
  argv: process.argv,
  scriptPath: fileURLToPath(import.meta.url),
});

if (reexec) {
  const result = spawnSync(reexec.command, reexec.args, { stdio: "inherit" });
  if (result.error) {
    const details =
      result.error instanceof Error
        ? result.error.message
        : String(result.error);
    console.error("Bun runtime is required to run gwt.");
    console.error(`Failed to launch Bun: ${details}`);
    process.exit(1);
  }
  process.exit(result.status ?? 1);
}

import("../dist/index.js")
  .then((module) => {
    if (module.main) {
      return module.main();
    }
    throw new Error("main function not found in index.js");
  })
  .catch((err) => {
    console.error(err);
    process.exit(1);
  });
