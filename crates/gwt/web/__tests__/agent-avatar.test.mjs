/* Issue #7635 (SPEC #7007 T-017) — the one avatar part every Agent display
 * uses. A member without an avatar, or with one this build cannot draw,
 * shows its initial so the display never breaks (AC-1 / AC-4).
 */
import { test } from "node:test";
import assert from "node:assert/strict";

import { AVATAR_PRESETS, agentAvatar } from "../agent-avatar.js";

test("presets match the ids the ledger accepts", () => {
  assert.deepEqual(AVATAR_PRESETS.map(preset => preset.id), ["cat", "owl", "bear", "fox", "penguin", "robot"]);
  assert.ok(AVATAR_PRESETS.every(preset => preset.glyph && preset.label));
});

test("a preset draws its glyph and anything else draws the initial", () => {
  assert.deepEqual(agentAvatar({ name: "Birch", avatar: { kind: "preset", preset_id: "cat" } }),
    { kind: "preset", presetId: "cat", glyph: "🐱", label: "Cat" });
  for (const member of [{ name: "Birch" }, { name: "Birch", avatar: null }, { name: "Birch", avatar: { kind: "default" } },
    { name: "Birch", avatar: { kind: "preset", preset_id: "unicorn" } }, { name: "Birch", avatar: { kind: "upload" } }]) {
    assert.deepEqual(agentAvatar(member), { kind: "default", presetId: null, glyph: "B", label: "Initial" });
  }
  assert.equal(agentAvatar({ name: "" }).glyph, "?");
  assert.equal(agentAvatar(null).glyph, "?");
  assert.equal(agentAvatar({ name: "小田 洋平" }).glyph, "小");
});

/* Issue #7636 (T-018): an uploaded image is drawn in place of the glyph, and
 * a picked file is checked, cropped to its centre square and scaled before it
 * is saved. A refused file yields a message and no avatar (AC-2).
 */
import { AVATAR_IMAGE_SIZE, AVATAR_UPLOAD_MAX_BYTES, prepareAvatarUpload } from "../agent-avatar.js";

const PNG = "data:image/png;base64,iVBORw0KGgo=";

test("an uploaded image is drawn, and a malformed one falls back to the initial", () => {
  assert.deepEqual(agentAvatar({ name: "Birch", avatar: { kind: "upload", image: PNG } }),
    { kind: "upload", presetId: null, glyph: "B", imageUrl: PNG, label: "Uploaded image" });
  for (const image of [undefined, "", "https://example.com/a.png", "data:image/jpeg;base64,AAAA"]) {
    assert.equal(agentAvatar({ name: "Birch", avatar: { kind: "upload", image } }).kind, "default");
  }
});

function canvasRecorder(dataUrl) {
  const calls = [];
  const canvas = { getContext: () => ({ drawImage: (...args) => calls.push(args) }), toDataURL: type => { calls.push(["toDataURL", type]); return dataUrl; } };
  return { calls, canvas, createCanvas: () => canvas };
}

test("a picked image is cropped to its centre square and scaled to a small PNG", async () => {
  const recorder = canvasRecorder(PNG);
  const bitmap = { width: 300, height: 200, close() {} };
  const result = await prepareAvatarUpload({ type: "image/jpeg", size: 1024 }, { decode: async () => bitmap, createCanvas: recorder.createCanvas });
  assert.deepEqual(result, { avatar: { kind: "upload", image: PNG } });
  assert.equal(recorder.canvas.width, AVATAR_IMAGE_SIZE);
  assert.equal(recorder.canvas.height, AVATAR_IMAGE_SIZE);
  assert.deepEqual(recorder.calls[0], [bitmap, 50, 0, 200, 200, 0, 0, AVATAR_IMAGE_SIZE, AVATAR_IMAGE_SIZE]);
  assert.deepEqual(recorder.calls[1], ["toDataURL", "image/png"]);
});

test("an unsupported, oversized, unreadable or too detailed image is refused with a message", async () => {
  const decode = async () => ({ width: 10, height: 10 });
  const ok = canvasRecorder(PNG).createCanvas;
  for (const [file, options] of [
    [{ type: "image/svg+xml", size: 10 }, { decode, createCanvas: ok }],
    [{ type: "text/plain", size: 10 }, { decode, createCanvas: ok }],
    [null, { decode, createCanvas: ok }],
    [{ type: "image/png", size: AVATAR_UPLOAD_MAX_BYTES + 1 }, { decode, createCanvas: ok }],
    [{ type: "image/png", size: 10 }, { decode: async () => { throw new Error("corrupt"); }, createCanvas: ok }],
    [{ type: "image/png", size: 10 }, { decode, createCanvas: canvasRecorder(`data:image/png;base64,${"A".repeat(100_000)}`).createCanvas }],
  ]) {
    const result = await prepareAvatarUpload(file, options);
    assert.equal(result.avatar, undefined);
    assert.match(result.error, /\S/);
  }
});
