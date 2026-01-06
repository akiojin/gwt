/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { ConfirmScreen } from "../../../screens/solid/ConfirmScreen.js";

const renderScreen = async (props: {
  message: string;
  defaultNo?: boolean;
  onConfirm: (confirmed: boolean) => void;
}) => {
  const testSetup = await testRender(
    () => (
      <ConfirmScreen
        message={props.message}
        defaultNo={props.defaultNo}
        onConfirm={props.onConfirm}
      />
    ),
    {
      width: 40,
      height: 6,
    },
  );

  await testSetup.renderOnce();

  const cleanup = () => {
    testSetup.renderer.destroy();
  };

  return {
    ...testSetup,
    cleanup,
  };
};

describe("Solid ConfirmScreen", () => {
  it("renders message and options", async () => {
    const { captureCharFrame, cleanup } = await renderScreen({
      message: "Delete branch?",
      onConfirm: () => {},
    });

    try {
      const frame = captureCharFrame();
      expect(frame).toContain("Delete branch?");
      expect(frame).toContain("Yes");
      expect(frame).toContain("No");
    } finally {
      cleanup();
    }
  });

  it("confirms default selection on enter", async () => {
    const confirmations: boolean[] = [];
    const { mockInput, renderOnce, cleanup } = await renderScreen({
      message: "Proceed?",
      defaultNo: true,
      onConfirm: (confirmed) => confirmations.push(confirmed),
    });

    try {
      mockInput.pressEnter();
      await renderOnce();
      expect(confirmations[0]).toBe(false);
    } finally {
      cleanup();
    }
  });
});
