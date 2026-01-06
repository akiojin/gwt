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

  // T403: ウィザードはステップ表示を持つ
  describe("step display", () => {
    it("shows step indicator when visible", async () => {
      const { captureCharFrame, cleanup } = await renderWizard({
        visible: true,
      });

      try {
        const frame = captureCharFrame();
        // ステップインジケーターが表示されることを確認
        expect(frame).toContain("Step");
      } finally {
        cleanup();
      }
    });
  });

  // T404: ステップ表示の確認テスト
  describe("step content", () => {
    it("displays wizard content when visible", async () => {
      const { captureCharFrame, cleanup } = await renderWizard({
        visible: true,
      });

      try {
        const frame = captureCharFrame();
        // ウィザードコンテンツが表示されることを確認
        expect(frame).toContain("Select");
        expect(frame).toContain("Step 1");
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
