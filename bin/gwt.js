#!/usr/bin/env node
/**
 * Wrapper script to execute the gwt Rust binary.
 * If the binary is not found (e.g., bunx skips postinstall),
 * it will be downloaded on-demand from GitHub Releases.
 */

const { spawn } = require("child_process");
const path = require("path");
const fs = require("fs");
const https = require("https");
const os = require("os");

const REPO = "akiojin/gwt";
const BIN_DIR = __dirname;
const BIN_NAME = process.platform === "win32" ? "gwt-tui.exe" : "gwt-tui";
const BIN_PATH = path.join(BIN_DIR, BIN_NAME);

function releaseAssetName() {
  const platform = os.platform();
  const arch = os.arch();

  if (platform === "darwin" && arch === "arm64") return "gwt-macos-aarch64";
  if (platform === "darwin" && arch === "x64") return "gwt-macos-x86_64";
  if (platform === "linux" && arch === "x64") return "gwt-linux-x86_64";
  if (platform === "linux" && arch === "arm64") return "gwt-linux-aarch64";
  if (platform === "win32" && arch === "x64") return "gwt-windows-x86_64.exe";

  console.error(`Unsupported platform: ${platform}-${arch}`);
  process.exit(1);
}

function readVersion() {
  const pkg = path.join(__dirname, "..", "package.json");
  return JSON.parse(fs.readFileSync(pkg, "utf8")).version;
}

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
            reject(new Error(`HTTP ${res.statusCode} for ${u}`));
            return;
          }
          res.pipe(file);
          file.on("finish", () => file.close(resolve));
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

async function ensureBinary() {
  if (fs.existsSync(BIN_PATH)) return;

  const version = readVersion();
  const asset = releaseAssetName();
  const tag = `v${version}`;
  const url = `https://github.com/${REPO}/releases/download/${tag}/${asset}`;

  console.log(`Downloading gwt binary for ${os.platform()}-${os.arch()}...`);
  console.log(`Downloading from: ${url}`);

  fs.mkdirSync(BIN_DIR, { recursive: true });
  await download(url, BIN_PATH);

  if (os.platform() !== "win32") {
    fs.chmodSync(BIN_PATH, 0o755);
  }

  console.log("gwt binary installed successfully!");
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

main();
