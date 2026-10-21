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

test('frame selectors batch five streams without dropping synchronous updates or waking unrelated views', () => {
  const frames = [];
  const store = createUiStateStore({ windows: [0, 0, 0, 0, 0], unrelated: 0 }, {
    scheduleFrame: callback => { frames.push(callback); return () => {}; },
  });
  const packets = [], renders = [[], [], [], [], []], unrelated = [];
  store.subscribe(state => state.windows, windows => packets.push([...windows]));
  renders.forEach((values, i) => store.subscribeFrame(state => state.windows[i], value => values.push(value)));
  store.subscribeFrame(state => state.unrelated, value => unrelated.push(value));
  for (let value = 1; value <= 3; value++) {
    for (let i = 0; i < 5; i++) {
      store.update(state => ({ ...state, windows: state.windows.map((old, j) => j === i ? value : old) }));
    }
  }
  assert.equal(packets.length, 16, 'all packets are consumed synchronously');
  assert.deepEqual(store.read().windows, [3, 3, 3, 3, 3]);
  assert.equal(frames.length, 1, 'one frame for the store, rather than one per view');
  assert.deepEqual(renders, [[0], [0], [0], [0], [0]]);
  frames.shift()();
  assert.deepEqual(renders, [[0, 3], [0, 3], [0, 3], [0, 3], [0, 3]]);
  assert.deepEqual(unrelated, [0]);
  store.dispose();
});

test('frame subscriptions cancel on unsubscribe/dispose and ignore obsolete callbacks', () => {
  const frames = [], cancelled = [], values = [];
  const store = createUiStateStore(0, { scheduleFrame: callback => {
    frames.push(callback);
    return () => cancelled.push(callback);
  } });
  const unsubscribe = store.subscribeFrame(state => state, value => values.push(value));
  store.update(() => 1);
  unsubscribe();
  assert.equal(cancelled.length, 1);
  const remount = store.subscribeFrame(state => state, value => values.push(value));
  store.update(() => 2);
  frames[0]();
  assert.deepEqual(values, [0, 1], 'an old cancelled callback cannot flush a new subscription');
  frames[1]();
  assert.deepEqual(values, [0, 1, 2]);
  store.update(() => 3);
  store.dispose();
  assert.equal(cancelled.length, 2);
  frames[2]();
  assert.deepEqual(values, [0, 1, 2]);
  remount();
});

test('a failed frame view does not starve siblings or reentrant updates', () => {
  const frames = [], values = [];
  const store = createUiStateStore(0, { scheduleFrame: callback => {
    frames.push(callback);
    return () => {};
  } });
  let broken = true;
  store.subscribeFrame(state => state, value => {
    if (value === 1 && broken) throw new Error('broken frame view');
  });
  store.subscribeFrame(state => state, value => {
    values.push(value);
    if (value === 1) store.update(() => 0);
  });
  store.update(() => 1);
  assert.throws(() => frames.shift()(), /broken frame view/);
  assert.deepEqual(values, [0, 1]);
  broken = false;
  assert.equal(frames.length, 1, 'returning to the old selection still needs a follow-up frame');
  frames.shift()();
  assert.deepEqual(values, [0, 1, 0]);
  store.dispose();
});

test('frame fallback runs while rAF is suspended and cancels the losing callback', () => {
  const original = Object.fromEntries(['requestAnimationFrame', 'cancelAnimationFrame', 'setTimeout', 'clearTimeout']
    .map(key => [key, globalThis[key]]));
  const frames = [], timers = [], cancelledFrames = [], cancelledTimers = [], values = [];
  globalThis.requestAnimationFrame = callback => { frames.push(callback); return frames.length; };
  globalThis.cancelAnimationFrame = id => cancelledFrames.push(id);
  globalThis.setTimeout = (callback, delay) => { assert.equal(delay, 100); timers.push(callback); return timers.length; };
  globalThis.clearTimeout = id => cancelledTimers.push(id);
  const store = createUiStateStore(0);
  try {
    store.subscribeFrame(state => state, value => values.push(value));
    store.update(() => 1);
    timers[0]();
    assert.deepEqual(values, [0, 1]);
    assert.deepEqual(cancelledFrames, [1]);
    store.update(() => 2);
    frames[0]();
    assert.deepEqual(values, [0, 1], 'late frame from the first batch is harmless');
    frames[1]();
    assert.deepEqual(values, [0, 1, 2]);
    assert.deepEqual(cancelledTimers, [1, 2]);
  } finally {
    store.dispose();
    Object.assign(globalThis, original);
  }
});
