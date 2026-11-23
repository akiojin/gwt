// Issue #7635 (SPEC #7007 T-017): the one avatar part every Agent display
// uses. The avatar belongs to the agent (design item 18), so it is read from
// the ledger member, never from the tool a session runs on. A member with no
// avatar, or one this build cannot draw, shows its initial (item 20).

// The ids must match `AVATAR_PRESET_IDS` in crates/gwt/src/agent_names.rs.
export const AVATAR_PRESETS = Object.freeze([
  { id: "cat", glyph: "🐱", label: "Cat" },
  { id: "owl", glyph: "🦉", label: "Owl" },
  { id: "bear", glyph: "🐻", label: "Bear" },
  { id: "fox", glyph: "🦊", label: "Fox" },
  { id: "penguin", glyph: "🐧", label: "Penguin" },
  { id: "robot", glyph: "🤖", label: "Robot" },
].map(preset => Object.freeze(preset)));

export function agentAvatar(member) {
  const avatar = member?.avatar;
  const preset = avatar?.kind === "preset" ? AVATAR_PRESETS.find(item => item.id === avatar.preset_id) : null;
  if (preset) return { kind: "preset", presetId: preset.id, glyph: preset.glyph, label: preset.label };
  return { kind: "default", presetId: null, glyph: Array.from(String(member?.name || "").trim() || "?")[0], label: "Initial" };
}

