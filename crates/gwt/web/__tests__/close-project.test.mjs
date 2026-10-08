import { test } from 'node:test';
import assert from 'node:assert/strict';
import { parseHTML } from 'linkedom';
import { createCloseProjectController } from '../close-project-confirm-modal.js';

const token = { project_key: '0123456789abcdef', generation: 1, nonce: 'nonce' };
function fixture() {
  const { document } = parseHTML('<html><body><button id="origin">Close Project</button></body></html>');
  const sent = [];
  const controller = createCloseProjectController({ document, send: (message) => sent.push(message) });
  document.body.append(controller.modal);
  return { document, controller, sent };
}
test('idle preview confirms directly with its exact token', () => {
  const { controller, sent } = fixture();
  controller.request(token.project_key);
  assert.deepEqual(sent.shift(), { kind: 'preview_close_project', project_key: token.project_key });
  controller.receive({ kind: 'close_project_preview', token, title: 'Alpha', running_agents: [] });
  assert.deepEqual(sent, [{ kind: 'confirm_close_project', token }]);
  assert.equal(controller.modal.classList.contains('open'), false);
});
test('running preview waits for confirmation and cancel consumes only the token', () => {
  const { controller, sent, document } = fixture();
  controller.receive({ kind: 'close_project_preview', token, title: 'Alpha', running_agents: [{ display_name: 'Codex' }] });
  assert.equal(sent.length, 0);
  assert.equal(controller.modal.querySelector('[role="dialog"]').getAttribute('aria-modal'), 'true');
  for (const primitive of ['modal-header', 'modal-body', 'modal-footer']) assert.ok(controller.modal.querySelector(`.${primitive}`));
  document.querySelector('[data-role="close-project-cancel"]').click();
  assert.deepEqual(sent, [{ kind: 'cancel_close_project', token }]);
  controller.receive({ kind: 'close_project_preview', token, title: 'Alpha', running_agents: [{ display_name: 'Codex' }] });
  document.querySelector('[data-role="close-project-confirm"]').click();
  assert.deepEqual(sent.at(-1), { kind: 'confirm_close_project', token });
});

test('Project shell removes project switching and retains window focus navigation', async () => {
  const { readFileSync } = await import('node:fs');
  const read = (file) => readFileSync(new URL(`../${file}`, import.meta.url), 'utf8');
  assert.doesNotMatch(read('index.html'), /id="project-tabs"|id="project-switcher-button"/);
  assert.doesNotMatch(read('app.js') + read('project-shell-surface.js'), /select_project_tab|handleProjectSwitcherShortcut/);
  assert.match(read('app.js'), /function shouldHandleFocusShortcut/);
  assert.match(read('app.js'), /cycleFocus\(event.key === "ArrowRight"/);
});

test('repeated close requests cancel the current preview before requesting another', () => {
  const { controller, sent } = fixture();
  controller.request(token.project_key);
  controller.request(token.project_key);
  assert.equal(sent.length, 1, 'only one preview may be in flight');
  controller.receive({ kind: 'close_project_preview', token, title: 'Alpha', running_agents: [{ display_name: 'Codex' }] });
  controller.request(token.project_key);
  assert.deepEqual(sent.slice(1), [
    { kind: 'cancel_close_project', token },
    { kind: 'preview_close_project', project_key: token.project_key },
  ]);
  controller.receive({ kind: 'close_project_preview', token: { ...token, nonce: 'next' }, title: 'Alpha', running_agents: [] });
  assert.equal(controller.modal.classList.contains('open'), false);
  assert.deepEqual(sent.at(-1), { kind: 'confirm_close_project', token: { ...token, nonce: 'next' } });
});

test('disconnect releases an unanswered preview and discards old modal authority without sending', () => {
  const { controller, sent } = fixture();
  controller.request(token.project_key);
  controller.connectionLost();
  assert.equal(sent.length, 1, 'disconnect must not send a cancel on another connection');
  controller.request(token.project_key);
  assert.equal(sent.length, 2, 'retry must request a fresh preview after reconnect');
  controller.receive({ kind: 'close_project_preview', token, title: 'Alpha', running_agents: [{ display_name: 'Codex' }] });
  controller.connectionLost();
  assert.equal(controller.modal.classList.contains('open'), false);
  assert.equal(sent.length, 2, 'old authority is discarded locally');
  controller.request(token.project_key);
  assert.deepEqual(sent.at(-1), { kind: 'preview_close_project', project_key: token.project_key });
});
