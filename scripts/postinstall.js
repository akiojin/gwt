#!/usr/bin/env node
/**
 * Postinstall script to download the appropriate gwt binary
 * for the current platform from GitHub Releases.
 */

import {
  createWriteStream,
  existsSync,
  mkdirSync,
  chmodSync,
  unlinkSync,
  readFileSync,
} from 'fs';
import { dirname, join } from 'path';
import { fileURLToPath, pathToFileURL } from 'url';
import { get } from 'https';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

const REPO = 'akiojin/gwt';
const BIN_DIR = join(__dirname, '..', 'bin');
const BIN_NAME = process.platform === 'win32' ? 'gwt.exe' : 'gwt';
const BIN_PATH = join(BIN_DIR, BIN_NAME);

const RETRY_CONFIG = {
  maxAttempts: 5,
  initialDelayMs: 500,
  backoffFactor: 2,
  maxDelayMs: 5000,
};

const MAX_REDIRECTS = 5;

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

function getPackageVersion() {
  const packagePath = join(__dirname, '..', 'package.json');
  const pkg = JSON.parse(readFileSync(packagePath, 'utf8'));
  if (!pkg.version) {
    throw new Error('package.json does not contain a version field');
  }
  return pkg.version;
}

function buildReleaseDownloadUrl(version, artifact) {
  return `https://github.com/${REPO}/releases/download/v${version}/${artifact}`;
}

function getRetryDelayMs(attempt, config = RETRY_CONFIG) {
  const delay = config.initialDelayMs * Math.pow(config.backoffFactor, Math.max(0, attempt - 1));
  return Math.min(delay, config.maxDelayMs);
}

function isRetryableStatus(statusCode) {
  return statusCode === 403 || statusCode === 404 || statusCode >= 500;
}

function isRetryableError(error) {
  if (error && typeof error.statusCode === 'number') {
    return isRetryableStatus(error.statusCode);
  }
  return true;
}

function formatFailureGuidance(version, artifact) {
  const versionedUrl = version && version !== 'unknown'
    ? buildReleaseDownloadUrl(version, artifact)
    : null;
  return {
    versionedUrl,
    releasePageUrl: `https://github.com/${REPO}/releases`,
    buildCommand: 'cargo build --release',
  };
}

async function getReleaseAssetUrl(version, artifact, httpGet = get) {
  return new Promise((resolve) => {
    const options = {
      hostname: 'api.github.com',
      path: `/repos/${REPO}/releases/tags/v${version}`,
      headers: {
        'User-Agent': 'gwt-postinstall',
        'Accept': 'application/vnd.github.v3+json',
      },
    };

    const req = httpGet(options, (res) => {
      let data = '';
      res.on('data', (chunk) => data += chunk);
      res.on('end', () => {
        if (res.statusCode !== 200) {
          resolve(null);
          return;
        }
        try {
          const release = JSON.parse(data);
          const asset = release.assets?.find((entry) => entry.name === artifact);
          resolve(asset?.browser_download_url ?? null);
        } catch {
          resolve(null);
        }
      });
    });

    req.on('error', () => resolve(null));
  });
}

async function getDownloadUrl(version, artifact) {
  const assetUrl = await getReleaseAssetUrl(version, artifact);
  return assetUrl ?? buildReleaseDownloadUrl(version, artifact);
}

async function downloadFile(url, dest, httpGet = get) {
  return new Promise((resolve, reject) => {
    const request = (currentUrl, redirectCount) => {
      const req = httpGet(currentUrl, (res) => {
        const statusCode = res.statusCode ?? 0;

        if ([301, 302, 303, 307, 308].includes(statusCode)) {
          const location = res.headers.location;
          res.resume();
          if (!location) {
            const error = new Error(`Redirect without location (HTTP ${statusCode})`);
            error.statusCode = statusCode;
            reject(error);
            return;
          }
          if (redirectCount >= MAX_REDIRECTS) {
            const error = new Error('Too many redirects');
            error.statusCode = statusCode;
            reject(error);
            return;
          }
          const resolved = new URL(location, currentUrl).toString();
          request(resolved, redirectCount + 1);
          return;
        }

        if (statusCode !== 200) {
          res.resume();
          const error = new Error(`Failed to download: HTTP ${statusCode}`);
          error.statusCode = statusCode;
          reject(error);
          return;
        }

        const file = createWriteStream(dest);
        res.pipe(file);

        const cleanup = (err) => {
          file.close(() => {
            if (existsSync(dest)) unlinkSync(dest);
            reject(err);
          });
        };

        file.on('finish', () => file.close(resolve));
        file.on('error', cleanup);
        res.on('error', cleanup);
      });

      req.on('error', (err) => {
        if (existsSync(dest)) unlinkSync(dest);
        reject(err);
      });
    };

    request(url, 0);
  });
}

async function downloadWithRetry(url, dest, config = RETRY_CONFIG) {
  let attempt = 0;

  while (attempt < config.maxAttempts) {
    attempt += 1;
    try {
      await downloadFile(url, dest);
      return;
    } catch (error) {
      if (!isRetryableError(error) || attempt >= config.maxAttempts) {
        throw error;
      }
      const delayMs = getRetryDelayMs(attempt, config);
      console.log(`Retrying download in ${delayMs}ms (attempt ${attempt}/${config.maxAttempts})...`);
      await new Promise((resolve) => setTimeout(resolve, delayMs));
    }
  }
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
  let version = 'unknown';
  console.log(`Downloading gwt binary for ${process.platform}-${process.arch}...`);

  try {
    version = getPackageVersion();
    const url = await getDownloadUrl(version, artifact);
    console.log(`Downloading from: ${url}`);

    if (!existsSync(BIN_DIR)) {
      mkdirSync(BIN_DIR, { recursive: true });
    }

    await downloadWithRetry(url, BIN_PATH);

    // Make executable on Unix
    if (process.platform !== 'win32') {
      chmodSync(BIN_PATH, 0o755);
    }

    console.log('gwt binary installed successfully!');
  } catch (error) {
    const guidance = formatFailureGuidance(version, artifact);
    console.error('Failed to download gwt binary:', error.message);
    console.error('');
    console.error('You can manually download the binary from:');
    if (guidance.versionedUrl) {
      console.error(guidance.versionedUrl);
    }
    console.error(guidance.releasePageUrl);
    console.error('');
    console.error(`Or build from source with: ${guidance.buildCommand}`);
    process.exit(1);
  }
}

const isDirectRun = process.argv[1]
  && pathToFileURL(process.argv[1]).href === import.meta.url;

if (isDirectRun) {
  main();
}

export {
  RETRY_CONFIG,
  buildReleaseDownloadUrl,
  formatFailureGuidance,
  getRetryDelayMs,
  getReleaseAssetUrl,
  getDownloadUrl,
  getPackageVersion,
  getPlatformArtifact,
  isRetryableError,
  isRetryableStatus,
};
