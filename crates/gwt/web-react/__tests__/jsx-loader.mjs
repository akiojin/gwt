// Node ESM loader that compiles .jsx sources with the lockfile-pinned esbuild,
// so node --test can import React surfaces without a bundling step.
import { readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { transform } from "esbuild";

export async function load(url, context, nextLoad) {
  if (url.endsWith(".jsx")) {
    const source = await readFile(fileURLToPath(url), "utf8");
    const { code } = await transform(source, { loader: "jsx", jsx: "automatic", format: "esm" });
    return { format: "module", source: code, shortCircuit: true };
  }
  return nextLoad(url, context);
}
