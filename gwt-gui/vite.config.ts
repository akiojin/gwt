import path from "node:path";
import { configDefaults, defineConfig } from "vitest/config";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import istanbul from "vite-plugin-istanbul";

export default defineConfig({
  plugins: [
    svelte(),
    ...(process.env.E2E_COVERAGE === "1"
      ? [
          istanbul({
            include: "src/**/*",
            exclude: ["src/**/*.test.ts", "src/**/*.spec.ts", "src/app.d.ts"],
            extension: [".ts", ".svelte"],
            requireEnv: false,
            forceBuildInstrument: true,
          }),
        ]
      : []),
  ],
  clearScreen: false,
  resolve: {
    alias: {
      $lib: path.resolve(__dirname, "src/lib"),
    },
    // Ensure Svelte resolves to the client build (mount available) under Vitest.
    conditions: ["browser"],
  },
  server: {
    port: 5173,
    strictPort: true,
    watch: {
      ignored: ["**/coverage-e2e/**", "**/.nyc_output/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_"],
  build: {
    target: "esnext",
    minify: !process.env.TAURI_DEBUG ? "esbuild" : false,
    sourcemap: !!process.env.TAURI_DEBUG || process.env.E2E_COVERAGE === "1",
  },
  test: {
    environment: "jsdom",
    globals: true,
    setupFiles: ["./vitest.setup.ts"],
    exclude: [...configDefaults.exclude, "e2e/**", "e2e-tauri/**"],
    coverage: {
      provider: "v8",
      reporter: ["text", "lcov", "html"],
      reportsDirectory: "./coverage",
      include: ["src/**/*.{ts,svelte}"],
      exclude: [
        "src/**/*.test.ts",
        "src/**/*.spec.ts",
        "src/lib/types.ts",
        "src/App.svelte",
        "src/main.ts",
        "src/app.d.ts",
        "src/vite-env.d.ts",
        // Svelte template branch instrumentation is noisy for UI-heavy components.
        // Keep these covered via integration tests, while thresholding logic modules here.
        "src/lib/components/**/*.svelte",
        // prPolling.svelte.ts reports 0% coverage in full suite due to vitest V8
        // coverage merge bug (93.75% when tested in isolation). Exclude to prevent
        // false coverage penalty. Tracked by 9 dedicated tests in prPolling.test.ts.
        "src/lib/prPolling.svelte.ts",
      ],
      thresholds: { lines: 90, functions: 90, branches: 90, statements: 90 },
    },
  },
});
