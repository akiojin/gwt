import { describe, expect, it } from "vitest";
import { branchInventoryKey } from "./branchInventory";

describe("branchInventory", () => {
  it("normalizes remote refs to canonical keys", () => {
    expect(branchInventoryKey("origin/feature/demo")).toBe("feature/demo");
  });

  it("keeps local refs unchanged", () => {
    expect(branchInventoryKey("feature/demo")).toBe("feature/demo");
  });
});
