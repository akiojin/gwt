import solidPlugin from "@opentui/solid/bun-plugin";

const OUT_DIR = ".tmp-tests/solid";
const glob = new Bun.Glob("src/cli/ui/__tests__/solid/**/*.test.tsx");
const rawArgs = Bun.argv.slice(2);
const includeAll = rawArgs.includes("--include-all");
const filters = rawArgs.filter((arg) => arg !== "--include-all");
const entrypoints: string[] = [];

for await (const file of glob.scan(".")) {
  if (!includeAll && file.includes("AllScreens.integration.test.tsx")) {
    continue;
  }
  if (filters.length === 0 || filters.some((filter) => file.includes(filter))) {
    entrypoints.push(file);
  }
}

if (entrypoints.length === 0) {
  if (!includeAll) {
    console.log(
      "All Solid test files were filtered out. Use --include-all to include integration tests.",
    );
  } else {
    console.log("No Solid tests found.");
  }
  process.exit(0);
}

const buildResult = await Bun.build({
  entrypoints,
  outdir: OUT_DIR,
  target: "bun",
  format: "esm",
  splitting: false,
  plugins: [solidPlugin],
});

if (!buildResult.success) {
  console.error("Solid test build failed.");
  for (const log of buildResult.logs) {
    console.error(log);
  }
  process.exit(1);
}

const outputFiles = buildResult.outputs
  .map((output) => output.path)
  .filter((outputPath) => outputPath.endsWith(".js"));

if (outputFiles.length === 0) {
  console.error("No compiled Solid test outputs found.");
  process.exit(1);
}

if (!includeAll) {
  console.log(
    "Skipping AllScreens.integration.test.tsx (Bun segfault). Use --include-all to run it.",
  );
}

for (const file of outputFiles) {
  const proc = Bun.spawn(["bun", "test", file], {
    stdout: "inherit",
    stderr: "inherit",
    stdin: "inherit",
  });
  const exitCode = await proc.exited;
  if (exitCode !== 0) {
    process.exit(exitCode);
  }
}

process.exit(0);
