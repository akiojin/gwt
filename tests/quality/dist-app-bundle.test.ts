import { access, readFile } from "node:fs/promises";
import { constants } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";

const bundlePath = resolve(
  process.cwd(),
  "dist/cli/ui/screens/solid/BranchListScreen.js",
);

describe("Dist bundle integrity", () => {
  it("reflects BranchList cleanup UI implementation", async () => {
    await expect(access(bundlePath, constants.F_OK)).resolves.toBeUndefined();

    const content = await readFile(bundlePath, "utf8");

    expect(content).toContain("cleanupUI");
    expect(content).not.toMatch(/PRCleanupScreen/);
  });
});
