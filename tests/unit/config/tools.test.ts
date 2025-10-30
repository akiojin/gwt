/**
 * カスタムツール設定管理機能のテスト
 */

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { readFile } from "node:fs/promises";
import { homedir } from "node:os";
import path from "node:path";

// テスト対象の関数（実装前なので一時的にany型）
// eslint-disable-next-line @typescript-eslint/no-explicit-any
let loadToolsConfig: any;
// eslint-disable-next-line @typescript-eslint/no-explicit-any
let validateToolConfig: any;
// eslint-disable-next-line @typescript-eslint/no-explicit-any
let getToolById: any;
// eslint-disable-next-line @typescript-eslint/no-explicit-any
let getAllTools: any;

// 実装後にインポートを有効化
// import {
//   loadToolsConfig,
//   validateToolConfig,
//   getToolById,
//   getAllTools,
// } from "../../../src/config/tools.js";

describe("loadToolsConfig", () => {
  beforeEach(() => {
    // 実装前は空の関数をモック
    loadToolsConfig = vi.fn();
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("設定ファイルが存在する場合、正常に読み込める", async () => {
    // TODO: 実装後にテストを記述
    expect(true).toBe(true);
  });

  it("設定ファイルが存在しない場合、空のツール配列を返す", async () => {
    // TODO: 実装後にテストを記述
    expect(true).toBe(true);
  });

  it("JSON構文エラーがある場合、エラーメッセージを表示", async () => {
    // TODO: 実装後にテストを記述
    expect(true).toBe(true);
  });

  it("検証エラーがある場合、エラーメッセージを表示", async () => {
    // TODO: 実装後にテストを記述
    expect(true).toBe(true);
  });
});

describe("validateToolConfig", () => {
  beforeEach(() => {
    validateToolConfig = vi.fn();
  });

  it("必須フィールドが全て存在する場合、検証が成功", () => {
    const validTool = {
      id: "test-tool",
      displayName: "Test Tool",
      type: "bunx",
      command: "test-package@latest",
      modeArgs: { normal: [] },
    };
    // TODO: 実装後にテストを記述
    expect(true).toBe(true);
  });

  it("idフィールドが存在しない場合、エラーをスロー", () => {
    const invalidTool = {
      displayName: "Test Tool",
      type: "bunx",
      command: "test-package@latest",
      modeArgs: { normal: [] },
    };
    // TODO: 実装後にテストを記述
    expect(true).toBe(true);
  });

  it("displayNameフィールドが存在しない場合、エラーをスロー", () => {
    // TODO: 実装後にテストを記述
    expect(true).toBe(true);
  });

  it("typeフィールドが存在しない場合、エラーをスロー", () => {
    // TODO: 実装後にテストを記述
    expect(true).toBe(true);
  });

  it("commandフィールドが存在しない場合、エラーをスロー", () => {
    // TODO: 実装後にテストを記述
    expect(true).toBe(true);
  });

  it("modeArgsフィールドが存在しない場合、エラーをスロー", () => {
    // TODO: 実装後にテストを記述
    expect(true).toBe(true);
  });

  it("typeフィールドが'path','bunx','command'以外の場合、エラーをスロー", () => {
    const invalidTool = {
      id: "test-tool",
      displayName: "Test Tool",
      type: "invalid",
      command: "test-package@latest",
      modeArgs: { normal: [] },
    };
    // TODO: 実装後にテストを記述
    expect(true).toBe(true);
  });

  it("id重複がある場合、エラーをスロー", () => {
    const tools = [
      {
        id: "duplicate-id",
        displayName: "Tool 1",
        type: "bunx",
        command: "package1@latest",
        modeArgs: { normal: [] },
      },
      {
        id: "duplicate-id",
        displayName: "Tool 2",
        type: "bunx",
        command: "package2@latest",
        modeArgs: { normal: [] },
      },
    ];
    // TODO: 実装後にテストを記述
    expect(true).toBe(true);
  });

  it("id形式が不正な場合、エラーをスロー", () => {
    const invalidTool = {
      id: "Invalid_ID!",
      displayName: "Test Tool",
      type: "bunx",
      command: "test-package@latest",
      modeArgs: { normal: [] },
    };
    // TODO: 実装後にテストを記述
    expect(true).toBe(true);
  });

  it("type='path'でcommandが絶対パスでない場合、エラーをスロー", () => {
    const invalidTool = {
      id: "test-tool",
      displayName: "Test Tool",
      type: "path",
      command: "relative/path/tool",
      modeArgs: { normal: [] },
    };
    // TODO: 実装後にテストを記述
    expect(true).toBe(true);
  });
});

describe("getToolById", () => {
  beforeEach(() => {
    getToolById = vi.fn();
  });

  it("存在するIDの場合、ツールを返す", () => {
    // TODO: 実装後にテストを記述
    expect(true).toBe(true);
  });

  it("存在しないIDの場合、undefinedを返す", () => {
    // TODO: 実装後にテストを記述
    expect(true).toBe(true);
  });
});

describe("getAllTools", () => {
  beforeEach(() => {
    getAllTools = vi.fn();
  });

  it("ビルトインツール（Claude Code, Codex CLI）が含まれる", () => {
    // TODO: 実装後にテストを記述
    expect(true).toBe(true);
  });

  it("カスタムツールが存在する場合、ビルトイン+カスタムが統合される", () => {
    // TODO: 実装後にテストを記述
    expect(true).toBe(true);
  });

  it("カスタムツールが存在しない場合、ビルトインツールのみ返す", () => {
    // TODO: 実装後にテストを記述
    expect(true).toBe(true);
  });

  it("ビルトインツールはisBuiltin=trueとしてマークされる", () => {
    // TODO: 実装後にテストを記述
    expect(true).toBe(true);
  });

  it("カスタムツールはisBuiltin=falseとしてマークされる", () => {
    // TODO: 実装後にテストを記述
    expect(true).toBe(true);
  });
});
