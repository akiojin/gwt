#!/usr/bin/env node
/**
 * Shared helpers for resolving gwt release download URLs.
 */

import { readFileSync } from 'fs';
import { dirname, join } from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = dirname(__filename);

const REPO = 'akiojin/gwt';

const PLATFORM_ARTIFACTS = [
  ['darwin-x64', 'gwt-macos-x86_64'],
  ['darwin-arm64', 'gwt-macos-aarch64'],
  ['linux-x64', 'gwt-linux-x86_64'],
  ['linux-arm64', 'gwt-linux-aarch64'],
  ['win32-x64', 'gwt-windows-x86_64.exe'],
];

function getSupportedPlatformKeys() {
  return PLATFORM_ARTIFACTS.map(([key]) => key);
}

function resolvePlatformArtifact(platform = process.platform, arch = process.arch) {
  const key = `${platform}-${arch}`;
  const entry = PLATFORM_ARTIFACTS.find(([platformKey]) => platformKey === key);
  return entry ? entry[1] : null;
}

function getPlatformArtifact() {
  return resolvePlatformArtifact();
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

function getVersionedDownloadUrl(artifact) {
  const version = getPackageVersion();
  return {
    version,
    url: buildReleaseDownloadUrl(version, artifact),
  };
}

export {
  REPO,
  buildReleaseDownloadUrl,
  getPackageVersion,
  getPlatformArtifact,
  getSupportedPlatformKeys,
  getVersionedDownloadUrl,
  resolvePlatformArtifact,
};
