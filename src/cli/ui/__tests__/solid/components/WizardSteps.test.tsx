/** @jsxImportSource @opentui/solid */
import { beforeEach, describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import {
  BranchTypeStep,
  BranchNameStep,
  AgentSelectStep,
  VersionSelectStep,
  ModelSelectStep,
  ReasoningLevelStep,
  ExecutionModeStep,
  SkipPermissionsStep,
} from "../../../components/solid/WizardSteps.js";
import {
  clearInstalledVersionCache,
  setInstalledVersionCache,
} from "../../../utils/installedVersionCache.js";
import {
  clearVersionCache,
  setVersionCache,
} from "../../../utils/versionCache.js";

// T405: ブランチタイプ選択ステップのテスト
describe("BranchTypeStep", () => {
  it("renders branch type options", async () => {
    const testSetup = await testRender(
      () => <BranchTypeStep onSelect={() => {}} onBack={() => {}} />,
      { width: 60, height: 20 },
    );
    await testSetup.renderOnce();

    try {
      const frame = testSetup.captureCharFrame();
      // タイトルと最初の選択肢（feature/）が表示されることを確認
      expect(frame).toContain("Select branch type");
      expect(frame).toContain("feature/");
    } finally {
      testSetup.renderer.destroy();
    }
  });

  it("calls onSelect with selected branch type", async () => {
    let selectedType = "";
    const testSetup = await testRender(
      () => (
        <BranchTypeStep
          onSelect={(type) => {
            selectedType = type;
          }}
          onBack={() => {}}
        />
      ),
      { width: 60, height: 20 },
    );
    await testSetup.renderOnce();

    try {
      // Enterキーで選択
      testSetup.mockInput.pressEnter();
      await testSetup.renderOnce();
      expect(selectedType).toBe("feature/");
    } finally {
      testSetup.renderer.destroy();
    }
  });
});

// T406: ブランチ名入力ステップのテスト
describe("BranchNameStep", () => {
  it("renders branch name input", async () => {
    const testSetup = await testRender(
      () => (
        <BranchNameStep
          branchType="feature/"
          onSubmit={() => {}}
          onBack={() => {}}
        />
      ),
      { width: 60, height: 20 },
    );
    await testSetup.renderOnce();

    try {
      const frame = testSetup.captureCharFrame();
      expect(frame).toContain("feature/");
      expect(frame).toContain("Branch name");
    } finally {
      testSetup.renderer.destroy();
    }
  });

  it("calls onSubmit with entered branch name", async () => {
    let submittedName = "";
    const testSetup = await testRender(
      () => (
        <BranchNameStep
          branchType="feature/"
          onSubmit={(name) => {
            submittedName = name;
          }}
          onBack={() => {}}
        />
      ),
      { width: 60, height: 20 },
    );
    await testSetup.renderOnce();

    try {
      // ブランチ名を入力
      await testSetup.mockInput.typeText("my-feature");
      await testSetup.renderOnce();
      // Enterで確定
      testSetup.mockInput.pressEnter();
      await testSetup.renderOnce();
      expect(submittedName).toBe("my-feature");
    } finally {
      testSetup.renderer.destroy();
    }
  });
});

// T407: コーディングエージェント選択ステップのテスト
describe("AgentSelectStep", () => {
  it("renders agent options", async () => {
    const testSetup = await testRender(
      () => <AgentSelectStep onSelect={() => {}} onBack={() => {}} />,
      { width: 60, height: 20 },
    );
    await testSetup.renderOnce();

    try {
      const frame = testSetup.captureCharFrame();
      expect(frame).toContain("Claude Code");
    } finally {
      testSetup.renderer.destroy();
    }
  });
});

// T407a: バージョン選択ステップのテスト
describe("VersionSelectStep", () => {
  beforeEach(() => {
    clearInstalledVersionCache();
    clearVersionCache();
  });

  it("renders installed option from cache", async () => {
    setInstalledVersionCache("claude-code", {
      version: "1.2.3",
      path: "/usr/local/bin/claude",
    });
    setVersionCache("claude-code", []);

    const testSetup = await testRender(
      () => (
        <VersionSelectStep
          agentId="claude-code"
          onSelect={() => {}}
          onBack={() => {}}
        />
      ),
      { width: 60, height: 20 },
    );
    await testSetup.renderOnce();

    try {
      const frame = testSetup.captureCharFrame();
      expect(frame).toContain("Select version");
      expect(frame).toContain("installed@1.2.3");
      expect(frame).toContain("latest");
    } finally {
      testSetup.renderer.destroy();
    }
  });

  it("does not render installed option when cache is empty", async () => {
    const testSetup = await testRender(
      () => (
        <VersionSelectStep
          agentId="claude-code"
          onSelect={() => {}}
          onBack={() => {}}
        />
      ),
      { width: 60, height: 20 },
    );
    await testSetup.renderOnce();

    try {
      const frame = testSetup.captureCharFrame();
      expect(frame).toContain("latest");
      expect(frame).not.toContain("installed@");
    } finally {
      testSetup.renderer.destroy();
    }
  });
});

// T408: モデル選択ステップのテスト
describe("ModelSelectStep", () => {
  it("renders model options for Claude Code", async () => {
    const testSetup = await testRender(
      () => (
        <ModelSelectStep
          agentId="claude-code"
          onSelect={() => {}}
          onBack={() => {}}
        />
      ),
      { width: 60, height: 20 },
    );
    await testSetup.renderOnce();

    try {
      const frame = testSetup.captureCharFrame();
      expect(frame).toContain("Model");
    } finally {
      testSetup.renderer.destroy();
    }
  });

  it("renders model options for OpenCode", async () => {
    const testSetup = await testRender(
      () => (
        <ModelSelectStep
          agentId="opencode"
          onSelect={() => {}}
          onBack={() => {}}
        />
      ),
      { width: 60, height: 20 },
    );
    await testSetup.renderOnce();

    try {
      const frame = testSetup.captureCharFrame();
      expect(frame).toContain("Default (Auto)");
      expect(frame).toContain("Custom");
    } finally {
      testSetup.renderer.destroy();
    }
  });
});

// T409: 推論レベル選択ステップ（Codexのみ）のテスト
describe("ReasoningLevelStep", () => {
  it("renders reasoning level options", async () => {
    const testSetup = await testRender(
      () => <ReasoningLevelStep onSelect={() => {}} onBack={() => {}} />,
      { width: 60, height: 20 },
    );
    await testSetup.renderOnce();

    try {
      const frame = testSetup.captureCharFrame();
      // タイトルと最初の選択肢（low）が表示されることを確認
      expect(frame).toContain("reasoning level");
      expect(frame).toContain("low");
    } finally {
      testSetup.renderer.destroy();
    }
  });
});

// T410: 実行モード選択ステップのテスト
describe("ExecutionModeStep", () => {
  it("renders execution mode options", async () => {
    const testSetup = await testRender(
      () => <ExecutionModeStep onSelect={() => {}} onBack={() => {}} />,
      { width: 60, height: 20 },
    );
    await testSetup.renderOnce();

    try {
      const frame = testSetup.captureCharFrame();
      // タイトルと最初の選択肢（Normal）が表示されることを確認
      expect(frame).toContain("execution mode");
      expect(frame).toContain("Normal");
    } finally {
      testSetup.renderer.destroy();
    }
  });
});

// T411: 権限スキップ確認ステップのテスト
describe("SkipPermissionsStep", () => {
  it("renders skip permissions options", async () => {
    const testSetup = await testRender(
      () => <SkipPermissionsStep onSelect={() => {}} onBack={() => {}} />,
      { width: 60, height: 20 },
    );
    await testSetup.renderOnce();

    try {
      const frame = testSetup.captureCharFrame();
      // タイトルと最初の選択肢（Yes）が表示されることを確認
      expect(frame).toContain("Skip permission");
      expect(frame).toContain("Yes");
    } finally {
      testSetup.renderer.destroy();
    }
  });

  it("calls onSelect with true when Yes is selected", async () => {
    let selected: boolean | null = null;
    const testSetup = await testRender(
      () => (
        <SkipPermissionsStep
          onSelect={(value) => {
            selected = value;
          }}
          onBack={() => {}}
        />
      ),
      { width: 60, height: 20 },
    );
    await testSetup.renderOnce();

    try {
      // Enterキーで選択（デフォルトはYes）
      testSetup.mockInput.pressEnter();
      await testSetup.renderOnce();
      if (selected === null) {
        throw new Error("Expected selection");
      }
      expect(selected === true).toBe(true);
    } finally {
      testSetup.renderer.destroy();
    }
  });
});
