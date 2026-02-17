import { describe, it, expect } from "vitest";
import { render } from "@testing-library/svelte";

async function renderRenderer(markdown: string) {
  const { default: MarkdownRenderer } = await import("./MarkdownRenderer.svelte");
  return render(MarkdownRenderer, { props: { text: markdown } });
}

describe("MarkdownRenderer", () => {
  it("renders headings h1-h6", async () => {
    const rendered = await renderRenderer(
      "# H1\n## H2\n### H3\n#### H4\n##### H5\n###### H6"
    );
    expect(rendered.container.querySelector("h1")?.textContent).toBe("H1");
    expect(rendered.container.querySelector("h2")?.textContent).toBe("H2");
    expect(rendered.container.querySelector("h3")?.textContent).toBe("H3");
    expect(rendered.container.querySelector("h4")?.textContent).toBe("H4");
    expect(rendered.container.querySelector("h5")?.textContent).toBe("H5");
    expect(rendered.container.querySelector("h6")?.textContent).toBe("H6");
  });

  it("renders code blocks with language", async () => {
    const rendered = await renderRenderer("```js\nconsole.log('hi');\n```");
    const pre = rendered.container.querySelector("pre");
    expect(pre).toBeTruthy();
    const code = pre?.querySelector("code");
    expect(code).toBeTruthy();
    expect(code?.textContent).toContain("console.log");
  });

  it("renders code blocks without language", async () => {
    const rendered = await renderRenderer("```\nhello world\n```");
    const pre = rendered.container.querySelector("pre");
    expect(pre).toBeTruthy();
    expect(pre?.textContent).toContain("hello world");
  });

  it("renders GFM tables", async () => {
    const rendered = await renderRenderer(
      "| Col A | Col B |\n| --- | --- |\n| 1 | 2 |"
    );
    expect(rendered.container.querySelector("table")).toBeTruthy();
    expect(rendered.container.querySelector("th")?.textContent).toBe("Col A");
    expect(rendered.container.querySelectorAll("td").length).toBe(2);
  });

  it("renders GFM checkboxes", async () => {
    const rendered = await renderRenderer("- [x] Done\n- [ ] Not done");
    const inputs = rendered.container.querySelectorAll("input[type='checkbox']");
    expect(inputs.length).toBe(2);
  });

  it("renders GFM strikethrough", async () => {
    const rendered = await renderRenderer("~~deleted~~");
    const del = rendered.container.querySelector("del");
    expect(del).toBeTruthy();
    expect(del?.textContent).toBe("deleted");
  });

  it("renders links with target=_blank", async () => {
    const rendered = await renderRenderer("[example](https://example.com)");
    const anchor = rendered.container.querySelector("a");
    expect(anchor).toBeTruthy();
    expect(anchor?.getAttribute("href")).toBe("https://example.com");
    expect(anchor?.getAttribute("target")).toBe("_blank");
  });

  it("renders images", async () => {
    const rendered = await renderRenderer("![alt text](https://example.com/img.png)");
    const img = rendered.container.querySelector("img");
    expect(img).toBeTruthy();
    expect(img?.getAttribute("src")).toBe("https://example.com/img.png");
    expect(img?.getAttribute("alt")).toBe("alt text");
  });

  it("sanitizes script tags (XSS)", async () => {
    const rendered = await renderRenderer("<script>alert('xss')</script>");
    expect(rendered.container.querySelector("script")).toBeNull();
    expect(rendered.container.innerHTML).not.toContain("<script>");
  });

  it("sanitizes onerror attributes (XSS)", async () => {
    const rendered = await renderRenderer('<img src=x onerror="alert(1)">');
    expect(rendered.container.innerHTML).not.toContain("onerror");
  });

  it("sanitizes javascript: URLs (XSS)", async () => {
    const rendered = await renderRenderer("[click](javascript:alert(1))");
    const anchor = rendered.container.querySelector("a");
    // DOMPurify should either remove the link entirely or strip the href
    if (anchor) {
      const href = anchor.getAttribute("href");
      // href should be null/empty or not contain javascript:
      expect(!href || !href.includes("javascript:")).toBe(true);
    }
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

  it("renders lists", async () => {
    const rendered = await renderRenderer("## Summary\n- A\n- B");

    expect(rendered.container.querySelector("h2")?.textContent).toBe("Summary");
    expect(rendered.container.querySelectorAll("li")).toHaveLength(2);
  });
});
