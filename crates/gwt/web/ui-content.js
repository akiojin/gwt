// Renderer selection follows the content model type, rather than per-view wiring.
// body_html retains the existing backend-sanitized Markdown contract; raw user
// text always uses textContent. This module never parses arbitrary HTML inputs.
export function markdownContent(section) {
  return { type: 'markdown', body: section?.body || '', body_html: section?.body_html || '' };
}

export function renderUiContent(document, content, className = '') {
  if (!['text', 'markdown'].includes(content?.type)) {
    throw new TypeError(`Unsupported UI content: ${content?.type}`);
  }
  const node = document.createElement('div');
  node.className = className;
  if (content.type === 'markdown') node.classList.add('knowledge-markdown-body');
  const html = content.type === 'markdown' && typeof content.body_html === 'string'
    ? content.body_html.trim() : '';
  if (html) node.innerHTML = html;
  else {
    node.classList.add('is-plaintext');
    node.textContent = content.body || '';
  }
  return node;
}
