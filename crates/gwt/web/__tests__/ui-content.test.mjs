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
  const typed = renderUiContent(document, markdownContent({ body: '**Typed**', content: [
    { type: 'element', tag: 'p', children: [
      { type: 'element', tag: 'strong', children: [{ type: 'text', text: 'Typed' }] },
      { type: 'text', text: '<script>literal</script>' },
      { type: 'element', tag: 'a', href: 'https://example.com', children: [{ type: 'text', text: 'safe' }] },
      { type: 'element', tag: 'a', href: 'javascript:alert(1)', children: [{ type: 'text', text: 'unsafe' }] },
    ] },
  ] }));
  assert.equal(typed.querySelector('strong').textContent, 'Typed');
  assert.equal(typed.querySelector('script'), null);
  assert.equal(typed.querySelector('a').getAttribute('href'), 'https://example.com');
  assert.equal(typed.querySelectorAll('a')[1].hasAttribute('href'), false);
  assert.throws(() => renderUiContent(document, markdownContent({ content: [
    { type: 'element', tag: 'script', children: [] },
  ] })), /Unsupported Markdown/);
});
