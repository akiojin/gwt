import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseHTML } from 'linkedom';
import { readFileSync } from 'node:fs';
import { markdownContent, renderUiContent } from '../ui-content.js';

test('views render model-typed content without selecting Markdown or injecting a renderer', () => {
  for (const [file, bindings] of [['pm-chat.js', 1], ['board-logs-surface.js', 1], ['knowledge-kanban-surface.js', 3], ['release-notes-window.js', 1]]) {
    const source = readFileSync(new URL(`../${file}`, import.meta.url), 'utf8');
    assert.doesNotMatch(source, /markdownContent|createKnowledgeMarkdownBody/, file);
    assert.equal([...source.matchAll(/renderUiContent\(document, \w+\.content\b/g)].length, bindings,
      `${file}: every body must use its model content type`);
  }
  const app = readFileSync(new URL('../app.js', import.meta.url), 'utf8');
  assert.doesNotMatch(app, /createKnowledgeMarkdownBody|markdownContent|renderUiContent/);
});

test('the model type selects the shared renderer and preserves sanitized markdown and plaintext fallback', () => {
  const { document } = parseHTML('<html><body></body></html>');
  const content = markdownContent({body:'**Answer**',body_html:'<p><strong>Answer</strong></p>'});
  const knowledge = renderUiContent(document, content, 'knowledge-section-body');
  const board = renderUiContent(document, content, 'board-body');
  assert.equal(knowledge.innerHTML, board.innerHTML);
  assert.equal(knowledge.querySelector('strong').textContent, 'Answer');
  assert.ok(knowledge.classList.contains('knowledge-markdown-body'));
  const fallback = renderUiContent(document, markdownContent({body:'<script>literal</script>'}));
  assert.equal(fallback.querySelector('script'), null);
  assert.equal(fallback.textContent, '<script>literal</script>');
  assert.ok(fallback.classList.contains('is-plaintext'));
  assert.throws(() => renderUiContent(document, {type:'unregistered',body:'answer'}), /Unsupported UI content/);
});
