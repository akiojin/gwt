<script lang="ts">
  import { marked } from 'marked';
  import DOMPurify from 'dompurify';

  let {
    text = "",
    className = "",
  }: {
    text?: string | null;
    className?: string;
  } = $props();

  // Configure marked renderer to add target="_blank" to links
  const renderer = new marked.Renderer();
  renderer.link = ({ href, text: linkText }) => {
    const safeHref = DOMPurify.sanitize(href ?? '', { ALLOWED_TAGS: [] });
    return `<a href="${safeHref}" target="_blank" rel="noopener noreferrer">${linkText ?? ''}</a>`;
  };

  marked.setOptions({
    gfm: true,
    breaks: false,
    renderer,
  });

  const renderMarkdown = (markdown: string | null | undefined): string => {
    const source = String(markdown ?? '').trim();
    if (!source) return '';

    const rawHtml = marked.parse(source, { async: false }) as string;
    return DOMPurify.sanitize(rawHtml, {
      ADD_ATTR: ['target'],
    });
  };

  const contentHtml = $derived(renderMarkdown(text));
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
    border-radius: var(--radius-sm);
    padding: 0.06rem 0.24rem;
  }

  .markdown-renderer :global(pre) {
    margin: 0 0 0.55rem;
    border: 1px solid var(--border-color);
    border-radius: var(--radius-lg);
    background: var(--bg-primary);
    padding: var(--space-3);
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

  .markdown-renderer :global(table) {
    border-collapse: collapse;
    margin: 0 0 0.55rem;
    width: 100%;
  }

  .markdown-renderer :global(th),
  .markdown-renderer :global(td) {
    border: 1px solid var(--border-color);
    padding: var(--space-2) var(--space-3);
    text-align: left;
  }

  .markdown-renderer :global(th) {
    font-weight: 700;
    color: var(--text-primary);
    background: var(--bg-surface);
  }

  .markdown-renderer :global(del) {
    text-decoration: line-through;
  }

  .markdown-renderer :global(img) {
    max-width: 100%;
    height: auto;
    border-radius: var(--radius-sm);
  }

  .markdown-renderer :global(input[type="checkbox"]) {
    margin-right: var(--space-1);
    pointer-events: none;
  }
</style>
