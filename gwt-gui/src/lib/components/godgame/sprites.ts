/**
 * Pixel art sprite data for God Game UI.
 * Each sprite is defined as { palette, frames } where:
 *   palette: string[] of hex colors (index 0 = transparent)
 *   frames: number[][][] (array of 2D grids of palette indices)
 */

export interface SpriteData {
  palette: string[];
  frames: number[][][];
  width: number;
  height: number;
}

// --- Agent Sprites (16x16) ---

const AGENT_PALETTE_CLAUDE = [
  '', // 0 = transparent
  '#1e1e2e', // 1 = outline dark
  '#f9e2af', // 2 = robe yellow
  '#fab387', // 3 = skin
  '#cdd6f4', // 4 = eyes/detail white
  '#6c5ce7', // 5 = staff purple
  '#b4befe', // 6 = magic glow
  '#f5c2e7', // 7 = hat accent
];

// Claude wizard: yellow robe, pointed hat, staff
const CLAUDE_FRAME_IDLE: number[][] = [
  [0,0,0,0,0,0,1,1,0,0,0,0,0,0,0,0],
  [0,0,0,0,0,1,7,7,1,0,0,0,0,0,0,0],
  [0,0,0,0,1,7,7,7,7,1,0,0,0,0,0,0],
  [0,0,0,0,1,2,2,2,2,1,0,0,0,0,0,0],
  [0,0,0,0,1,3,4,4,3,1,0,0,0,0,0,0],
  [0,0,0,0,0,1,3,3,1,0,0,0,0,0,0,0],
  [0,0,0,1,1,2,2,2,2,1,1,0,0,0,0,0],
  [0,0,1,2,2,2,2,2,2,2,2,1,0,0,0,0],
  [0,0,1,2,2,2,2,2,2,2,2,1,5,0,0,0],
  [0,0,0,1,2,2,2,2,2,2,1,0,5,0,0,0],
  [0,0,0,1,2,2,2,2,2,2,1,0,5,0,0,0],
  [0,0,0,0,1,2,2,2,2,1,0,0,5,0,0,0],
  [0,0,0,0,1,2,2,2,2,1,0,6,5,6,0,0],
  [0,0,0,0,0,1,1,1,1,0,0,0,5,0,0,0],
  [0,0,0,0,0,1,0,0,1,0,0,0,5,0,0,0],
  [0,0,0,0,1,1,0,0,1,1,0,0,5,0,0,0],
];

const CLAUDE_FRAME_RUNNING: number[][] = [
  [0,0,0,0,0,0,1,1,0,0,0,0,0,0,0,0],
  [0,0,0,0,0,1,7,7,1,0,0,0,0,0,0,0],
  [0,0,0,0,1,7,7,7,7,1,0,0,0,0,0,0],
  [0,0,0,0,1,2,2,2,2,1,0,0,0,0,0,0],
  [0,0,0,0,1,3,4,4,3,1,0,0,0,0,0,0],
  [0,0,0,0,0,1,3,3,1,0,0,0,0,0,0,0],
  [0,0,0,1,1,2,2,2,2,1,1,0,0,0,0,0],
  [0,0,1,2,2,2,2,2,2,2,2,1,0,0,0,0],
  [0,5,1,2,2,2,2,2,2,2,2,1,0,0,0,0],
  [0,5,0,1,2,2,2,2,2,2,1,0,0,0,0,0],
  [0,5,0,1,2,2,2,2,2,2,1,0,0,0,0,0],
  [0,5,0,0,1,2,2,2,2,1,0,0,0,0,0,0],
  [6,5,6,0,1,2,2,2,2,1,0,0,0,0,0,0],
  [0,5,0,0,0,1,1,1,1,0,0,0,0,0,0,0],
  [0,5,0,0,1,0,0,0,0,1,0,0,0,0,0,0],
  [0,5,0,0,1,0,0,0,1,0,0,0,0,0,0,0],
];

export const CLAUDE_SPRITE: SpriteData = {
  palette: AGENT_PALETTE_CLAUDE,
  frames: [CLAUDE_FRAME_IDLE, CLAUDE_FRAME_RUNNING],
  width: 16,
  height: 16,
};

const AGENT_PALETTE_CODEX = [
  '', // 0 = transparent
  '#1e1e2e', // 1 = outline
  '#94e2d5', // 2 = body cyan
  '#74c7ec', // 3 = accent light blue
  '#cdd6f4', // 4 = eyes/visor white
  '#45475a', // 5 = dark detail
  '#a6e3a1', // 6 = antenna glow
  '#585b70', // 7 = mid gray
];

const CODEX_FRAME_IDLE: number[][] = [
  [0,0,0,0,0,0,6,6,0,0,0,0,0,0,0,0],
  [0,0,0,0,0,0,1,1,0,0,0,0,0,0,0,0],
  [0,0,0,0,1,1,1,1,1,1,0,0,0,0,0,0],
  [0,0,0,1,2,2,2,2,2,2,1,0,0,0,0,0],
  [0,0,0,1,4,4,5,5,4,4,1,0,0,0,0,0],
  [0,0,0,1,2,2,2,2,2,2,1,0,0,0,0,0],
  [0,0,0,0,1,1,1,1,1,1,0,0,0,0,0,0],
  [0,0,0,1,2,2,2,2,2,2,1,0,0,0,0,0],
  [0,0,3,1,2,2,2,2,2,2,1,3,0,0,0,0],
  [0,0,3,0,1,2,2,2,2,1,0,3,0,0,0,0],
  [0,0,0,0,1,2,2,2,2,1,0,0,0,0,0,0],
  [0,0,0,0,1,2,7,7,2,1,0,0,0,0,0,0],
  [0,0,0,0,1,2,2,2,2,1,0,0,0,0,0,0],
  [0,0,0,0,0,1,1,1,1,0,0,0,0,0,0,0],
  [0,0,0,0,0,1,0,0,1,0,0,0,0,0,0,0],
  [0,0,0,0,1,1,0,0,1,1,0,0,0,0,0,0],
];

const CODEX_FRAME_RUNNING: number[][] = [
  [0,0,0,0,0,6,6,0,0,0,0,0,0,0,0,0],
  [0,0,0,0,0,0,1,1,0,0,0,0,0,0,0,0],
  [0,0,0,0,1,1,1,1,1,1,0,0,0,0,0,0],
  [0,0,0,1,2,2,2,2,2,2,1,0,0,0,0,0],
  [0,0,0,1,4,4,5,5,4,4,1,0,0,0,0,0],
  [0,0,0,1,2,2,2,2,2,2,1,0,0,0,0,0],
  [0,0,0,0,1,1,1,1,1,1,0,0,0,0,0,0],
  [0,0,0,1,2,2,2,2,2,2,1,0,0,0,0,0],
  [0,3,0,1,2,2,2,2,2,2,1,0,3,0,0,0],
  [0,3,0,0,1,2,2,2,2,1,0,0,3,0,0,0],
  [0,0,0,0,1,2,2,2,2,1,0,0,0,0,0,0],
  [0,0,0,0,1,2,7,7,2,1,0,0,0,0,0,0],
  [0,0,0,0,1,2,2,2,2,1,0,0,0,0,0,0],
  [0,0,0,0,0,1,1,1,1,0,0,0,0,0,0,0],
  [0,0,0,0,1,0,0,0,0,1,0,0,0,0,0,0],
  [0,0,0,0,0,1,0,0,1,0,0,0,0,0,0,0],
];

export const CODEX_SPRITE: SpriteData = {
  palette: AGENT_PALETTE_CODEX,
  frames: [CODEX_FRAME_IDLE, CODEX_FRAME_RUNNING],
  width: 16,
  height: 16,
};

const AGENT_PALETTE_GEMINI = [
  '', // 0 = transparent
  '#1e1e2e', // 1 = outline
  '#cba6f7', // 2 = body magenta
  '#f5c2e7', // 3 = accent pink
  '#cdd6f4', // 4 = eyes white
  '#45475a', // 5 = dark detail
  '#f9e2af', // 6 = star glow
  '#b4befe', // 7 = twin accent
];

const GEMINI_FRAME_IDLE: number[][] = [
  [0,0,0,0,0,6,0,0,6,0,0,0,0,0,0,0],
  [0,0,0,0,6,0,0,0,0,6,0,0,0,0,0,0],
  [0,0,0,0,1,1,1,1,1,1,0,0,0,0,0,0],
  [0,0,0,1,2,3,2,2,3,2,1,0,0,0,0,0],
  [0,0,0,1,4,4,5,5,4,4,1,0,0,0,0,0],
  [0,0,0,1,2,2,3,3,2,2,1,0,0,0,0,0],
  [0,0,0,0,1,1,1,1,1,1,0,0,0,0,0,0],
  [0,0,0,1,2,2,2,2,2,2,1,0,0,0,0,0],
  [0,0,7,1,2,2,2,2,2,2,1,7,0,0,0,0],
  [0,0,7,0,1,2,2,2,2,1,0,7,0,0,0,0],
  [0,0,0,0,1,2,2,2,2,1,0,0,0,0,0,0],
  [0,0,0,0,1,3,2,2,3,1,0,0,0,0,0,0],
  [0,0,0,0,1,2,2,2,2,1,0,0,0,0,0,0],
  [0,0,0,0,0,1,1,1,1,0,0,0,0,0,0,0],
  [0,0,0,0,0,1,0,0,1,0,0,0,0,0,0,0],
  [0,0,0,0,1,1,0,0,1,1,0,0,0,0,0,0],
];

const GEMINI_FRAME_RUNNING: number[][] = [
  [0,0,0,0,6,0,0,0,0,6,0,0,0,0,0,0],
  [0,0,0,0,0,6,0,0,6,0,0,0,0,0,0,0],
  [0,0,0,0,1,1,1,1,1,1,0,0,0,0,0,0],
  [0,0,0,1,2,3,2,2,3,2,1,0,0,0,0,0],
  [0,0,0,1,4,4,5,5,4,4,1,0,0,0,0,0],
  [0,0,0,1,2,2,3,3,2,2,1,0,0,0,0,0],
  [0,0,0,0,1,1,1,1,1,1,0,0,0,0,0,0],
  [0,0,0,1,2,2,2,2,2,2,1,0,0,0,0,0],
  [0,7,0,1,2,2,2,2,2,2,1,0,7,0,0,0],
  [0,7,0,0,1,2,2,2,2,1,0,0,7,0,0,0],
  [0,0,0,0,1,2,2,2,2,1,0,0,0,0,0,0],
  [0,0,0,0,1,3,2,2,3,1,0,0,0,0,0,0],
  [0,0,0,0,1,2,2,2,2,1,0,0,0,0,0,0],
  [0,0,0,0,0,1,1,1,1,0,0,0,0,0,0,0],
  [0,0,0,0,1,0,0,0,0,1,0,0,0,0,0,0],
  [0,0,0,0,0,1,0,0,1,0,0,0,0,0,0,0],
];

export const GEMINI_SPRITE: SpriteData = {
  palette: AGENT_PALETTE_GEMINI,
  frames: [GEMINI_FRAME_IDLE, GEMINI_FRAME_RUNNING],
  width: 16,
  height: 16,
};

export function getAgentSprite(agentType: string): SpriteData {
  switch (agentType) {
    case 'claude': return CLAUDE_SPRITE;
    case 'codex': return CODEX_SPRITE;
    case 'gemini': return GEMINI_SPRITE;
    default: return CLAUDE_SPRITE;
  }
}

// --- Lead Sprite (32x32) ---

const LEAD_PALETTE = [
  '', // 0 = transparent
  '#1e1e2e', // 1 = outline
  '#b4befe', // 2 = robe lavender
  '#6c5ce7', // 3 = robe accent deep purple
  '#fab387', // 4 = skin
  '#cdd6f4', // 5 = eyes/detail white
  '#f9e2af', // 6 = staff gold
  '#a6e3a1', // 7 = magic green glow
  '#89b4fa', // 8 = cape blue
  '#cba6f7', // 9 = crown accent
];

// Simplified 32x32 lead — only key rows defined, rest filled with transparency
function makeLeadFrame(variant: 'idle' | 'thinking' | 'orchestrating'): number[][] {
  const f: number[][] = Array.from({ length: 32 }, () => new Array(32).fill(0));
  // Crown/head area (rows 2-9)
  const crownGlow = variant === 'orchestrating' ? 7 : variant === 'thinking' ? 8 : 9;
  // Crown tips
  f[2][14] = crownGlow; f[2][15] = crownGlow; f[2][16] = crownGlow; f[2][17] = crownGlow;
  f[3][13] = crownGlow; f[3][14] = 9; f[3][15] = 9; f[3][16] = 9; f[3][17] = 9; f[3][18] = crownGlow;
  f[4][13] = 1; f[4][14] = 9; f[4][15] = 9; f[4][16] = 9; f[4][17] = 9; f[4][18] = 1;
  // Head
  for (let x = 13; x <= 18; x++) { f[5][x] = 1; }
  for (let x = 12; x <= 19; x++) { f[6][x] = x === 12 || x === 19 ? 1 : 4; }
  // Eyes
  f[7][12] = 1; f[7][13] = 4; f[7][14] = 5; f[7][15] = 4; f[7][16] = 4; f[7][17] = 5; f[7][18] = 4; f[7][19] = 1;
  for (let x = 13; x <= 18; x++) { f[8][x] = 4; } f[8][12] = 1; f[8][19] = 1;
  for (let x = 13; x <= 18; x++) { f[9][x] = 1; }
  // Neck
  f[10][15] = 4; f[10][16] = 4;
  // Robe body (rows 11-24)
  for (let row = 11; row <= 24; row++) {
    const halfW = Math.min(6, 3 + Math.floor((row - 11) * 0.4));
    const cx = 15;
    for (let dx = -halfW; dx <= halfW + 1; dx++) {
      const x = cx + dx;
      if (x < 0 || x >= 32) continue;
      if (dx === -halfW || dx === halfW + 1) {
        f[row][x] = 1;
      } else if (dx === -halfW + 1 || dx === halfW) {
        f[row][x] = 3;
      } else {
        f[row][x] = 2;
      }
    }
  }
  // Cape on left
  for (let row = 12; row <= 22; row++) {
    const cx = 15;
    const halfW = Math.min(6, 3 + Math.floor((row - 11) * 0.4));
    const leftEdge = cx - halfW - 1;
    if (leftEdge >= 0) f[row][leftEdge] = 8;
    if (leftEdge - 1 >= 0 && row > 13) f[row][leftEdge - 1] = 8;
  }
  // Staff on right
  const staffX = 23;
  for (let row = 4; row <= 26; row++) { f[row][staffX] = 6; }
  // Staff orb
  f[3][staffX] = variant === 'orchestrating' ? 7 : variant === 'thinking' ? 8 : 6;
  f[2][staffX] = variant === 'orchestrating' ? 7 : 0;
  f[4][staffX] = 6;
  // Feet (rows 25-26)
  f[25][14] = 1; f[25][15] = 1; f[25][17] = 1; f[25][18] = 1;
  f[26][13] = 1; f[26][14] = 1; f[26][18] = 1; f[26][19] = 1;
  // Magic particles for orchestrating
  if (variant === 'orchestrating') {
    f[14][10] = 7; f[16][24] = 7; f[20][9] = 7; f[18][25] = 7;
    f[12][25] = 7; f[22][10] = 7;
  }
  if (variant === 'thinking') {
    f[4][22] = 8; f[3][22] = 8; f[2][21] = 8;
  }
  return f;
}

export const LEAD_SPRITE: SpriteData = {
  palette: LEAD_PALETTE,
  frames: [
    makeLeadFrame('idle'),
    makeLeadFrame('thinking'),
    makeLeadFrame('orchestrating'),
  ],
  width: 32,
  height: 32,
};

export type LeadSpriteFrame = 0 | 1 | 2; // idle=0, thinking=1, orchestrating=2

// --- Building Sprites (24x32) ---

const BUILDING_PALETTE = [
  '', // 0 = transparent
  '#1e1e2e', // 1 = outline
  '#a6e3a1', // 2 = grass green
  '#585b70', // 3 = fence/stone gray
  '#74c7ec', // 4 = blueprint blue
  '#f9e2af', // 5 = construction yellow
  '#cdd6f4', // 6 = building white
  '#f38ba8', // 7 = failed red
  '#b4befe', // 8 = window lavender
  '#fab387', // 9 = flag/accent orange
];

function makeBuildingPending(): number[][] {
  const f: number[][] = Array.from({ length: 32 }, () => new Array(24).fill(0));
  // Ground
  for (let x = 0; x < 24; x++) { f[30][x] = 2; f[31][x] = 2; }
  // Fence posts
  for (let row = 24; row <= 29; row++) { f[row][4] = 3; f[row][11] = 3; f[row][19] = 3; }
  // Fence rails
  for (let x = 4; x <= 19; x++) { f[26][x] = 3; f[28][x] = 3; }
  // Grass tufts
  f[29][7] = 2; f[29][8] = 2; f[29][14] = 2; f[29][15] = 2;
  return f;
}

function makeBuildingPlanned(): number[][] {
  const f: number[][] = Array.from({ length: 32 }, () => new Array(24).fill(0));
  // Ground
  for (let x = 0; x < 24; x++) { f[30][x] = 2; f[31][x] = 2; }
  // Blueprint wireframe building
  for (let row = 8; row <= 29; row++) { f[row][5] = 4; f[row][18] = 4; }
  for (let x = 5; x <= 18; x++) { f[8][x] = 4; f[29][x] = 4; }
  // Roof outline
  for (let x = 5; x <= 18; x++) { f[7][x] = 4; }
  f[6][7] = 4; f[6][8] = 4; f[6][15] = 4; f[6][16] = 4;
  // Window wireframes
  for (let x = 8; x <= 10; x++) { f[14][x] = 4; f[18][x] = 4; }
  f[14][8] = 4; f[18][8] = 4; f[14][10] = 4; f[18][10] = 4;
  for (let row = 14; row <= 18; row++) { f[row][8] = 4; f[row][10] = 4; }
  for (let x = 13; x <= 15; x++) { f[14][x] = 4; f[18][x] = 4; }
  for (let row = 14; row <= 18; row++) { f[row][13] = 4; f[row][15] = 4; }
  // Door wireframe
  for (let row = 22; row <= 29; row++) { f[row][10] = 4; f[row][13] = 4; }
  for (let x = 10; x <= 13; x++) { f[22][x] = 4; }
  return f;
}

function makeBuildingProgress(): number[][] {
  const f: number[][] = Array.from({ length: 32 }, () => new Array(24).fill(0));
  // Ground
  for (let x = 0; x < 24; x++) { f[30][x] = 2; f[31][x] = 2; }
  // Partial walls (bottom half solid)
  for (let row = 18; row <= 29; row++) {
    f[row][5] = 1; f[row][18] = 1;
    for (let x = 6; x <= 17; x++) { f[row][x] = 6; }
  }
  for (let x = 5; x <= 18; x++) { f[18][x] = 1; }
  // Scaffolding (top half)
  for (let row = 8; row <= 17; row++) { f[row][4] = 5; f[row][19] = 5; }
  for (let x = 4; x <= 19; x++) { f[12][x] = 5; f[8][x] = 5; }
  // Crane arm
  for (let x = 16; x <= 23; x++) { f[3][x] = 5; }
  for (let row = 3; row <= 8; row++) { f[row][19] = 5; }
  f[4][22] = 1; f[5][22] = 1; f[6][22] = 3;
  // Window openings
  f[22][8] = 8; f[22][9] = 8; f[23][8] = 8; f[23][9] = 8;
  f[22][14] = 8; f[22][15] = 8; f[23][14] = 8; f[23][15] = 8;
  return f;
}

function makeBuildingComplete(): number[][] {
  const f: number[][] = Array.from({ length: 32 }, () => new Array(24).fill(0));
  // Ground
  for (let x = 0; x < 24; x++) { f[30][x] = 2; f[31][x] = 2; }
  // Full walls
  for (let row = 8; row <= 29; row++) {
    f[row][5] = 1; f[row][18] = 1;
    for (let x = 6; x <= 17; x++) { f[row][x] = 6; }
  }
  // Roof
  for (let x = 4; x <= 19; x++) { f[7][x] = 1; f[8][x] = 1; }
  f[6][6] = 1; f[6][7] = 1; f[6][16] = 1; f[6][17] = 1;
  f[5][8] = 1; f[5][9] = 1; f[5][14] = 1; f[5][15] = 1;
  f[4][10] = 1; f[4][11] = 1; f[4][12] = 1; f[4][13] = 1;
  // Flag
  f[2][11] = 9; f[2][12] = 9; f[2][13] = 9;
  f[3][11] = 9; f[3][12] = 9;
  for (let row = 2; row <= 7; row++) { f[row][10] = 1; }
  // Windows
  for (let x = 8; x <= 10; x++) {
    for (let row = 13; row <= 16; row++) { f[row][x] = 8; }
  }
  for (let x = 13; x <= 15; x++) {
    for (let row = 13; row <= 16; row++) { f[row][x] = 8; }
  }
  // Door
  for (let row = 24; row <= 29; row++) {
    f[row][10] = 1; f[row][11] = 3; f[row][12] = 3; f[row][13] = 1;
  }
  f[24][11] = 1; f[24][12] = 1;
  return f;
}

function makeBuildingFailed(): number[][] {
  const f: number[][] = Array.from({ length: 32 }, () => new Array(24).fill(0));
  // Ground
  for (let x = 0; x < 24; x++) { f[30][x] = 2; f[31][x] = 2; }
  // Rubble pile
  for (let x = 6; x <= 17; x++) { f[29][x] = 3; }
  for (let x = 7; x <= 16; x++) { f[28][x] = 3; }
  for (let x = 8; x <= 15; x++) { f[27][x] = 7; }
  for (let x = 9; x <= 14; x++) { f[26][x] = 7; }
  f[25][10] = 3; f[25][11] = 7; f[25][12] = 3; f[25][13] = 7;
  // Broken wall fragment
  for (let row = 20; row <= 29; row++) { f[row][5] = 1; }
  f[20][6] = 6; f[21][6] = 6; f[22][6] = 6;
  f[20][7] = 6;
  // Smoke wisps
  f[22][12] = 3; f[21][13] = 3; f[20][11] = 3;
  f[19][12] = 3; f[18][11] = 3;
  return f;
}

export const BUILDING_PENDING: SpriteData = {
  palette: BUILDING_PALETTE,
  frames: [makeBuildingPending()],
  width: 24,
  height: 32,
};

export const BUILDING_PLANNED: SpriteData = {
  palette: BUILDING_PALETTE,
  frames: [makeBuildingPlanned()],
  width: 24,
  height: 32,
};

export const BUILDING_PROGRESS: SpriteData = {
  palette: BUILDING_PALETTE,
  frames: [makeBuildingProgress()],
  width: 24,
  height: 32,
};

export const BUILDING_COMPLETE: SpriteData = {
  palette: BUILDING_PALETTE,
  frames: [makeBuildingComplete()],
  width: 24,
  height: 32,
};

export const BUILDING_FAILED: SpriteData = {
  palette: BUILDING_PALETTE,
  frames: [makeBuildingFailed()],
  width: 24,
  height: 32,
};

export function getBuildingSprite(status: string): SpriteData {
  switch (status) {
    case 'pending': return BUILDING_PENDING;
    case 'planned': return BUILDING_PLANNED;
    case 'in_progress':
    case 'ci_fail': return BUILDING_PROGRESS;
    case 'completed': return BUILDING_COMPLETE;
    case 'failed': return BUILDING_FAILED;
    default: return BUILDING_PENDING;
  }
}
