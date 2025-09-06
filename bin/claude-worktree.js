#!/usr/bin/env bun

import { existsSync } from 'node:fs';
import { fileURLToPath, pathToFileURL } from 'node:url';
import path from 'node:path';

async function resolveEntry() {
  const binDir = path.dirname(fileURLToPath(import.meta.url));
  const distEntry = path.join(binDir, '..', 'dist', 'index.js');
  if (existsSync(distEntry)) {
    return pathToFileURL(distEntry).href;
  }
  // Fallback to TypeScript source when dist is unavailable
  const srcEntry = path.join(binDir, '..', 'src', 'index.ts');
  return pathToFileURL(srcEntry).href;
}

const entry = await resolveEntry();
const { main } = await import(entry);

main().catch(error => {
  console.error('Error:', error.message);
  process.exit(1);
});
