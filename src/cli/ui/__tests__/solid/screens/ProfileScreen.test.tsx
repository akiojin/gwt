/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { ProfileScreen } from "../../../screens/solid/ProfileScreen.js";

const renderScreen = async (props: {
  profiles: { name: string; displayName?: string; isActive?: boolean }[];
}) => {
  const testSetup = await testRender(
    () => <ProfileScreen profiles={props.profiles} />,
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

describe("Solid ProfileScreen", () => {
  it("renders profiles with active marker", async () => {
    const { captureCharFrame, cleanup } = await renderScreen({
      profiles: [
        { name: "default", displayName: "Default", isActive: true },
        { name: "dev", displayName: "Dev" },
      ],
    });

    try {
      const frame = captureCharFrame();
      expect(frame).toContain("Default (active)");
      expect(frame).toContain("Dev");
    } finally {
      cleanup();
    }
  });
});
