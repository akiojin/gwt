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

// Issue #7636 (T-018): an uploaded image is cropped to its centre square and
// scaled to AVATAR_IMAGE_SIZE, then kept as a PNG data URL in this machine's
// ledger. It never leaves the machine. The ledger refuses a PNG larger than
// `AVATAR_IMAGE_MAX_BYTES` in crates/gwt/src/agent_names.rs.
const AVATAR_IMAGE_PREFIX = "data:image/png;base64,";
export const AVATAR_IMAGE_SIZE = 128;
export const AVATAR_UPLOAD_TYPES = Object.freeze(["image/png", "image/jpeg", "image/webp", "image/gif"]);
export const AVATAR_UPLOAD_MAX_BYTES = 5 * 1024 * 1024;
const AVATAR_IMAGE_MAX_BYTES = 64 * 1024;

export function agentAvatar(member) {
  const avatar = member?.avatar;
  const initial = Array.from(String(member?.name || "").trim() || "?")[0];
  if (avatar?.kind === "upload" && typeof avatar.image === "string" && avatar.image.startsWith(AVATAR_IMAGE_PREFIX)) {
    return { kind: "upload", presetId: null, glyph: initial, imageUrl: avatar.image, label: "Uploaded image" };
  }
  const preset = avatar?.kind === "preset" ? AVATAR_PRESETS.find(item => item.id === avatar.preset_id) : null;
  if (preset) return { kind: "preset", presetId: preset.id, glyph: preset.glyph, label: preset.label };
  return { kind: "default", presetId: null, glyph: initial, label: "Initial" };
}

// Turn a picked file into the avatar to save, or a message saying why not.
// The caller supplies the canvas; `decode` defaults to the page's own image
// decoder. Both are injected so the rules are testable without a browser.
export async function prepareAvatarUpload(file, { createCanvas, decode = blob => globalThis.createImageBitmap(blob) }) {
  if (!file || !AVATAR_UPLOAD_TYPES.includes(file.type)) {
    return { error: "Choose a PNG, JPEG, WebP or GIF image." };
  }
  if (file.size > AVATAR_UPLOAD_MAX_BYTES) {
    return { error: `The image is larger than ${AVATAR_UPLOAD_MAX_BYTES / 1024 / 1024} MB.` };
  }
  let bitmap;
  try { bitmap = await decode(file); } catch { return { error: "The image could not be read." }; }
  const side = Math.min(bitmap.width, bitmap.height);
  if (!(side > 0)) return { error: "The image could not be read." };
  const canvas = createCanvas();
  canvas.width = AVATAR_IMAGE_SIZE; canvas.height = AVATAR_IMAGE_SIZE;
  canvas.getContext("2d").drawImage(bitmap, (bitmap.width - side) / 2, (bitmap.height - side) / 2, side, side, 0, 0, AVATAR_IMAGE_SIZE, AVATAR_IMAGE_SIZE);
  bitmap.close?.();
  const image = canvas.toDataURL("image/png");
  const bytes = Math.floor((image.length - AVATAR_IMAGE_PREFIX.length) * 3 / 4);
  if (!image.startsWith(AVATAR_IMAGE_PREFIX) || bytes > AVATAR_IMAGE_MAX_BYTES) {
    return { error: "The image is too detailed to keep as an avatar. Try a simpler image." };
  }
  return { avatar: { kind: "upload", image } };
}
