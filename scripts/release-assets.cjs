const fs = require("fs");
const https = require("https");
const os = require("os");
const path = require("path");
const { execFileSync } = require("child_process");

const RELEASE_CONTRACT_PATH = path.join(__dirname, "..", "assets", "release-assets.json");
let releaseContractCache = null;

function readReleaseContract() {
  if (!releaseContractCache) {
    releaseContractCache = JSON.parse(fs.readFileSync(RELEASE_CONTRACT_PATH, "utf8"));
  }
  return releaseContractCache;
}

function normalizeContractPlatform(platform = os.platform()) {
  if (platform === "darwin") {
    return "macos";
  }
  if (platform === "win32") {
    return "windows";
  }
  return platform;
}

function normalizeContractArch(arch = os.arch()) {
  if (arch === "arm64") {
    return "aarch64";
  }
  if (arch === "x64") {
    return "x86_64";
  }
  return arch;
}

function binaryNameForPlatform(platform = os.platform()) {
  return bundleBinaryNamesForPlatform(platform)[0];
}

function daemonBinaryNameForPlatform(platform = os.platform()) {
  return bundleBinaryNamesForPlatform(platform)[1];
}

function bundleBinaryNamesForPlatform(platform = os.platform()) {
  const binaries = readReleaseContract().bundle_binaries[normalizeContractPlatform(platform)];
  if (!Array.isArray(binaries) || binaries.length === 0) {
    throw new Error(`Unsupported platform: ${platform}`);
  }
  return binaries;
}

function releaseAssetName(platform = os.platform(), arch = os.arch()) {
  const key = `${normalizeContractPlatform(platform)}-${normalizeContractArch(arch)}`;
  const asset = readReleaseContract().portable_assets[key];
  if (!asset) {
    throw new Error(`Unsupported platform: ${platform}-${arch}`);
  }
  return asset;
}

function releaseAssetUrl(repo, version, platform = os.platform(), arch = os.arch()) {
  const asset = releaseAssetName(platform, arch);
  const tag = `v${version}`;
  return {
    asset,
    tag,
    url: `https://github.com/${repo}/releases/download/${tag}/${asset}`,
  };
}

function download(url, dest) {
  return new Promise((resolve, reject) => {
    const file = fs.createWriteStream(dest);
    const request = (nextUrl) => {
      https
        .get(nextUrl, { headers: { "User-Agent": "gwt-postinstall" } }, (res) => {
          if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
            request(res.headers.location);
            return;
          }
          if (res.statusCode !== 200) {
            file.close();
            fs.rmSync(dest, { force: true });
            reject(new Error(`Download failed: HTTP ${res.statusCode} for ${nextUrl}`));
            return;
          }
          res.pipe(file);
          file.on("finish", () => {
            file.close(resolve);
          });
        })
        .on("error", (err) => {
          file.close();
          fs.rmSync(dest, { force: true });
          reject(err);
        });
    };
    request(url);
  });
}

function findFileRecursive(root, fileName) {
  const entries = fs.readdirSync(root, { withFileTypes: true });
  for (const entry of entries) {
    const fullPath = path.join(root, entry.name);
    if (entry.isFile() && entry.name === fileName) {
      return fullPath;
    }
    if (entry.isDirectory()) {
      const nested = findFileRecursive(fullPath, fileName);
      if (nested) {
        return nested;
      }
    }
  }
  return null;
}

function extractArchive(archivePath, extractDir, asset) {
  if (asset.endsWith(".zip")) {
    execFileSync(
      "powershell.exe",
      [
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        `Expand-Archive -LiteralPath '${archivePath.replace(/'/g, "''")}' -DestinationPath '${extractDir.replace(/'/g, "''")}' -Force`,
      ]
    );
    return;
  }

  if (asset.endsWith(".tar.gz")) {
    execFileSync("tar", ["-xzf", archivePath, "-C", extractDir]);
    return;
  }

  throw new Error(`Unsupported archive type: ${asset}`);
}

function cleanupTempDir(tempRoot) {
  if (os.platform() === "win32") {
    return;
  }

  try {
    fs.rmSync(tempRoot, { recursive: true, force: true });
  } catch {
    // Best-effort cleanup only.
  }
}

function installBundleFromArchive({
  archivePath,
  asset,
  binDir,
  platform = os.platform(),
  binaryNames = bundleBinaryNamesForPlatform(platform),
}) {
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "gwt-extract-"));
  const extractDir = path.join(tempRoot, "extract");

  fs.mkdirSync(binDir, { recursive: true });
  fs.mkdirSync(extractDir, { recursive: true });

  try {
    extractArchive(archivePath, extractDir, asset);

    const destinations = {};
    for (const binaryName of binaryNames) {
      const extractedBinary = findFileRecursive(extractDir, binaryName);
      if (!extractedBinary) {
        throw new Error(`Extracted archive does not contain ${binaryName}`);
      }

      const dest = path.join(binDir, binaryName);
      fs.copyFileSync(extractedBinary, dest);
      if (platform !== "win32") {
        fs.chmodSync(dest, 0o755);
      }
      destinations[binaryName] = dest;
    }

    return { asset, destinations };
  } finally {
    cleanupTempDir(tempRoot);
  }
}

async function installReleaseBinary({
  repo,
  version,
  binDir,
  platform = os.platform(),
  arch = os.arch(),
}) {
  const { asset, url } = releaseAssetUrl(repo, version, platform, arch);
  const tempRoot = fs.mkdtempSync(path.join(os.tmpdir(), "gwt-release-"));
  const archivePath = path.join(tempRoot, asset);

  try {
    await download(url, archivePath);
    return installBundleFromArchive({
      archivePath,
      asset,
      binDir,
      platform,
    });
  } finally {
    cleanupTempDir(tempRoot);
  }
}

module.exports = {
  binaryNameForPlatform,
  bundleBinaryNamesForPlatform,
  daemonBinaryNameForPlatform,
  download,
  installBinaryFromArchive: installBundleFromArchive,
  installBundleFromArchive,
  installReleaseBinary,
  readReleaseContract,
  releaseAssetName,
  releaseAssetUrl,
};
