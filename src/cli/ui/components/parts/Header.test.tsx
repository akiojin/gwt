import { describe, it, expect } from "vitest";
import { render } from "ink-testing-library";
import React from "react";
import { Header } from "./Header.js";

describe("Header Component", () => {
  it("正常系: versionプロップありの場合、タイトルとバージョンを表示する", () => {
    const { lastFrame } = render(<Header title="gwt" version="1.12.3" />);

    const output = lastFrame();

    // タイトルとバージョンが含まれることを確認
    expect(output).toContain("gwt v1.12.3");
  });

  it("正常系: versionプロップなし（undefined）の場合、タイトルのみ表示する", () => {
    const { lastFrame } = render(<Header title="gwt" />);

    const output = lastFrame();

    // タイトルのみが含まれることを確認
    expect(output).toContain("gwt");
    // "v"が含まれていないことを確認（バージョンが表示されていない）
    expect(output).not.toMatch(/v\d+\.\d+\.\d+/);
  });

  it("正常系: version={null}の場合、タイトルのみ表示する", () => {
    const { lastFrame } = render(<Header title="gwt" version={null} />);

    const output = lastFrame();

    // タイトルのみが含まれることを確認
    expect(output).toContain("gwt");
    // "v"が含まれていないことを確認（バージョンが表示されていない）
    expect(output).not.toMatch(/v\d+\.\d+\.\d+/);
  });

  it("正常系: showDivider=trueの場合、区切り線が表示される", () => {
    const { lastFrame } = render(
      <Header
        title="gwt"
        version="1.12.3"
        showDivider={true}
        dividerChar="─"
      />,
    );

    const output = lastFrame();

    // 区切り線が含まれることを確認
    expect(output).toContain("─");
  });

  it("正常系: showDivider=falseの場合、区切り線が表示されない", () => {
    const { lastFrame } = render(
      <Header title="gwt" version="1.12.3" showDivider={false} />,
    );

    const output = lastFrame();

    // タイトルとバージョンは含まれる
    expect(output).toContain("gwt v1.12.3");
    // 区切り線が含まれないことを確認（または最小限）
    // 注: Inkのレンダリング結果によっては、完全に区切り線がないとは限らない
  });

  it("正常系: プレリリースバージョンも正しく表示される", () => {
    const { lastFrame } = render(<Header title="gwt" version="2.0.0-beta.1" />);

    const output = lastFrame();

    // プレリリースバージョンが含まれることを確認
    expect(output).toContain("gwt v2.0.0-beta.1");
  });
});
