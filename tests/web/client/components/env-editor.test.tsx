import { describe, it, expect, mock } from "bun:test";
import { fireEvent, render, screen } from "@testing-library/react";
import React from "react";
import {
  EnvEditor,
  createEnvRow,
} from "../../../../src/web/client/src/components/EnvEditor";

describe("EnvEditor", () => {
  it("converts keys to uppercase and underscores", () => {
    const rows = [createEnvRow({ id: "row-1" })];
    const handleChange = mock();

    render(
      <EnvEditor
        title="Test"
        rows={rows}
        onChange={handleChange}
        description=""
      />,
    );

    const keyInput = screen.getByPlaceholderText("EXAMPLE_KEY");
    fireEvent.change(keyInput, { target: { value: "abc-def" } });

    expect(handleChange).toHaveBeenCalled();
    const firstCall = handleChange.mock.calls[0];
    if (!firstCall) {
      throw new Error("Expected change handler to be called");
    }
    const nextRows = firstCall[0];
    if (!nextRows?.[0]) {
      throw new Error("Expected updated rows");
    }
    expect(nextRows[0].key).toBe("ABC_DEF");
  });

  it("adds a new row when clicking the add button", () => {
    const handleChange = mock();
    render(
      <EnvEditor
        title="Test"
        rows={[]}
        onChange={handleChange}
        description=""
      />,
    );

    fireEvent.click(screen.getByText("変数を追加"));
    expect(handleChange).toHaveBeenCalled();
    const firstCall = handleChange.mock.calls[0];
    if (!firstCall) {
      throw new Error("Expected change handler to be called");
    }
    expect(firstCall[0]).toHaveLength(1);
  });
});
