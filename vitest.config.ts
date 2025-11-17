import { defineConfig } from "vitest/config";
import path from "path";

export default defineConfig({
  test: {
    globals: true,
    environment: "happy-dom",
    setupFiles: ["./vitest.setup.ts"],
    include: [
      "tests/**/*.test.ts",
      "tests/**/*.test.tsx",
      "tests/**/*.spec.ts",
      "tests/**/*.spec.tsx",
      "src/**/*.test.ts",
      "src/**/*.test.tsx",
    ],
    exclude: ["node_modules", "dist", "build"],
    coverage: {
      provider: "v8",
      reporter: ["text", "json", "html", "lcov"],
      exclude: [
        "node_modules/",
        "dist/",
        "build/",
        "tests/",
        "**/*.test.ts",
        "**/*.spec.ts",
        "**/types.ts",
        "bin/",
        "vitest.config.ts",
        "eslint.config.js",
        "src/index.ts",
        "src/utils.ts",
        "src/claude.ts",
        "src/codex.ts",
        "src/github.ts",
        "src/ui/display.ts",
        "src/ui/prompts.ts",
        "src/services/**",
        "src/repositories/**",
        "src/claude-history.ts",
      ],
      thresholds: {
        lines: 30,
        functions: 50,
        branches: 70,
        statements: 30,
      },
    },
    testTimeout: 10000,
    hookTimeout: 10000,
  },
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
      "@tests": path.resolve(__dirname, "./tests"),
    },
    extensions: [".ts", ".tsx", ".js", ".jsx", ".json"],
  },
});
