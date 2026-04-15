#!/usr/bin/env node
"use strict";

const os = require("os");
const fs = require("fs");
const path = require("path");
const https = require("https");

const REPO = "akiojin/gwt";
const BIN_DIR = path.join(__dirname, "..", "bin");
const BINARY_NAME = os.platform() === "win32" ? "gwt.exe" : "gwt";

/**
 * Detect the GitHub Release asset name for the current platform/arch.
 * Matches the naming convention: gwt-{os}-{arch}[.exe]
 */
function releaseAssetName() {
  const platform = os.platform();
  const arch = os.arch();

  if (platform === "darwin" && arch === "arm64") {
    return "gwt-macos-aarch64";
  }
  if (platform === "darwin" && arch === "x64") {
    return "gwt-macos-x86_64";
  }
  if (platform === "linux" && arch === "x64") {
    return "gwt-linux-x86_64";
  }
  if (platform === "linux" && arch === "arm64") {
    return "gwt-linux-aarch64";
  }
  if (platform === "win32" && arch === "x64") {
    return "gwt-windows-x86_64.exe";
  }

  throw new Error(`Unsupported platform: ${platform}-${arch}`);
}

/**
 * Read the target version from package.json.
 */
function readVersion() {
  const pkg = path.join(__dirname, "..", "package.json");
  const data = JSON.parse(fs.readFileSync(pkg, "utf8"));
  return data.version;
}

/**
 * Follow redirects and download a URL to a local path.
 */
function download(url, dest) {
  return new Promise((resolve, reject) => {
    const file = fs.createWriteStream(dest);
    const request = (u) => {
      https
        .get(u, { headers: { "User-Agent": "gwt-postinstall" } }, (res) => {
          if (res.statusCode >= 300 && res.statusCode < 400 && res.headers.location) {
            request(res.headers.location);
            return;
          }
          if (res.statusCode !== 200) {
            file.close();
            fs.unlink(dest, () => {});
            reject(new Error(`Download failed: HTTP ${res.statusCode} for ${u}`));
            return;
          }
          res.pipe(file);
          file.on("finish", () => {
            file.close(resolve);
          });
        })
        .on("error", (err) => {
          file.close();
          fs.unlink(dest, () => {});
          reject(err);
        });
    };
    request(url);
  });
}

async function main() {
  // Skip in CI or when running from source
  if (process.env.CI) {
    console.log("gwt: skipping binary download in CI");
    return;
  }

  const version = readVersion();
  const asset = releaseAssetName();
  const tag = `v${version}`;
  const binaryUrl = `https://github.com/${REPO}/releases/download/${tag}/${asset}`;

  fs.mkdirSync(BIN_DIR, { recursive: true });

  const dest = path.join(BIN_DIR, BINARY_NAME);

  console.log(`Downloading gwt binary for ${os.platform()}-${os.arch()}...`);
  console.log(`Downloading from: ${binaryUrl}`);

  try {
    await download(binaryUrl, dest);
    if (os.platform() !== "win32") {
      fs.chmodSync(dest, 0o755);
    }
    console.log(`gwt binary installed successfully!`);
  } catch (err) {
    console.error(`gwt: failed to download binary - ${err.message}`);
    console.error("gwt: you can build from source with: cargo build --release -p gwt");
    process.exitCode = 0; // non-fatal so npm install does not fail
  }
}

main();
