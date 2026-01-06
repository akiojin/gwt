/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { EnvironmentScreen } from "../../../screens/solid/EnvironmentScreen.js";

const renderScreen = async (props: {
  variables: { key: string; value: string }[];
}) => {
  const testSetup = await testRender(
    () => <EnvironmentScreen variables={props.variables} />,
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

describe("Solid EnvironmentScreen", () => {
  it("renders environment variables", async () => {
    const { captureCharFrame, cleanup } = await renderScreen({
      variables: [
        { key: "API_KEY", value: "secret" },
        { key: "REGION", value: "us-east-1" },
      ],
    });

    try {
      const frame = captureCharFrame();
      expect(frame).toContain("API_KEY=secret");
      expect(frame).toContain("REGION=us-east-1");
    } finally {
      cleanup();
    }
  });
});
