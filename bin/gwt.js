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

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

const REPO = 'akiojin/gwt';
const BIN_NAME = process.platform === 'win32' ? 'gwt.exe' : 'gwt';
const BIN_PATH = join(__dirname, BIN_NAME);

function getPlatformArtifact() {
  const platform = process.platform;
  const arch = process.arch;

  const mapping = {
    'darwin-x64': 'gwt-macos-x86_64',
    'darwin-arm64': 'gwt-macos-aarch64',
    'linux-x64': 'gwt-linux-x86_64',
    'linux-arm64': 'gwt-linux-aarch64',
    'win32-x64': 'gwt-windows-x86_64.exe',
  };

  const key = `${platform}-${arch}`;
  return mapping[key];
}

async function getLatestReleaseUrl(artifact) {
  return new Promise((resolve, reject) => {
    const options = {
      hostname: 'api.github.com',
      path: `/repos/${REPO}/releases/latest`,
      headers: {
        'User-Agent': 'gwt-wrapper',
        'Accept': 'application/vnd.github.v3+json',
      },
    };

    get(options, (res) => {
      let data = '';
      res.on('data', (chunk) => data += chunk);
      res.on('end', () => {
        try {
          const release = JSON.parse(data);
          const asset = release.assets?.find(a => a.name === artifact);
          if (asset) {
            resolve(asset.browser_download_url);
          } else {
            resolve(`https://github.com/${REPO}/releases/latest/download/${artifact}`);
          }
        } catch {
          resolve(`https://github.com/${REPO}/releases/latest/download/${artifact}`);
        }
      });
    }).on('error', reject);
  });
}

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

async function downloadBinary() {
  const artifact = getPlatformArtifact();

  if (!artifact) {
    console.error(`Unsupported platform: ${process.platform}-${process.arch}`);
    console.error('Supported platforms: darwin-x64, darwin-arm64, linux-x64, linux-arm64, win32-x64');
    process.exit(1);
  }

  console.log(`Downloading gwt binary for ${process.platform}-${process.arch}...`);

  const url = await getLatestReleaseUrl(artifact);
  console.log(`Downloading from: ${url}`);

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
    try {
      await downloadBinary();
    } catch (error) {
      console.error('Failed to download gwt binary:', error.message);
      console.error('');
      console.error('You can manually download the binary from:');
      console.error(`https://github.com/${REPO}/releases`);
      console.error('');
      console.error('Or build from source with: cargo build --release');
      process.exit(1);
    }
  }

  runBinary();
}

main();
