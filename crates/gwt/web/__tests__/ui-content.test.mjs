import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseHTML } from 'linkedom';
import { markdownContent, renderUiContent } from '../ui-content.js';

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
