import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import * as utils from "./utils";

// showVersion関数のテスト（TDD Green phase）
// 注: showVersion関数はindex.ts内で定義されているため、直接importできない
// そのため、main()関数経由でテストするか、関数をexportする必要がある

describe("showVersion via CLI args", () => {
  let consoleLogSpy: ReturnType<typeof vi.spyOn>;
  let consoleErrorSpy: ReturnType<typeof vi.spyOn>;
  let processExitSpy: ReturnType<typeof vi.spyOn>;
  let originalArgv: string[];

  beforeEach(() => {
    // console.log, console.error, process.exitをモック
    consoleLogSpy = vi.spyOn(console, "log").mockImplementation(() => {});
    consoleErrorSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    processExitSpy = vi
      .spyOn(process, "exit")
      .mockImplementation((() => {}) as any);

    // 元のprocess.argvを保存
    originalArgv = [...process.argv];
  });

  afterEach(() => {
    // モックをリストア
    consoleLogSpy.mockRestore();
    consoleErrorSpy.mockRestore();
    processExitSpy.mockRestore();

    // process.argvを復元
    process.argv = originalArgv;

    // モジュールキャッシュをクリア
    vi.resetModules();
  });

  it("正常系: --versionフラグでバージョンを表示する", async () => {
    // Arrange: CLIフラグを設定
    process.argv = ["node", "index.js", "--version"];

    // getPackageVersion()をモック
    const mockVersion = "1.12.3";
    vi.spyOn(utils, "getPackageVersion").mockResolvedValue(mockVersion);

    // Act: main()を呼び出す
    const { main } = await import("./index");
    await main();

    // Assert: 標準出力にバージョンが表示されることを期待
    expect(consoleLogSpy).toHaveBeenCalledWith(mockVersion);
  });

  it("正常系: -vフラグでバージョンを表示する", async () => {
    // Arrange: CLIフラグを設定
    process.argv = ["node", "index.js", "-v"];

    // getPackageVersion()をモック
    const mockVersion = "1.12.3";
    vi.spyOn(utils, "getPackageVersion").mockResolvedValue(mockVersion);

    // Act: main()を呼び出す
    const { main } = await import("./index");
    await main();

    // Assert: 標準出力にバージョンが表示されることを期待
    expect(consoleLogSpy).toHaveBeenCalledWith(mockVersion);
  });

  it("異常系: バージョン取得失敗時、エラーメッセージを表示してexit(1)", async () => {
    // Arrange: CLIフラグを設定
    process.argv = ["node", "index.js", "--version"];

    // getPackageVersion()をモックしてnullを返す
    vi.spyOn(utils, "getPackageVersion").mockResolvedValue(null);

    // Act: main()を呼び出す
    const { main } = await import("./index");
    await main();

    // Assert: エラーメッセージが標準エラー出力に表示され、exit(1)が呼ばれることを期待
    expect(consoleErrorSpy).toHaveBeenCalledWith(
      expect.stringContaining("Error"),
    );
    expect(processExitSpy).toHaveBeenCalledWith(1);
  });
});
