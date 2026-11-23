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
