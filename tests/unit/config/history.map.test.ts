import {
  describe,
  it,
  expect,
  mock,
  beforeEach,
  afterEach,
  spyOn,
} from "bun:test";
import * as fsPromises from "node:fs/promises";

let readFile: ReturnType<typeof spyOn<typeof fsPromises, "readFile">>;
let getLastToolUsageMap: typeof import("../../../src/config/index.ts").getLastToolUsageMap;
let importCounter = 0;

describe("getLastToolUsageMap", () => {
  beforeEach(async () => {
    mock.restore();
    readFile = spyOn(fsPromises, "readFile");
    importCounter += 1;
    ({ getLastToolUsageMap } = await import(
      `../../../src/config/index.ts?history-map=${importCounter}`
    ));
  });

  afterEach(() => {
    mock.restore();
  });

  it("returns latest entry per branch", async () => {
    readFile.mockResolvedValue(
      JSON.stringify({
        history: [
          {
            branch: "feature/a",
            worktreePath: "/wt/a",
            toolId: "codex-cli",
            toolLabel: "Codex",
            mode: "normal",
            model: null,
            timestamp: 10,
          },
          {
            branch: "feature/a",
            worktreePath: "/wt/a",
            toolId: "claude-code",
            toolLabel: "Claude",
            mode: "continue",
            model: null,
            timestamp: 20,
          },
          {
            branch: "feature/b",
            worktreePath: "/wt/b",
            toolId: "custom-tool",
            toolLabel: "MyTool",
            mode: null,
            model: null,
            timestamp: 5,
          },
        ],
      }),
    );

    const map = await getLastToolUsageMap("/repo");

    const a = map.get("feature/a");
    const b = map.get("feature/b");
    expect(a?.toolId).toBe("claude-code");
    expect(a?.timestamp).toBe(20);
    expect(b?.toolId).toBe("custom-tool");
  });

  it("handles missing file gracefully", async () => {
    readFile.mockRejectedValue(new Error("not found"));

    const map = await getLastToolUsageMap("/repo");
    expect(map.size).toBe(0);
  });
});
