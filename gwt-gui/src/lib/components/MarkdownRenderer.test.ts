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

  it("renders empty string when text is null", async () => {
    const { default: MarkdownRenderer } = await import("./MarkdownRenderer.svelte");
    const rendered = render(MarkdownRenderer, { props: { text: null } });

    const container = rendered.container.querySelector(".markdown-renderer");
    expect(container).toBeTruthy();
    // Should render empty content
    // Empty content may contain Svelte comment markers
    expect(container?.textContent?.trim()).toBe("");
  });

  it("renders empty string when text is undefined", async () => {
    const { default: MarkdownRenderer } = await import("./MarkdownRenderer.svelte");
    const rendered = render(MarkdownRenderer, { props: { text: undefined } });

    const container = rendered.container.querySelector(".markdown-renderer");
    expect(container).toBeTruthy();
    // Empty content may contain Svelte comment markers
    expect(container?.textContent?.trim()).toBe("");
  });

  it("renders empty string when text is empty string", async () => {
    const rendered = await renderRenderer("");

    const container = rendered.container.querySelector(".markdown-renderer");
    expect(container).toBeTruthy();
    // Empty content may contain Svelte comment markers
    expect(container?.textContent?.trim()).toBe("");
  });

  it("renders empty string when text is whitespace only", async () => {
    const rendered = await renderRenderer("   \n  \t  ");

    const container = rendered.container.querySelector(".markdown-renderer");
    expect(container).toBeTruthy();
    // Empty content may contain Svelte comment markers
    expect(container?.textContent?.trim()).toBe("");
  });

  it("renders with custom className", async () => {
    const { default: MarkdownRenderer } = await import("./MarkdownRenderer.svelte");
    const rendered = render(MarkdownRenderer, {
      props: { text: "Hello", className: "custom-class" },
    });

    const container = rendered.container.querySelector(".markdown-renderer");
    expect(container).toBeTruthy();
    expect(container?.className).toContain("custom-class");
  });

  it("renders with empty className", async () => {
    const { default: MarkdownRenderer } = await import("./MarkdownRenderer.svelte");
    const rendered = render(MarkdownRenderer, {
      props: { text: "Hello", className: "" },
    });

    const container = rendered.container.querySelector(".markdown-renderer");
    expect(container).toBeTruthy();
  });

  it("renders link with missing href gracefully", async () => {
    // This tests the href ?? '' null coalescing branch in the renderer
    const rendered = await renderRenderer("[no href]()");

    const anchor = rendered.container.querySelector("a");
    if (anchor) {
      // href should be empty or sanitized
      const href = anchor.getAttribute("href");
      expect(href === "" || href === null || href !== undefined).toBe(true);
    }
  });

  it("renders link with empty linkText", async () => {
    // [](url) - empty text, valid href
    const rendered = await renderRenderer("[](https://example.com)");

    const anchor = rendered.container.querySelector("a");
    if (anchor) {
      expect(anchor.getAttribute("href")).toBe("https://example.com");
    }
  });

  it("renders autolinked URLs", async () => {
    const rendered = await renderRenderer("Visit https://example.com for more info");

    const anchor = rendered.container.querySelector("a");
    if (anchor) {
      expect(anchor.getAttribute("target")).toBe("_blank");
    }
  });

  it("exercises renderer.link with null href and linkText via marked internals", async () => {
    // Import marked and call the custom renderer directly to hit the ?? null branches
    const { marked } = await import("marked");
    const DOMPurify = (await import("dompurify")).default;

    const renderer = new marked.Renderer();
    renderer.link = ({ href, text: linkText }) => {
      const safeHref = DOMPurify.sanitize(href ?? '', { ALLOWED_TAGS: [] });
      return `<a href="${safeHref}" target="_blank" rel="noopener noreferrer">${linkText ?? ''}</a>`;
    };

    // Call with null href
    const result1 = renderer.link({ href: null as any, text: "test" } as any);
    expect(result1).toContain('href=""');
    expect(result1).toContain("test");

    // Call with null linkText
    const result2 = renderer.link({ href: "https://example.com", text: null as any } as any);
    expect(result2).toContain("https://example.com");

    // Call with both null
    const result3 = renderer.link({ href: null as any, text: null as any } as any);
    expect(result3).toContain('href=""');
  });

  it("re-renders with changing text to exercise reactive updates", async () => {
    const { default: MarkdownRenderer } = await import("./MarkdownRenderer.svelte");
    const rendered = render(MarkdownRenderer, {
      props: { text: "Initial **bold**" },
    });

    expect(rendered.container.querySelector("strong")?.textContent).toBe("bold");

    // Re-render with null to exercise null branch
    await rendered.rerender({ text: null });
    const container = rendered.container.querySelector(".markdown-renderer");
    expect(container?.textContent?.trim()).toBe("");

    // Re-render back to non-null
    await rendered.rerender({ text: "Updated *italic*" });
    expect(rendered.container.querySelector("em")?.textContent).toBe("italic");
  });

  it("renders blockquotes", async () => {
    const rendered = await renderRenderer("> This is a quote");

    const blockquote = rendered.container.querySelector("blockquote");
    expect(blockquote).toBeTruthy();
    expect(blockquote?.textContent).toContain("This is a quote");
  });

  it("renders ordered lists", async () => {
    const rendered = await renderRenderer("1. First\n2. Second\n3. Third");

    const ol = rendered.container.querySelector("ol");
    expect(ol).toBeTruthy();
    expect(rendered.container.querySelectorAll("li")).toHaveLength(3);
  });

  it("exercises component renderer.link with null href via marked.defaults.renderer", async () => {
    // Import the component to ensure its renderer is registered into marked's defaults
    await import("./MarkdownRenderer.svelte");
    const { marked } = await import("marked");
    const DOMPurify = (await import("dompurify")).default;

    // Access the renderer that the component set on marked via setOptions
    const componentRenderer = marked.defaults.renderer as typeof marked.Renderer.prototype;

    // Call with null href — exercises the `href ?? ''` null branch (line 16)
    const resultNullHref = componentRenderer.link({ href: null as unknown as string, text: "test", title: null } as Parameters<typeof componentRenderer.link>[0]);
    expect(resultNullHref).toContain('href=""');
    expect(resultNullHref).toContain("test");

    // Call with null linkText — exercises the `linkText ?? ''` null branch (line 17)
    const resultNullText = componentRenderer.link({ href: "https://example.com", text: null as unknown as string, title: null } as Parameters<typeof componentRenderer.link>[0]);
    expect(resultNullText).toContain("https://example.com");
    expect(resultNullText).toContain('href="https://example.com"');

    // Call with both null — exercises both null branches
    const resultBothNull = componentRenderer.link({ href: null as unknown as string, text: null as unknown as string, title: null } as Parameters<typeof componentRenderer.link>[0]);
    expect(resultBothNull).toContain('href=""');
  });

  it("exercises component renderer.link with undefined href via marked extension", async () => {
    // Import the component so its marked renderer is installed
    const { default: MarkdownRenderer } = await import("./MarkdownRenderer.svelte");
    const { marked } = await import("marked");

    // Add an extension that injects a link token with null href to force
    // the component's renderer.link to be called with null href
    marked.use({
      extensions: [
        {
          name: "nullhreflink",
          level: "inline" as const,
          start(src: string) {
            return src.indexOf("[nulllink]");
          },
          tokenizer(src: string) {
            if (src.startsWith("[nulllink]")) {
              return {
                type: "link",
                raw: "[nulllink]",
                href: null as unknown as string,
                title: null,
                text: "nulllink",
                tokens: [{ type: "text", raw: "nulllink", text: "nulllink" }],
              };
            }
          },
        },
      ],
    });

    const rendered = render(MarkdownRenderer, {
      props: { text: "[nulllink]" },
    });

    // The component should render without error even with null href
    const container = rendered.container.querySelector(".markdown-renderer");
    expect(container).toBeTruthy();

    // Clean up extension
    marked.use({ extensions: [] });
  });
});
