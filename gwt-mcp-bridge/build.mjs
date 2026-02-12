import * as esbuild from "esbuild";

await esbuild.build({
  entryPoints: ["src/index.ts"],
  bundle: true,
  platform: "node",
  format: "esm",
  target: "node18",
  outfile: "dist/gwt-mcp-bridge.js",
  minify: true,
  banner: {
    js: "#!/usr/bin/env node",
  },
  external: [],
});

console.log("Build complete: dist/gwt-mcp-bridge.js");
