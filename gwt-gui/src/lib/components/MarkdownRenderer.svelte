<script lang="ts">
  let {
    text = "",
    className = "",
  }: {
    text?: string | null;
    className?: string;
  } = $props();

  const markdownToHtml = (markdown: string | null | undefined): string => {
    const source = normalizeMarkdown(markdown);
    if (!source) return "";

    const lines = source.split("\n");
    let html = "";
    let listKind: "ul" | "ol" | null = null;
    let inCodeBlock = false;
    const paragraphLines: string[] = [];

    const closeList = () => {
      if (listKind) {
        html += `</${listKind}>`;
        listKind = null;
      }
    };

    const flushParagraph = () => {
      if (paragraphLines.length === 0) return;
      html += `<p>${paragraphLines.join("<br />")}</p>`;
      paragraphLines.length = 0;
    };

    const startList = (nextKind: "ul" | "ol") => {
      if (listKind === nextKind) return;
      closeList();
      flushParagraph();
      html += `<${nextKind}>`;
      listKind = nextKind;
    };

    const appendParagraphLine = (line: string) => {
      if (line === "") {
        if (listKind) closeList();
        flushParagraph();
      } else {
        paragraphLines.push(applyInlineMarkdown(line));
      }
    };

    for (const rawLine of lines) {
      const line = normalizeLine(rawLine);

      if (inCodeBlock) {
        if (line.trim() === "```") {
          html += "</code></pre>";
          inCodeBlock = false;
          continue;
        }
        html += `${escapeHtml(line)}\n`;
        continue;
      }

      if (line.trim() === "") {
        appendParagraphLine("");
        continue;
      }

      if (line.trim().startsWith("```")) {
        closeList();
        flushParagraph();
        inCodeBlock = true;
        html += "<pre><code>";
        continue;
      }

      const headingMatch = line.match(/^(#{1,6})\s+(.*)$/);
      if (headingMatch) {
        closeList();
        flushParagraph();
        const level = Math.min(6, headingMatch[1].length);
        html += `<h${level}>${applyInlineMarkdown(headingMatch[2])}</h${level}>`;
        continue;
      }

      const unorderedMatch = line.match(/^(?:\s{0,3})(?:-|\+|\*)\s+(.*)$/);
      if (unorderedMatch) {
        startList("ul");
        html += `<li>${applyInlineMarkdown(unorderedMatch[1])}</li>`;
        continue;
      }

      const orderedMatch = line.match(/^(?:\s{0,3})(\d+)\.\s+(.*)$/);
      if (orderedMatch) {
        startList("ol");
        html += `<li>${applyInlineMarkdown(orderedMatch[2])}</li>`;
        continue;
      }

      const quoteMatch = line.match(/^>\s?(.*)$/);
      if (quoteMatch) {
        closeList();
        flushParagraph();
        html += `<blockquote>${applyInlineMarkdown(quoteMatch[1])}</blockquote>`;
        continue;
      }

      if (listKind) {
        // Plain lines break list context and start a new paragraph.
        closeList();
      }
      appendParagraphLine(line);
    }

    if (inCodeBlock) {
      html += "</code></pre>";
    }
    closeList();
    flushParagraph();

    return html;
  };

  const normalizeMarkdown = (markdown: string | null | undefined): string =>
    String(markdown ?? "")
      .replace(/\r\n?/g, "\n")
      .replace(/\n{3,}/g, "\n\n")
      .trim();

  const normalizeLine = (line: string): string => {
    return line.replace(/\t/g, "  ");
  };

  const applyInlineMarkdown = (text: string): string => {
    let out = escapeHtml(text);
    out = out.replace(
      /`([^`]+)`/g,
      (match, code) => `<code>${escapeHtml(String(code))}</code>`
    );
    out = out.replace(/\*\*([^\*]+?)\*\*/g, "<strong>$1</strong>");
    out = out.replace(/\*([^\*]+?)\*/g, "<em>$1</em>");
    out = out.replace(
      /\[([^\]]+)\]\(([^)\s]+)(?:\s+"[^"]*")?\)/g,
      (match, label, href) => {
        const sanitized = sanitizeUrl(String(href));
        if (!sanitized) return `<code>${label}</code>`;
        return `<a href="${escapeAttribute(sanitized)}" target="_blank" rel="noopener noreferrer">${label}</a>`;
      }
    );
    return out;
  };

  const escapeHtml = (value: string): string =>
    value
      .replace(/&/g, "&amp;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;")
      .replace(/"/g, "&quot;")
      .replace(/'/g, "&#39;");

  const escapeAttribute = (value: string): string =>
    value
      .replace(/&/g, "&amp;")
      .replace(/\"/g, "&quot;")
      .replace(/'/g, "&#39;")
      .replace(/</g, "&lt;")
      .replace(/>/g, "&gt;");

  const sanitizeUrl = (value: string): string => {
    const candidate = value.trim();
    const lowered = candidate.toLowerCase();

    if (!candidate) return "";
    if (lowered.startsWith("javascript:") || lowered.startsWith("vbscript:") || lowered.startsWith("data:")) {
      return "";
    }

    return candidate;
  };

  const contentHtml = $derived(markdownToHtml(text));
</script>

<div class={`markdown-renderer ${className}`.trim()}>{@html contentHtml}</div>

<style>
  .markdown-renderer {
    width: 100%;
    box-sizing: border-box;
    color: var(--text-secondary);
    font-size: var(--ui-font-sm);
    line-height: 1.5;
    max-width: 100%;
    overflow-wrap: anywhere;
    word-break: break-word;
  }

  .markdown-renderer :global(p) {
    margin: 0 0 0.55rem 0;
  }

  .markdown-renderer :global(h1),
  .markdown-renderer :global(h2),
  .markdown-renderer :global(h3),
  .markdown-renderer :global(h4),
  .markdown-renderer :global(h5),
  .markdown-renderer :global(h6) {
    margin: 0.65rem 0 0.35rem;
    color: var(--text-primary);
    line-height: 1.4;
    font-weight: 700;
  }

  .markdown-renderer :global(h1) {
    font-size: var(--ui-font-lg);
  }

  .markdown-renderer :global(h2) {
    font-size: var(--ui-font-md);
  }

  .markdown-renderer :global(h3),
  .markdown-renderer :global(h4),
  .markdown-renderer :global(h5),
  .markdown-renderer :global(h6) {
    font-size: var(--ui-font-sm);
  }

  .markdown-renderer :global(ul),
  .markdown-renderer :global(ol) {
    margin: 0 0 0.55rem 1rem;
    padding: 0;
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
  }

  .markdown-renderer :global(li) {
    margin: 0;
    padding: 0;
  }

  .markdown-renderer :global(blockquote) {
    border-left: 2px solid var(--border-color);
    margin: 0 0 0.55rem;
    padding: 0.1rem 0 0.1rem 0.7rem;
    color: var(--text-muted);
  }

  .markdown-renderer :global(code) {
    font-family: ui-monospace, SFMono-Regular, Menlo, Monaco, Consolas,
      "Liberation Mono", "Courier New", monospace;
    font-size: 0.92em;
    background: rgba(255, 255, 255, 0.06);
    border-radius: 6px;
    padding: 0.06rem 0.24rem;
  }

  .markdown-renderer :global(pre) {
    margin: 0 0 0.55rem;
    border: 1px solid var(--border-color);
    border-radius: 10px;
    background: var(--bg-primary);
    padding: 10px 12px;
    overflow: auto;
    max-width: 100%;
    white-space: pre-wrap;
    overflow-wrap: anywhere;
    word-break: break-word;
  }

  .markdown-renderer :global(pre code) {
    background: transparent;
    border: none;
    padding: 0;
  }

  .markdown-renderer :global(a) {
    color: var(--cyan);
    text-decoration: underline;
    text-underline-offset: 2px;
  }

  .markdown-renderer :global(a:hover) {
    color: var(--cyan-hover);
  }

  .markdown-renderer :global(strong) {
    font-weight: 700;
    color: var(--text-primary);
  }

  .markdown-renderer :global(em) {
    font-style: italic;
  }
</style>
