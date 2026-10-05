import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { parseHTML } from 'linkedom';

async function fixture() {
  const { document, window } = parseHTML('<html><body><div id="host"></div></body></html>');
  const root = document.getElementById('host');
  const sent = [], visibility = [];
  const { createPmChat } = await import('../pm-chat.js');
  const chat = createPmChat({ document, root, windowId: 'pm', sessionId: 'session', send: message => { sent.push(message); return 'sent'; }, onLogVisibility: visible => visibility.push(visible) });
  const update = (messages = [], availability = 'ready') => chat.update({ conversation_id: 'conversation', availability, messages });
  const submit = () => root.querySelector('form').dispatchEvent(new window.Event('submit', { cancelable: true }));
  return { root, chat, sent, visibility, update, submit };
}

test('chat starts with a notice and renders only nonempty conversation text safely', async () => {
  const { root, update, visibility } = await fixture();
  assert.equal(visibility.at(-1), false);
  assert.match(root.querySelector('[role="status"]').textContent, /waiting/i);
  update([{ id: '1', role: 'user', text: '<script>hello</script>' }, { id: '2', role: 'assistant', text: 'Answer' }, { id: '3', role: 'assistant', text: '  ' }]);
  assert.equal(root.querySelectorAll('.pm-chat__message').length, 2);
  assert.equal(root.querySelector('script'), null);
  assert.match(root.textContent, /<script>hello<\/script>/);
});

test('unchanged updates preserve nodes, draft and scroll; empty cycles add no bubble', async () => {
  const { root, update } = await fixture();
  const messages = [{ id: '1', role: 'assistant', text: 'Answer' }];
  update(messages);
  const message = root.querySelector('.pm-chat__message');
  const input = root.querySelector('textarea');
  input.value = 'Draft';
  const transcript = root.querySelector('.pm-chat__transcript');
  Object.defineProperties(transcript, { scrollHeight: { value: 1000, configurable: true }, clientHeight: { value: 200 } });
  transcript.scrollTop = 20;
  update([...messages]);
  assert.ok(root.querySelector('.pm-chat__message') === message, 'message node is retained');
  assert.equal(input.value, 'Draft');
  update([...messages, { id: '2', role: 'assistant', text: 'Next' }]);
  assert.equal(transcript.scrollTop, 20);
  update([]);
  assert.equal(root.querySelectorAll('.pm-chat__message').length, 0);
  assert.match(root.querySelector('[role="status"]').textContent, /messages/i);
});

test('execution log toggle preserves chat and unsupported providers use the terminal', async () => {
  const { root, update, visibility } = await fixture();
  update([{ id: '1', role: 'assistant', text: 'Answer' }]);
  root.querySelector('textarea').value = 'Draft';
  const buttons = root.querySelectorAll('.pm-chat__toolbar button');
  buttons[1].click();
  assert.equal(visibility.at(-1), true);
  assert.equal(buttons[1].getAttribute('aria-pressed'), 'true');
  buttons[0].click();
  assert.equal(visibility.at(-1), false);
  assert.equal(root.querySelector('textarea').value, 'Draft');
  update([], 'unavailable');
  assert.match(root.querySelector('[role="status"]').textContent, /unavailable/i);
  update([], 'unsupported');
  assert.equal(root.hidden, true);
  assert.equal(visibility.at(-1), true);
});

test('send waits for success, preserves rejected input and resets across sessions', async () => {
  const { root, chat, sent, update, submit } = await fixture();
  update();
  const input = root.querySelector('textarea');
  input.value = '   '; submit(); assert.equal(sent.length, 0);
  input.value = 'Please investigate'; submit();
  assert.deepEqual(sent[0], { kind: 'pane_send_input', session_id: 'session', text: 'Please investigate' });
  assert.equal(input.disabled, true);
  chat.handleSendResult({ window_id: 'another', ok: true });
  assert.equal(input.disabled, true);
  chat.handleSendResult({ window_id: 'pm', ok: false, error: 'Pane closed' });
  assert.equal(input.value, 'Please investigate');
  assert.equal(input.disabled, false);
  assert.match(root.textContent, /Pane closed/);
  submit(); chat.handleSendResult({ window_id: 'pm', ok: true });
  assert.equal(input.value, '');
  input.value = 'Old session'; submit(); chat.setSession('replacement');
  assert.equal(input.value, '');
  assert.equal(root.querySelectorAll('.pm-chat__message').length, 0);
  chat.dispose();
  assert.equal(root.children.length, 0);
});

test('PM chat uses a provider-neutral renderer and Operator tokens without an overlay shell', async () => {
  const renderer = await readFile(new URL('../pm-chat.js', import.meta.url), 'utf8');
  assert.doesNotMatch(renderer, /Claude|Codex|OpenCode|agent_id|⏺|⎿|\\x1b/i);
  const css = await readFile(new URL('../styles/components.css', import.meta.url), 'utf8');
  const section = css.slice(css.indexOf('/* PM conversation */'));
  assert.match(section, /\.pm-chat/);
  assert.match(section, /var\(--color-/);
  assert.doesNotMatch(section, /#[\da-f]{3,8}\b|rgba?\(|position:\s*(fixed|absolute)/i);
});

test('temporary unavailable and waiting snapshots retain history until the bound session changes', async () => {
  const { root, chat, update } = await fixture();
  update([{ id: '1', role: 'assistant', text: 'Keep this answer' }]);
  const message = root.querySelector('.pm-chat__message');
  chat.update({ conversation_id: null, availability: 'unavailable', messages: [], detail: 'Connection interrupted' });
  assert.ok(root.querySelector('.pm-chat__message') === message, 'message node is retained');
  assert.match(root.querySelector('[role="status"]').textContent, /Connection interrupted/);
  chat.update({ conversation_id: 'conversation', availability: 'waiting', messages: [] });
  assert.ok(root.querySelector('.pm-chat__message') === message, 'message node is retained');
  chat.update({ conversation_id: 'new-native-conversation', availability: 'waiting', messages: [] });
  assert.equal(root.querySelectorAll('.pm-chat__message').length, 0, 'a known new conversation cannot retain the previous conversation');
  chat.setSession('new-session');
  assert.equal(root.querySelectorAll('.pm-chat__message').length, 0);
});

test('ready snapshots expose history limitation details', async () => {
  const { root, chat } = await fixture();
  chat.update({ conversation_id: 'conversation', availability: 'ready', messages: [{ id: '1', role: 'assistant', text: 'Recent answer' }], detail: 'Only recent conversation history is available.' });
  const notice = root.querySelector('[role="status"]');
  assert.equal(notice.hidden, false);
  assert.match(notice.textContent, /Only recent conversation history/);
});
