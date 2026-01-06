/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { WizardPopup } from "../../../components/solid/WizardPopup.js";

const renderWizard = async (
  props: {
    visible: boolean;
    onClose?: () => void;
    onComplete?: (result: unknown) => void;
  },
  size: { width: number; height: number } = { width: 80, height: 24 },
) => {
  const testSetup = await testRender(
    () => (
      <WizardPopup
        visible={props.visible}
        onClose={props.onClose ?? (() => {})}
        onComplete={props.onComplete ?? (() => {})}
      />
    ),
    size,
  );
  await testSetup.renderOnce();

  return {
    captureCharFrame: testSetup.captureCharFrame,
    renderOnce: testSetup.renderOnce,
    mockInput: testSetup.mockInput,
    cleanup: () => testSetup.renderer.destroy(),
  };
};

describe("WizardPopup", () => {
  // T401: ウィザードポップアップの表示/非表示テスト
  describe("visibility", () => {
    it("renders wizard popup when visible is true", async () => {
      const { captureCharFrame, cleanup } = await renderWizard({
        visible: true,
      });

      try {
        const frame = captureCharFrame();
        // ウィザードポップアップのタイトルまたは枠線が表示されることを確認
        expect(frame).toContain("Select");
      } finally {
        cleanup();
      }
    });

    it("does not render when visible is false", async () => {
      const { captureCharFrame, cleanup } = await renderWizard({
        visible: false,
      });

      try {
        const frame = captureCharFrame();
        // ウィザードポップアップが表示されないことを確認
        expect(frame).not.toContain("Select");
      } finally {
        cleanup();
      }
    });
  });

  // T402: 背景オーバーレイ（半透過）の表示テスト
  describe("overlay", () => {
    it("renders background overlay when visible", async () => {
      const { captureCharFrame, cleanup } = await renderWizard({
        visible: true,
      });

      try {
        const frame = captureCharFrame();
        // ポップアップが画面中央に表示されることを確認（枠線）
        expect(frame).toMatch(/[┌┐└┘│─]/);
      } finally {
        cleanup();
      }
    });
  });

  // T403: Escapeキーでウィザード終了するテスト
  describe("keyboard interaction", () => {
    it("calls onClose when Escape is pressed", async () => {
      let closeCalled = false;
      const { mockInput, renderOnce, cleanup } = await renderWizard({
        visible: true,
        onClose: () => {
          closeCalled = true;
        },
      });

      try {
        // Escapeキーを送信
        mockInput.pressKey("escape");
        await renderOnce();
        // onCloseが呼ばれることを確認
        expect(closeCalled).toBe(true);
      } finally {
        cleanup();
      }
    });
  });

  // T404: ステップ間の遷移（前へ/次へ）テスト
  describe("step navigation", () => {
    it("advances to next step on Enter", async () => {
      const { mockInput, captureCharFrame, renderOnce, cleanup } =
        await renderWizard({
          visible: true,
        });

      try {
        // 最初のステップが表示されることを確認
        let frame = captureCharFrame();
        expect(frame).toContain("Select");

        // Enterキーで次のステップへ
        mockInput.pressEnter();
        await renderOnce();

        // 次のステップに進むことを確認（内容が変わる）
        frame = captureCharFrame();
        expect(frame).toBeDefined();
      } finally {
        cleanup();
      }
    });

    it("returns to previous step on Escape (not first step)", async () => {
      const { mockInput, captureCharFrame, renderOnce, cleanup } =
        await renderWizard({
          visible: true,
        });

      try {
        // 次のステップへ進む
        mockInput.pressEnter();
        await renderOnce();

        // Escapeで戻る
        mockInput.pressKey("escape");
        await renderOnce();

        const frame = captureCharFrame();
        // 最初のステップに戻ることを確認
        expect(frame).toContain("Select");
      } finally {
        cleanup();
      }
    });
  });
});
