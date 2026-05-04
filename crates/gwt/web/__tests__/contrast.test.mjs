import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, resolve } from "node:path";

const here = dirname(fileURLToPath(import.meta.url));
const tokensPath = resolve(here, "../styles/tokens.css");

const tokensCss = readFileSync(tokensPath, "utf8");
const dark = extractTokens(tokensCss, "dark");
const light = extractTokens(tokensCss, "light");

const NORMAL_AA = 4.5;
const LARGE_AA = 3.0;

const REQUIRED_PAIRS = [
  ["--color-text", "--color-canvas", NORMAL_AA, "body text on canvas"],
  ["--color-text", "--color-surface", NORMAL_AA, "body text on surface"],
  ["--color-text-muted", "--color-canvas", NORMAL_AA, "muted text on canvas"],
  ["--color-text-strong", "--color-canvas", NORMAL_AA, "strong text on canvas"],
  ["--color-display-fg", "--color-canvas", LARGE_AA, "display text on canvas"],
  ["--color-status-strip-fg", "--color-status-strip-bg", NORMAL_AA, "status strip text"],
  ["--color-button-fg", "--color-button-bg", NORMAL_AA, "primary button label"],
  ["--color-link", "--color-canvas", NORMAL_AA, "link on canvas"],
  ["--agent-claude", "--color-canvas", LARGE_AA, "agent claude indicator"],
  ["--agent-codex", "--color-canvas", LARGE_AA, "agent codex indicator"],
  ["--agent-gemini", "--color-canvas", LARGE_AA, "agent gemini indicator"],
  ["--agent-opencode", "--color-canvas", LARGE_AA, "agent opencode indicator"],
  ["--agent-copilot", "--color-canvas", LARGE_AA, "agent copilot indicator"],
];

for (const themeName of ["dark", "light"]) {
  const theme = themeName === "dark" ? dark : light;
  for (const [fgName, bgName, threshold, label] of REQUIRED_PAIRS) {
    test(`[${themeName}] WCAG AA >= ${threshold}: ${label} (${fgName} on ${bgName})`, () => {
      const fg = theme[fgName];
      const bg = theme[bgName];
      assert.ok(fg, `token ${fgName} missing in ${themeName}`);
      assert.ok(bg, `token ${bgName} missing in ${themeName}`);
      const ratio = contrastRatio(fg, bg);
      assert.ok(
        ratio >= threshold,
        `${label}: contrast ${ratio.toFixed(2)} < ${threshold} (fg=${fg} on bg=${bg})`,
      );
    });
  }
}

test("dark and light token sets define identical semantic keys", () => {
  const darkKeys = Object.keys(dark).sort();
  const lightKeys = Object.keys(light).sort();
  assert.deepEqual(
    darkKeys,
    lightKeys,
    "dark and light token sets must define the same semantic keys",
  );
});

test("token names are kebab-case CSS custom properties", () => {
  const all = new Set([...Object.keys(dark), ...Object.keys(light)]);
  for (const name of all) {
    assert.match(name, /^--[a-z][a-z0-9-]*$/, `invalid token name: ${name}`);
  }
});

function extractTokens(css, themeName) {
  const blockRegex = themeName === "dark"
    ? /:root\[data-theme="dark"\]\s*\{([\s\S]*?)\}/
    : /:root\[data-theme="light"\]\s*\{([\s\S]*?)\}/;
  const match = css.match(blockRegex);
  assert.ok(match, `tokens.css must contain a :root[data-theme="${themeName}"] block`);
  const tokens = {};
  for (const line of match[1].split("\n")) {
    const m = line.match(/^\s*(--[a-z][a-z0-9-]*)\s*:\s*([^;]+);/);
    if (!m) continue;
    tokens[m[1]] = m[2].trim();
  }
  return tokens;
}

function parseColor(value) {
  const v = value.trim();
  let m;
  if ((m = v.match(/^#([0-9a-fA-F]{6})$/))) {
    const n = parseInt(m[1], 16);
    return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
  }
  if ((m = v.match(/^#([0-9a-fA-F]{3})$/))) {
    const r = parseInt(m[1][0] + m[1][0], 16);
    const g = parseInt(m[1][1] + m[1][1], 16);
    const b = parseInt(m[1][2] + m[1][2], 16);
    return [r, g, b];
  }
  if ((m = v.match(/^rgb\(\s*(\d+)\s*[\s,]\s*(\d+)\s*[\s,]\s*(\d+)\s*\)$/))) {
    return [Number(m[1]), Number(m[2]), Number(m[3])];
  }
  if ((m = v.match(/^hsl\(\s*([\d.]+)(?:deg)?\s*[\s,]\s*([\d.]+)%\s*[\s,]\s*([\d.]+)%\s*\)$/))) {
    return hslToRgb(Number(m[1]), Number(m[2]), Number(m[3]));
  }
  if ((m = v.match(/^oklch\(\s*([\d.]+%?)\s+([\d.]+)\s+([\d.]+)\s*\)$/))) {
    const l = m[1].endsWith("%") ? Number(m[1].slice(0, -1)) / 100 : Number(m[1]);
    return oklchToRgb(l, Number(m[2]), Number(m[3]));
  }
  throw new Error(`unsupported color literal: ${value}`);
}

function hslToRgb(h, s, l) {
  s /= 100; l /= 100;
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const hp = h / 60;
  const x = c * (1 - Math.abs((hp % 2) - 1));
  let r1 = 0, g1 = 0, b1 = 0;
  if (hp < 1) { r1 = c; g1 = x; }
  else if (hp < 2) { r1 = x; g1 = c; }
  else if (hp < 3) { g1 = c; b1 = x; }
  else if (hp < 4) { g1 = x; b1 = c; }
  else if (hp < 5) { r1 = x; b1 = c; }
  else { r1 = c; b1 = x; }
  const k = l - c / 2;
  return [Math.round((r1 + k) * 255), Math.round((g1 + k) * 255), Math.round((b1 + k) * 255)];
}

function oklchToRgb(L, C, h) {
  const a = C * Math.cos((h * Math.PI) / 180);
  const b = C * Math.sin((h * Math.PI) / 180);
  const l_ = L + 0.3963377774 * a + 0.2158037573 * b;
  const m_ = L - 0.1055613458 * a - 0.0638541728 * b;
  const s_ = L - 0.0894841775 * a - 1.2914855480 * b;
  const l = l_ ** 3;
  const m = m_ ** 3;
  const s = s_ ** 3;
  const lr = +4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s;
  const lg = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s;
  const lb = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s;
  const toSrgb = (x) => {
    const xc = Math.max(0, Math.min(1, x));
    return xc <= 0.0031308 ? 12.92 * xc : 1.055 * xc ** (1 / 2.4) - 0.055;
  };
  return [Math.round(toSrgb(lr) * 255), Math.round(toSrgb(lg) * 255), Math.round(toSrgb(lb) * 255)];
}

function relativeLuminance([r, g, b]) {
  const lin = (c) => {
    const cs = c / 255;
    return cs <= 0.03928 ? cs / 12.92 : ((cs + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b);
}

function contrastRatio(fg, bg) {
  const L1 = relativeLuminance(parseColor(fg));
  const L2 = relativeLuminance(parseColor(bg));
  const a = Math.max(L1, L2);
  const b = Math.min(L1, L2);
  return (a + 0.05) / (b + 0.05);
}
