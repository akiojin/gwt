#!/usr/bin/env node
/**
 * Wrapper script to execute the gwt Rust binary.
 * This allows npm/bunx distribution of the Rust CLI.
 *
 * If the binary is not found (e.g., bunx skips postinstall),
 * it will be downloaded on-demand from GitHub Releases.
 */

import { spawn } from 'child_process';
import { dirname, join } from 'path';
import { fileURLToPath } from 'url';
import { existsSync, createWriteStream, mkdirSync, chmodSync, unlinkSync } from 'fs';
import { get } from 'https';
import {
  REPO,
  getPlatformArtifact,
  getSupportedPlatformKeys,
  getVersionedDownloadUrl,
} from '../scripts/release-download.js';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

const BIN_NAME = process.platform === 'win32' ? 'gwt.exe' : 'gwt';
const BIN_PATH = join(__dirname, BIN_NAME);

async function downloadFile(url, dest) {
  return new Promise((resolve, reject) => {
    const file = createWriteStream(dest);

    const request = (url) => {
      get(url, (res) => {
        if (res.statusCode === 301 || res.statusCode === 302) {
          request(res.headers.location);
          return;
        }

        if (res.statusCode !== 200) {
          reject(new Error(`Failed to download: HTTP ${res.statusCode}`));
          return;
        }

        res.pipe(file);
        file.on('finish', () => {
          file.close();
          resolve();
        });
      }).on('error', (err) => {
        if (existsSync(dest)) unlinkSync(dest);
        reject(err);
      });
    };

    request(url);
  });
}

function requirePlatformArtifact() {
  const artifact = getPlatformArtifact();
  if (!artifact) {
    console.error(`Unsupported platform: ${process.platform}-${process.arch}`);
    console.error(`Supported platforms: ${getSupportedPlatformKeys().join(', ')}`);
    process.exit(1);
  }
  return artifact;
}

async function downloadBinary(url) {
  if (!existsSync(__dirname)) {
    mkdirSync(__dirname, { recursive: true });
  }

  await downloadFile(url, BIN_PATH);

  if (process.platform !== 'win32') {
    chmodSync(BIN_PATH, 0o755);
  }

  console.log('gwt binary installed successfully!');
  console.log('');
}

function runBinary() {
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
}

async function main() {
  if (!existsSync(BIN_PATH)) {
    const artifact = requirePlatformArtifact();
    let versionedUrl = null;
    try {
      const resolved = getVersionedDownloadUrl(artifact);
      versionedUrl = resolved.url;
      console.log(`Downloading gwt binary for ${process.platform}-${process.arch}...`);
      console.log(`Downloading from: ${versionedUrl}`);
      await downloadBinary(versionedUrl);
    } catch (error) {
      console.error('Failed to download gwt binary:', error.message);
      console.error('');
      console.error('You can manually download the binary from:');
      if (versionedUrl) {
        console.error(versionedUrl);
      }
      console.error(`https://github.com/${REPO}/releases`);
      console.error('');
      console.error('Or build from source with: cargo build --release');
      process.exit(1);
    }
  }

  runBinary();
}

main();
