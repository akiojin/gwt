/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { Stats } from "../../../components/solid/Stats.js";
import type { BranchViewMode, Statistics } from "../../../types.js";

const baseStats: Statistics = {
  localCount: 10,
  remoteCount: 8,
  worktreeCount: 3,
  changesCount: 2,
  lastUpdated: new Date("2026-01-01T00:00:00Z"),
};

const renderStats = async (options: {
  stats?: Statistics;
  separator?: string;
  lastUpdated?: Date | null;
  viewMode?: BranchViewMode;
  width?: number;
  height?: number;
}) => {
  const testSetup = await testRender(
    () => (
      <Stats
        stats={options.stats ?? baseStats}
        separator={options.separator}
        lastUpdated={options.lastUpdated}
        viewMode={options.viewMode}
      />
    ),
    {
      width: options.width ?? 80,
      height: options.height ?? 3,
    },
  );

  await testSetup.renderOnce();

  const cleanup = () => {
    testSetup.renderer.destroy();
  };

  return {
    ...testSetup,
    cleanup,
  };
};

const withMockedDate = async <T,>(nowMs: number, run: () => Promise<T>) => {
  const RealDate = Date;
  class MockDate extends RealDate {
    constructor(...args: ConstructorParameters<typeof Date>) {
      if (args.length === 0) {
        super(nowMs);
      } else {
        super(...args);
      }
    }

    static now() {
      return nowMs;
    }
  }

  globalThis.Date = MockDate as DateConstructor;
  try {
    return await run();
  } finally {
    globalThis.Date = RealDate;
  }
};

describe("Solid Stats", () => {
  it("renders stats values", async () => {
    const { captureCharFrame, cleanup } = await renderStats({});

    try {
      const frame = captureCharFrame();
      expect(frame).toContain("Local: 10");
      expect(frame).toContain("Remote: 8");
      expect(frame).toContain("Worktrees: 3");
      expect(frame).toContain("Changes: 2");
    } finally {
      cleanup();
    }
  });

  it("renders view mode", async () => {
    const { captureCharFrame, cleanup } = await renderStats({
      viewMode: "remote",
    });

    try {
      expect(captureCharFrame()).toContain("Mode: Remote");
    } finally {
      cleanup();
    }
  });

  it("accepts custom separator", async () => {
    const { captureCharFrame, cleanup } = await renderStats({
      separator: " | ",
    });

    try {
      expect(captureCharFrame()).toContain(" | ");
    } finally {
      cleanup();
    }
  });

  it("shows updated when lastUpdated is provided", async () => {
    const now = new Date("2026-01-05T12:00:00Z").getTime();
    await withMockedDate(now, async () => {
      const lastUpdated = new Date(now - 30_000);
      const { captureCharFrame, cleanup } = await renderStats({
        lastUpdated,
      });

      try {
        const frame = captureCharFrame();
        expect(frame).toContain("Updated:");
        expect(frame).toContain("ago");
      } finally {
        cleanup();
      }
    });
  });

  it("formats relative time in seconds", async () => {
    const now = new Date("2026-01-05T12:00:00Z").getTime();
    await withMockedDate(now, async () => {
      const lastUpdated = new Date(now - 30_000);
      const { captureCharFrame, cleanup } = await renderStats({
        lastUpdated,
      });

      try {
        expect(captureCharFrame()).toContain("30s ago");
      } finally {
        cleanup();
      }
    });
  });

  it("formats relative time in minutes", async () => {
    const now = new Date("2026-01-05T12:00:00Z").getTime();
    await withMockedDate(now, async () => {
      const lastUpdated = new Date(now - 120_000);
      const { captureCharFrame, cleanup } = await renderStats({
        lastUpdated,
      });

      try {
        expect(captureCharFrame()).toContain("2m ago");
      } finally {
        cleanup();
      }
    });
  });

  it("formats relative time in hours", async () => {
    const now = new Date("2026-01-05T12:00:00Z").getTime();
    await withMockedDate(now, async () => {
      const lastUpdated = new Date(now - 7_200_000);
      const { captureCharFrame, cleanup } = await renderStats({
        lastUpdated,
      });

      try {
        expect(captureCharFrame()).toContain("2h ago");
      } finally {
        cleanup();
      }
    });
  });

  it("does not render updated when lastUpdated is null", async () => {
    const { captureCharFrame, cleanup } = await renderStats({
      lastUpdated: null,
    });

    try {
      expect(captureCharFrame()).not.toContain("Updated:");
    } finally {
      cleanup();
    }
  });
});
