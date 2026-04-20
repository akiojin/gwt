#!/usr/bin/env node
"use strict";

const fs = require("fs");
const path = require("path");

const {
  binaryNameForPlatform,
  installReleaseBinary,
  releaseAssetUrl,
} = require("./release-assets.cjs");

const REPO = "akiojin/gwt";
const BIN_DIR = path.join(__dirname, "..", "bin");
const BINARY_NAME = binaryNameForPlatform();

function readVersion() {
  const pkg = path.join(__dirname, "..", "package.json");
  const data = JSON.parse(fs.readFileSync(pkg, "utf8"));
  return data.version;
}

async function main() {
  if (process.env.CI) {
    console.log("gwt: skipping binary download in CI");
    return;
  }

  const version = readVersion();
  const { url } = releaseAssetUrl(REPO, version);

  console.log(`Downloading gwt binary for ${process.platform}-${process.arch}...`);
  console.log(`Downloading from: ${url}`);

  try {
    await installReleaseBinary({
      repo: REPO,
      version,
      binDir: BIN_DIR,
      binaryName: BINARY_NAME,
    });
    console.log("gwt binary installed successfully!");
  } catch (err) {
    console.error(`gwt: failed to download binary - ${err.message}`);
    console.error("gwt: you can build from source with: cargo build --release -p gwt");
    process.exitCode = 0;
  }
}

if (require.main === module) {
  main();
}

module.exports = {
  main,
  readVersion,
};
