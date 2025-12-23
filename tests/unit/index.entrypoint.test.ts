import path from "node:path";
import { pathToFileURL } from "node:url";
import { describe, expect, it } from "vitest";
import { isEntryPoint } from "../../src/index.js";

describe("isEntryPoint", () => {
  it("matches resolved relative argv path", () => {
    const entryPath = path.join(process.cwd(), "dist/index.js");
    const metaUrl = pathToFileURL(entryPath).href;

    expect(isEntryPoint(metaUrl, "./dist/index.js")).toBe(true);
  });

  it("returns false when paths do not match", () => {
    const entryPath = path.join(process.cwd(), "dist/index.js");
    const metaUrl = pathToFileURL(entryPath).href;

    expect(isEntryPoint(metaUrl, "./dist/other.js")).toBe(false);
  });

  it("returns false when argv is missing", () => {
    const entryPath = path.join(process.cwd(), "dist/index.js");
    const metaUrl = pathToFileURL(entryPath).href;

    expect(isEntryPoint(metaUrl, undefined)).toBe(false);
  });
});
