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
  let mounted;
  const unsubscribe = store.subscribe(state => state.turns, turns => { mounted = turns; });
  assert.deepEqual(mounted.map(turn => turn.id), ['1', '2']);
  unsubscribe();
  store.update(state => ({ ...state, turns: [] }));
  assert.equal(mounted.length, 2, 'disposed views stop receiving changes');
});

test('push payloads and renderer code cannot mutate the canonical snapshot', () => {
  const payload = { turns: [{ id: '1' }] };
  const store = createUiStateStore(payload);
  payload.turns[0].id = 'outside';
  assert.equal(store.read().turns[0].id, '1');
  assert.throws(() => { store.read().turns[0].id = 'renderer'; }, TypeError);
  assert.throws(() => store.update(() => ({ node: { callback() {} } })), TypeError);
  assert.equal(store.read().turns[0].id, '1', 'invalid updates leave state intact');
});

test('a throwing view does not leave other views stale and nested updates settle at the latest model', () => {
  const store = createUiStateStore({ value: 0 });
  const values = [];
  store.subscribe(state => state.value, value => { if (value === 1) throw new Error('broken view'); });
  store.subscribe(state => state.value, value => values.push(value));
  assert.throws(() => store.update(() => ({ value: 1 })), /broken view/);
  assert.deepEqual(values, [0, 1]);
  store.subscribe(state => state.value, value => { if (value === 2) store.update(() => ({ value: 3 })); });
  store.update(() => ({ value: 2 }));
  assert.equal(values.at(-1), 3);
  store.dispose();
  assert.throws(() => store.update(() => ({ value: 4 })), /disposed/);
});
