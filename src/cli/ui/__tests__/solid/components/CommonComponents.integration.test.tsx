/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { Header } from "../../../components/solid/Header.js";
import { Stats } from "../../../components/solid/Stats.js";
import { SearchInput } from "../../../components/solid/SearchInput.js";
import { ScrollableList } from "../../../components/solid/ScrollableList.js";
import { SelectInput } from "../../../components/solid/SelectInput.js";
import { TextInput } from "../../../components/solid/TextInput.js";
import { Footer } from "../../../components/solid/Footer.js";
import type { Statistics } from "../../../types.js";

const renderCommonComponents = async () => {
  const stats: Statistics = {
    localCount: 1,
    remoteCount: 2,
    worktreeCount: 1,
    changesCount: 0,
    lastUpdated: new Date("2026-01-05T00:00:00Z"),
  };

  const testSetup = await testRender(
    () => (
      <box flexDirection="column">
        <Header title="gwt" version="1.0.0" activeProfile="dev" />
        <Stats stats={stats} />
        <SearchInput
          value="feat"
          onChange={() => {}}
          count={{ filtered: 1, total: 2 }}
        />
        <ScrollableList maxHeight={2}>
          <text>Item A</text>
          <text>Item B</text>
        </ScrollableList>
        <SelectInput
          items={[
            { label: "Option A", value: "a" },
            { label: "Option B", value: "b" },
          ]}
          selectedIndex={0}
          focused
        />
        <TextInput label="Name" value="Alice" onChange={() => {}} />
        <Footer actions={[{ key: "q", description: "Quit" }]} />
      </box>
    ),
    {
      width: 60,
      height: 14,
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

describe("Solid common components integration", () => {
  it("renders common components together", async () => {
    const { captureCharFrame, cleanup } = await renderCommonComponents();

    try {
      const frame = captureCharFrame();
      expect(frame).toContain("gwt v1.0.0");
      expect(frame).toContain("Local: 1");
      expect(frame).toContain("Search:");
      expect(frame).toContain("Item A");
      expect(frame).toContain("Option A");
      expect(frame).toContain("Name");
      expect(frame).toContain("[q] Quit");
    } finally {
      cleanup();
    }
  });
});
