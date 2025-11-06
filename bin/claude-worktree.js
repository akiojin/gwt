#!/usr/bin/env bun

import { existsSync } from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import path from "node:path";

async function resolveEntry() {
  const binDir = path.dirname(fileURLToPath(import.meta.url));
  const distEntry = path.join(binDir, "..", "dist", "index.js");
  if (existsSync(distEntry)) {
    return pathToFileURL(distEntry).href;
  }
  // Fallback to TypeScript source when dist is unavailable
  const srcEntry = path.join(binDir, "..", "src", "index.ts");
  return pathToFileURL(srcEntry).href;
}

async function waitForEnter() {
  const { stdin, stdout } = process;

  if (!stdin || !stdin.isTTY) {
    return;
  }

  if (typeof stdin.setRawMode === "function") {
    try {
      stdin.setRawMode(false);
    } catch {
      // ignore raw mode reset errors
    }
  }

  await new Promise((resolve) => {
    const cleanup = () => {
      stdin.removeListener("data", onData);
      if (typeof stdin.pause === "function") {
        stdin.pause();
      }
    };

    const onData = (chunk) => {
      const data =
        typeof chunk === "string" ? chunk : chunk.toString("utf8");
      if (data.includes("\n") || data.includes("\r")) {
        cleanup();
        resolve();
      }
    };

    stdout?.write?.("\nエラー内容を確認したら Enter キーを押してください。\n");

    if (typeof stdin.resume === "function") {
      stdin.resume();
    }

    stdin.on("data", onData);
  });
}

const entry = await resolveEntry();
const { main } = await import(entry);

main().catch(async (error) => {
  console.error("Error:", error.message);
  await waitForEnter();
  process.exit(1);
});
