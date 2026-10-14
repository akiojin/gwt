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

test('view batching keeps every stream packet synchronous and renders only the changed selector', () => {
  const frames = new Map();
  let nextFrame = 0;
  const store = createUiStateStore({ panes: [0, 0, 0, 0, 0], packet: null }, {
    scheduleView(callback) {
      const id = ++nextFrame;
      frames.set(id, callback);
      return () => frames.delete(id);
    },
  });
  const consumed = [], rendered = Array.from({ length: 5 }, () => []);
  store.subscribe(state => state.packet, packet => { if (packet) consumed.push(packet); });
  for (let pane = 0; pane < 5; pane++) {
    store.subscribeView(state => state.panes[pane], value => rendered[pane].push(value));
  }
  for (const packet of ['one', 'two', 'three']) {
    store.update(state => ({ ...state, packet, panes: [state.panes[0] + 1, 0, 0, 0, 0] }));
  }
  assert.deepEqual(consumed, ['one', 'two', 'three'], 'batching never coalesces RAW consumption');
  assert.deepEqual(rendered, [[0], [0], [0], [0], [0]]);
  assert.equal(frames.size, 1, 'unrelated selectors reserve no render');
  const flush = () => {
    const ready = [...frames.values()];
    frames.clear();
    for (const callback of ready) callback();
  };
  flush();
  assert.deepEqual(rendered, [[0, 3], [0], [0], [0], [0]]);
  for (const pane of [4, 2, 1, 3, 4]) {
    store.update(state => ({ ...state, packet: `pane-${pane}`,
      panes: state.panes.map((value, index) => value + Number(index === pane)) }));
  }
  assert.deepEqual(consumed, ['one', 'two', 'three', 'pane-4', 'pane-2', 'pane-1', 'pane-3', 'pane-4']);
  assert.equal(frames.size, 4);
  flush();
  assert.deepEqual(rendered, [[0, 3], [0, 1], [0, 1], [0, 1], [0, 2]]);
});

test('unsubscribing and disposing cancel queued views and fence callbacks already dispatched', () => {
  const callbacks = [], cancelled = [];
  const store = createUiStateStore(0, { scheduleView(callback) {
    callbacks.push(callback);
    return () => cancelled.push(callback);
  } });
  const values = [];
  const unsubscribe = store.subscribeView(state => state, value => values.push(value));
  store.update(() => 1);
  unsubscribe();
  callbacks[0]();
  assert.deepEqual(values, [0]);
  store.subscribeView(state => state, value => values.push(value));
  store.update(() => 2);
  store.dispose();
  callbacks[1]();
  assert.deepEqual(values, [0, 1]);
  assert.equal(cancelled.length, 2);
});

test('the default view scheduler falls back within 100ms when animation frames stop', () => {
  const original = Object.fromEntries(['requestAnimationFrame', 'cancelAnimationFrame', 'setTimeout', 'clearTimeout']
    .map(key => [key, globalThis[key]]));
  const frames = new Map(), timers = new Map(), delays = [];
  let id = 0;
  globalThis.requestAnimationFrame = callback => { frames.set(++id, callback); return id; };
  globalThis.cancelAnimationFrame = key => frames.delete(key);
  globalThis.setTimeout = (callback, delay) => { delays.push(delay); timers.set(++id, callback); return id; };
  globalThis.clearTimeout = key => timers.delete(key);
  const values = [];
  let store;
  try {
    store = createUiStateStore(0);
    store.subscribeView(state => state, value => values.push(value));
    store.update(() => 1);
    assert.deepEqual(delays, [100]);
    const racedFrame = [...frames.values()][0];
    [...timers.values()][0]();
    assert.deepEqual(values, [0, 1]);
    assert.equal(frames.size, 0);
    store.update(() => 2);
    racedFrame();
    assert.deepEqual(values, [0, 1], 'an old frame cannot flush the next reservation');
    assert.equal(frames.size, 1);
    [...frames.values()][0]();
    assert.deepEqual(values, [0, 1, 2]);
    assert.equal(timers.size, 0, 'a successful frame cancels its timer');
  } finally {
    store?.dispose();
    for (const [key, value] of Object.entries(original)) {
      if (value === undefined) delete globalThis[key];
      else globalThis[key] = value;
    }
  }
});
