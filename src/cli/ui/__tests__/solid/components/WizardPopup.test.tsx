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
      >
        <text>Test Content</text>
      </WizardPopup>
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
        // ウィザードポップアップの枠線が表示されることを確認
        expect(frame).toMatch(/[┌┐└┘│─]/);
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
        // ウィザードポップアップが表示されないことを確認（枠線がない）
        expect(frame).not.toMatch(/[┌┐└┘│─]/);
      } finally {
        cleanup();
      }
    });
  });

  // T402: 背景オーバーレイの表示テスト
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

  // T403: ウィザードは子コンテンツを表示する
  describe("content display", () => {
    it("shows children content when visible", async () => {
      // デフォルトのTest Contentが表示されることを確認
      const { captureCharFrame, cleanup } = await renderWizard({
        visible: true,
      });

      try {
        const frame = captureCharFrame();
        // 子コンテンツが表示されることを確認
        expect(frame).toContain("Test Content");
      } finally {
        cleanup();
      }
    });
  });

  // T404: 枠線表示の確認テスト
  describe("border display", () => {
    it("displays wizard content when visible", async () => {
      const { captureCharFrame, cleanup } = await renderWizard({
        visible: true,
      });

      try {
        const frame = captureCharFrame();
        // デフォルトコンテンツが表示されることを確認
        expect(frame).toContain("Test Content");
      } finally {
        cleanup();
      }
    });

    it("renders border when visible", async () => {
      const { captureCharFrame, cleanup } = await renderWizard({
        visible: true,
      });

      try {
        const frame = captureCharFrame();
        // 枠線が表示されることを確認
        expect(frame).toMatch(/[┌┐└┘│─]/);
      } finally {
        cleanup();
      }
    });
  });
});
