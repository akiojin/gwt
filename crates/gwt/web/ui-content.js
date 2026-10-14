// Renderer selection follows the content model type, rather than per-view wiring.
// body_html retains the existing backend-sanitized Markdown contract; raw user
// text always uses textContent. This module never parses arbitrary HTML inputs.
export function markdownContent(section) {
  return { type: 'markdown', body: section?.body || '', body_html: section?.body_html || '',
    nodes: section?.content };
}

const markdownTags = new Set(['p', 'h1', 'h2', 'h3', 'h4', 'h5', 'h6', 'blockquote',
  'pre', 'code', 'ul', 'ol', 'li', 'table', 'thead', 'tbody', 'tr', 'th', 'td',
  'em', 'strong', 'del', 'a', 'img', 'span', 'br', 'hr']);

function safeMarkdownUrl(value) {
  if (typeof value !== 'string' || /[\u0000-\u0020\\]/.test(value)) return false;
  return !/^[a-z][a-z\d+.-]*:/i.test(value) || /^(https?:|mailto:)/i.test(value);
}

function renderMarkdownNode(document, content) {
  if (content?.type === 'text' && typeof content.text === 'string') return document.createTextNode(content.text);
  if (content?.type !== 'element' || !markdownTags.has(content.tag) || !Array.isArray(content.children)) {
    throw new TypeError('Unsupported Markdown node');
  }
  const node = document.createElement(content.tag);
  if (content.tag === 'a' && safeMarkdownUrl(content.href)) node.setAttribute('href', content.href);
  if (content.tag === 'img') {
    if (safeMarkdownUrl(content.href) && !/^mailto:/i.test(content.href)) node.setAttribute('src', content.href);
    const alt = document.createElement('span');
    for (const child of content.children) alt.append(renderMarkdownNode(document, child));
    node.setAttribute('alt', alt.textContent);
    return node;
  }
  if (content.tag === 'ol' && Number.isSafeInteger(content.start) && content.start > 0) node.setAttribute('start', String(content.start));
  for (const child of content.children) node.append(renderMarkdownNode(document, child));
  return node;
}

export function renderUiContent(document, content, className = '') {
  if (!['text', 'markdown'].includes(content?.type)) {
    throw new TypeError(`Unsupported UI content: ${content?.type}`);
  }
  const node = document.createElement('div');
  node.className = className;
  if (content.type === 'markdown') node.classList.add('knowledge-markdown-body');
  if (content.type === 'markdown' && Array.isArray(content.nodes)) {
    for (const child of content.nodes) node.append(renderMarkdownNode(document, child));
    return node;
  }
  const html = content.type === 'markdown' && typeof content.body_html === 'string'
    ? content.body_html.trim() : '';
  if (html) node.innerHTML = html;
  else {
    node.classList.add('is-plaintext');
    node.textContent = content.body || '';
  }
  return node;
}
