/**
 * @vitest-environment happy-dom
 */
import React from "react";
import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { act, render } from "@testing-library/react";
import { LoadingIndicator } from "../../../components/common/LoadingIndicator.js";
import { Window } from "happy-dom";

const advanceTimersBy = async (ms: number) => {
  await act(async () => {
    if (typeof vi.advanceTimersByTimeAsync === "function") {
      await vi.advanceTimersByTimeAsync(ms);
    } else if (typeof vi.advanceTimersByTime === "function") {
      vi.advanceTimersByTime(ms);
    } else {
      await new Promise((resolve) => setTimeout(resolve, ms));
    }
  });
};

beforeEach(() => {
  if (typeof vi.useFakeTimers === "function") {
    vi.useFakeTimers();
  }
  const window = new Window();
  globalThis.window = window as any;
  globalThis.document = window.document as any;
});

afterEach(() => {
  if (typeof vi.clearAllTimers === "function") {
    vi.clearAllTimers();
  }
  if (typeof vi.useRealTimers === "function") {
    vi.useRealTimers();
  }
});

describe("LoadingIndicator", () => {
  const getSpinnerText = (container: HTMLElement) => {
    return container.querySelector("ink-text")?.textContent ?? "";
  };

  const getMessageText = (container: HTMLElement) => {
    const texts = container.querySelectorAll("ink-text");
    return texts.length > 1 ? (texts[1]?.textContent ?? "") : "";
  };

  it("does not render before the delay elapses", async () => {
    const { container } = render(
      <LoadingIndicator isLoading={true} message="Loading data" delay={50} />,
    );

    expect(container.textContent).toBe("");

    await advanceTimersBy(20);

    expect(container.textContent).toBe("");
  });

  it("renders after the delay elapses", async () => {
    const { container } = render(
      <LoadingIndicator isLoading={true} message="Loading data" delay={30} />,
    );

    await advanceTimersBy(30);

    expect(getMessageText(container)).toContain("Loading data");
  });

  it("stops rendering when loading becomes false", async () => {
    const { container, rerender } = render(
      <LoadingIndicator isLoading={true} message="Loading data" delay={10} />,
    );

    await advanceTimersBy(10);

    expect(getMessageText(container)).toContain("Loading data");

      await act(async () => {
        rerender(
          <LoadingIndicator
            isLoading={false}
            message="Loading data"
            delay={10}
          />,
        );
        if (typeof vi.advanceTimersByTimeAsync === "function") {
          await vi.advanceTimersByTimeAsync(0);
        }
      });

    expect(container.textContent).toBe("");
  });

  it("cycles through spinner frames over time", async () => {
    const customFrames = [".", "..", "..."];
    const { container } = render(
      <LoadingIndicator
        isLoading={true}
        message="Loading data"
        delay={0}
        interval={5}
        frames={customFrames}
      />,
    );

    await advanceTimersBy(0);

    const firstFrame = getSpinnerText(container);

    await advanceTimersBy(5);

    const secondFrame = getSpinnerText(container);

    await advanceTimersBy(5);

    const thirdFrame = getSpinnerText(container);

    expect(secondFrame).not.toEqual(firstFrame);
    expect(thirdFrame).not.toEqual(secondFrame);
    expect(customFrames).toContain(firstFrame ?? "");
    expect(customFrames).toContain(secondFrame ?? "");
    expect(customFrames).toContain(thirdFrame ?? "");
    expect(getMessageText(container)).toContain("Loading data");
  });

  it("keeps rendering even when only a single frame is provided", async () => {
    const { container } = render(
      <LoadingIndicator
        isLoading={true}
        message="Loading data"
        delay={0}
        interval={10}
        frames={["*"]}
      />,
    );

    await advanceTimersBy(0);
    expect(getSpinnerText(container)).toBe("*");

    await advanceTimersBy(30);
    expect(getSpinnerText(container)).toBe("*");
    expect(getMessageText(container)).toContain("Loading data");
  });
});
