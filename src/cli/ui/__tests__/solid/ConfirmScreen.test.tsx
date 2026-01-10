/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { TestRecorder } from "@opentui/core/testing";
import { ConfirmScreen } from "../../screens/solid/ConfirmScreen.js";

const getBgColor = (bg: Float32Array, width: number, x: number, y: number) => {
  const index = (y * width + x) * 4;
  return [
    bg[index] ?? 0,
    bg[index + 1] ?? 0,
    bg[index + 2] ?? 0,
    bg[index + 3] ?? 0,
  ];
};

const isSameColor = (a: number[], b: number[], epsilon = 0.001) =>
  a.length === b.length &&
  a.every((value, i) => Math.abs(value - b[i]) < epsilon);

describe("ConfirmScreen selection width", () => {
  it("limits highlight to provided width", async () => {
    const width = 20;
    const height = 5;
    const selectionWidth = 8;
    const testSetup = await testRender(
      () => (
        <ConfirmScreen
          message="Confirm?"
          onConfirm={() => {}}
          yesLabel="OK"
          noLabel="Cancel"
          width={selectionWidth}
        />
      ),
      { width, height },
    );

    const recorder = new TestRecorder(testSetup.renderer, {
      recordBuffers: { bg: true },
    });
    recorder.rec();

    try {
      await testSetup.renderOnce();
      recorder.stop();

      const frame = recorder.recordedFrames.at(-1);
      if (!frame?.buffers?.bg) {
        throw new Error("No background buffer recorded");
      }

      const bg = frame.buffers.bg;
      const defaultColor = getBgColor(bg, width, 0, 0);
      const targetRow = 1;
      let highlightLength = 0;

      for (let x = 0; x < width; x += 1) {
        const color = getBgColor(bg, width, x, targetRow);
        if (!isSameColor(color, defaultColor)) {
          highlightLength += 1;
        } else {
          break;
        }
      }

      expect(highlightLength).toBe(selectionWidth);

      for (let x = selectionWidth; x < width; x += 1) {
        const color = getBgColor(bg, width, x, targetRow);
        expect(isSameColor(color, defaultColor)).toBe(true);
      }
    } finally {
      testSetup.renderer.destroy();
    }
  });
});
