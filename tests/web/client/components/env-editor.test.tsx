import { describe, it, expect, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import React from "react";
import { EnvEditor, createEnvRow } from "../../../../src/web/client/src/components/EnvEditor";

describe("EnvEditor", () => {
  it("converts keys to uppercase and underscores", () => {
    const rows = [createEnvRow({ id: "row-1" })];
    const handleChange = vi.fn();

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
    const nextRows = handleChange.mock.calls[0][0];
    expect(nextRows[0].key).toBe("ABC_DEF");
  });

  it("adds a new row when clicking the add button", () => {
    const handleChange = vi.fn();
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
    expect(handleChange.mock.calls[0][0]).toHaveLength(1);
  });
});
