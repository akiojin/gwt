#!/usr/bin/env node
/**
 * Postinstall script to download the appropriate gwt binary
 * for the current platform from GitHub Releases.
 */

import { createWriteStream, existsSync, mkdirSync, chmodSync, unlinkSync } from 'fs';
import { dirname, join } from 'path';
import { fileURLToPath } from 'url';
import { get } from 'https';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

const REPO = 'akiojin/gwt';
const BIN_DIR = join(__dirname, '..', 'bin');
const BIN_NAME = process.platform === 'win32' ? 'gwt.exe' : 'gwt';
const BIN_PATH = join(BIN_DIR, BIN_NAME);

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
  const artifact = mapping[key];

  if (!artifact) {
    console.error(`Unsupported platform: ${platform}-${arch}`);
    console.error('Supported platforms: darwin-x64, darwin-arm64, linux-x64, linux-arm64, win32-x64');
    process.exit(1);
  }

  return artifact;
}

async function getLatestReleaseUrl(artifact) {
  return new Promise((resolve, reject) => {
    const options = {
      hostname: 'api.github.com',
      path: `/repos/${REPO}/releases/latest`,
      headers: {
        'User-Agent': 'gwt-postinstall',
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
            // Fallback to direct URL pattern
            resolve(`https://github.com/${REPO}/releases/latest/download/${artifact}`);
          }
        } catch (e) {
          // Fallback to direct URL pattern
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
        // Handle redirects
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

async function main() {
  // Skip in CI environments where binary might be built differently
  if (process.env.CI || process.env.GITHUB_ACTIONS) {
    console.log('Skipping binary download in CI environment');
    return;
  }

  // Skip if binary already exists (e.g., local development)
  if (existsSync(BIN_PATH)) {
    console.log('Binary already exists, skipping download');
    return;
  }

  const artifact = getPlatformArtifact();
  console.log(`Downloading gwt binary for ${process.platform}-${process.arch}...`);

  try {
    const url = await getLatestReleaseUrl(artifact);
    console.log(`Downloading from: ${url}`);

    if (!existsSync(BIN_DIR)) {
      mkdirSync(BIN_DIR, { recursive: true });
    }

    await downloadFile(url, BIN_PATH);

    // Make executable on Unix
    if (process.platform !== 'win32') {
      chmodSync(BIN_PATH, 0o755);
    }

    console.log('gwt binary installed successfully!');
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

main();
