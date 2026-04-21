#!/usr/bin/env node
/**
 * Wrapper script to execute the gwt Rust binary.
 * If the binary is not found (e.g., bunx skips postinstall),
 * it will be downloaded on-demand from GitHub Releases.
 */

const { spawn } = require("child_process");
const path = require("path");
const fs = require("fs");

const {
  bundleBinaryNamesForPlatform,
  binaryNameForPlatform,
  installReleaseBinary,
  releaseAssetUrl,
} = require("../scripts/release-assets.cjs");

const REPO = "akiojin/gwt";
const BIN_DIR = __dirname;
const BIN_NAME = binaryNameForPlatform(process.platform);
const BIN_PATH = path.join(BIN_DIR, BIN_NAME);
const BUNDLE_BINARIES = bundleBinaryNamesForPlatform(process.platform);

function readVersion() {
  const pkg = path.join(__dirname, "..", "package.json");
  return JSON.parse(fs.readFileSync(pkg, "utf8")).version;
}

async function ensureBinary() {
  if (BUNDLE_BINARIES.every((name) => fs.existsSync(path.join(BIN_DIR, name)))) {
    return;
  }

  const version = readVersion();
  const { url } = releaseAssetUrl(REPO, version, process.platform, process.arch);

  console.log(`Downloading gwt bundle for ${process.platform}-${process.arch}...`);
  console.log(`Downloading from: ${url}`);

  await installReleaseBinary({
    repo: REPO,
    version,
    binDir: BIN_DIR,
    platform: process.platform,
    arch: process.arch,
  });

  console.log(`gwt bundle installed successfully: ${BUNDLE_BINARIES.join(", ")}`);
}

async function main() {
  try {
    await ensureBinary();
  } catch (err) {
    console.error(`Failed to download gwt binary: ${err.message}`);
    console.error(`https://github.com/${REPO}/releases`);
    process.exit(1);
  }

  const child = spawn(BIN_PATH, process.argv.slice(2), {
    stdio: "inherit",
    env: process.env,
  });

  child.on("error", (err) => {
    console.error(`Failed to start gwt: ${err.message}`);
    process.exit(1);
  });

  child.on("exit", (code, signal) => {
    if (signal) {
      process.kill(process.pid, signal);
    } else {
      process.exit(code ?? 0);
    }
  });
}

if (require.main === module) {
  main();
}

module.exports = {
  ensureBinary,
  main,
  readVersion,
};
