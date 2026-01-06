import solidPlugin from "@opentui/solid/bun-plugin";

const result = await Bun.build({
  entrypoints: ["./src/opentui/index.solid.ts"],
  target: "bun",
  outdir: "./dist/opentui",
  plugins: [solidPlugin],
});

if (!result.success) {
  for (const log of result.logs) {
    console.error(log);
  }
  process.exit(1);
}
