/**
 * tools.json スキーママイグレーションのテスト
 * SPEC-29e16bd0: customTools → customCodingAgents マイグレーション
 *
 * マイグレーションロジックのユニットテスト
 * ファイルI/O統合テストは bun test で実行
 */
/* eslint-disable @typescript-eslint/no-non-null-assertion */
import { describe, it, expect } from "bun:test";
import type { CodingAgentsConfig } from "../../../src/types/tools.js";

/**
 * マイグレーションロジックの抽出テスト
 * 実際の loadCodingAgentsConfig と同じロジックを検証
 */
function migrateConfig(rawConfig: Record<string, unknown>): CodingAgentsConfig {
  const config = rawConfig as unknown as CodingAgentsConfig;

  // マイグレーション: customTools → customCodingAgents (後方互換性)
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const legacyConfig = config as any;
  if (!config.customCodingAgents && legacyConfig.customTools) {
    config.customCodingAgents = legacyConfig.customTools;
  }

  // フォールバック: undefined/null → 空配列
  if (!config.customCodingAgents) {
    config.customCodingAgents = [];
  }

  return config;
}

describe("tools.json schema migration (SPEC-29e16bd0)", () => {
  it("旧形式(customTools)から新形式(customCodingAgents)へ自動マイグレーション", () => {
    const rawConfig = {
      version: "1.0.0",
      customTools: [
        {
          id: "test-agent",
          displayName: "Test Agent",
          type: "bunx",
          command: "test-package@latest",
          modeArgs: { normal: [] },
        },
      ],
    };

    const config = migrateConfig(rawConfig);

    expect(config.customCodingAgents).toHaveLength(1);
    expect(config.customCodingAgents[0]!.id).toBe("test-agent");
    expect(config.customCodingAgents[0]!.displayName).toBe("Test Agent");
  });

  it("customCodingAgentsもcustomToolsも存在しない場合、空配列にフォールバック", () => {
    const rawConfig = {
      version: "1.0.0",
      env: {},
    };

    const config = migrateConfig(rawConfig);

    expect(config.customCodingAgents).toEqual([]);
  });

  it("新形式(customCodingAgents)が存在する場合はマイグレーション不要", () => {
    const rawConfig = {
      version: "1.0.0",
      customCodingAgents: [
        {
          id: "new-agent",
          displayName: "New Agent",
          type: "bunx",
          command: "new-package@latest",
          modeArgs: { normal: [] },
        },
      ],
    };

    const config = migrateConfig(rawConfig);

    expect(config.customCodingAgents).toHaveLength(1);
    expect(config.customCodingAgents[0]!.id).toBe("new-agent");
  });

  it("両方のフィールドが存在する場合、customCodingAgentsを優先", () => {
    const rawConfig = {
      version: "1.0.0",
      customCodingAgents: [
        {
          id: "new-agent",
          displayName: "New Agent",
          type: "bunx",
          command: "new-package@latest",
          modeArgs: { normal: [] },
        },
      ],
      customTools: [
        {
          id: "old-agent",
          displayName: "Old Agent",
          type: "bunx",
          command: "old-package@latest",
          modeArgs: { normal: [] },
        },
      ],
    };

    const config = migrateConfig(rawConfig);

    expect(config.customCodingAgents).toHaveLength(1);
    expect(config.customCodingAgents[0]!.id).toBe("new-agent");
  });

  it("customCodingAgentsがnullの場合、空配列にフォールバック", () => {
    const rawConfig = {
      version: "1.0.0",
      customCodingAgents: null,
    };

    const config = migrateConfig(rawConfig);

    expect(config.customCodingAgents).toEqual([]);
  });

  it("customCodingAgentsがundefinedの場合、空配列にフォールバック", () => {
    const rawConfig = {
      version: "1.0.0",
      customCodingAgents: undefined,
    };

    const config = migrateConfig(rawConfig);

    expect(config.customCodingAgents).toEqual([]);
  });
});
