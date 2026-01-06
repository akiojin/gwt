/** @jsxImportSource @opentui/solid */
import { describe, expect, it } from "bun:test";
import { testRender } from "@opentui/solid";
import { createSignal } from "solid-js";
import { SearchInput } from "../../../components/solid/SearchInput.js";

const renderSearchInput = async (options: {
  value?: string;
  placeholder?: string;
  count?: { filtered: number; total: number };
  width?: number;
  height?: number;
}) => {
  const changes: string[] = [];
  const testSetup = await testRender(
    () => {
      const [value, setValue] = createSignal(options.value ?? "");
      const handleChange = (nextValue: string) => {
        setValue(nextValue);
        changes.push(nextValue);
      };

      return (
        <SearchInput
          value={value()}
          onChange={handleChange}
          placeholder={options.placeholder}
          count={options.count}
          focused
        />
      );
    },
    {
      width: options.width ?? 60,
      height: options.height ?? 3,
    },
  );

  await testSetup.renderOnce();

  const cleanup = () => {
    testSetup.renderer.destroy();
  };

  return {
    ...testSetup,
    changes,
    cleanup,
  };
};

describe("Solid SearchInput", () => {
  it("renders label and count", async () => {
    const { captureCharFrame, cleanup } = await renderSearchInput({
      value: "query",
      count: { filtered: 2, total: 5 },
    });

    try {
      const frame = captureCharFrame();
      expect(frame).toContain("Search:");
      expect(frame).toContain("query");
      expect(frame).toContain("2 / 5");
    } finally {
      cleanup();
    }
  });

  it("renders placeholder when empty", async () => {
    const { captureCharFrame, cleanup } = await renderSearchInput({
      value: "",
      placeholder: "Type to search...",
    });

    try {
      expect(captureCharFrame()).toContain("Type to search");
    } finally {
      cleanup();
    }
  });
});
