// Builds the committed React bundle embedded by crates/gwt/src/embedded_web.rs.
// Deterministic: pinned esbuild, fixed target, no timestamps or sourcemaps.
// Usage: node build.mjs [outfile]  (outfile is used by scripts/check-frontend-build.sh)
import { build } from "esbuild";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const here = fileURLToPath(new URL(".", import.meta.url));
const outfile = process.argv[2]
  ? resolve(process.argv[2])
  : fileURLToPath(new URL("../web/react/islands.js", import.meta.url));

await build({
  absWorkingDir: here,
  entryPoints: ["src/islands/index.js"],
  outfile,
  bundle: true,
  format: "esm",
  platform: "browser",
  target: ["es2022"],
  minify: true,
  sourcemap: false,
  legalComments: "eof",
  charset: "utf8",
  jsx: "automatic",
  define: { "process.env.NODE_ENV": '"production"' },
  logLevel: "warning",
});
