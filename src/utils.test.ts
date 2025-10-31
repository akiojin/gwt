import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { getPackageVersion } from "./utils";
import { readFile } from "fs/promises";
import path from "node:path";

describe("getPackageVersion", () => {
  it("正常系: package.jsonが存在し、versionフィールドがある場合、バージョンを返す", async () => {
    const version = await getPackageVersion();

    // バージョンが取得できることを確認
    expect(version).not.toBeNull();
    expect(typeof version).toBe("string");

    // セマンティックバージョニング形式かチェック（基本形式）
    if (version) {
      expect(version).toMatch(/^\d+\.\d+\.\d+/);
    }
  });

  it("正常系: 取得したバージョンがpackage.jsonのversionと一致する", async () => {
    const version = await getPackageVersion();

    // package.jsonから直接読み取ってバージョンを確認
    const packageJsonContent = await readFile(
      path.resolve(process.cwd(), "package.json"),
      "utf-8",
    );
    const packageJson = JSON.parse(packageJsonContent);

    expect(version).toBe(packageJson.version);
  });

  it("正常系: プレリリースバージョンも正しく取得できる", async () => {
    // 注: このテストは実際のpackage.jsonのバージョンがプレリリース形式の場合にパスする
    // 現在のバージョンが通常バージョンの場合、このテストはスキップされる
    const version = await getPackageVersion();

    if (version && version.includes("-")) {
      // プレリリース識別子を含むバージョン（例: "2.0.0-beta.1"）
      expect(version).toMatch(/^\d+\.\d+\.\d+-[a-zA-Z0-9.-]+/);
    }
  });
});
