import { test } from 'node:test';
import assert from 'node:assert/strict';
import { createUiStateStore } from '../ui-state-store.js';

test('one model update reaches every view, including a newly mounted view', () => {
  const store = createUiStateStore({ turns: [] });
  const chat = [], log = [];
  store.subscribe(state => state.turns, turns => chat.push(turns.map(turn => turn.id)));
  store.subscribe(state => state.turns, turns => log.push(turns.map(turn => turn.id)));
  store.update(state => ({ ...state, turns: [{ id: '1' }, { id: '2' }] }));
  assert.deepEqual(chat, [[], ['1', '2']]);
  assert.deepEqual(log, chat);
  const turns = store.read().turns;
  store.update(state => ({ ...state, unrelated: true }));
  assert.equal(store.read().turns, turns, 'owned children retain their identity');
  assert.equal(chat.length, 2, 'unchanged selectors do not notify their views');
  assert.equal(log.length, 2);
  let mounted;
  const unsubscribe = store.subscribe(state => state.turns, turns => { mounted = turns; });
  assert.deepEqual(mounted.map(turn => turn.id), ['1', '2']);
  unsubscribe();
  store.update(state => ({ ...state, turns: [] }));
  assert.equal(mounted.length, 2, 'disposed views stop receiving changes');
});

test('push payloads and renderer code cannot mutate the canonical snapshot', () => {
  const payload = JSON.parse('{"turns":[{"id":"1"}],"__proto__":{"kind":"data"}}');
  const store = createUiStateStore(payload);
  assert.equal(Object.getPrototypeOf(store.read()), Object.prototype);
  assert.equal(Object.hasOwn(store.read(), '__proto__'), true);
  assert.throws(() => { store.read().__proto__.kind = 'renderer'; }, TypeError);
  payload.turns[0].id = 'outside';
  assert.equal(store.read().turns[0].id, '1');
  assert.throws(() => { store.read().turns[0].id = 'renderer'; }, TypeError);
  assert.throws(() => store.update(() => ({ node: { callback() {} } })), TypeError);
  assert.equal(store.read().turns[0].id, '1', 'invalid updates leave state intact');
});

test('snapshot reads all enumerable values before copying their children', () => {
  const child = { value: 'before' };
  const store = createUiStateStore({
    child,
    get updateChild() { child.value = 'after'; return true; },
  });
  assert.equal(store.read().child.value, 'after');
  assert.equal(store.read().updateChild, true);
});

test('a throwing view does not leave other views stale and nested updates settle at the latest model', () => {
  const store = createUiStateStore({ value: 0 });
  const values = [];
  let broken = true, rendered = 0;
  store.subscribe(state => state.value, value => {
    if (value === 1 && broken) throw new Error('broken view');
    rendered = value;
  });
  store.subscribe(state => state.value, value => values.push(value));
  assert.throws(() => store.update(() => ({ value: 1 })), /broken view/);
  assert.deepEqual(values, [0, 1]);
  broken = false;
  store.update(state => ({ ...state, unrelated: true }));
  assert.equal(rendered, 1, 'the next model update retries a view that failed to render');
  store.subscribe(state => state.value, value => { if (value === 2) store.update(() => ({ value: 3 })); });
  store.update(() => ({ value: 2 }));
  assert.equal(values.at(-1), 3);
  store.dispose();
  assert.throws(() => store.update(() => ({ value: 4 })), /disposed/);
});
