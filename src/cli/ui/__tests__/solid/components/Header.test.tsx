/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { Header } from "../../../components/solid/Header.js";

const renderHeader = async (props: {
  title: string;
  titleColor?: string;
  dividerChar?: string;
  showDivider?: boolean;
  width?: number;
  version?: string | null;
  workingDirectory?: string;
  activeProfile?: string | null;
}) => {
  const testSetup = await testRender(() => <Header {...props} />, {
    width: props.width ?? 40,
    height: 6,
  });

  await testSetup.renderOnce();

  const cleanup = () => {
    testSetup.renderer.destroy();
  };

  return {
    ...testSetup,
    cleanup,
  };
};

describe("Solid Header", () => {
  it("renders title with version and profile", async () => {
    const { captureCharFrame, cleanup } = await renderHeader({
      title: "gwt",
      version: "1.2.3",
      activeProfile: "dev",
    });

    try {
      expect(captureCharFrame()).toContain("gwt v1.2.3 | Profile: dev");
    } finally {
      cleanup();
    }
  });

  it("renders profile placeholder when activeProfile is null", async () => {
    const { captureCharFrame, cleanup } = await renderHeader({
      title: "gwt",
      version: "2.0.0",
      activeProfile: null,
    });

    try {
      expect(captureCharFrame()).toContain("Profile: (none)");
    } finally {
      cleanup();
    }
  });

  it("renders divider with custom width", async () => {
    const { captureCharFrame, cleanup } = await renderHeader({
      title: "gwt",
      dividerChar: "-",
      width: 12,
    });

    try {
      expect(captureCharFrame()).toContain("------------");
    } finally {
      cleanup();
    }
  });

  it("renders working directory line", async () => {
    const { captureCharFrame, cleanup } = await renderHeader({
      title: "gwt",
      workingDirectory: "/tmp/repo",
    });

    try {
      expect(captureCharFrame()).toContain("Working Directory: /tmp/repo");
    } finally {
      cleanup();
    }
  });

  it("hides divider when disabled", async () => {
    const { captureCharFrame, cleanup } = await renderHeader({
      title: "gwt",
      dividerChar: "-",
      width: 8,
      showDivider: false,
    });

    try {
      expect(captureCharFrame()).not.toContain("--------");
    } finally {
      cleanup();
    }
  });
});
