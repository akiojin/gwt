/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { SettingsScreen } from "../../../screens/solid/SettingsScreen.js";

const renderScreen = async (props: {
  settings: { label: string; value: string }[];
}) => {
  const testSetup = await testRender(
    () => <SettingsScreen settings={props.settings} />,
    { width: 60, height: 6 },
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

describe("Solid SettingsScreen", () => {
  it("renders settings items", async () => {
    const { captureCharFrame, cleanup } = await renderScreen({
      settings: [
        { label: "Theme", value: "theme" },
        { label: "Telemetry", value: "telemetry" },
      ],
    });

    try {
      const frame = captureCharFrame();
      expect(frame).toContain("Theme");
      expect(frame).toContain("Telemetry");
    } finally {
      cleanup();
    }
  });
});
