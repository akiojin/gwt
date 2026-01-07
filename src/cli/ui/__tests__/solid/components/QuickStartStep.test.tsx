/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { QuickStartStep } from "../../../components/solid/QuickStartStep.js";
import type { ToolSessionEntry } from "../../../../../config/index.js";

const mockHistory: ToolSessionEntry[] = [
  {
    toolId: "claude-code",
    toolLabel: "Claude Code",
    branch: "feature/test",
    worktreePath: "/path/to/worktree",
    model: "claude-sonnet-4-20250514",
    mode: "normal",
    timestamp: Date.now(),
    sessionId: "session-123",
  },
  {
    toolId: "codex-cli",
    toolLabel: "Codex CLI",
    branch: "feature/test",
    worktreePath: "/path/to/worktree",
    model: "o3-mini",
    mode: "normal",
    timestamp: Date.now() - 1000,
    sessionId: "session-456",
    reasoningLevel: "high",
  },
];

// T501: クイック選択画面の表示テスト
describe("QuickStartStep", () => {
  describe("display", () => {
    it("renders Quick Start title", async () => {
      const testSetup = await testRender(
        () => (
          <QuickStartStep
            history={mockHistory}
            onResume={() => {}}
            onStartNew={() => {}}
            onChooseDifferent={() => {}}
            onBack={() => {}}
          />
        ),
        { width: 60, height: 24 },
      );
      await testSetup.renderOnce();

      try {
        const frame = testSetup.captureCharFrame();
        expect(frame).toContain("Quick Start");
      } finally {
        testSetup.renderer.destroy();
      }
    });

    it("renders session id for resume description when available", async () => {
      const testSetup = await testRender(
        () => (
          <QuickStartStep
            history={mockHistory}
            onResume={() => {}}
            onStartNew={() => {}}
            onChooseDifferent={() => {}}
            onBack={() => {}}
          />
        ),
        { width: 80, height: 24 },
      );
      await testSetup.renderOnce();

      try {
        const frame = testSetup.captureCharFrame();
        expect(frame).toContain("session-123");
      } finally {
        testSetup.renderer.destroy();
      }
    });
  });

  // T502: ヘルプテキストの表示テスト
  describe("help display", () => {
    it("displays help text", async () => {
      const testSetup = await testRender(
        () => (
          <QuickStartStep
            history={mockHistory}
            onResume={() => {}}
            onStartNew={() => {}}
            onChooseDifferent={() => {}}
            onBack={() => {}}
          />
        ),
        { width: 80, height: 24 },
      );
      await testSetup.renderOnce();

      try {
        const frame = testSetup.captureCharFrame();
        // ヘルプテキストが表示される
        expect(frame).toContain("[Esc] Cancel");
        expect(frame).toContain("[Enter] Select");
      } finally {
        testSetup.renderer.destroy();
      }
    });
  });

  // T503: 「Resume with previous settings」選択時の動作テスト
  describe("Resume with previous settings", () => {
    it("calls onResume with selected entry when Resume is selected", async () => {
      let resumedEntry: ToolSessionEntry | null = null;
      const testSetup = await testRender(
        () => (
          <QuickStartStep
            history={mockHistory}
            onResume={(entry) => {
              resumedEntry = entry;
            }}
            onStartNew={() => {}}
            onChooseDifferent={() => {}}
            onBack={() => {}}
          />
        ),
        { width: 60, height: 24 },
      );
      await testSetup.renderOnce();

      try {
        // 最初の項目（Claude Code の Resume）を選択
        testSetup.mockInput.pressEnter();
        await testSetup.renderOnce();
        expect(resumedEntry).not.toBeNull();
        expect(resumedEntry?.toolId).toBe("claude-code");
      } finally {
        testSetup.renderer.destroy();
      }
    });
  });

  // T504: 「Start new with previous settings」選択時の動作テスト
  describe("Start new with previous settings", () => {
    it("calls onStartNew with selected entry when Start new is selected", async () => {
      let startNewEntry: ToolSessionEntry | null = null;
      const testSetup = await testRender(
        () => (
          <QuickStartStep
            history={mockHistory}
            onResume={() => {}}
            onStartNew={(entry) => {
              startNewEntry = entry;
            }}
            onChooseDifferent={() => {}}
            onBack={() => {}}
          />
        ),
        { width: 60, height: 24 },
      );
      await testSetup.renderOnce();

      try {
        // 矢印キーで Start new を選択
        testSetup.mockInput.pressArrow("down");
        await testSetup.renderOnce();
        testSetup.mockInput.pressEnter();
        await testSetup.renderOnce();
        expect(startNewEntry).not.toBeNull();
      } finally {
        testSetup.renderer.destroy();
      }
    });
  });

  // T505: 「Choose different settings...」選択時の動作テスト
  describe("Choose different settings", () => {
    it("calls onChooseDifferent when selected", async () => {
      let chooseDifferentCalled = false;
      const testSetup = await testRender(
        () => (
          <QuickStartStep
            history={mockHistory}
            onResume={() => {}}
            onStartNew={() => {}}
            onChooseDifferent={() => {
              chooseDifferentCalled = true;
            }}
            onBack={() => {}}
          />
        ),
        { width: 60, height: 24 },
      );
      await testSetup.renderOnce();

      try {
        // 矢印キーで最後の項目（Choose different settings）を選択
        for (let i = 0; i < 5; i++) {
          testSetup.mockInput.pressArrow("down");
          await testSetup.renderOnce();
        }
        testSetup.mockInput.pressEnter();
        await testSetup.renderOnce();
        expect(chooseDifferentCalled).toBe(true);
      } finally {
        testSetup.renderer.destroy();
      }
    });
  });

  // T506: 履歴がない場合のスキップテスト
  describe("empty history", () => {
    it("calls onChooseDifferent immediately when history is empty", async () => {
      let chooseDifferentCalled = false;
      const testSetup = await testRender(
        () => (
          <QuickStartStep
            history={[]}
            onResume={() => {}}
            onStartNew={() => {}}
            onChooseDifferent={() => {
              chooseDifferentCalled = true;
            }}
            onBack={() => {}}
          />
        ),
        { width: 60, height: 24 },
      );
      await testSetup.renderOnce();

      try {
        // 履歴がない場合は自動的に onChooseDifferent が呼ばれる
        expect(chooseDifferentCalled).toBe(true);
      } finally {
        testSetup.renderer.destroy();
      }
    });
  });
});
