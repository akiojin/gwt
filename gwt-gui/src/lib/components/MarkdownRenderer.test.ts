import { describe, it, expect } from "vitest";
import { render } from "@testing-library/svelte";

async function renderRenderer(markdown: string) {
  const { default: MarkdownRenderer } = await import("./MarkdownRenderer.svelte");
  return render(MarkdownRenderer, { props: { text: markdown } });
}

describe("MarkdownRenderer", () => {
  it("renders headings and lists from markdown", async () => {
    const rendered = await renderRenderer("## 要約\n- A\n- B");

    expect(rendered.container.querySelector("h2")?.textContent).toBe("要約");
    expect(rendered.container.querySelectorAll("li")).toHaveLength(2);
  });

  it("renders inline markdown elements", async () => {
    const rendered = await renderRenderer(
      "Use **bold**, *italic*, and `code` with [docs](https://example.com)."
    );

    expect(rendered.container.querySelector("strong")?.textContent).toBe("bold");
    expect(rendered.container.querySelector("em")?.textContent).toBe("italic");
    expect(rendered.container.querySelector("code")?.textContent).toBe("code");
    expect(rendered.container.querySelector("a")?.getAttribute("href")).toBe(
      "https://example.com"
    );
  });

  it("escapes potentially dangerous html tags", async () => {
    const rendered = await renderRenderer("<script>alert('x')</script>");
    expect(rendered.container.querySelector("script")).toBeNull();
    expect(rendered.container.textContent).toContain("<script>alert('x')</script>");
  });
});
