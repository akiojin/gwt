#!/usr/bin/env node
/**
 * Wrapper script to execute the gwt Rust binary.
 * This allows npm/bunx distribution of the Rust CLI.
 */

import { spawn } from 'child_process';
import { dirname, join } from 'path';
import { fileURLToPath } from 'url';
import { existsSync } from 'fs';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

const BIN_NAME = process.platform === 'win32' ? 'gwt.exe' : 'gwt';
const BIN_PATH = join(__dirname, BIN_NAME);

if (!existsSync(BIN_PATH)) {
  console.error('Error: gwt binary not found.');
  console.error('');
  console.error('The binary may not have been downloaded during installation.');
  console.error('Please try reinstalling: npm install -g @akiojin/gwt');
  console.error('');
  console.error('Or download manually from:');
  console.error('https://github.com/akiojin/gwt/releases');
  process.exit(1);
}

// Forward all arguments to the native binary
const child = spawn(BIN_PATH, process.argv.slice(2), {
  stdio: 'inherit',
  env: process.env,
});

child.on('error', (err) => {
  console.error('Failed to start gwt:', err.message);
  process.exit(1);
});

child.on('exit', (code, signal) => {
  if (signal) {
    process.kill(process.pid, signal);
  } else {
    process.exit(code ?? 0);
  }
});
